use std::{collections::BTreeMap, str::FromStr};

use std::collections::BTreeMap as ChangeMap;

use chrono::{Datelike, Days, Months, NaiveDate};
use serde_json::{Value, json};

use crate::{
    common::{day_of, day_timestamp, parse_day, task6_note},
    ids::ThingsId,
    store::{Task, ThingsStore},
    wire::{
        notes::TaskNotes,
        recurrence::{FrequencyUnit, RECURRENCE_END_NEVER, RecurrenceRule, RecurrenceType},
        task::{TaskPatch, TaskProps, TaskStart, TaskStatus, TaskType},
        wire_object::{EntityType, WireObject},
    },
};

/// A repeat rule as typed on the command line: `daily`, `weekly:mon,thu`,
/// `monthly:15`, `monthly:last`, `yearly:12-31`, `after:2w`, with `/N` on the
/// fixed cadences for every N units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepeatSpec {
    pub cadence: Cadence,
    pub every: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cadence {
    Daily,
    /// weekdays counted from Sunday as 0, empty for the first day's weekday
    Weekly(Vec<u32>),
    /// 1 to 31, -1 for the last day, None for the first day's day
    Monthly(Option<i32>),
    /// month and day, None for the first day's month and day
    Yearly(Option<(u32, u32)>),
    AfterCompletion(FrequencyUnit),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    Never,
    Until(NaiveDate),
    Times(i32),
}

const WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

impl FromStr for RepeatSpec {
    type Err = String;

    fn from_str(spec: &str) -> Result<Self, String> {
        let (head, selector) = match spec.split_once(':') {
            Some((head, selector)) => (head, Some(selector)),
            None => (spec, None),
        };
        let (unit, every) = match head.split_once('/') {
            Some((unit, every)) => (
                unit,
                every
                    .parse::<i32>()
                    .ok()
                    .filter(|every| *every >= 1)
                    .ok_or_else(|| {
                        format!("Invalid --repeat {spec}: expected a positive number after /")
                    })?,
            ),
            None => (head, 1),
        };
        let cadence = match unit.to_ascii_lowercase().as_str() {
            "daily" if selector.is_none() => Cadence::Daily,
            "weekly" => Cadence::Weekly(
                selector
                    .map(parse_weekdays)
                    .transpose()?
                    .unwrap_or_default(),
            ),
            "monthly" => Cadence::Monthly(selector.map(parse_month_day).transpose()?),
            "yearly" => Cadence::Yearly(selector.map(parse_month_and_day).transpose()?),
            "after" if every == 1 => {
                let (every, unit) = parse_interval(selector.ok_or_else(|| {
                    format!("Invalid --repeat {spec}: after needs an interval such as after:2w")
                })?)?;
                return Ok(Self {
                    cadence: Cadence::AfterCompletion(unit),
                    every,
                });
            }
            _ => {
                return Err(format!(
                    "Invalid --repeat {spec}: expected daily, weekly, monthly, yearly or after"
                ));
            }
        };
        Ok(Self { cadence, every })
    }
}

fn parse_weekdays(list: &str) -> Result<Vec<u32>, String> {
    let mut days: Vec<u32> = list
        .split(',')
        .map(|name| {
            WEEKDAYS
                .iter()
                .position(|day| day.eq_ignore_ascii_case(name.trim()))
                .map(|index| index as u32)
                .ok_or_else(|| {
                    format!("Invalid weekday {name}: expected sun, mon, tue, wed, thu, fri or sat")
                })
        })
        .collect::<Result<_, _>>()?;
    days.sort_unstable();
    days.dedup();
    Ok(days)
}

fn parse_month_day(day: &str) -> Result<i32, String> {
    if day.eq_ignore_ascii_case("last") {
        return Ok(-1);
    }
    day.parse::<i32>()
        .ok()
        .filter(|day| (1..=31).contains(day))
        .ok_or_else(|| format!("Invalid day of month {day}: expected 1 to 31 or last"))
}

