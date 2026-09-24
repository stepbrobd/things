//! the sync cache, the server's history appended to a journal one item per line, a cursor naming the history the journal holds and how much of it the two agree on, and the state folded from the journal so far
//!
//! every reader and writer holds the directory's lock
//! the journal is synced to disk before the cursor claims its bytes
//! a sync repairs or fetches again a journal the cursor cannot vouch for before appending to it
//! the offline fold, `--no-cloud` or a sync that failed, shows the journal as it lies, repaired or not

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
use tracing::{debug, warn};

use crate::{
    client::{HttpStatus, ThingsCloudClient},
    common::{eprint_line, now_ts_f64, printable_plain},
    dirs::create_private_dir,
    store::{RawState, fold_item},
    wire::wire_object::WireItem,
};

const LOG_FILE: &str = "things.log";
const CURSOR_FILE: &str = "cursor.json";
const STATE_CACHE_FILE: &str = "state_cache.json";
const LOCK_FILE: &str = "lock";
const STATE_CACHE_VERSION: u8 = 8;

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
    /// the email whose sign-in gave the history key, absent on cursors written before the field existed
    ///
    /// those cursors sign in once more
    #[serde(default)]
    email: Option<String>,
}

impl CursorData {
    fn same_position(&self, other: &Self) -> bool {
        self.next_start_index == other.next_start_index
            && self.history_key == other.history_key
            && self.head_index == other.head_index
            && self.log_offset == other.log_offset
    }
}

/// the state folded from the first `log_offset` bytes of the journal
///
/// the CRC32 of those bytes is `checksum`
/// the state comes with the hash of every line folded
/// the hashes make a line the journal repeats fold once
#[derive(Deserialize)]
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

/// the identity of a journal line
///
/// the journal was seen to repeat a line
/// folding a note delta twice would corrupt the note
fn line_hash(line: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    line.hash(&mut hasher);
    hasher.finish()
}

/// held by every reader and writer of the cache directory
///
/// the lock keeps two processes from interleaving appends, cursor moves and state cache writes
pub struct CacheLock {
    _file: File,
}

fn lock_cache(cache_dir: &Path) -> Result<CacheLock> {
    create_private_dir(cache_dir)?;
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

/// the stored cursor, a blank one when the file is missing, and a blank one with a warning when the file cannot be read or parsed
///
/// the history binding then treats a blank one as nobody's
fn read_cursor(cache_dir: &Path) -> CursorData {
    let path = cache_dir.join(CURSOR_FILE);
    match fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str(&raw) {
            Ok(cursor) => cursor,
            Err(error) => {
                warn!(target: "things::sync", path = %path.display(), %error, "the cursor does not parse, the cache is treated as nobody's");
                CursorData::default()
            }
        },
        Err(error) if error.kind() == ErrorKind::NotFound => CursorData::default(),
        Err(error) => {
            warn!(target: "things::sync", path = %path.display(), %error, "the cursor cannot be read, the cache is treated as nobody's");
            CursorData::default()
        }
    }
}

