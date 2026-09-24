use std::sync::Arc;

use anyhow::{Result, bail};
use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Utc};
use clap::{ArgGroup, Args};
use iocraft::prelude::*;

use crate::{
    app::Cli,
    arg_types::IdentifierToken,
    commands::{Command, DetailedArgs, detailed_json_conflict, write_json},
    common::resolve_single_tag,
    ids::ThingsId,
    store::{Task, ThingsStore},
    ui::{
        render_element_to_string,
        views::{
            find::{FindRow, FindView},
            json::common::build_tasks_json,
        },
    },
    wire::task::{TaskStart, TaskStatus},
};

#[derive(Debug, Clone, Copy)]
struct MatchResult {
    matched: bool,
    checklist_only: bool,
}

impl MatchResult {
    fn no() -> Self {
        Self {
            matched: false,
            checklist_only: false,
        }
    }

    fn yes(checklist_only: bool) -> Self {
        Self {
            matched: true,
            checklist_only,
        }
    }
}

/// a container filter as typed and lowercased once
///
/// it matches an id prefix or a title substring
struct ContainerFilter {
    token: String,
    lowered: String,
}

impl ContainerFilter {
    fn new(filter: &IdentifierToken) -> Self {
        Self {
            token: filter.as_str().to_string(),
            lowered: filter.as_str().to_lowercase(),
        }
    }

    fn matches(&self, uuid: &str, title_lower: &str) -> bool {
        uuid.starts_with(&self.token) || title_lower.contains(&self.lowered)
    }
}

/// what the filters need
///
/// computed once for the whole search rather than per task
struct Prepared {
    query: Option<String>,
    statuses: Option<Vec<TaskStatus>>,
    project_filters: Vec<ContainerFilter>,
    area_filters: Vec<ContainerFilter>,
    deadline: Vec<(&'static str, DateTime<Utc>)>,
    scheduled: Vec<(&'static str, DateTime<Utc>)>,
    created: Vec<(&'static str, DateTime<Utc>)>,
    completed_on: Vec<(&'static str, DateTime<Utc>)>,
}

impl Prepared {
    fn new(args: &FindArgs, today: &DateTime<Utc>) -> Result<Self, String> {
        let dates = |flag: &str, exprs: &[String]| {
            exprs
                .iter()
                .map(|expr| parse_date_expr(expr, flag, today))
                .collect::<Result<Vec<_>, _>>()
        };
        Ok(Self {
            query: args.query.as_ref().map(|query| query.to_lowercase()),
            statuses: build_status_set(args),
            project_filters: args
                .project_filters
                .iter()
                .map(ContainerFilter::new)
                .collect(),
            area_filters: args.area_filters.iter().map(ContainerFilter::new).collect(),
            deadline: dates("--deadline", &args.deadline)?,
            scheduled: dates("--scheduled", &args.scheduled)?,
            created: dates("--created", &args.created)?,
            completed_on: dates("--completed-on", &args.completed_on)?,
        })
    }
}

#[derive(Debug, Default, Args)]
#[command(about = "Search and filter tasks")]
#[command(
    after_help = "Date filter syntax:  --deadline OP DATE\n  OP is one of: >  <  >=  <=  =\n  DATE is YYYY-MM-DD or a keyword: today, tomorrow, yesterday\n\n  Examples:\n    --deadline \"<today\"          overdue tasks\n    --deadline \">=2026-01-01\"    deadline on or after date\n    --created \">=2026-01-01\" --created \"<=2026-03-31\"   date range"
)]
#[command(group(ArgGroup::new("status").args(["incomplete", "completed", "canceled", "any_status"]).multiple(false)))]
#[command(group(ArgGroup::new("deadline_presence").args(["has_deadline", "no_deadline"]).multiple(false)))]
pub struct FindArgs {
    #[command(flatten)]
    pub detailed: DetailedArgs,
    #[arg(help = "Case-insensitive substring to match against task title")]
    pub query: Option<String>,
    #[arg(long, short = 'i', help = "Only incomplete tasks (default)")]
    pub incomplete: bool,
    #[arg(long, short = 'n', help = "Also search query against note text")]
    pub notes: bool,
    #[arg(
        long,
        short = 'k',
        help = "Also search query against checklist item titles, which implies --detailed for checklist-only matches"
    )]
    pub checklists: bool,
    #[arg(long, short = 'c', help = "Only completed tasks")]
    pub completed: bool,
    #[arg(long, short = 'x', help = "Only canceled tasks")]
    pub canceled: bool,
    #[arg(
        long = "any-status",
        short = 'A',
        help = "Match tasks regardless of status"
    )]
    pub any_status: bool,
    #[arg(
        long = "tag",
        short = 't',
        value_name = "TAG",
        help = "Has this tag (title or UUID prefix), repeatable with OR logic"
    )]
    tag_filters: Vec<IdentifierToken>,
    #[arg(
        long = "project",
        short = 'p',
        value_name = "PROJECT",
        help = "In this project (title substring or UUID prefix), repeatable with OR logic"
    )]
    project_filters: Vec<IdentifierToken>,
    #[arg(
        long = "area",
        short = 'a',
        value_name = "AREA",
        help = "In this area (title substring or UUID prefix), repeatable with OR logic"
    )]
    area_filters: Vec<IdentifierToken>,
    #[arg(long, short = 'I', help = "In Inbox view")]
    pub inbox: bool,
    #[arg(long, short = 'T', help = "In Today view")]
    pub today: bool,
    #[arg(long, short = 's', help = "In Someday")]
    pub someday: bool,
    #[arg(long, short = 'e', help = "Evening flag set")]
    pub evening: bool,
    #[arg(long = "has-deadline", short = 'H', help = "Has any deadline set")]
    pub has_deadline: bool,
    #[arg(long = "no-deadline", short = 'N', help = "No deadline set")]
    pub no_deadline: bool,
    #[arg(long, short = 'r', help = "Only recurring tasks")]
    pub recurring: bool,
    #[arg(long, help = "Search the Trash, of any status unless one is given")]
    pub trashed: bool,
    #[arg(
        long,
        short = 'l',
        value_name = "EXPR",
        help = "Deadline filter, e.g. '<today' or '>=2026-04-01' (repeatable for range)"
    )]
    pub deadline: Vec<String>,
    #[arg(
        long,
        short = 'S',
        value_name = "EXPR",
        help = "Scheduled start date filter (repeatable)"
    )]
    pub scheduled: Vec<String>,
    #[arg(
        long,
        short = 'C',
        value_name = "EXPR",
        help = "Creation date filter (repeatable)"
    )]
    pub created: Vec<String>,
    #[arg(
        long = "completed-on",
        short = 'o',
        value_name = "EXPR",
        help = "Completion date filter, implies --completed when no status is given (repeatable)"
    )]
    pub completed_on: Vec<String>,
}