fn parse_month_and_day(date: &str) -> Result<(u32, u32), String> {
    let invalid = || format!("Invalid yearly date {date}: expected MM-DD");
    let (month, day) = date.split_once('-').ok_or_else(invalid)?;
    let month = month.parse::<u32>().map_err(|_| invalid())?;
    let day = day.parse::<u32>().map_err(|_| invalid())?;
    // 2024 is a leap year so february 29 passes
    NaiveDate::from_ymd_opt(2024, month, day).ok_or_else(invalid)?;
    Ok((month, day))
}

fn parse_interval(interval: &str) -> Result<(i32, FrequencyUnit), String> {
    let invalid =
        || format!("Invalid interval {interval}: expected a number and d, w, m or y, such as 2w");
    let (amount, unit) = interval.split_at(interval.len().saturating_sub(1));
    let amount = amount
        .parse::<i32>()
        .ok()
        .filter(|amount| *amount >= 1)
        .ok_or_else(invalid)?;
    let unit = match unit {
        "d" => FrequencyUnit::Daily,
        "w" => FrequencyUnit::Weekly,
        "m" => FrequencyUnit::Monthly,
        "y" => FrequencyUnit::Yearly,
        _ => return Err(invalid()),
    };
    Ok((amount, unit))
}

/// the end condition from `--times` and `--until`
pub fn bound(times: Option<i32>, until: Option<&str>) -> Result<Bound, String> {
    match (times, until) {
        (Some(_), Some(_)) => Err("--times and --until exclude each other".to_string()),
        (Some(times), None) if times >= 1 => Ok(Bound::Times(times)),
        (Some(times), None) => Err(format!("--times {times}: expected a positive number")),
        (None, Some(until)) => Ok(Bound::Until(
            parse_day(Some(until), "--until")?.expect("a day was given"),
        )),
        (None, None) => Ok(Bound::Never),
    }
}

fn weekday_index(day: NaiveDate) -> u32 {
    day.weekday().num_days_from_sunday()
}

fn week_start(day: NaiveDate) -> NaiveDate {
    day - Days::new(u64::from(weekday_index(day)))
}

/// the day of the month, -1 for the last, clamped to the month's length
fn day_in_month(year: i32, month: u32, day: i32) -> NaiveDate {
    let first_of_next = NaiveDate::from_ymd_opt(year, month, 1).expect("month") + Months::new(1);
    let last = first_of_next.pred_opt().expect("day before the first");
    if day < 0 {
        return last;
    }
    NaiveDate::from_ymd_opt(year, month, (day as u32).min(last.day())).expect("clamped day")
}

fn minus_interval(day: NaiveDate, unit: FrequencyUnit, every: i32) -> NaiveDate {
    let every = every as u32;
    match unit {
        FrequencyUnit::Daily => day - Days::new(u64::from(every)),
        FrequencyUnit::Weekly => day - Days::new(u64::from(7 * every)),
        FrequencyUnit::Monthly => day - Months::new(every),
        _ => day - Months::new(12 * every),
    }
}

fn offset(fields: &[(&str, i64)]) -> BTreeMap<String, Value> {
    fields
        .iter()
        .map(|(key, value)| ((*key).to_string(), Value::from(*value)))
        .collect()
}

impl RepeatSpec {
    /// the first occurrence on or after `from`
    pub fn first_occurrence(&self, from: NaiveDate) -> NaiveDate {
        match &self.cadence {
            Cadence::Weekly(days) if !days.is_empty() => (0..7)
                .map(|ahead| from + Days::new(ahead))
                .find(|day| days.contains(&weekday_index(*day)))
                .expect("a weekday within seven days"),
            Cadence::Monthly(Some(day)) => {
                let this_month = day_in_month(from.year(), from.month(), *day);
                if this_month >= from {
                    return this_month;
                }
                let next = from + Months::new(1);
                day_in_month(next.year(), next.month(), *day)
            }
            Cadence::Yearly(Some((month, day))) => {
                let this_year = day_in_month(from.year(), *month, *day as i32);
                if this_year >= from {
                    return this_year;
                }
                day_in_month(from.year() + 1, *month, *day as i32)
            }
            _ => from,
        }
    }