/// write through a staging file synced to disk and renamed into place
///
/// a crash leaves the old file or the whole new one
fn write_durable(path: &Path, payload: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    // a stale staging file, or a link planted under its name, goes first
    // create_new refuses to follow anything
    remove_if_present(&tmp)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .with_context(|| format!("failed to write {}", tmp.display()))?;
    file.write_all(payload)
        .with_context(|| format!("failed to write {}", tmp.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

fn write_cursor(cache_dir: &Path, cursor: &CursorData) -> Result<()> {
    write_durable(
        &cache_dir.join(CURSOR_FILE),
        serde_json::to_string(cursor)?.as_bytes(),
    )
}

/// the folded state on disk when it is of this version, otherwise nothing
///
/// the journal is then folded from its first line, with a warning when the file is there and cannot be used
fn read_state_cache(cache_dir: &Path) -> Option<StateCacheData> {
    let path = cache_dir.join(STATE_CACHE_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => return None,
        Err(error) => {
            warn!(target: "things::sync", path = %path.display(), %error, "the state cache cannot be read, the journal is folded from its first line");
            return None;
        }
    };
    let cache: StateCacheData = match serde_json::from_str(&raw) {
        Ok(cache) => cache,
        Err(error) => {
            warn!(target: "things::sync", path = %path.display(), %error, "the state cache does not parse, the journal is folded from its first line");
            return None;
        }
    };
    if cache.version != STATE_CACHE_VERSION {
        debug!(target: "things::sync", found = cache.version, expected = STATE_CACHE_VERSION, "the state cache is of another version, the journal is folded from its first line");
        return None;
    }
    Some(cache)
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

/// the CRC32 of the first `len` bytes of the journal
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

/// drop the journal, the cursor and the folded state together
///
/// the next fetch starts at the first item
fn reset_cache(cache_dir: &Path) -> Result<()> {
    for name in [LOG_FILE, CURSOR_FILE, STATE_CACHE_FILE] {
        remove_if_present(&cache_dir.join(name))?;
    }
    Ok(())
}

/// the cursor for `history_key`, the stored one when it names that history
///
/// otherwise the cache holds another account's history or one nobody vouches for
/// the cache then starts over
fn cursor_for_history(cache_dir: &Path, history_key: &str) -> Result<CursorData> {
    let cursor = read_cursor(cache_dir);
    if cursor.history_key == history_key {
        return Ok(cursor);
    }
    if !cursor.history_key.is_empty() || cache_dir.join(LOG_FILE).exists() {
        eprint_line(
            "The sync cache is not bound to this account's history, fetching the history from the start",
        );
    }
    reset_cache(cache_dir)?;
    let cursor = CursorData {
        history_key: history_key.to_string(),
        log_offset: Some(0),
        updated_at: Some(now_ts_f64()),
        ..Default::default()
    };
    write_cursor(cache_dir, &cursor)?;
    Ok(cursor)
}

/// the byte length of the journal up to and including its last newline, and the lines holding text within it
fn complete_lines(log_path: &Path) -> Result<(u64, i64)> {
    let bytes =
        fs::read(log_path).with_context(|| format!("failed to read {}", log_path.display()))?;
    let length = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at + 1);
    let lines = bytes[..length]
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
        .count();
    Ok((length as u64, lines as i64))
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
/// bytes past the acknowledged length are an interrupted run's, complete or not, and get fetched again
/// a journal shorter than the cursor claims, or missing, is not trusted and gets fetched from the start
/// a cursor from before the acknowledged length existed adopts the complete lines and keeps its item index
/// the server handed out that index
/// the journal was seen to hold repeated lines
/// that means its line count says nothing exact about that index
/// a page fetched twice is folded once
/// fewer lines than the index mean a tail was lost
/// the journal is then fetched from the start
/// a folded state past the acknowledged bytes is dropped with them
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
        None => {
            let (complete, lines) = complete_lines(&log_path)?;
            if lines < cursor.next_start_index {
                warn!(target: "things::sync", lines, index = cursor.next_start_index, "the journal holds fewer lines than the cursor acknowledges, it is fetched from the start");
                cursor.next_start_index = 0;
                0
            } else {
                complete
            }
        }
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

/// the history key the cursor holds for `email`
///
/// a run uses it instead of signing in again
fn stored_key_for<'a>(cursor: &'a CursorData, email: &str) -> Option<&'a str> {
    (!cursor.history_key.is_empty() && cursor.email.as_deref() == Some(email))
        .then_some(cursor.history_key.as_str())
}

/// sign in with the configured credentials
///
/// the cursor then names the history they reach and the email that reached it
fn sign_in(client: &mut ThingsCloudClient, cache_dir: &Path) -> Result<CursorData> {
    let history_key = client.authenticate()?;
    let mut cursor = cursor_for_history(cache_dir, &history_key)?;
    if cursor.email.as_deref() != Some(client.email.as_str()) {
        cursor.email = Some(client.email.clone());
        write_cursor(cache_dir, &cursor)?;
    }
    Ok(cursor)
}

/// append what the server holds past the cursor
///
/// the history key a sign-in gave stands in for the next sign-in while the configured email stays the same, as in the app
/// a run signs in when the cursor holds no key for that email, and once more when the server answers the stored key with an error
/// a sign-in that reaches another history starts the journal over
/// only an error the server answered leads to that second sign-in
fn sync_locked(client: &mut ThingsCloudClient, cache_dir: &Path) -> Result<()> {
    // what the disk holds now
    // it lets a repair alone be persisted even when the server has nothing new
    let stored = read_cursor(cache_dir);
    let reused = stored_key_for(&stored, &client.email).map(str::to_string);
    let mut cursor = match &reused {
        Some(history_key) => {
            client.history_key = Some(history_key.clone());
            stored.clone()
        }
        None => sign_in(client, cache_dir)?,
    };
    repair_log(cache_dir, &mut cursor)?;
    let mut page = match client.get_items_page(cursor.next_start_index) {
        Err(error) if reused.is_some() && error.downcast_ref::<HttpStatus>().is_some() => {
            warn!(target: "things::sync", "the stored history key was answered with an error, signing in: {}", printable_plain(&format!("{error:#}")));
            cursor = sign_in(client, cache_dir)?;
            repair_log(cache_dir, &mut cursor)?;
            client.get_items_page(cursor.next_start_index)?
        }
        page => page?,
    };

    let log_path = cache_dir.join(LOG_FILE);
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("failed to open {}", log_path.display()))?;

    loop {
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
            log.write_all(lines.as_bytes())
                .with_context(|| format!("failed to append to {}", log_path.display()))?;
            // the data reaches disk before the cursor claims it
            log.sync_all()
                .with_context(|| format!("failed to sync {}", log_path.display()))?;
            cursor.next_start_index += items.len() as i64;
            cursor.log_offset = Some(
                log.metadata()
                    .with_context(|| {
                        format!("failed to read the length of {}", log_path.display())
                    })?
                    .len(),
            );
            cursor.head_index = client.head_index;
            cursor.updated_at = Some(now_ts_f64());
            write_cursor(cache_dir, &cursor)?;
        }

        if items.is_empty() || end >= latest {
            break;
        }
        page = client.get_items_page(cursor.next_start_index)?;
    }

    cursor.head_index = client.head_index;
    if !cursor.same_position(&stored) {
        cursor.updated_at = Some(now_ts_f64());
        write_cursor(cache_dir, &cursor)?;
    }
    Ok(())
}

