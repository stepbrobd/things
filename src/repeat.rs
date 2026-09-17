use std::{collections::BTreeMap, str::FromStr};

use std::collections::BTreeMap as ChangeMap;

use chrono::{Datelike, Days, Months, NaiveDate};
use serde_json::{Value, json};

use crate::{
    common::{day_of, day_timestamp, parse_day, task6_note},
    ids::ThingsId,
    store::{Task, ThingsStore},
    wire::{
        checklist::ChecklistItemProps,
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

/// the largest interval accepted from the command line or the wire, which keeps every date step representable
pub const MAX_EVERY: i32 = 999;

fn parse_every(text: &str) -> Option<i32> {
    text.parse::<i32>()
        .ok()
        .filter(|every| (1..=MAX_EVERY).contains(every))
}

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
                parse_every(every).ok_or_else(|| {
                    format!(
                        "Invalid --repeat {spec}: expected a number from 1 to {MAX_EVERY} after /"
                    )
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
    let invalid = || {
        format!(
            "Invalid interval {interval}: expected a number from 1 to {MAX_EVERY} and d, w, m or y, such as 2w"
        )
    };
    let units = [
        ("d", FrequencyUnit::Daily),
        ("w", FrequencyUnit::Weekly),
        ("m", FrequencyUnit::Monthly),
        ("y", FrequencyUnit::Yearly),
    ];
    let (amount, unit) = units
        .into_iter()
        .find_map(|(suffix, unit)| interval.strip_suffix(suffix).map(|amount| (amount, unit)))
        .ok_or_else(invalid)?;
    Ok((parse_every(amount).ok_or_else(invalid)?, unit))
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

/// the day of the month, -1 for the last, clamped to the month's length, None outside the representable years
fn day_in_month(year: i32, month: u32, day: i32) -> Option<NaiveDate> {
    let last = NaiveDate::from_ymd_opt(year, month, 1)?
        .checked_add_months(Months::new(1))?
        .pred_opt()?;
    if day < 0 {
        return Some(last);
    }
    NaiveDate::from_ymd_opt(year, month, (day as u32).min(last.day()))
}

fn minus_interval(day: NaiveDate, unit: FrequencyUnit, every: i32) -> Option<NaiveDate> {
    let every = every as u32;
    match unit {
        FrequencyUnit::Daily => day.checked_sub_days(Days::new(u64::from(every))),
        FrequencyUnit::Weekly => day.checked_sub_days(Days::new(u64::from(7 * every))),
        FrequencyUnit::Monthly => day.checked_sub_months(Months::new(every)),
        _ => day.checked_sub_months(Months::new(12 * every)),
    }
}

fn offset(fields: &[(&str, i64)]) -> BTreeMap<String, Value> {
    fields
        .iter()
        .map(|(key, value)| ((*key).to_string(), Value::from(*value)))
        .collect()
}

impl RepeatSpec {
    /// the first occurrence on or after `from`, or `from` itself when no day is representable
    pub fn first_occurrence(&self, from: NaiveDate) -> NaiveDate {
        let found = match &self.cadence {
            Cadence::Weekly(days) if !days.is_empty() => (0..7)
                .filter_map(|ahead| from.checked_add_days(Days::new(ahead)))
                .find(|day| days.contains(&weekday_index(*day))),
            Cadence::Monthly(Some(day)) => day_in_month(from.year(), from.month(), *day)
                .filter(|this_month| *this_month >= from)
                .or_else(|| {
                    let next = from.checked_add_months(Months::new(1))?;
                    day_in_month(next.year(), next.month(), *day)
                }),
            Cadence::Yearly(Some((month, day))) => day_in_month(from.year(), *month, *day as i32)
                .filter(|this_year| *this_year >= from)
                .or_else(|| day_in_month(from.year().checked_add(1)?, *month, *day as i32)),
            _ => Some(from),
        };
        found.unwrap_or(from)
    }

    /// the occurrence after `after`, counting intervals from `anchor`, which need not be an occurrence itself, None after completion or past the representable years
    pub fn next_occurrence(&self, anchor: NaiveDate, after: NaiveDate) -> Option<NaiveDate> {
        let every = i64::from(self.every);
        match &self.cadence {
            Cadence::Daily => {
                if after < anchor {
                    return Some(anchor);
                }
                let elapsed = (after - anchor).num_days();
                anchor.checked_add_days(Days::new((elapsed / every * every + every) as u64))
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
                    after.checked_add_days(Days::new(1))?
                };
                (0..=7 * every + 7)
                    .filter_map(|ahead| start.checked_add_days(Days::new(ahead as u64)))
                    .find(|day| {
                        days.contains(&weekday_index(*day))
                            && ((week_start(*day) - base).num_days() / 7) % every == 0
                    })
            }
            Cadence::Monthly(day) => {
                let day = day.unwrap_or(anchor.day() as i32);
                (0u32..)
                    .map(|steps| {
                        let month = anchor.checked_add_months(Months::new(
                            steps.checked_mul(self.every as u32)?,
                        ))?;
                        day_in_month(month.year(), month.month(), day)
                    })
                    .take_while(Option::is_some)
                    .flatten()
                    .find(|candidate| *candidate > after && *candidate >= anchor)
            }
            Cadence::Yearly(month_day) => {
                let (month, day) = month_day.unwrap_or((anchor.month(), anchor.day()));
                (0i32..)
                    .map(|steps| {
                        let year = anchor.year().checked_add(steps.checked_mul(self.every)?)?;
                        day_in_month(year, month, day as i32)
                    })
                    .take_while(Option::is_some)
                    .flatten()
                    .find(|candidate| *candidate > after && *candidate >= anchor)
            }
            Cadence::AfterCompletion(_) => None,
        }
    }

    /// the spec and anchor behind a fixed schedule rule from the wire
    ///
    /// None for after completion and for every offset shape other than the
    /// ones the app was seen to write: a rule the CLI cannot evaluate exactly
    /// is shown but never projected or materialized, since a guess would put
    /// instances on the wrong days
    pub fn from_rule(rule: &RecurrenceRule) -> Option<(Self, NaiveDate)> {
        if rule.recurrence_type != RecurrenceType::FixedSchedule
            || !(1..=MAX_EVERY).contains(&rule.frequency_amount)
        {
            return None;
        }
        let anchor = rule
            .interval_anchor
            .filter(|anchor| *anchor > 0)
            .or(rule.start_date)
            .and_then(day_of)?;
        fn keys(offset: &BTreeMap<String, Value>) -> Vec<&str> {
            offset.keys().map(String::as_str).collect()
        }
        fn field(offset: &BTreeMap<String, Value>, key: &str) -> Option<i64> {
            offset.get(key).and_then(Value::as_i64)
        }
        let cadence = match rule.frequency_unit {
            FrequencyUnit::Daily => {
                if !rule
                    .offsets
                    .iter()
                    .all(|offset| keys(offset) == ["dy"] && field(offset, "dy") == Some(0))
                {
                    return None;
                }
                Cadence::Daily
            }
            FrequencyUnit::Weekly => {
                let mut days = rule
                    .offsets
                    .iter()
                    .map(|offset| {
                        (keys(offset) == ["wd"])
                            .then(|| field(offset, "wd"))
                            .flatten()
                            .filter(|day| (0..=6).contains(day))
                            .map(|day| day as u32)
                    })
                    .collect::<Option<Vec<u32>>>()?;
                days.sort_unstable();
                days.dedup();
                Cadence::Weekly(days)
            }
            FrequencyUnit::Monthly => Cadence::Monthly(match rule.offsets.as_slice() {
                [] => None,
                [offset] if keys(offset) == ["dy"] => {
                    let day = field(offset, "dy").filter(|day| (-1..=30).contains(day))?;
                    Some(if day < 0 { -1 } else { day as i32 + 1 })
                }
                _ => return None,
            }),
            FrequencyUnit::Yearly => Cadence::Yearly(match rule.offsets.as_slice() {
                [] => None,
                [offset] if keys(offset) == ["dy", "mo"] => {
                    let day = field(offset, "dy").filter(|day| (0..=30).contains(day))?;
                    let month = field(offset, "mo").filter(|month| (0..=11).contains(month))?;
                    Some((month as u32 + 1, day as u32 + 1))
                }
                _ => return None,
            }),
            FrequencyUnit::Unknown(_) => return None,
        };
        let spec = Self {
            cadence,
            every: rule.frequency_amount,
        };
        // an interval counts from its anchor, which has to be an occurrence, otherwise the phase of the rule is unknown
        if spec.every > 1 && spec.first_occurrence(anchor) != anchor {
            return None;
        }
        Some((spec, anchor))
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
///
/// `icsd` on a template is the day the search for the next instance starts
/// from, not the day the next instance is due: the app sets it to the next
/// occurrence when it makes the template and to the day after each instance it
/// creates. the due day is therefore the first occurrence on or after it,
/// bounded by the rule's end day and count, and a template past its end is
/// never due
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
            let search_from = template.instance_creation_start_date.and_then(day_of)?;
            let due = next_occurrence_of_rule(
                rule,
                search_from.pred_opt()?,
                template.instance_creation_count,
            )?;
            if due > today {
                return None;
            }
            let created = template.instance_creation_count + 1;
            // the search resumes tomorrow, which makes a missed stretch yield this one instance and not one per missed day
            let resume = today.succ_opt()?;
            let following = next_occurrence_of_rule(rule, today, created);
            let instance_id = next_id();
            let instance_uuid: ThingsId = instance_id
                .parse()
                .expect("the id generator yields Things ids");
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
                instance_creation_start_date: Some(Some(day_timestamp(resume))),
                today_index_reference: Some(following.map(day_timestamp)),
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
            // the instance gets its own copy of the template's checklist, open again
            let checklist = template
                .checklist_items
                .iter()
                .map(|item| ChecklistCopy {
                    title: item.title.clone(),
                    index: item.index,
                })
                .collect::<Vec<_>>();
            changes.extend(checklist_items(&checklist, &instance_uuid, now, next_id));
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
    /// the checklist as the to-do has it after the edit, copied with fresh ids onto the template and from there onto every instance
    pub checklist: Vec<ChecklistCopy>,
}

/// one checklist item to copy, every copy starts open
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistCopy {
    pub title: String,
    pub index: i32,
}

/// fresh checklist items for `owner`, one create per copy
pub fn checklist_items(
    checklist: &[ChecklistCopy],
    owner: &ThingsId,
    now: f64,
    next_id: &mut dyn FnMut() -> String,
) -> Vec<(String, WireObject)> {
    checklist
        .iter()
        .map(|item| {
            (
                next_id(),
                WireObject::create(
                    EntityType::ChecklistItem3,
                    ChecklistItemProps {
                        title: item.title.clone(),
                        task_ids: vec![owner.clone()],
                        status: TaskStatus::Incomplete,
                        sort_index: item.index,
                        creation_date: Some(now),
                        modification_date: Some(now),
                        ..Default::default()
                    },
                ),
            )
        })
        .collect()
}

/// the hidden template behind a repeating to-do whose first instance is on `first`
///
/// `icsd` points at the occurrence after the first, as the app writes it, or
/// at the day after the first when the count or the end day allows only the
/// one instance, and `tir` is that occurrence or nothing
pub fn template(
    spec: &RepeatSpec,
    rule: RecurrenceRule,
    first: NaiveDate,
    source: TemplateSource,
    now: f64,
) -> TaskProps {
    // parsed days have four digit years, the day after one is representable
    let day_after_first = first.succ_opt().expect("the day after a parsed day");
    let next = match &spec.cadence {
        Cadence::AfterCompletion(_) => Some(day_after_first),
        _ => next_occurrence_of_rule(&rule, first, 1),
    };
    let after_completion_reference_date = match &spec.cadence {
        Cadence::AfterCompletion(unit) => {
            minus_interval(first, *unit, spec.every).map(day_timestamp)
        }
        _ => None,
    };
    TaskProps {
        title: source.title,
        notes: source.notes,
        start_location: TaskStart::Someday,
        scheduled_date: None,
        today_index_reference: next.map(day_timestamp),
        tag_ids: source.tag_ids,
        parent_project_ids: source.parent_project_ids,
        area_ids: source.area_ids,
        action_group_ids: source.action_group_ids,
        sort_index: source.sort_index,
        today_sort_index: source.today_sort_index,
        recurrence_rule: Some(rule),
        instance_creation_start_date: Some(day_timestamp(next.unwrap_or(day_after_first))),
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
    use crate::{
        store::fold_items,
        wire::{task::TaskProps, wire_object::WireItem},
    };

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("date")
    }

    fn spec(text: &str) -> RepeatSpec {
        text.parse().expect("spec")
    }

    fn rule(json: &str) -> RecurrenceRule {
        serde_json::from_str(json).expect("rule")
    }

    // a yearly rule on september 17 that ends on 2027-09-17, as the app wrote it
    const YEARLY_SEP_17: &str = r#"{"ed":1821139200,"fa":1,"fu":4,"ia":1789603200,"of":[{"dy":16,"mo":8}],"rc":0,"rrv":4,"sr":1789603200,"tp":0,"ts":0}"#;
    // a daily rule anchored on 2026-03-16 that ends on 2026-03-25
    const DAILY_UNTIL_MAR_25: &str = r#"{"ed":1774396800,"fa":1,"fu":16,"ia":1773619200,"of":[{"dy":0}],"rc":0,"rrv":4,"sr":1773619200,"tp":0,"ts":0}"#;
    const TEMPLATE: &str = "Tt11111111111111111111";
    const INSTANCE: &str = "Ji11111111111111111111";

    fn template_object(
        rule_json: &str,
        search_from: &str,
        instances_created: i32,
    ) -> (String, WireObject) {
        (
            TEMPLATE.to_string(),
            WireObject::create(
                EntityType::Task7,
                TaskProps {
                    title: "Renew".to_string(),
                    start_location: TaskStart::Someday,
                    recurrence_rule: Some(rule(rule_json)),
                    instance_creation_start_date: Some(day_timestamp(day(search_from))),
                    instance_creation_count: instances_created,
                    creation_date: Some(1.0),
                    modification_date: Some(1.0),
                    ..Default::default()
                },
            ),
        )
    }

    fn instance_object(uuid: &str, on: &str) -> (String, WireObject) {
        (
            uuid.to_string(),
            WireObject::create(
                EntityType::Task7,
                TaskProps {
                    title: "Renew".to_string(),
                    start_location: TaskStart::Anytime,
                    scheduled_date: Some(day_timestamp(day(on))),
                    recurrence_template_ids: vec![TEMPLATE.parse().expect("template id")],
                    creation_date: Some(1.0),
                    modification_date: Some(1.0),
                    ..Default::default()
                },
            ),
        )
    }

    fn store_of(objects: Vec<(String, WireObject)>) -> ThingsStore {
        ThingsStore::from_raw_state(&fold_items([objects.into_iter().collect::<WireItem>()]))
    }

    /// the nth id the test generator hands out
    fn id(n: u128) -> String {
        ThingsId::from_u128(n).to_string()
    }

    fn due_on(store: &ThingsStore, today: &str) -> Vec<Materialized> {
        let mut ids = (1..).map(id);
        let mut next_id = || ids.next().expect("id");
        due_instances(store, day(today), 2.0, &mut next_id)
    }

    fn template_patch(made: &Materialized) -> BTreeMap<String, Value> {
        made.changes
            .get(TEMPLATE)
            .expect("the template advances")
            .properties_map()
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
        // intervals past the bound and suffixes that are not a unit are errors, not panics
        assert!("daily/2147483647".parse::<RepeatSpec>().is_err());
        assert!("daily/1000".parse::<RepeatSpec>().is_err());
        assert!("daily/0".parse::<RepeatSpec>().is_err());
        assert!("after:2w\u{e9}".parse::<RepeatSpec>().is_err());
        assert!("after:\u{e9}".parse::<RepeatSpec>().is_err());
        assert!("after:2147483648d".parse::<RepeatSpec>().is_err());
        assert_eq!(spec("daily/999").every, 999);
    }

    #[test]
    fn occurrences_past_the_representable_years_are_none() {
        let far = NaiveDate::MAX;
        assert_eq!(spec("daily").next_occurrence(far, far), None);
        assert_eq!(spec("monthly:15").next_occurrence(far, far), None);
        assert_eq!(spec("yearly:12-31").next_occurrence(far, far), None);
        assert_eq!(spec("weekly:mon").next_occurrence(far, far), None);
        assert_eq!(spec("monthly:15").first_occurrence(far), far);
        // a wire rule with an interval past the bound is not evaluated
        assert!(
            RepeatSpec::from_rule(&rule(
                r#"{"fa":1000,"fu":16,"ia":1789603200,"of":[{"dy":0}],"rc":0,"sr":1789603200,"tp":0}"#
            ))
            .is_none()
        );
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
    fn from_rule_declines_shapes_it_cannot_evaluate() {
        let evaluates = |json: &str| RepeatSpec::from_rule(&rule(json)).is_some();
        // the captured shapes
        assert!(evaluates(
            r#"{"fa":1,"fu":16,"ia":1789603200,"of":[{"dy":0}],"rc":0,"sr":1789603200,"tp":0}"#
        ));
        assert!(evaluates(
            r#"{"fa":2,"fu":256,"ia":1789603200,"of":[{"wd":1},{"wd":4}],"rc":0,"sr":1789603200,"tp":0}"#
        ));
        assert!(evaluates(
            r#"{"fa":1,"fu":8,"ia":1789603200,"of":[{"dy":-1}],"rc":0,"sr":1789603200,"tp":0}"#
        ));
        assert!(evaluates(YEARLY_SEP_17));
        // the last thursday of november, a weekday ordinal the CLI does not compute
        assert!(!evaluates(
            r#"{"fa":1,"fu":4,"ia":1767225600,"of":[{"mo":10,"wd":4,"wdo":-1}],"rc":0,"sr":1767225600,"tp":0}"#
        ));
        assert!(!evaluates(
            r#"{"fa":1,"fu":8,"ia":1789603200,"of":[{"wd":5,"wdo":-1}],"rc":0,"sr":1789603200,"tp":0}"#
        ));
        // two days a month, which a single day selector cannot carry
        assert!(!evaluates(
            r#"{"fa":1,"fu":8,"ia":1789603200,"of":[{"dy":0},{"dy":14}],"rc":0,"sr":1789603200,"tp":0}"#
        ));
        // values outside the app's ranges and keys it never writes
        assert!(!evaluates(
            r#"{"fa":1,"fu":256,"ia":1789603200,"of":[{"wd":7}],"rc":0,"sr":1789603200,"tp":0}"#
        ));
        assert!(!evaluates(
            r#"{"fa":1,"fu":4,"ia":1789603200,"of":[{"dy":16,"mo":12}],"rc":0,"sr":1789603200,"tp":0}"#
        ));
        assert!(!evaluates(
            r#"{"fa":1,"fu":16,"ia":1789603200,"of":[{"dy":3}],"rc":0,"sr":1789603200,"tp":0}"#
        ));
        assert!(!evaluates(
            r#"{"fa":0,"fu":16,"ia":1789603200,"of":[{"dy":0}],"rc":0,"sr":1789603200,"tp":0}"#
        ));
    }

    #[test]
    fn an_unsupported_rule_is_never_materialized() {
        let store = store_of(vec![template_object(
            r#"{"fa":1,"fu":4,"ia":1767225600,"of":[{"mo":10,"wd":4,"wdo":-1}],"rc":0,"sr":1767225600,"tp":0}"#,
            "2026-11-26",
            1,
        )]);
        assert!(due_on(&store, "2026-11-27").is_empty());
        assert!(store.projected_repeats(day("2026-11-27")).is_empty());
    }

    #[test]
    fn the_day_after_an_app_made_instance_is_not_due() {
        // the app created the 2026-09-17 instance and moved icsd to the next day
        let store = store_of(vec![
            template_object(YEARLY_SEP_17, "2026-09-18", 1),
            instance_object(INSTANCE, "2026-09-17"),
        ]);
        assert!(due_on(&store, "2026-09-18").is_empty());
        assert!(due_on(&store, "2027-09-16").is_empty());

        let made = due_on(&store, "2027-09-17");
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].day, day("2027-09-17"));
        let patch = template_patch(&made[0]);
        assert_eq!(patch.get("icc"), Some(&json!(2)));
        assert_eq!(
            patch.get("icsd"),
            Some(&json!(day_timestamp(day("2027-09-18"))))
        );
        // the rule ended with this occurrence
        assert_eq!(patch.get("tir"), Some(&Value::Null));
    }

    #[test]
    fn the_last_allowed_day_creates_its_instance_and_no_day_after_it() {
        let store = store_of(vec![
            template_object(DAILY_UNTIL_MAR_25, "2026-03-25", 1),
            instance_object(INSTANCE, "2026-03-24"),
        ]);
        let made = due_on(&store, "2026-03-25");
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].day, day("2026-03-25"));
        let patch = template_patch(&made[0]);
        assert_eq!(
            patch.get("icsd"),
            Some(&json!(day_timestamp(day("2026-03-26"))))
        );
        assert_eq!(patch.get("tir"), Some(&Value::Null));

        // the day after, with that commit folded in
        let mut state = fold_items([[
            template_object(DAILY_UNTIL_MAR_25, "2026-03-25", 1),
            instance_object(INSTANCE, "2026-03-24"),
        ]
        .into_iter()
        .collect::<WireItem>()]);
        crate::store::fold_item(made.into_iter().next().expect("made").changes, &mut state);
        let store = ThingsStore::from_raw_state(&state);
        assert!(due_on(&store, "2026-03-26").is_empty());
        assert!(due_on(&store, "2026-04-01").is_empty());

        // a template an earlier version advanced past its end day
        let store = store_of(vec![template_object(DAILY_UNTIL_MAR_25, "2026-03-26", 2)]);
        assert!(due_on(&store, "2026-03-26").is_empty());
    }

    #[test]
    fn a_rescheduled_instance_does_not_hold_the_template_back() {
        let daily = r#"{"ed":64092211200,"fa":1,"fu":16,"ia":1773619200,"of":[{"dy":0}],"rc":0,"rrv":4,"sr":1773619200,"tp":0,"ts":0}"#;
        // the user moved the instance of 2026-03-24 to 2026-03-27, the search still starts on 03-25
        let store = store_of(vec![
            template_object(daily, "2026-03-25", 2),
            instance_object(INSTANCE, "2026-03-27"),
        ]);
        let made = due_on(&store, "2026-03-25");
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].day, day("2026-03-25"));
    }

    #[test]
    fn an_interval_rule_counts_from_an_anchor_that_is_an_occurrence() {
        // every two weeks on monday, anchored on a thursday, the phase is unknown
        assert!(
            RepeatSpec::from_rule(&rule(
                r#"{"fa":2,"fu":256,"ia":1789603200,"of":[{"wd":1}],"rc":0,"sr":1789603200,"tp":0}"#
            ))
            .is_none()
        );
        // anchored on a monday it counts from there
        assert!(
            RepeatSpec::from_rule(&rule(
                r#"{"fa":2,"fu":256,"ia":1789948800,"of":[{"wd":1}],"rc":0,"sr":1789603200,"tp":0}"#
            ))
            .is_some()
        );
        // every week the phase does not matter
        assert!(
            RepeatSpec::from_rule(&rule(
                r#"{"fa":1,"fu":256,"ia":1789603200,"of":[{"wd":1}],"rc":0,"sr":1789603200,"tp":0}"#
            ))
            .is_some()
        );
        // every third month on the 22nd, anchored on the 17th
        assert!(
            RepeatSpec::from_rule(&rule(
                r#"{"fa":3,"fu":8,"ia":1789603200,"of":[{"dy":21}],"rc":0,"sr":1789603200,"tp":0}"#
            ))
            .is_none()
        );
        // every two days counts from any anchor
        assert!(
            RepeatSpec::from_rule(&rule(
                r#"{"fa":2,"fu":16,"ia":1789603200,"of":[{"dy":0}],"rc":0,"sr":1789603200,"tp":0}"#
            ))
            .is_some()
        );
    }

    #[test]
    fn an_instance_gets_a_fresh_copy_of_the_template_checklist() {
        let daily = r#"{"ed":64092211200,"fa":1,"fu":16,"ia":1773619200,"of":[{"dy":0}],"rc":0,"rrv":4,"sr":1773619200,"tp":0,"ts":0}"#;
        let item_id = "Ck11111111111111111111";
        let store = store_of(vec![
            template_object(daily, "2026-03-25", 1),
            (
                item_id.to_string(),
                WireObject::create(
                    EntityType::ChecklistItem3,
                    ChecklistItemProps {
                        title: "Stretch".to_string(),
                        task_ids: vec![TEMPLATE.parse().expect("id")],
                        status: TaskStatus::Completed,
                        sort_index: 7,
                        ..Default::default()
                    },
                ),
            ),
        ]);
        let made = due_on(&store, "2026-03-25");
        assert_eq!(made.len(), 1);
        let copy = made[0]
            .changes
            .get(&id(2))
            .expect("the copy takes the id after the instance")
            .properties_map();
        assert_eq!(copy.get("tt"), Some(&json!("Stretch")));
        assert_eq!(copy.get("ts"), Some(&json!([id(1)])));
        assert_eq!(copy.get("ss"), Some(&json!(0)));
        assert_eq!(copy.get("ix"), Some(&json!(7)));
        assert!(!made[0].changes.contains_key(item_id));
    }

    #[test]
    fn a_missed_stretch_yields_one_instance_on_the_first_missed_day() {
        let daily = r#"{"ed":64092211200,"fa":1,"fu":16,"ia":1773619200,"of":[{"dy":0}],"rc":0,"rrv":4,"sr":1773619200,"tp":0,"ts":0}"#;
        let store = store_of(vec![template_object(daily, "2026-03-20", 1)]);
        let made = due_on(&store, "2026-03-25");
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].day, day("2026-03-20"));
        let patch = template_patch(&made[0]);
        assert_eq!(
            patch.get("icsd"),
            Some(&json!(day_timestamp(day("2026-03-26"))))
        );
        assert_eq!(
            patch.get("tir"),
            Some(&json!(day_timestamp(day("2026-03-26"))))
        );

        // the count bounds the search as well as the end day
        let store = store_of(vec![template_object(
            r#"{"fa":1,"fu":16,"ia":1773619200,"of":[{"dy":0}],"rc":2,"sr":1773619200,"tp":0}"#,
            "2026-03-20",
            2,
        )]);
        assert!(due_on(&store, "2026-03-25").is_empty());
    }

    #[test]
    fn template_ends_with_the_first_instance_when_the_bound_allows_one() {
        let today = day("2026-09-17");
        let source = || TemplateSource {
            title: "once".to_string(),
            notes: None,
            tag_ids: Vec::new(),
            parent_project_ids: Vec::new(),
            area_ids: Vec::new(),
            action_group_ids: Vec::new(),
            alarm_time_offset: None,
            sort_index: 0,
            today_sort_index: 0,
            conflict_overrides: None,
            checklist: Vec::new(),
        };
        let daily = spec("daily");
        for bound in [Bound::Times(1), Bound::Until(today)] {
            let made = template(
                &daily,
                daily.rule(today, today, bound),
                today,
                source(),
                1.0,
            );
            assert_eq!(
                made.instance_creation_start_date,
                Some(day_timestamp(day("2026-09-18")))
            );
            assert_eq!(made.today_index_reference, None);
        }
        let made = template(
            &daily,
            daily.rule(today, today, Bound::Times(2)),
            today,
            source(),
            1.0,
        );
        assert_eq!(
            made.today_index_reference,
            Some(day_timestamp(day("2026-09-18")))
        );
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
            checklist: Vec::new(),
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
