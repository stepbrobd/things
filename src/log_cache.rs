//! the sync cache: the server's history appended to a journal one item per
//! line, a cursor naming the history the journal holds and how much of it the
//! two agree on, and the state folded from the journal so far
//!
//! every reader and writer holds the directory's lock, the journal is synced
//! to disk before the cursor claims its bytes, and a journal the cursor cannot
//! vouch for is repaired or fetched again rather than folded as it is

use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    hash::{DefaultHasher, Hash, Hasher as _},
    io::{BufRead, BufReader, ErrorKind, Read, Seek, SeekFrom, Write},
    path::Path,
};

use anyhow::{Context, Result, anyhow};
use crc32fast::Hasher;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

use crate::{
    client::{ThingsCloudClient, now_timestamp},
    dirs::create_private_dir,
    store::{RawState, fold_item},
    wire::wire_object::WireItem,
};

const LOG_FILE: &str = "things.log";
const CURSOR_FILE: &str = "cursor.json";
const STATE_CACHE_FILE: &str = "state_cache.json";
const LOCK_FILE: &str = "lock";
const STATE_CACHE_VERSION: u8 = 4;

/// which history the journal holds and how much of it the cursor vouches for
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
struct CursorData {
    next_start_index: i64,
    history_key: String,
    #[serde(default)]
    head_index: i64,
    /// the journal length these indices acknowledge, absent on cursors written before the field existed
    #[serde(default)]
    log_offset: Option<u64>,
    #[serde(default)]
    updated_at: Option<f64>,
}

impl CursorData {
    fn same_position(&self, other: &Self) -> bool {
        self.next_start_index == other.next_start_index
            && self.history_key == other.history_key
            && self.head_index == other.head_index
            && self.log_offset == other.log_offset
    }
}

/// the state folded from the first `log_offset` bytes of the journal, whose crc32 is `checksum`, with the hash of every line folded so a line the journal repeats is folded once
#[derive(Debug, Clone, Deserialize, Default)]
struct StateCacheData {
    #[serde(default)]
    version: u8,
    log_offset: u64,
    #[serde(default)]
    checksum: u32,
    #[serde(default)]
    lines: Vec<u64>,
    state: RawState,
}

#[derive(Serialize)]
struct StateCacheRef<'a> {
    version: u8,
    log_offset: u64,
    checksum: u32,
    lines: &'a [u64],
    state: &'a RawState,
}

/// the identity of a journal line, the journal was seen to repeat a line and folding a note delta twice would corrupt the note
fn line_hash(line: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    line.hash(&mut hasher);
    hasher.finish()
}

/// held by every reader and writer of the cache directory: two processes never interleave appends, cursor moves and state cache writes
struct CacheLock {
    _file: File,
}

fn lock_cache(cache_dir: &Path) -> Result<CacheLock> {
    create_private_dir(cache_dir)
        .with_context(|| format!("failed to create {}", cache_dir.display()))?;
    let path = cache_dir.join(LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    file.lock()
        .with_context(|| format!("failed to lock {}", path.display()))?;
    Ok(CacheLock { _file: file })
}

fn read_cursor(cache_dir: &Path) -> CursorData {
    fs::read_to_string(cache_dir.join(CURSOR_FILE))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// write through a staging file synced to disk and renamed into place: a crash leaves the old file or the whole new one
fn write_durable(path: &Path, payload: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file =
        File::create(&tmp).with_context(|| format!("failed to write {}", tmp.display()))?;
    file.write_all(payload)?;
    file.sync_all()?;
    fs::rename(&tmp, path).with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

fn write_cursor(cache_dir: &Path, cursor: &CursorData) -> Result<()> {
    write_durable(
        &cache_dir.join(CURSOR_FILE),
        serde_json::to_string(cursor)?.as_bytes(),
    )
}

fn read_state_cache(cache_dir: &Path) -> Option<StateCacheData> {
    let raw = fs::read_to_string(cache_dir.join(STATE_CACHE_FILE)).ok()?;
    let cache: StateCacheData = serde_json::from_str(&raw).ok()?;
    (cache.version == STATE_CACHE_VERSION).then_some(cache)
}

fn write_state_cache(
    cache_dir: &Path,
    state: &RawState,
    log_offset: u64,
    checksum: u32,
    lines: &[u64],
) -> Result<()> {
    let payload = serde_json::to_string(&StateCacheRef {
        version: STATE_CACHE_VERSION,
        log_offset,
        checksum,
        lines,
        state,
    })?;
    write_durable(&cache_dir.join(STATE_CACHE_FILE), payload.as_bytes())
}

/// the crc32 of the first `len` bytes of the journal
fn prefix_checksum(log_path: &Path, len: u64) -> Result<u32> {
    let file =
        File::open(log_path).with_context(|| format!("failed to open {}", log_path.display()))?;
    let mut reader = BufReader::new(file).take(len);
    let mut hasher = Hasher::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize())
}

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove {}", path.display())),
    }
}