    /// the occurrence after `after`, counting intervals from `anchor`, which need not be an occurrence itself, None after completion
    pub fn next_occurrence(&self, anchor: NaiveDate, after: NaiveDate) -> Option<NaiveDate> {
        let every = i64::from(self.every);
        match &self.cadence {
            Cadence::Daily => {
                if after < anchor {
                    return Some(anchor);
                }
                let elapsed = (after - anchor).num_days();
                Some(anchor + Days::new((elapsed / every * every + every) as u64))
            }
            Cadence::Weekly(days) => {
                let days = if days.is_empty() {
                    vec![weekday_index(anchor)]
                } else {
                    days.clone()
                };
                let base = week_start(anchor);
                let start = if after < anchor {
                    anchor
                } else {
                    after + Days::new(1)
                };
                (0..=7 * every + 7)
                    .map(|ahead| start + Days::new(ahead as u64))
                    .find(|day| {
                        days.contains(&weekday_index(*day))
                            && ((week_start(*day) - base).num_days() / 7) % every == 0
                    })
            }
            Cadence::Monthly(day) => {
                let day = day.unwrap_or(anchor.day() as i32);
                (0..)
                    .map(|steps| {
                        let month = anchor + Months::new(steps * self.every as u32);
                        day_in_month(month.year(), month.month(), day)
                    })
                    .find(|candidate| *candidate > after && *candidate >= anchor)
            }
            Cadence::Yearly(month_day) => {
                let (month, day) = month_day.unwrap_or((anchor.month(), anchor.day()));
                (0..)
                    .map(|steps| {
                        day_in_month(anchor.year() + steps * self.every, month, day as i32)
                    })
                    .find(|candidate| *candidate > after && *candidate >= anchor)
            }
            Cadence::AfterCompletion(_) => None,
        }
    }

    /// the spec and anchor behind a fixed schedule rule from the wire, None for after completion or shapes the CLI cannot evaluate
    pub fn from_rule(rule: &RecurrenceRule) -> Option<(Self, NaiveDate)> {
        if rule.recurrence_type != RecurrenceType::FixedSchedule || rule.frequency_amount < 1 {
            return None;
        }
        let anchor = rule
            .interval_anchor
            .filter(|anchor| *anchor > 0)
            .or(rule.start_date)
            .and_then(day_of)?;
        let field = |key: &str| -> Vec<i64> {
            rule.offsets
                .iter()
                .filter_map(|offset| offset.get(key).and_then(Value::as_i64))
                .collect()
        };
        let cadence = match rule.frequency_unit {
            FrequencyUnit::Daily => Cadence::Daily,
            FrequencyUnit::Weekly => Cadence::Weekly(
                field("wd")
                    .into_iter()
                    .filter_map(|day| u32::try_from(day).ok())
                    .collect(),
            ),
            FrequencyUnit::Monthly => {
                // the nth weekday of a month is not evaluated
                if !field("wd").is_empty() {
                    return None;
                }
                Cadence::Monthly(
                    field("dy")
                        .first()
                        .map(|day| if *day < 0 { -1 } else { *day as i32 + 1 }),
                )
            }
            FrequencyUnit::Yearly => {
                Cadence::Yearly(match (field("mo").first(), field("dy").first()) {
                    (Some(month), Some(day)) => Some((*month as u32 + 1, *day as u32 + 1)),
                    _ => None,
                })
            }
            FrequencyUnit::Unknown(_) => return None,
        };
        Some((
            Self {
                cadence,
                every: rule.frequency_amount,
            },
            anchor,
        ))
    }