/// fold the journal past the cached state
///
/// a cached state is used only when the journal still starts with the bytes it was folded from
/// checked by length and checksum
/// a journal that was replaced or cut is therefore folded again from its first line
/// an incomplete last line waits for the run that completes it
/// a line whose bytes were folded before, anywhere in the journal, is skipped
/// repeated lines were seen in the journal
/// a run from before the acknowledged length existed could append a page twice
/// a later line that repeats an earlier one with another meaning would take a repeated delete of a reused id
/// no client writes such a delete
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

    // the state cache saves a later run the fold
    // one that cannot be written leaves this state standing
    if (new_lines > 0 || stale)
        && let Err(error) =
            write_state_cache(cache_dir, &state, safe_offset, hasher.finalize(), &lines)
    {
        warn!(target: "things::sync", error = %format!("{error:#}"), "the state cache cannot be written, a later run folds the journal again");
    }

    Ok(state)
}

/// synchronize the journal with the server and fold it under the cache lock
///
/// the caller holds the lock through the repeat pass's commit
/// a failed sync still folds the journal on disk
/// its error comes back beside that state
/// only a journal that the configured email's sign-in wrote is a fallback
/// a cursor from before the email was kept vouches for no account
pub fn get_state_with_append_log(
    client: &mut ThingsCloudClient,
    cache_dir: &Path,
) -> Result<(RawState, CacheLock, Option<anyhow::Error>)> {
    let lock = lock_cache(cache_dir).context("Failed to lock the sync cache")?;
    let sync_error = match sync_locked(client, cache_dir) {
        Ok(()) => None,
        Err(error) => {
            let cursor = read_cursor(cache_dir);
            let bound = cursor.email.as_deref() == Some(client.email.as_str());
            // a journal the repair emptied holds no state either
            let journal = fs::metadata(cache_dir.join(LOG_FILE)).map_or(0, |meta| meta.len());
            if !bound || journal == 0 {
                return Err(error.context(
                    "Sync failed, and no cached state is known to belong to this account",
                ));
            }
            Some(error)
        }
    };
    // a cache that cannot be read fails the run
    // the sync failure before it is named too
    let state = fold_locked(cache_dir).map_err(|error| match &sync_error {
        Some(sync) => error.context(format!(
            "Sync failed ({sync:#}), and the sync cache cannot be read"
        )),
        None => error.context("Failed to read the sync cache"),
    })?;
    Ok((state, lock, sync_error))
}

/// the state folded from the journal on disk, without touching the server
pub fn fold_state_from_append_log(cache_dir: &Path) -> Result<RawState> {
    let _lock = lock_cache(cache_dir).context("Failed to lock the sync cache")?;
    fold_locked(cache_dir).context("Failed to read the sync cache")
}

#[cfg(test)]
mod tests {
    use std::fs::TryLockError;

    use super::*;

    const TASK_ID: &str = "A7h5eCi24RvAWKC3Hv3muf";
    const SETTINGS_ONE: &str = r#"{"Se11111111111111111111":{"t":0,"e":"Settings5","p":{}}}"#;
    const SETTINGS_TWO: &str = r#"{"Se21111111111111111111":{"t":0,"e":"Settings5","p":{}}}"#;

