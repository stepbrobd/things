use std::{collections::HashSet, fmt::Write as _, io::Write as _};

use chrono::{DateTime, Local, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use crc32fast::Hasher;

use crate::{
    ids::ThingsId,
    store::{Tag, Task, ThingsStore},
    wire::{
        notes::{StructuredTaskNotes, TaskNotes},
        task::TaskType,
    },
};

/// today, the local calendar day at UTC midnight
pub fn today_utc() -> DateTime<Utc> {
    local_date_as_utc_midnight(Local::now())
}

fn local_date_as_utc_midnight<Tz: TimeZone>(now: DateTime<Tz>) -> DateTime<Utc> {
    let today = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
    Utc.from_utc_datetime(&today)
}

/// the current wall-clock time in fractional unix seconds
pub fn now_ts_f64() -> f64 {
    Utc::now().timestamp_millis() as f64 / 1000.0
}

pub const RESET: &str = "\x1b[0m";
pub const DIM: &str = "\x1b[2m";
pub const GREEN: &str = "\x1b[32m";

pub struct Icons {
    // sidebar and view icons
    pub inbox: &'static str,
    pub today: &'static str,
    pub upcoming: &'static str,
    pub anytime: &'static str,
    pub find: &'static str,

    // task and grouping icons
    pub task_open: &'static str,
    pub task_done: &'static str,
    pub task_someday: &'static str,
    pub task_canceled: &'static str,
    pub today_staged: &'static str,
    pub project: &'static str,
    pub project_someday: &'static str,
    pub area: &'static str,
    pub tag: &'static str,
    pub evening: &'static str,
    pub repeat: &'static str,

    // project progress icons
    pub progress_empty: &'static str,
    pub progress_quarter: &'static str,
    pub progress_half: &'static str,
    pub progress_three_quarter: &'static str,
    pub progress_full: &'static str,

    // status and event icons
    pub deadline: &'static str,
    pub done: &'static str,
    pub incomplete: &'static str,
    pub canceled: &'static str,
    pub deleted: &'static str,

    // checklist icons
    pub checklist_open: &'static str,
    pub checklist_done: &'static str,
    pub checklist_canceled: &'static str,
}

pub const ICONS: Icons = Icons {
    inbox: "⬓",
    today: "⭑",
    upcoming: "▷",
    anytime: "≋",
    find: "⌕",

    task_open: "▢",
    task_done: "◼",
    task_someday: "⬚",
    task_canceled: "☒",
    today_staged: "●",
    project: "●",
    project_someday: "◌",
    area: "◆",
    tag: "⌗",
    evening: "☽",
    repeat: "↻",

    progress_empty: "◯",
    progress_quarter: "◔",
    progress_half: "◑",
    progress_three_quarter: "◕",
    progress_full: "◉",

    deadline: "⚑",
    done: "✓",
    incomplete: "↺",
    canceled: "☒",
    deleted: "×",

    checklist_open: "○",
    checklist_done: "●",
    checklist_canceled: "×",
};

/// a line of diagnostics on stderr
///
/// a reader that closed stderr takes the line with it
/// no other channel is left to report that
pub fn eprint_line(text: &str) {
    let _ = writeln!(std::io::stderr(), "{text}");
}

/// a count with its noun, singular for one
pub fn counted(count: usize, noun: &str) -> String {
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {noun}{plural}")
}

/// NO_COLOR with any value but the empty string asks for output without color
pub fn no_color_requested() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
}

pub fn colored<T: ToString>(text: T, codes: &[&str], no_color: bool) -> String {
    let text = text.to_string();
    if no_color {
        return text;
    }
    let mut out = String::new();
    for code in codes {
        out.push_str(code);
    }
    out.push_str(&text);
    out.push_str(RESET);
    out
}