    /// the wire rule as the app writes it, `sr` the day the rule was made, `ia` the first occurrence
    pub fn rule(&self, first: NaiveDate, today: NaiveDate, bound: Bound) -> RecurrenceRule {
        let (recurrence_type, frequency_unit, offsets, interval_anchor) = match &self.cadence {
            Cadence::Daily => (
                RecurrenceType::FixedSchedule,
                FrequencyUnit::Daily,
                vec![offset(&[("dy", 0)])],
                day_timestamp(first),
            ),
            Cadence::Weekly(days) => {
                let days = if days.is_empty() {
                    vec![weekday_index(first)]
                } else {
                    days.clone()
                };
                (
                    RecurrenceType::FixedSchedule,
                    FrequencyUnit::Weekly,
                    days.iter()
                        .map(|day| offset(&[("wd", i64::from(*day))]))
                        .collect(),
                    day_timestamp(first),
                )
            }
            Cadence::Monthly(day) => {
                let day = day.unwrap_or(first.day() as i32);
                let dy = if day < 0 { -1 } else { i64::from(day) - 1 };
                (
                    RecurrenceType::FixedSchedule,
                    FrequencyUnit::Monthly,
                    vec![offset(&[("dy", dy)])],
                    day_timestamp(first),
                )
            }
            Cadence::Yearly(month_day) => {
                let (month, day) = month_day.unwrap_or((first.month(), first.day()));
                (
                    RecurrenceType::FixedSchedule,
                    FrequencyUnit::Yearly,
                    vec![offset(&[
                        ("dy", i64::from(day) - 1),
                        ("mo", i64::from(month) - 1),
                    ])],
                    day_timestamp(first),
                )
            }
            Cadence::AfterCompletion(unit) => {
                (RecurrenceType::AfterCompletion, *unit, Vec::new(), 0)
            }
        };
        RecurrenceRule {
            recurrence_type,
            frequency_unit,
            frequency_amount: self.every,
            offsets,
            start_date: Some(day_timestamp(today)),
            interval_anchor: Some(interval_anchor),
            end_date: match bound {
                Bound::Until(day) => Some(day_timestamp(day)),
                Bound::Times(_) => None,
                Bound::Never => Some(RECURRENCE_END_NEVER),
            },
            repeat_count: match bound {
                Bound::Times(times) => times,
                _ => 0,
            },
            time_span_in_days: 0,
            version: 4,
        }
    }
}

/// the next occurrence of a wire rule after `after`, honoring the end day and the repeat count against the instances made so far
pub fn next_occurrence_of_rule(
    rule: &RecurrenceRule,
    after: NaiveDate,
    instances_created: i32,
) -> Option<NaiveDate> {
    if rule.repeat_count > 0 && instances_created >= rule.repeat_count {
        return None;
    }
    let (spec, anchor) = RepeatSpec::from_rule(rule)?;
    let next = spec.next_occurrence(anchor, after)?;
    match rule.end_date {
        Some(end) if end != RECURRENCE_END_NEVER => (next <= day_of(end)?).then_some(next),
        _ => Some(next),
    }
}

/// an instance a template is due for on `day`, the shape the app writes when it creates the next copy
pub struct Materialized {
    pub title: String,
    pub day: NaiveDate,
    pub instance_id: String,
    pub changes: ChangeMap<String, WireObject>,
}