/// drop the journal, the cursor and the folded state together, the next fetch starts at the first item
fn reset_cache(cache_dir: &Path) -> Result<()> {
    for name in [LOG_FILE, CURSOR_FILE, STATE_CACHE_FILE] {
        remove_if_present(&cache_dir.join(name))?;
    }
    Ok(())
}

/// the cursor for `history_key`: the stored one when it names that history, otherwise the cache holds another account's history or one nobody vouches for, and it starts over
fn cursor_for_history(cache_dir: &Path, history_key: &str) -> Result<CursorData> {
    let cursor = read_cursor(cache_dir);
    if cursor.history_key == history_key {
        return Ok(cursor);
    }
    if !cursor.history_key.is_empty() || cache_dir.join(LOG_FILE).exists() {
        eprintln!(
            "The sync cache is not bound to this account's history, fetching the history from the start"
        );
    }
    reset_cache(cache_dir)?;
    let cursor = CursorData {
        history_key: history_key.to_string(),
        log_offset: Some(0),
        updated_at: Some(now_timestamp()),
        ..Default::default()
    };
    write_cursor(cache_dir, &cursor)?;
    Ok(cursor)
}

/// the byte length of the journal up to and including its last newline
fn complete_length(log_path: &Path) -> Result<u64> {
    let bytes =
        fs::read(log_path).with_context(|| format!("failed to read {}", log_path.display()))?;
    Ok(bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at as u64 + 1))
}

fn truncate_log(log_path: &Path, len: u64) -> Result<()> {
    OpenOptions::new()
        .write(true)
        .open(log_path)
        .with_context(|| format!("failed to open {}", log_path.display()))?
        .set_len(len)
        .with_context(|| format!("failed to truncate {}", log_path.display()))?;
    Ok(())
}

/// make the journal and the cursor agree before appending
///
/// bytes past the acknowledged length are an interrupted run's, complete or
/// not, and get fetched again. a journal shorter than the cursor claims, or
/// missing, is not trusted and gets fetched from the start. a cursor from
/// before the acknowledged length existed adopts the complete lines and keeps
/// its item index, which the server handed out: the journal was seen to hold
/// repeated lines, so its line count says nothing about that index, and a
/// page fetched twice is folded once. a folded state past the acknowledged
/// bytes is dropped with them
fn repair_log(cache_dir: &Path, cursor: &mut CursorData) -> Result<()> {
    let log_path = cache_dir.join(LOG_FILE);
    let length = match fs::metadata(&log_path) {
        Ok(meta) => meta.len(),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if cursor.next_start_index > 0 {
                remove_if_present(&cache_dir.join(STATE_CACHE_FILE))?;
                cursor.next_start_index = 0;
            }
            cursor.log_offset = Some(0);
            return Ok(());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", log_path.display()));
        }
    };
    let acknowledged = match cursor.log_offset {
        Some(offset) if offset <= length => offset,
        Some(_) => {
            cursor.next_start_index = 0;
            0
        }
        None => complete_length(&log_path)?,
    };
    if acknowledged < length {
        truncate_log(&log_path, acknowledged)?;
    }
    cursor.log_offset = Some(acknowledged);
    if read_state_cache(cache_dir).is_some_and(|cache| cache.log_offset > acknowledged) {
        remove_if_present(&cache_dir.join(STATE_CACHE_FILE))?;
    }
    Ok(())
}