impl Command for FindArgs {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn std::io::Write,
        ctx: &mut dyn crate::cmd_ctx::CmdCtx,
    ) -> Result<()> {
        let store = Arc::new(cli.load_store()?);
        let today = ctx.today();
        let prepared = Prepared::new(self, &today).map_err(anyhow::Error::msg)?;

        let mut resolved_tag_uuids = Vec::new();
        for tag_filter in &self.tag_filters {
            let (tag, err) = resolve_single_tag(&store, tag_filter.as_str());
            if !err.is_empty() {
                bail!("{err}");
            }
            if let Some(tag) = tag {
                resolved_tag_uuids.push(tag.uuid);
            }
        }

        let mut matched: Vec<(Task, MatchResult)> = store
            .tasks_by_uuid
            .values()
            .filter_map(|task| {
                let result = matches(task, &store, self, &resolved_tag_uuids, &prepared, &today);
                if result.matched {
                    Some((task.clone(), result))
                } else {
                    None
                }
            })
            .collect();

        matched.sort_by(|(a, _), (b, _)| {
            let a_proj = if a.is_project() { 0 } else { 1 };
            let b_proj = if b.is_project() { 0 } else { 1 };
            (a_proj, a.index, &a.uuid).cmp(&(b_proj, b.index, &b.uuid))
        });

        let json = cli.json;
        if json {
            detailed_json_conflict(json, self.detailed.detailed)?;
            let tasks = matched
                .iter()
                .map(|(task, _)| task.clone())
                .collect::<Vec<_>>();
            write_json(out, &build_tasks_json(&tasks, &store, &today))?;
            return Ok(());
        }

        let rows = matched
            .iter()
            .map(|(task, result)| FindRow {
                task,
                force_detailed: result.checklist_only,
            })
            .collect::<Vec<_>>();

        let mut ui = element! {
            ContextProvider(value: Context::owned(store.clone())) {
                ContextProvider(value: Context::owned(today)) {
                    FindView(rows, detailed: self.detailed.detailed)
                }
            }
        };
        let rendered = render_element_to_string(&mut ui, cli.no_color());
        writeln!(out, "{}", rendered)?;

        Ok(())
    }
}