/// the instances whose day has come, for every fixed schedule template not yet served for that day
pub fn due_instances(
    store: &ThingsStore,
    today: NaiveDate,
    now: f64,
    next_id: &mut dyn FnMut() -> String,
) -> Vec<Materialized> {
    let mut templates: Vec<&Task> = store
        .tasks_by_uuid
        .values()
        .filter(|template| {
            template.is_recurrence_template()
                && !template.trashed
                && template.status == TaskStatus::Incomplete
                && !template.instance_creation_paused
        })
        .collect();
    templates.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    templates
        .into_iter()
        .filter_map(|template| {
            let rule = template.recurrence_rule.as_ref()?;
            RepeatSpec::from_rule(rule)?;
            let due = template.instance_creation_start_date.and_then(day_of)?;
            if due > today {
                return None;
            }
            if rule.repeat_count > 0 && template.instance_creation_count >= rule.repeat_count {
                return None;
            }
            let served = store.tasks_by_uuid.values().any(|task| {
                !task.trashed
                    && task.recurrence_templates.contains(&template.uuid)
                    && task.start_date.is_some_and(|day| day.date_naive() >= due)
            });
            if served {
                return None;
            }
            let created = template.instance_creation_count + 1;
            let following = next_occurrence_of_rule(rule, due.max(today), created)
                .unwrap_or_else(|| today + Days::new(1));
            let instance_id = next_id();
            let day_ts = day_timestamp(due);
            let instance = TaskProps {
                title: template.title.clone(),
                notes: template.notes.as_deref().map(task6_note),
                item_type: TaskType::Todo,
                status: TaskStatus::Incomplete,
                start_location: TaskStart::Anytime,
                scheduled_date: Some(day_ts),
                today_index_reference: Some(day_ts),
                tag_ids: template.tags.clone(),
                parent_project_ids: template.project.iter().cloned().collect(),
                area_ids: template.area.iter().cloned().collect(),
                action_group_ids: template.action_group.iter().cloned().collect(),
                sort_index: template.index,
                today_sort_index: template.today_index,
                recurrence_template_ids: vec![template.uuid.clone()],
                alarm_time_offset: template.alarm_time_offset,
                conflict_overrides: Some(json!({"_t": "oo", "sn": {}})),
                creation_date: Some(now),
                modification_date: Some(now),
                ..Default::default()
            };
            let advance = TaskPatch {
                instance_creation_count: Some(created),
                instance_creation_start_date: Some(Some(day_timestamp(following))),
                today_index_reference: Some(Some(day_timestamp(following))),
                modification_date: Some(Some(now)),
                ..Default::default()
            };
            let mut changes = ChangeMap::new();
            changes.insert(
                instance_id.clone(),
                WireObject::create(EntityType::Task7, instance),
            );
            changes.insert(
                template.uuid.to_string(),
                WireObject::update(EntityType::Task7, advance),
            );
            Some(Materialized {
                title: template.title.clone(),
                day: due,
                instance_id,
                changes,
            })
        })
        .collect()
}

/// what the template copies from the to-do that becomes its first instance
pub struct TemplateSource {
    pub title: String,
    pub notes: Option<TaskNotes>,
    pub tag_ids: Vec<ThingsId>,
    pub parent_project_ids: Vec<ThingsId>,
    pub area_ids: Vec<ThingsId>,
    pub action_group_ids: Vec<ThingsId>,
    pub alarm_time_offset: Option<i64>,
    pub sort_index: i32,
    pub today_sort_index: i32,
    pub conflict_overrides: Option<Value>,
}