/// append what the server holds past the cursor, authenticating first to bind the journal to the account behind the credentials
fn sync_locked(client: &mut ThingsCloudClient, cache_dir: &Path) -> Result<()> {
    // what the disk holds now, so a repair alone is persisted even when the server has nothing new
    let stored = read_cursor(cache_dir);
    let history_key = client.authenticate()?;
    let mut cursor = cursor_for_history(cache_dir, &history_key)?;
    repair_log(cache_dir, &mut cursor)?;

    let log_path = cache_dir.join(LOG_FILE);
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("failed to open {}", log_path.display()))?;

    loop {
        let page = client.get_items_page(cursor.next_start_index)?;
        let items = page
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let end = page
            .get("end-total-content-size")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let latest = page
            .get("latest-total-content-size")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        client.head_index = page
            .get("current-item-index")
            .and_then(Value::as_i64)
            .unwrap_or(client.head_index);

        if !items.is_empty() {
            let mut lines = String::new();
            for item in &items {
                lines.push_str(&serde_json::to_string(item)?);
                lines.push('\n');
            }
            log.write_all(lines.as_bytes())?;
            // the data reaches disk before the cursor claims it
            log.sync_all()?;
            cursor.next_start_index += items.len() as i64;
            cursor.log_offset = Some(log.metadata()?.len());
            cursor.head_index = client.head_index;
            cursor.updated_at = Some(now_timestamp());
            write_cursor(cache_dir, &cursor)?;
        }

        if items.is_empty() || end >= latest {
            break;
        }
    }

    cursor.head_index = client.head_index;
    if !cursor.same_position(&stored) {
        cursor.updated_at = Some(now_timestamp());
        write_cursor(cache_dir, &cursor)?;
    }
    Ok(())
}

/// fold the journal past the cached state
///
/// a cached state is used only when the journal still starts with the bytes
/// it was folded from, checked by length and checksum. a journal that was
/// replaced or cut is therefore folded again from its first line, and an
/// incomplete last line waits for the run that completes it
fn fold_locked(cache_dir: &Path) -> Result<RawState> {
    let log_path = cache_dir.join(LOG_FILE);
    let length = match fs::metadata(&log_path) {
        Ok(meta) => meta.len(),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(RawState::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", log_path.display()));
        }
    };

    let cache = match read_state_cache(cache_dir).filter(|cache| cache.log_offset <= length) {
        Some(cache) if prefix_checksum(&log_path, cache.log_offset)? == cache.checksum => {
            Some(cache)
        }
        _ => None,
    };
    let stale = cache.is_none() && cache_dir.join(STATE_CACHE_FILE).exists();
    let (mut state, byte_offset, mut hasher, mut lines) = cache.map_or_else(
        || (RawState::new(), 0, Hasher::new(), Vec::new()),
        |cache| {
            (
                cache.state,
                cache.log_offset,
                Hasher::new_with_initial(cache.checksum),
                cache.lines,
            )
        },
    );
    let mut seen: HashSet<u64> = lines.iter().copied().collect();
    let mut new_lines = 0u64;

    let mut file =
        File::open(&log_path).with_context(|| format!("failed to open {}", log_path.display()))?;
    file.seek(SeekFrom::Start(byte_offset))?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut safe_offset = byte_offset;

    loop {
        let entry_offset = reader.stream_position()?;
        line.clear();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            break;
        }

        if !line.ends_with('\n') {
            break;
        }
        hasher.update(line.as_bytes());

        let stripped = line.trim();
        if stripped.is_empty() {
            safe_offset = reader.stream_position()?;
            continue;
        }
        let hash = line_hash(stripped);
        if !seen.insert(hash) {
            warn!(target: "things::replay", byte = entry_offset, "a repeated journal line, folded once");
            safe_offset = reader.stream_position()?;
            continue;
        }
        lines.push(hash);
        let item: WireItem = serde_json::from_str(stripped).map_err(|error| {
            anyhow!(
                "Corrupt log entry at {} byte {}: {}",
                log_path.display(),
                entry_offset,
                error
            )
        })?;
        fold_item(item, &mut state);
        new_lines += 1;
        safe_offset = reader.stream_position()?;
    }

    if new_lines > 0 || stale {
        write_state_cache(cache_dir, &state, safe_offset, hasher.finalize(), &lines)?;
    }

    Ok(state)
}

/// synchronize the journal with the server and fold it, under the cache lock
pub fn get_state_with_append_log(
    client: &mut ThingsCloudClient,
    cache_dir: &Path,
) -> Result<RawState> {
    let _lock = lock_cache(cache_dir)?;
    sync_locked(client, cache_dir)?;
    fold_locked(cache_dir)
}