fn parse_date_value(
    value: &str,
    flag: &str,
    today: &DateTime<Utc>,
) -> Result<DateTime<Utc>, String> {
    let lowered = value.trim().to_ascii_lowercase();
    match lowered.as_str() {
        "today" => Ok(*today),
        "tomorrow" => Ok(*today + Duration::days(1)),
        "yesterday" => Ok(*today - Duration::days(1)),
        _ => {
            let parsed = NaiveDate::parse_from_str(&lowered, "%Y-%m-%d").map_err(|_| {
                format!(
                    "Invalid date for {flag}: {value:?}. Expected YYYY-MM-DD, 'today', 'tomorrow', or 'yesterday'."
                )
            })?;
            let ndt = parsed.and_hms_opt(0, 0, 0).ok_or_else(|| {
                format!(
                    "Invalid date for {flag}: {value:?}. Expected YYYY-MM-DD, 'today', 'tomorrow', or 'yesterday'."
                )
            })?;
            Ok(Utc.from_utc_datetime(&ndt))
        }
    }
}

fn parse_date_expr(
    raw: &str,
    flag: &str,
    today: &DateTime<Utc>,
) -> Result<(&'static str, DateTime<Utc>), String> {
    let value = raw.trim();
    let (op, date_part) = if let Some(rest) = value.strip_prefix(">=") {
        (">=", rest)
    } else if let Some(rest) = value.strip_prefix("<=") {
        ("<=", rest)
    } else if let Some(rest) = value.strip_prefix('>') {
        (">", rest)
    } else if let Some(rest) = value.strip_prefix('<') {
        ("<", rest)
    } else if let Some(rest) = value.strip_prefix('=') {
        ("=", rest)
    } else {
        return Err(format!(
            "Invalid date expression for {flag}: {raw:?}. Expected an operator prefix: >, <, >=, <=, or =  (e.g. '<=2026-03-31')"
        ));
    };
    let date = parse_date_value(date_part, flag, today)?;
    Ok((op, date))
}

/// compare a field to a day
///
/// compare a day stamp by the day it names
/// compare an instant by its local day
/// the Logbook files it that way
fn date_matches(
    field: Option<DateTime<Utc>>,
    instant: bool,
    op: &str,
    threshold: DateTime<Utc>,
) -> bool {
    let Some(field) = field else {
        return false;
    };

    let field_day = if instant {
        field.with_timezone(&Local).date_naive()
    } else {
        field.date_naive()
    };
    let threshold_day = threshold.date_naive();

    match op {
        ">" => field_day > threshold_day,
        "<" => field_day < threshold_day,
        ">=" => field_day >= threshold_day,
        "<=" => field_day <= threshold_day,
        "=" => field_day == threshold_day,
        _ => false,
    }
}

fn build_status_set(args: &FindArgs) -> Option<Vec<TaskStatus>> {
    if args.any_status {
        return None;
    }

    let mut chosen = Vec::new();
    if args.incomplete {
        chosen.push(TaskStatus::Incomplete);
    }
    if args.completed {
        chosen.push(TaskStatus::Completed);
    }
    if args.canceled {
        chosen.push(TaskStatus::Canceled);
    }

    if chosen.is_empty() && !args.completed_on.is_empty() {
        return Some(vec![TaskStatus::Completed]);
    }
    if chosen.is_empty() {
        // the Trash holds done and canceled items as well
        // the app lists them alongside open ones
        return (!args.trashed).then(|| vec![TaskStatus::Incomplete]);
    }
    Some(chosen)
}