pub fn fmt_date(dt: Option<DateTime<Utc>>) -> String {
    dt.map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// the local calendar day of an instant, with the offset in force at that instant rather than today's
pub fn fmt_date_local(dt: Option<DateTime<Utc>>) -> String {
    dt.map(|d| d.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// a calendar day as the wire stores it, the unix timestamp of 00:00 UTC on that day
pub fn day_timestamp(day: NaiveDate) -> i64 {
    day.and_hms_opt(0, 0, 0)
        .expect("midnight")
        .and_utc()
        .timestamp()
}

/// the calendar day a wire day timestamp names
pub fn day_of(timestamp: i64) -> Option<NaiveDate> {
    DateTime::from_timestamp(timestamp, 0).map(|day| day.date_naive())
}

/// a YYYY-MM-DD flag as the calendar day it names
///
/// no time zone takes part
/// the day goes on the wire at UTC midnight through `day_timestamp`
/// the local offset of today or of that day cannot move it to the day before
pub fn parse_day(day: &str, label: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map_err(|_| format!("Invalid {label} date: {day} (expected YYYY-MM-DD)"))
}

/// an RFC 3339 instant, or a YYYY-MM-DD day taken at local midnight, the instant that day began where the command runs, as a wire timestamp
///
/// an instant, unlike a day stamp, is shown under its local day
/// a day given for one has to fall there
pub fn parse_instant(text: &str, label: &str) -> Result<f64, String> {
    if let Ok(instant) = DateTime::parse_from_rfc3339(text) {
        return Ok(instant.timestamp_millis() as f64 / 1000.0);
    }
    let day = parse_day(text, label)
        .map_err(|_| format!("Invalid {label}: {text} (expected RFC 3339 or YYYY-MM-DD)"))?;
    Local
        .from_local_datetime(&day.and_hms_opt(0, 0, 0).expect("midnight"))
        .earliest()
        .map(|instant| instant.timestamp() as f64)
        .ok_or_else(|| format!("Invalid {label}: {text} has no midnight in the local time zone"))
}

pub fn parse_reminder(time: &str) -> Result<i64, String> {
    NaiveTime::parse_from_str(time, "%H:%M")
        .map(|t| i64::from(t.num_seconds_from_midnight()))
        .map_err(|_| format!("Invalid --reminder time: {time} (expected HH:MM)"))
}

/// how `sanitized` shows the text it is given
#[derive(PartialEq)]
enum Shown {
    /// the output on a terminal that shows colors
    ///
    /// the CLI's own SGR sequences pass
    Colored,
    /// text without colors of its own
    ///
    /// errors and output off a terminal
    Plain,
    /// JSON
    ///
    /// its escapes decode to the same text
    Json,
    /// one field of cloud text
    ///
    /// kept on its line
    Field,
}

/// the finished output on a terminal that shows colors
///
/// the CLI's own SGR colors pass
/// other C0 controls and DEL show in caret notation
/// C1 and the bidi controls show as U+FFFD
/// titles and notes come from the cloud
/// anyone who can mail a to-do into the Inbox writes them there
/// the text they add passes `one_line` before it gets here
/// that keeps their escape sequences out
pub fn printable(text: &str) -> String {
    sanitized(text, Shown::Colored)
}

/// errors and output without colors
///
/// no escape sequence passes
/// the CLI adds none
pub fn printable_plain(text: &str) -> String {
    sanitized(text, Shown::Plain)
}

/// JSON for the terminal
///
/// the controls `serde_json` writes raw leave as `\u` escapes that decode to the same text
pub fn printable_json(text: &str) -> String {
    sanitized(text, Shown::Json)
}

/// one field of cloud text, a title for instance, for a line of terminal output
///
/// every control character shows escaped, line breaks and escape sequences included
pub fn one_line(text: &str) -> String {
    sanitized(text, Shown::Field)
}

/// a title as the lists show it, "(untitled)" when it holds nothing
pub fn shown_title(title: &str) -> String {
    if title.trim().is_empty() {
        "(untitled)".to_string()
    } else {
        one_line(title)
    }
}

/// a note from the cloud for the terminal
///
/// each line passes `one_line`
/// the layout then breaks the note at its own line breaks alone
pub fn note_lines(text: &str) -> String {
    text.lines().map(one_line).collect::<Vec<_>>().join("\n")
}

fn sanitized(text: &str, shown: Shown) -> String {
    let mut out = String::with_capacity(text.len());
    let mut skip_to = 0;
    for (at, c) in text.char_indices() {
        if at < skip_to {
            continue;
        }
        match c {
            '\n' | '\t' if shown != Shown::Field => out.push(c),
            '\u{1b}' if matches!(shown, Shown::Colored | Shown::Json) => match sgr(&text[at..]) {
                Some(sequence) => {
                    out.push_str(sequence);
                    skip_to = at + sequence.len();
                }
                None => out.push_str("^["),
            },
            // serde_json escapes C0 alone
            // DEL, C1 and the bidi controls stand raw in its strings
            c if shown == Shown::Json && (c == '\u{7f}' || replaced(c)) => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            '\u{7f}' => out.push_str("^?"),
            c if c.is_ascii_control() => {
                out.push('^');
                out.push(char::from(c as u8 + 0x40));
            }
            c if replaced(c) => out.push('\u{fffd}'),
            // the line and paragraph separators end a row in the layout
            '\u{2028}' | '\u{2029}' if shown == Shown::Field => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    out
}

/// C1, and the embedding, override and isolate controls that reorder the text around them on a terminal that applies bidi
fn replaced(c: char) -> bool {
    matches!(c, '\u{80}'..='\u{9f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// the select graphic rendition sequence at the start of `text`
fn sgr(text: &str) -> Option<&str> {
    let params = text.strip_prefix("\u{1b}[")?;
    let len = params.find(|c: char| !c.is_ascii_digit() && c != ';')?;
    params[len..].starts_with('m').then(|| &text[..len + 3])
}

pub fn task6_note(value: &str) -> TaskNotes {
    let mut hasher = Hasher::new();
    hasher.update(value.as_bytes());
    let checksum = hasher.finalize();
    TaskNotes::Structured(StructuredTaskNotes {
        object_type: Some("tx".to_string()),
        format_type: 1,
        ch: Some(checksum),
        v: Some(value.to_string()),
        ps: Vec::new(),
        unknown_fields: Default::default(),
    })
}

/// the project or area a to-do goes into
pub enum Container {
    Project(ThingsId),
    Area(ThingsId),
}

/// an item named by its kind with its article
///
/// a kind this CLI does not know is named by its number, as `show` names it
pub fn kind_with_article(task: &Task) -> String {
    match task.item_type {
        TaskType::Todo => "a to-do".to_string(),
        TaskType::Project => "a project".to_string(),
        TaskType::Heading => "a heading".to_string(),
        TaskType::Unknown(raw) => format!("an item of unknown kind {raw}"),
    }
}

/// the project or area `target` names for a to-do, or why a to-do cannot go there
///
/// `flag` names the option in messages
pub fn resolve_container(
    store: &ThingsStore,
    target: &str,
    flag: &str,
) -> Result<Container, String> {
    let (item, _, item_candidates) = store.resolve_task_identifier(target);
    let (area, _, area_candidates) = store.resolve_area_identifier(target);
    if !item_candidates.is_empty() {
        return Err(format!(
            "Ambiguous {flag} target '{target}' (matches several items)."
        ));
    }
    if !area_candidates.is_empty() {
        return Err(format!(
            "Ambiguous {flag} target '{target}' (matches several areas)."
        ));
    }
    let project = match (item, area) {
        (Some(_), Some(_)) => {
            return Err(format!(
                "Ambiguous {flag} target '{target}' (matches an item and an area)."
            ));
        }
        // an area that did not replay completely may not be what it shows
        (None, Some(area)) if area.degraded => {
            return Err(format!(
                "Container did not replay completely: {}",
                shown_title(&area.title)
            ));
        }
        (None, Some(area)) => return Ok(Container::Area(area.uuid)),
        (None, None) => return Err(format!("Container not found: {target}")),
        (Some(project), None) if project.is_project() => project,
        (Some(item), None) => {
            return Err(format!(
                "Container is {}: {}",
                kind_with_article(&item),
                shown_title(&item.title)
            ));
        }
    };
    // a project of a kind the writes do not know may be closed or in the Trash without showing it
    if !project.entity.can_upgrade_to_task7() {
        return Err(format!(
            "Container is of kind {}: {}",
            project.entity,
            shown_title(&project.title)
        ));
    }
    // one that did not replay completely may be as well
    if project.degraded {
        return Err(format!(
            "Container did not replay completely: {}",
            shown_title(&project.title)
        ));
    }
    // a closed project lists no incomplete to-do
    // one placed there would show nowhere
    if let Some(state) = store.closed_state(&project) {
        return Err(format!(
            "Container is {state}: {}",
            shown_title(&project.title)
        ));
    }
    // the template of a repeating project shows in no list
    // a to-do placed there would show nowhere either
    if project.is_recurrence_template() {
        return Err(format!(
            "Container is a repeat template: {}",
            shown_title(&project.title)
        ));
    }
    Ok(Container::Project(project.uuid))
}

pub fn resolve_single_tag(store: &ThingsStore, identifier: &str) -> (Option<Tag>, String) {
    // the whole argument names one tag, whatever commas its title holds
    let all_tags: Vec<Tag> = store.tags_by_uuid.values().cloned().collect();
    match resolve_single_tag_id(&all_tags, identifier) {
        Ok(id) => (store.tags_by_uuid.get(&id).cloned(), String::new()),
        Err(err) => (None, err),
    }
}

pub fn resolve_tag_ids(store: &ThingsStore, raw_tags: &str) -> (Vec<ThingsId>, String) {
    resolve_tags(store, raw_tags, false)
}

/// the tags a removal names
///
/// a tag deleted before its carriers were cleared leaves its id on them
/// the full id of such a tag still names it while something carries it
pub fn resolve_removable_tag_ids(store: &ThingsStore, raw_tags: &str) -> (Vec<ThingsId>, String) {
    resolve_tags(store, raw_tags, true)
}

fn resolve_tags(store: &ThingsStore, raw_tags: &str, removable: bool) -> (Vec<ThingsId>, String) {
    let tokens = raw_tags
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return (Vec::new(), String::new());
    }

    // a tag of a blank title still resolves by its id
    let all_tags: Vec<Tag> = store.tags_by_uuid.values().cloned().collect();
    let mut resolved = Vec::new();
    let mut seen = HashSet::new();

    for token in tokens {
        let tag_uuid = match resolve_single_tag_id(&all_tags, token) {
            Ok(tag_uuid) => tag_uuid,
            Err(err) => match token.parse::<ThingsId>() {
                Ok(id) if removable && store.tag_is_carried(&id) => id,
                _ => return (Vec::new(), err),
            },
        };
        // a tag that did not replay completely may not be what it shows
        // taking one off an item stays allowed
        if !removable
            && store
                .tags_by_uuid
                .get(&tag_uuid)
                .is_some_and(|tag| tag.degraded)
        {
            return (
                Vec::new(),
                format!(
                    "Tag did not replay completely: {}",
                    one_line(&store.resolve_tag_title(&tag_uuid))
                ),
            );
        }
        if seen.insert(tag_uuid.clone()) {
            resolved.push(tag_uuid);
        }
    }

    (resolved, String::new())
}

fn resolve_single_tag_id(tags: &[Tag], token: &str) -> Result<ThingsId, String> {
    let exact = tags
        .iter()
        .filter(|tag| tag.title.trim().to_lowercase() == token.to_lowercase())
        .map(|tag| tag.uuid.clone())
        .collect::<Vec<_>>();
    if exact.len() == 1 {
        return Ok(exact[0].clone());
    }
    if exact.len() > 1 {
        return Err(format!("Ambiguous tag title: {token}"));
    }

    let prefix = tags
        .iter()
        .filter(|tag| tag.uuid.starts_with(token))
        .map(|tag| tag.uuid.clone())
        .collect::<Vec<_>>();
    if prefix.len() == 1 {
        return Ok(prefix[0].clone());
    }
    if prefix.len() > 1 {
        return Err(format!("Ambiguous tag ID prefix: {token}"));
    }

    Err(format!("Tag not found: {token}"))
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, Local, NaiveTime, TimeZone, Utc};

    use super::{
        day_of, day_timestamp, local_date_as_utc_midnight, note_lines, one_line, parse_day,
        parse_instant, printable, printable_json, printable_plain,
    };

    #[test]
    fn printable_keeps_colors_and_escapes_other_controls() {
        assert_eq!(
            printable("\u{1b}[1;32m✓ Done\u{1b}[0m\tnote\n"),
            "\u{1b}[1;32m✓ Done\u{1b}[0m\tnote\n"
        );
        assert_eq!(
            printable("Pay rent\u{1b}]52;c;aGk=\u{7}\u{1b}[2J\rX\u{7f}\u{9b}"),
            "Pay rent^[]52;c;aGk=^G^[[2J^MX^?\u{fffd}"
        );
        assert_eq!(
            printable("Invoice \u{202e}fdp.exe\u{202c} \u{2067}x\u{2069}"),
            "Invoice \u{fffd}fdp.exe\u{fffd} \u{fffd}x\u{fffd}"
        );
    }

    #[test]
    fn output_without_colors_passes_no_escape_sequence() {
        assert_eq!(
            printable_plain("A\u{1b}[8mB\u{1b}[0m\tnote\r\nnext"),
            "A^[[8mB^[[0m\tnote^M\nnext"
        );
    }

    #[test]
    fn a_note_keeps_its_own_lines_and_escapes_the_rest() {
        assert_eq!(
            note_lines("bring photos\rand the passport\nlandlord\u{2028}IBAN\u{85}lease"),
            "bring photos^Mand the passport\nlandlord\u{fffd}IBAN\u{fffd}lease"
        );
        // an ESC before a sequence that a stripper removes would join what follows
        assert_eq!(
            note_lines("plan \u{1b}\u{1b}[31m[8mconcealed"),
            "plan ^[^[[31m[8mconcealed"
        );
    }

    #[test]
    fn a_field_stays_on_its_line_without_escape_sequences() {
        assert_eq!(
            one_line("Renew\nKind: to-do, done\r\u{1b}[8m\t\u{85}\u{2028}\u{202e}"),
            "Renew^JKind: to-do, done^M^[[8m^I\u{fffd}\u{fffd}\u{fffd}"
        );
    }

    #[test]
    fn printable_json_keeps_every_string_as_it_decodes() {
        let title = "Rent\u{7f} due\u{85}\u{9b}2J \u{1b}]52;c;aGk=\u{7} \u{202e}fdp.exe";
        let json = serde_json::to_string(&serde_json::json!({ "title": title })).unwrap();
        let shown = printable_json(&json);
        assert!(
            !shown.chars().any(|c| c.is_control() || c == '\u{202e}'),
            "{shown}"
        );
        let read: serde_json::Value = serde_json::from_str(&shown).unwrap();
        assert_eq!(read["title"], title);
    }

    #[test]
    fn today_uses_the_local_date_after_utc_midnight() {
        let eastern = FixedOffset::west_opt(4 * 60 * 60).unwrap();
        let local_now = eastern.with_ymd_and_hms(2026, 8, 27, 21, 23, 0).unwrap();

        assert_eq!(
            local_date_as_utc_midnight(local_now),
            Utc.with_ymd_and_hms(2026, 8, 27, 0, 0, 0).unwrap()
        );
    }

    #[test]
    fn a_day_flag_goes_on_the_wire_at_utc_midnight_in_winter_and_summer() {
        // the values the app writes for these days, whatever offset the process runs under today and whatever offset holds on the day itself
        let wire = |text: &str| parse_day(text, "--when").map(day_timestamp);
        assert_eq!(wire("2027-01-15"), Ok(1_799_971_200));
        assert_eq!(wire("2027-01-20"), Ok(1_800_403_200));
        assert_eq!(wire("2027-07-15"), Ok(1_815_609_600));
        assert!(parse_day("2027-13-01", "--when").is_err());
    }

    #[test]
    fn a_day_given_for_an_instant_falls_inside_that_local_day() {
        let instant = parse_instant("2027-07-15", "--completed-on").expect("instant");
        let local = Local
            .timestamp_opt(instant as i64, 0)
            .single()
            .expect("an instant");
        assert_eq!(local.date_naive(), day_of(1_815_609_600).expect("day"));
        assert_eq!(local.time(), NaiveTime::MIN);
        assert_eq!(
            parse_instant("2027-07-15T10:20:30Z", "--completed-on"),
            Ok(1_815_646_830.0)
        );
        assert!(parse_instant("July", "--completed-on").is_err());
    }
}