/// the state folded from the journal on disk, without touching the server
pub fn fold_state_from_append_log(cache_dir: &Path) -> Result<RawState> {
    let _lock = lock_cache(cache_dir)?;
    fold_locked(cache_dir)
}

#[cfg(test)]
mod tests {
    use std::fs::TryLockError;

    use super::*;

    const TASK_ID: &str = "A7h5eCi24RvAWKC3Hv3muf";
    const SETTINGS_ONE: &str =
        r#"{"3C6BBD49-8D11-4FFF-8B0E-B8F33FA9C00A":{"t":0,"e":"Settings5","p":{}}}"#;
    const SETTINGS_TWO: &str =
        r#"{"4C6BBD49-8D11-4FFF-8B0E-B8F33FA9C00B":{"t":0,"e":"Settings5","p":{}}}"#;

    fn seed_log(cache_dir: &Path, content: &str) {
        fs::write(cache_dir.join(LOG_FILE), content).expect("seed log");
    }

    fn log_content(cache_dir: &Path) -> String {
        fs::read_to_string(cache_dir.join(LOG_FILE)).expect("log")
    }

    #[test]
    fn state_cache_version_change_refolds_the_append_log() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let cache_dir = temp_dir.path();
        let task_id = TASK_ID;
        let log = format!(
            "{{\"{task_id}\":{{\"t\":0,\"e\":\"Task6\",\"p\":{{\"tt\":\"Current task\",\"ss\":0}}}}}}\n\
             {{\"{task_id}\":{{\"t\":1,\"e\":\"Task7\",\"p\":{{\"md\":2.0}}}}}}\n"
        );
        seed_log(cache_dir, &log);
        fs::write(
            cache_dir.join(STATE_CACHE_FILE),
            format!(
                "{{\"version\":{},\"log_offset\":{},\"state\":{{}}}}",
                STATE_CACHE_VERSION - 1,
                log.len()
            ),
        )
        .expect("seed stale state cache");

        let state = fold_state_from_append_log(cache_dir).expect("refold stale cache");