fn matches(
    task: &Task,
    store: &ThingsStore,
    args: &FindArgs,
    resolved_tag_uuids: &[ThingsId],
    prepared: &Prepared,
    today: &DateTime<Utc>,
) -> MatchResult {
    if task.is_heading() || store.in_trash(task) != args.trashed {
        return MatchResult::no();
    }

    if let Some(allowed_statuses) = &prepared.statuses
        && !allowed_statuses.contains(&task.status)
    {
        return MatchResult::no();
    }

    let mut checklist_only = false;
    if let Some(q) = &prepared.query {
        let title_match = task.title.to_lowercase().contains(q);
        let notes_match = args.notes
            && task
                .notes
                .as_ref()
                .map(|n| n.to_lowercase().contains(q))
                .unwrap_or(false);
        let checklist_match = args.checklists
            && task
                .checklist_items
                .iter()
                .any(|item| item.title.to_lowercase().contains(q));

        if !title_match && !notes_match && !checklist_match {
            return MatchResult::no();
        }
        checklist_only = checklist_match && !title_match && !notes_match;
    }

    if !args.tag_filters.is_empty()
        && !resolved_tag_uuids
            .iter()
            .any(|tag_uuid| task.tags.iter().any(|task_tag| task_tag == tag_uuid))
    {
        return MatchResult::no();
    }

    if !args.project_filters.is_empty() {
        let Some(project_uuid) = store.effective_project_uuid(task) else {
            return MatchResult::no();
        };
        let Some(project) = store.get_task(&project_uuid.to_string()) else {
            return MatchResult::no();
        };

        let project_title = project.title.to_lowercase();
        let project_uuid = project_uuid.to_string();
        let matched = prepared
            .project_filters
            .iter()
            .any(|filter| filter.matches(&project_uuid, &project_title));
        if !matched {
            return MatchResult::no();
        }
    }

    if !args.area_filters.is_empty() {
        let Some(area_uuid) = store.effective_area_uuid(task) else {
            return MatchResult::no();
        };
        let Some(area) = store.get_area(&area_uuid.to_string()) else {
            return MatchResult::no();
        };

        let area_title = area.title.to_lowercase();
        let area_uuid = area_uuid.to_string();
        let matched = prepared
            .area_filters
            .iter()
            .any(|filter| filter.matches(&area_uuid, &area_title));
        if !matched {
            return MatchResult::no();
        }
    }

    // no view lists what is in the Trash
    if (args.inbox || args.today || args.someday) && store.in_trash(task) {
        return MatchResult::no();
    }
    if args.inbox && task.start != TaskStart::Inbox {
        return MatchResult::no();
    }
    if args.today && !task.is_today(today) {
        return MatchResult::no();
    }
    if args.someday && !task.in_someday() {
        return MatchResult::no();
    }
    if args.evening && !task.evening {
        return MatchResult::no();
    }
    if args.has_deadline && task.deadline.is_none() {
        return MatchResult::no();
    }
    if args.no_deadline && task.deadline.is_some() {
        return MatchResult::no();
    }
    if args.recurring && task.recurrence_rule.is_none() {
        return MatchResult::no();
    }

    // deadline and scheduled day are day stamps
    // creation and completion are instants
    let date_filters = [
        (task.deadline, false, &prepared.deadline),
        (task.start_date, false, &prepared.scheduled),
        (task.creation_date, true, &prepared.created),
        (task.stop_date, true, &prepared.completed_on),
    ];
    for (field, instant, filters) in date_filters {
        for (op, threshold) in filters {
            if !date_matches(field, instant, op, *threshold) {
                return MatchResult::no();
            }
        }
    }

    MatchResult::yes(checklist_only)
}

#[cfg(test)]
mod tests {
    use chrono::{Local, NaiveDate, TimeZone, Utc};

    use super::date_matches;

    #[test]
    fn an_instant_matches_the_day_it_falls_in_locally_and_a_day_stamp_its_own_day() {
        let day = NaiveDate::from_ymd_opt(2026, 3, 25).expect("day");
        let threshold = Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0).expect("midnight"));
        // the first instant of that local day, whatever the zone
        let instant = Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).expect("midnight"))
            .earliest()
            .expect("midnight")
            .with_timezone(&Utc);
        assert!(date_matches(Some(instant), true, "=", threshold));
        assert!(date_matches(Some(instant), true, ">=", threshold));
        assert!(!date_matches(Some(instant), true, "<", threshold));
        // a day stamp is UTC midnight of its day
        assert!(date_matches(Some(threshold), false, "=", threshold));
        assert!(!date_matches(None, false, "=", threshold));
    }
}