/// the hidden template behind a repeating to-do whose first instance is on `first`
pub fn template(
    spec: &RepeatSpec,
    rule: RecurrenceRule,
    first: NaiveDate,
    source: TemplateSource,
    now: f64,
) -> TaskProps {
    let next = spec
        .next_occurrence(first, first)
        .unwrap_or_else(|| first + Days::new(1));
    let after_completion_reference_date = match &spec.cadence {
        Cadence::AfterCompletion(unit) => {
            Some(day_timestamp(minus_interval(first, *unit, spec.every)))
        }
        _ => None,
    };
    TaskProps {
        title: source.title,
        notes: source.notes,
        start_location: TaskStart::Someday,
        scheduled_date: None,
        today_index_reference: Some(day_timestamp(next)),
        tag_ids: source.tag_ids,
        parent_project_ids: source.parent_project_ids,
        area_ids: source.area_ids,
        action_group_ids: source.action_group_ids,
        sort_index: source.sort_index,
        today_sort_index: source.today_sort_index,
        recurrence_rule: Some(rule),
        instance_creation_start_date: Some(day_timestamp(next)),
        instance_creation_count: 1,
        instance_creation_paused: false,
        after_completion_reference_date,
        alarm_time_offset: source.alarm_time_offset,
        conflict_overrides: source.conflict_overrides,
        creation_date: Some(now),
        modification_date: Some(now),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("date")
    }

    fn spec(text: &str) -> RepeatSpec {
        text.parse().expect("spec")
    }

    #[test]
    fn parses_every_form() {
        assert_eq!(
            spec("daily/3"),
            RepeatSpec {
                cadence: Cadence::Daily,
                every: 3
            }
        );
        assert_eq!(
            spec("weekly:thu,mon"),
            RepeatSpec {
                cadence: Cadence::Weekly(vec![1, 4]),
                every: 1
            }
        );
        assert_eq!(
            spec("monthly:last"),
            RepeatSpec {
                cadence: Cadence::Monthly(Some(-1)),
                every: 1
            }
        );
        assert_eq!(
            spec("yearly/2:02-29"),
            RepeatSpec {
                cadence: Cadence::Yearly(Some((2, 29))),
                every: 2
            }
        );
        assert_eq!(
            spec("after:2w"),
            RepeatSpec {
                cadence: Cadence::AfterCompletion(FrequencyUnit::Weekly),
                every: 2
            }
        );
        assert!("daily:mon".parse::<RepeatSpec>().is_err());
        assert!("monthly:32".parse::<RepeatSpec>().is_err());
        assert!("after".parse::<RepeatSpec>().is_err());
    }

    #[test]
    fn first_occurrence_moves_to_the_selector() {
        // 2026-09-17 is a thursday
        assert_eq!(
            spec("weekly:mon,thu").first_occurrence(day("2026-09-17")),
            day("2026-09-17")
        );
        assert_eq!(
            spec("weekly:mon").first_occurrence(day("2026-09-17")),
            day("2026-09-21")
        );
        assert_eq!(
            spec("monthly:15").first_occurrence(day("2026-09-17")),
            day("2026-10-15")
        );
        assert_eq!(
            spec("monthly:last").first_occurrence(day("2026-09-17")),
            day("2026-09-30")
        );
        assert_eq!(
            spec("yearly:12-31").first_occurrence(day("2026-09-17")),
            day("2026-12-31")
        );
        assert_eq!(
            spec("yearly:01-05").first_occurrence(day("2026-09-17")),
            day("2027-01-05")
        );
        assert_eq!(
            spec("daily").first_occurrence(day("2026-09-17")),
            day("2026-09-17")
        );
    }

    #[test]
    fn next_occurrence_counts_from_the_anchor() {
        let thursday = day("2026-09-17");
        assert_eq!(
            spec("daily/3").next_occurrence(thursday, thursday),
            Some(day("2026-09-20"))
        );
        assert_eq!(
            spec("weekly:mon,thu").next_occurrence(thursday, thursday),
            Some(day("2026-09-21"))
        );
        assert_eq!(
            spec("weekly/2").next_occurrence(thursday, thursday),
            Some(day("2026-10-01"))
        );
        assert_eq!(
            spec("weekly/2:mon,thu").next_occurrence(thursday, thursday),
            Some(day("2026-09-28"))
        );
        assert_eq!(
            spec("monthly:31").next_occurrence(day("2026-01-31"), day("2026-01-31")),
            Some(day("2026-02-28"))
        );
        assert_eq!(
            spec("monthly:last").next_occurrence(day("2026-09-30"), day("2026-09-30")),
            Some(day("2026-10-31"))
        );
        assert_eq!(
            spec("yearly:02-29").next_occurrence(day("2024-02-29"), day("2024-02-29")),
            Some(day("2025-02-28"))
        );
        assert_eq!(spec("after:2w").next_occurrence(thursday, thursday), None);
    }

    #[test]
    fn wire_rules_evaluate_like_the_app() {
        let today = day("2026-09-17");
        // the app's monthly rule anchored on its creation day, first occurrence october 15
        let monthly: RecurrenceRule = serde_json::from_str(
            r#"{"fa":1,"fu":8,"ia":1789603200,"of":[{"dy":14}],"rc":3,"rrv":4,"sr":1789603200,"tp":0,"ts":0}"#,
        )
        .expect("rule");
        assert_eq!(
            next_occurrence_of_rule(&monthly, today - Days::new(1), 0),
            Some(day("2026-10-15"))
        );
        assert_eq!(
            next_occurrence_of_rule(&monthly, day("2026-10-15"), 3),
            None
        );
        // yearly ending on its own day
        let yearly: RecurrenceRule = serde_json::from_str(
            r#"{"ed":1821139200,"fa":1,"fu":4,"ia":1789603200,"of":[{"dy":16,"mo":8}],"rc":0,"rrv":4,"sr":1789603200,"tp":0,"ts":0}"#,
        )
        .expect("rule");
        assert_eq!(
            next_occurrence_of_rule(&yearly, today, 1),
            Some(day("2027-09-17"))
        );
        assert_eq!(next_occurrence_of_rule(&yearly, day("2027-09-17"), 2), None);
        // after completion is never projected
        let after: RecurrenceRule = serde_json::from_str(
            r#"{"ed":64092211200,"fa":2,"fu":256,"ia":0,"of":[],"rc":0,"rrv":4,"sr":1789603200,"tp":1,"ts":0}"#,
        )
        .expect("rule");
        assert_eq!(next_occurrence_of_rule(&after, today, 0), None);
    }

    #[test]
    fn rule_matches_the_captured_shapes() {
        let today = day("2026-09-17");
        let weekly = spec("weekly:mon,thu").rule(today, today, Bound::Never);
        assert_eq!(
            weekly.offsets,
            vec![offset(&[("wd", 1)]), offset(&[("wd", 4)])]
        );
        assert_eq!(weekly.end_date, Some(RECURRENCE_END_NEVER));
        assert_eq!(weekly.interval_anchor, Some(1789603200));

        let monthly = spec("monthly:15").rule(day("2026-10-15"), today, Bound::Times(3));
        assert_eq!(monthly.offsets, vec![offset(&[("dy", 14)])]);
        assert_eq!(monthly.end_date, None);
        assert_eq!(monthly.repeat_count, 3);

        let yearly = spec("yearly:09-17").rule(today, today, Bound::Until(day("2027-09-17")));
        assert_eq!(yearly.offsets, vec![offset(&[("dy", 16), ("mo", 8)])]);
        assert_eq!(yearly.end_date, Some(1821139200));

        let last = spec("monthly:last").rule(day("2026-09-30"), today, Bound::Never);
        assert_eq!(last.offsets, vec![offset(&[("dy", -1)])]);

        let after = spec("after:2w").rule(day("2027-12-31"), today, Bound::Never);
        assert_eq!(after.recurrence_type, RecurrenceType::AfterCompletion);
        assert_eq!(after.interval_anchor, Some(0));
        assert!(after.offsets.is_empty());
    }

    #[test]
    fn template_points_at_the_following_occurrence() {
        let today = day("2026-09-17");
        let source = || TemplateSource {
            title: "rp".to_string(),
            notes: None,
            tag_ids: Vec::new(),
            parent_project_ids: Vec::new(),
            area_ids: Vec::new(),
            action_group_ids: Vec::new(),
            alarm_time_offset: Some(32400),
            sort_index: -844,
            today_sort_index: -520,
            conflict_overrides: None,
        };
        let weekly = spec("weekly:mon,thu");
        let made = template(
            &weekly,
            weekly.rule(today, today, Bound::Never),
            today,
            source(),
            1.0,
        );
        assert_eq!(
            made.instance_creation_start_date,
            Some(day_timestamp(day("2026-09-21")))
        );
        assert_eq!(
            made.today_index_reference,
            Some(day_timestamp(day("2026-09-21")))
        );
        assert_eq!(made.instance_creation_count, 1);
        assert_eq!(made.start_location, TaskStart::Someday);
        assert_eq!(made.alarm_time_offset, Some(32400));

        let after = spec("after:2w");
        let first = day("2027-12-31");
        let made = template(
            &after,
            after.rule(first, today, Bound::Never),
            first,
            source(),
            1.0,
        );
        assert_eq!(
            made.after_completion_reference_date,
            Some(day_timestamp(day("2027-12-17")))
        );
        assert_eq!(
            made.instance_creation_start_date,
            Some(day_timestamp(day("2028-01-01")))
        );
    }
}