        assert!(state.contains_key(&task_id.parse().expect("task id")));
        let store = crate::store::ThingsStore::from_raw_state(&state);
        let task = store.get_task(task_id).expect("Task7 task after refold");
        assert_eq!(task.title, "Current task");
        assert_eq!(task.entity, crate::wire::wire_object::EntityType::Task7);
        let cache = read_state_cache(cache_dir).expect("rewritten cache");
        assert_eq!(cache.state, state);
        assert_eq!(cache.log_offset, log.len() as u64);
    }

    #[test]
    fn fold_state_accepts_legacy_action_group_ids() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let cache_dir = temp_dir.path();
        let action_group_id = "ACTIONGROUP-11111111-2222-4333-8444-555555555555";
        let task_id = "3C6BBD49-8D11-4FFF-8B0E-B8F33FA9C00A";
        let log = format!(
            r#"{{"{action_group_id}":{{"t":0,"e":"Task3","p":{{"tt":"Heading","ss":0,"tp":2,"st":1}}}},"{task_id}":{{"t":0,"e":"Task3","p":{{"tt":"Legacy child","ss":0,"tp":0,"st":1,"agr":["{action_group_id}"]}}}}}}"#
        ) + "\n";
        seed_log(cache_dir, &log);

        let state = fold_state_from_append_log(cache_dir).expect("fold legacy action-group IDs");
        let store = crate::store::ThingsStore::from_raw_state(&state);
        let task = store.get_task(task_id).expect("legacy child task");

        assert_eq!(
            task.action_group,
            Some(action_group_id.parse().expect("action-group ID"))
        );
    }

    #[test]
    fn fold_state_reports_the_offset_and_parse_error() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let cache_dir = temp_dir.path();
        seed_log(cache_dir, "{not-json}\n");

        let error = fold_state_from_append_log(cache_dir)
            .expect_err("corrupt log must fail")
            .to_string();

        assert!(error.contains("byte 0"));
        assert!(error.contains("key must be a string"));
    }

    #[test]
    fn fold_state_ignores_trailing_partial_line() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let cache_dir = temp_dir.path();
        let split_at = SETTINGS_TWO.len() / 2;
        seed_log(
            cache_dir,
            &format!("{}\n{}", SETTINGS_ONE, &SETTINGS_TWO[..split_at]),
        );

        let first_state = fold_state_from_append_log(cache_dir).expect("first fold");
        assert_eq!(first_state.len(), 1);
        let first_offset = read_state_cache(cache_dir).expect("cache").log_offset;
        assert_eq!(first_offset, (SETTINGS_ONE.len() + 1) as u64);

        let mut fp = OpenOptions::new()
            .append(true)
            .open(cache_dir.join(LOG_FILE))
            .expect("open log for append");
        writeln!(fp, "{}", &SETTINGS_TWO[split_at..]).expect("append line remainder");

        let second_state = fold_state_from_append_log(cache_dir).expect("second fold");
        assert_eq!(second_state.len(), 2);

        let expected_offset = fs::metadata(cache_dir.join(LOG_FILE))
            .expect("log metadata")
            .len();
        let second_offset = read_state_cache(cache_dir).expect("cache").log_offset;
        assert_eq!(second_offset, expected_offset);
    }

    #[test]
    fn fold_state_drops_a_cached_state_past_the_end_of_the_journal() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let cache_dir = temp_dir.path();
        seed_log(cache_dir, &format!("{SETTINGS_ONE}\n"));
        assert_eq!(
            fold_state_from_append_log(cache_dir).expect("fold").len(),
            1
        );

        // the journal is cut behind the cache's back
        seed_log(cache_dir, "");
        assert!(
            fold_state_from_append_log(cache_dir)
                .expect("refold")
                .is_empty()
        );
        assert_eq!(read_state_cache(cache_dir).expect("cache").log_offset, 0);

        // and replaced by a shorter one with another object
        seed_log(cache_dir, &format!("{SETTINGS_ONE}\n"));
        assert_eq!(
            fold_state_from_append_log(cache_dir).expect("fold").len(),
            1
        );
        seed_log(cache_dir, &format!("{SETTINGS_TWO}\n"));
        let state = fold_state_from_append_log(cache_dir).expect("refold");
        assert_eq!(state.len(), 1);
        assert!(state.contains_key(&"4C6BBD49-8D11-4FFF-8B0E-B8F33FA9C00B".parse().expect("id")));
    }

    #[test]
    fn a_repeated_journal_line_is_folded_once_even_across_folds() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let cache_dir = temp_dir.path();
        let checksum = crc32fast::hash("done".as_bytes());
        let create = format!(
            r#"{{"{TASK_ID}":{{"t":0,"e":"Task6","p":{{"tt":"Notes","tp":0,"ss":0,"st":1,"nt":{{"_t":"tx","t":1,"ch":0,"v":"todo"}}}}}}}}"#
        );
        let delta = format!(
            r#"{{"{TASK_ID}":{{"t":1,"e":"Task7","p":{{"nt":{{"_t":"tx","t":2,"ps":[{{"p":0,"l":4,"r":"done","ch":{checksum}}}]}}}}}}}}"#
        );
        // the delta twice in a row, then once more after a fold in between
        seed_log(cache_dir, &format!("{create}\n{delta}\n{delta}\n"));
        let state = fold_state_from_append_log(cache_dir).expect("fold");
        let task = crate::store::ThingsStore::from_raw_state(&state)
            .get_task(TASK_ID)
            .expect("task");
        assert_eq!(task.notes.as_deref(), Some("done"));
        assert!(!task.degraded);
        assert_eq!(read_state_cache(cache_dir).expect("cache").lines.len(), 2);

        let mut fp = OpenOptions::new()
            .append(true)
            .open(cache_dir.join(LOG_FILE))
            .expect("append");
        writeln!(fp, "{delta}").expect("append the delta again");
        let state = fold_state_from_append_log(cache_dir).expect("fold");
        let task = crate::store::ThingsStore::from_raw_state(&state)
            .get_task(TASK_ID)
            .expect("task");
        assert_eq!(task.notes.as_deref(), Some("done"));
        assert!(!task.degraded);
    }

    #[test]
    fn repair_cuts_what_the_cursor_does_not_acknowledge() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let cache_dir = temp_dir.path();
        let acknowledged = format!("{SETTINGS_ONE}\n");
        // a complete line and a partial one from an interrupted run
        seed_log(
            cache_dir,
            &format!("{acknowledged}{SETTINGS_TWO}\n{}", &SETTINGS_TWO[..10]),
        );
        // folded before the interruption was noticed
        write_state_cache(
            cache_dir,
            &RawState::new(),
            (acknowledged.len() + SETTINGS_TWO.len() + 1) as u64,
            0,
            &[],
        )
        .expect("seed cache");
        let mut cursor = CursorData {
            next_start_index: 1,
            history_key: "h".to_string(),
            head_index: 1,
            log_offset: Some(acknowledged.len() as u64),
            updated_at: None,
        };

        repair_log(cache_dir, &mut cursor).expect("repair");

        assert_eq!(log_content(cache_dir), acknowledged);
        assert_eq!(cursor.next_start_index, 1);
        assert_eq!(cursor.log_offset, Some(acknowledged.len() as u64));
        assert!(read_state_cache(cache_dir).is_none());
    }

    #[test]
    fn repair_adopts_the_complete_lines_for_a_cursor_without_an_offset() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let cache_dir = temp_dir.path();
        let complete = format!("{SETTINGS_ONE}\n{SETTINGS_TWO}\n");
        seed_log(cache_dir, &format!("{complete}{}", &SETTINGS_ONE[..7]));
        let mut cursor = CursorData {
            next_start_index: 2,
            history_key: "h".to_string(),
            head_index: 2,
            log_offset: None,
            updated_at: None,
        };

        repair_log(cache_dir, &mut cursor).expect("repair");

        assert_eq!(log_content(cache_dir), complete);
        assert_eq!(cursor.next_start_index, 2);
        assert_eq!(cursor.log_offset, Some(complete.len() as u64));
    }

    #[test]
    fn repair_starts_over_when_the_journal_shrank_or_vanished() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let cache_dir = temp_dir.path();
        seed_log(cache_dir, &format!("{SETTINGS_ONE}\n"));
        write_state_cache(cache_dir, &RawState::new(), 500, 0, &[]).expect("seed cache");
        let mut cursor = CursorData {
            next_start_index: 7,
            history_key: "h".to_string(),
            head_index: 7,
            log_offset: Some(500),
            updated_at: None,
        };

        repair_log(cache_dir, &mut cursor).expect("repair");
        assert_eq!(log_content(cache_dir), "");
        assert_eq!(cursor.next_start_index, 0);
        assert_eq!(cursor.log_offset, Some(0));
        assert!(read_state_cache(cache_dir).is_none());

        fs::remove_file(cache_dir.join(LOG_FILE)).expect("remove log");
        write_state_cache(cache_dir, &RawState::new(), 0, 0, &[]).expect("seed cache");
        let mut cursor = CursorData {
            next_start_index: 7,
            history_key: "h".to_string(),
            head_index: 7,
            log_offset: Some(0),
            updated_at: None,
        };
        repair_log(cache_dir, &mut cursor).expect("repair");
        assert_eq!(cursor.next_start_index, 0);
        assert!(read_state_cache(cache_dir).is_none());
    }

    #[test]
    fn another_history_resets_the_whole_cache() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let cache_dir = temp_dir.path();
        seed_log(cache_dir, &format!("{SETTINGS_ONE}\n"));
        write_state_cache(cache_dir, &RawState::new(), 9, 0, &[]).expect("seed cache");
        let stored = CursorData {
            next_start_index: 1,
            history_key: "old".to_string(),
            head_index: 1,
            log_offset: Some(9),
            updated_at: None,
        };
        write_cursor(cache_dir, &stored).expect("seed cursor");

        let same = cursor_for_history(cache_dir, "old").expect("same history");
        assert_eq!(same, stored);
        assert!(cache_dir.join(LOG_FILE).exists());

        let fresh = cursor_for_history(cache_dir, "new").expect("other history");
        assert_eq!(fresh.history_key, "new");
        assert_eq!(fresh.next_start_index, 0);
        assert_eq!(fresh.log_offset, Some(0));
        assert!(!cache_dir.join(LOG_FILE).exists());
        assert!(read_state_cache(cache_dir).is_none());
        assert_eq!(read_cursor(cache_dir).history_key, "new");
    }

    #[test]
    fn the_cache_lock_excludes_a_second_holder() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let cache_dir = temp_dir.path();
        let held = lock_cache(cache_dir).expect("lock");
        let other = File::open(cache_dir.join(LOCK_FILE)).expect("lock file");
        assert!(matches!(other.try_lock(), Err(TryLockError::WouldBlock)));
        drop(held);
        other.try_lock().expect("free after release");
    }
}