    #[test]
    fn a_stored_key_stands_in_for_a_sign_in_for_its_own_email_alone() {
        let cursor: CursorData = serde_json::from_str(
            r#"{"next_start_index":7,"history_key":"h","email":"user@example.com"}"#,
        )
        .expect("cursor");
        assert_eq!(stored_key_for(&cursor, "user@example.com"), Some("h"));
        assert_eq!(stored_key_for(&cursor, "other@example.com"), None);
        // a cursor from before the email was kept signs in once more
        let older: CursorData =
            serde_json::from_str(r#"{"next_start_index":7,"history_key":"h"}"#).expect("cursor");
        assert_eq!(stored_key_for(&older, "user@example.com"), None);
        let keyless = CursorData {
            email: Some("user@example.com".to_string()),
            ..Default::default()
        };
        assert_eq!(stored_key_for(&keyless, "user@example.com"), None);
    }

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
    fn fold_state_reports_the_offset_and_parse_error() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let cache_dir = temp_dir.path();
        seed_log(cache_dir, "{not-json}\n");

        let error = fold_state_from_append_log(cache_dir).expect_err("corrupt log must fail");
        let error = format!("{error:#}");

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

        // it grows again with one object
        seed_log(cache_dir, &format!("{SETTINGS_ONE}\n"));
        assert_eq!(
            fold_state_from_append_log(cache_dir).expect("fold").len(),
            1
        );
        // a journal of the same length with another object replaces it
        seed_log(cache_dir, &format!("{SETTINGS_TWO}\n"));
        let state = fold_state_from_append_log(cache_dir).expect("refold");
        assert_eq!(state.len(), 1);
        assert!(state.contains_key(&"Se21111111111111111111".parse().expect("id")));
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
    fn a_state_cache_that_cannot_be_written_leaves_the_fold_standing() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let cache_dir = temp_dir.path();
        seed_log(
            cache_dir,
            &format!(
                "{{\"{TASK_ID}\":{{\"t\":0,\"e\":\"Task7\",\"p\":{{\"tt\":\"Notes\",\"tp\":0,\"ss\":0,\"st\":1}}}}}}\n"
            ),
        );
        // a directory at the staging name is no file to remove
        let staging = cache_dir.join(STATE_CACHE_FILE).with_extension("tmp");
        fs::create_dir(&staging).expect("staging dir");
        fs::write(staging.join("held"), "x").expect("hold it");

        let state = fold_state_from_append_log(cache_dir).expect("fold");
        assert!(state.contains_key(&TASK_ID.parse().expect("id")));
        assert!(!cache_dir.join(STATE_CACHE_FILE).exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_planted_at_the_staging_name_is_not_followed() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let cache_dir = temp_dir.path();
        let victim = cache_dir.join("victim");
        fs::write(&victim, "original").expect("seed victim");
        let target = cache_dir.join(STATE_CACHE_FILE);
        std::os::unix::fs::symlink(&victim, target.with_extension("tmp")).expect("plant link");

        write_durable(&target, b"{}").expect("write");

        assert_eq!(fs::read_to_string(&victim).expect("victim"), "original");
        assert_eq!(fs::read_to_string(&target).expect("target"), "{}");
        assert!(
            !fs::symlink_metadata(&target)
                .expect("metadata")
                .file_type()
                .is_symlink()
        );
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
            email: None,
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
            email: None,
        };

        repair_log(cache_dir, &mut cursor).expect("repair");

        assert_eq!(log_content(cache_dir), complete);
        assert_eq!(cursor.next_start_index, 2);
        assert_eq!(cursor.log_offset, Some(complete.len() as u64));
    }

    #[test]
    fn repair_starts_over_for_a_cursor_without_an_offset_beyond_the_journal() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let cache_dir = temp_dir.path();
        seed_log(cache_dir, &format!("{SETTINGS_ONE}\n"));
        write_state_cache(cache_dir, &RawState::new(), 60, 0, &[]).expect("seed cache");
        let mut cursor = CursorData {
            next_start_index: 7,
            history_key: "h".to_string(),
            head_index: 7,
            log_offset: None,
            updated_at: None,
            email: None,
        };

        repair_log(cache_dir, &mut cursor).expect("repair");

        assert_eq!(log_content(cache_dir), "");
        assert_eq!(cursor.next_start_index, 0);
        assert_eq!(cursor.log_offset, Some(0));
        assert!(read_state_cache(cache_dir).is_none());
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
            email: None,
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
            email: None,
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
            email: None,
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
