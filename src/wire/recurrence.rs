use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use chrono::{TimeZone, Utc};
use num_enum::{FromPrimitive, IntoPrimitive};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum::{Display, EnumString};

/// Recurrence rule payload (`rr`) for recurring templates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecurrenceRule {
    /// `tp`: recurrence rule type.
    #[serde(rename = "tp", default)]
    pub recurrence_type: RecurrenceType,

    /// `fu`: frequency unit bitmask.
    #[serde(rename = "fu", default = "default_frequency_unit")]
    pub frequency_unit: FrequencyUnit,

    /// `fa`: frequency amount (every N units).
    #[serde(rename = "fa", default = "default_frequency_amount")]
    pub frequency_amount: i32,

    /// `of`: offsets (weekday/day/ordinal selectors).
    #[serde(rename = "of", default)]
    pub offsets: Vec<BTreeMap<String, Value>>,

    /// `sr`: recurrence start day timestamp.
    #[serde(rename = "sr", default)]
    pub start_date: Option<i64>,

    /// `ia`: interval anchor day timestamp for recurrence calculations.
    #[serde(rename = "ia", default)]
    pub interval_anchor: Option<i64>,

    /// `ed`: recurrence end day timestamp (`64092211200` ~= effectively never), absent when a repeat count bounds the rule.
    #[serde(
        rename = "ed",
        default = "default_recurrence_end_date",
        skip_serializing_if = "Option::is_none"
    )]
    pub end_date: Option<i64>,

    /// `rc`: repeat count.
    #[serde(rename = "rc", default)]
    pub repeat_count: i32,

    /// `ts`: recurrence time span in days (`-1` is an observed sentinel).
    #[serde(rename = "ts", default)]
    pub time_span_in_days: i32,

    /// `rrv`: recurrence rule version.
    #[serde(rename = "rrv", default = "default_version")]
    pub version: i32,
}

impl Default for RecurrenceRule {
    fn default() -> Self {
        Self {
            recurrence_type: RecurrenceType::default(),
            frequency_unit: default_frequency_unit(),
            frequency_amount: default_frequency_amount(),
            offsets: Vec::new(),
            start_date: None,
            interval_anchor: None,
            end_date: default_recurrence_end_date(),
            repeat_count: 0,
            time_span_in_days: 0,
            version: default_version(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecurrenceDescriptionError {
    InvalidFrequencyAmount(i32),
    InvalidRepeatCount(i32),
    UnknownRecurrenceType(i32),
    UnknownFrequencyUnit(i32),
    InvalidOffset(usize),
    InvalidStartDate(i64),
    InvalidEndDate(i64),
    ConflictingEndConditions,
}

impl fmt::Display for RecurrenceDescriptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFrequencyAmount(amount) => {
                write!(f, "invalid recurrence frequency amount: {amount}")
            }
            Self::InvalidRepeatCount(count) => {
                write!(f, "invalid recurrence repeat count: {count}")
            }
            Self::UnknownRecurrenceType(value) => {
                write!(f, "unknown recurrence type: {value}")
            }
            Self::UnknownFrequencyUnit(value) => {
                write!(f, "unknown recurrence frequency unit: {value}")
            }
            Self::InvalidOffset(index) => write!(f, "invalid recurrence offset at index {index}"),
            Self::InvalidStartDate(timestamp) => {
                write!(f, "invalid recurrence start date: {timestamp}")
            }
            Self::InvalidEndDate(timestamp) => {
                write!(f, "invalid recurrence end date: {timestamp}")
            }
            Self::ConflictingEndConditions => {
                write!(f, "recurrence has both an end date and a repeat count")
            }
        }
    }
}

impl std::error::Error for RecurrenceDescriptionError {}

impl RecurrenceRule {
    /// Formats a legacy recurrence rule using the observed English Things phrasing.
    pub fn human_readable(&self) -> Result<String, RecurrenceDescriptionError> {
        if self.frequency_amount < 1 {
            return Err(RecurrenceDescriptionError::InvalidFrequencyAmount(
                self.frequency_amount,
            ));
        }

        if self.repeat_count < 0 {
            return Err(RecurrenceDescriptionError::InvalidRepeatCount(
                self.repeat_count,
            ));
        }

        let cadence = match self.recurrence_type {
            RecurrenceType::FixedSchedule => self.fixed_schedule_description(),
            RecurrenceType::AfterCompletion => self.after_completion_description(),
            RecurrenceType::Unknown(value) => {
                Err(RecurrenceDescriptionError::UnknownRecurrenceType(value))
            }
        }?;

        self.with_bounds(cadence)
    }

    fn fixed_schedule_description(&self) -> Result<String, RecurrenceDescriptionError> {
        let cadence = match self.frequency_unit {
            FrequencyUnit::Daily if self.frequency_amount == 1 => "daily".to_string(),
            FrequencyUnit::Daily => format!("every {} days", self.frequency_amount),
            FrequencyUnit::Weekly if self.frequency_amount == 1 => "every week".to_string(),
            FrequencyUnit::Weekly => format!("every {} weeks", self.frequency_amount),
            FrequencyUnit::Monthly if self.frequency_amount == 1 => "every month".to_string(),
            FrequencyUnit::Monthly => format!("every {} months", self.frequency_amount),
            FrequencyUnit::Yearly if self.frequency_amount == 1 => "every year".to_string(),
            FrequencyUnit::Yearly => format!("every {} years", self.frequency_amount),
            FrequencyUnit::Unknown(value) => {
                return Err(RecurrenceDescriptionError::UnknownFrequencyUnit(value));
            }
        };

        let selector = match self.frequency_unit {
            FrequencyUnit::Daily => None,
            FrequencyUnit::Weekly => self.weekly_selector()?,
            FrequencyUnit::Monthly => self.monthly_selector()?,
            FrequencyUnit::Yearly => self.yearly_selector()?,
            FrequencyUnit::Unknown(_) => unreachable!(),
        };

        Ok(match selector {
            Some(selector) => format!("Repeat {cadence} {selector}"),
            None => format!("Repeat {cadence}"),
        })
    }

    fn after_completion_description(&self) -> Result<String, RecurrenceDescriptionError> {
        let unit = match self.frequency_unit {
            FrequencyUnit::Daily => pluralized(self.frequency_amount, "day", "days"),
            FrequencyUnit::Weekly => pluralized(self.frequency_amount, "week", "weeks"),
            FrequencyUnit::Monthly => pluralized(self.frequency_amount, "month", "months"),
            FrequencyUnit::Yearly => pluralized(self.frequency_amount, "year", "years"),
            FrequencyUnit::Unknown(value) => {
                return Err(RecurrenceDescriptionError::UnknownFrequencyUnit(value));
            }
        };

        Ok(format!(
            "Repeat {unit} after the previous to-do has been completed"
        ))
    }

    fn weekly_selector(&self) -> Result<Option<String>, RecurrenceDescriptionError> {
        if self.offsets.is_empty() {
            return Ok(None);
        }

        let weekdays = self
            .offsets
            .iter()
            .enumerate()
            .map(|(index, offset)| {
                if offset.len() != 1 {
                    return Err(RecurrenceDescriptionError::InvalidOffset(index));
                }

                offset
                    .get("wd")
                    .and_then(Value::as_i64)
                    .and_then(weekday_name)
                    .ok_or(RecurrenceDescriptionError::InvalidOffset(index))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let weekday_set = weekdays.iter().copied().collect::<BTreeSet<_>>();

        let selector = if weekday_set
            == BTreeSet::from(["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"])
        {
            "on weekdays".to_string()
        } else if weekday_set == BTreeSet::from(["Sunday", "Saturday"]) {
            "on weekends".to_string()
        } else {
            format!("on {}", join_list(&weekdays))
        };

        Ok(Some(selector))
    }

    fn monthly_selector(&self) -> Result<Option<String>, RecurrenceDescriptionError> {
        if self.offsets.is_empty() {
            return Ok(None);
        }

        let offsets = self
            .offsets
            .iter()
            .enumerate()
            .map(|(index, offset)| monthly_offset(index, offset))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Some(format!("on the {}", join_list(&offsets))))
    }

    fn yearly_selector(&self) -> Result<Option<String>, RecurrenceDescriptionError> {
        if self.offsets.is_empty() {
            return Ok(None);
        }

        let offsets = self
            .offsets
            .iter()
            .enumerate()
            .map(|(index, offset)| yearly_offset(index, offset))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Some(format!("on the {}", join_list(&offsets))))
    }

    fn with_bounds(&self, cadence: String) -> Result<String, RecurrenceDescriptionError> {
        let start = self
            .start_date
            .filter(|timestamp| *timestamp != RECURRENCE_START_DATE_SENTINEL)
            .map(|timestamp| {
                format_day(timestamp).ok_or(RecurrenceDescriptionError::InvalidStartDate(timestamp))
            })
            .transpose()?;
        let end = self
            .end_date
            .filter(|end| *end != RECURRENCE_END_NEVER)
            .map(|end| format_day(end).ok_or(RecurrenceDescriptionError::InvalidEndDate(end)))
            .transpose()?;

        if end.is_some() && self.repeat_count > 0 {
            return Err(RecurrenceDescriptionError::ConflictingEndConditions);
        }

        let mut bounds = Vec::new();
        if let Some(start) = start {
            bounds.push(format!("starts {start}"));
        }
        if let Some(end) = end {
            bounds.push(format!("ends {end}"));
        } else if self.repeat_count > 0 {
            bounds.push(format!(
                "ends after {}",
                pluralized(self.repeat_count, "repetition", "repetitions")
            ));
        }

        if bounds.is_empty() {
            return Ok(cadence);
        }

        Ok(format!("{cadence}; {}", bounds.join("; ")))
    }
}

fn monthly_offset(
    index: usize,
    offset: &BTreeMap<String, Value>,
) -> Result<String, RecurrenceDescriptionError> {
    if offset.len() == 1 {
        let day = offset
            .get("dy")
            .and_then(Value::as_i64)
            .filter(|day| (-1..=30).contains(day))
            .ok_or(RecurrenceDescriptionError::InvalidOffset(index))?;

        // -1 is the last day of the month, as the app writes monthly:last
        if day < 0 {
            return Ok("last day".to_string());
        }
        return Ok(format!("{} day", ordinal(day + 1)));
    }

    if offset.len() == 2 {
        let weekday = offset
            .get("wd")
            .and_then(Value::as_i64)
            .and_then(weekday_name)
            .ok_or(RecurrenceDescriptionError::InvalidOffset(index))?;
        let ordinal = offset
            .get("wdo")
            .and_then(Value::as_i64)
            .and_then(weekday_ordinal)
            .ok_or(RecurrenceDescriptionError::InvalidOffset(index))?;

        return Ok(format!("{ordinal} {weekday}"));
    }

    Err(RecurrenceDescriptionError::InvalidOffset(index))
}

fn yearly_offset(
    index: usize,
    offset: &BTreeMap<String, Value>,
) -> Result<String, RecurrenceDescriptionError> {
    let month = offset
        .get("mo")
        .and_then(Value::as_i64)
        .and_then(month_name)
        .ok_or(RecurrenceDescriptionError::InvalidOffset(index))?;

    if offset.len() == 2 {
        let day = offset
            .get("dy")
            .and_then(Value::as_i64)
            .filter(|day| (0..=30).contains(day))
            .ok_or(RecurrenceDescriptionError::InvalidOffset(index))?;

        return Ok(format!("{} day, {month}", ordinal(day + 1)));
    }

    if offset.len() == 3 {
        let weekday = offset
            .get("wd")
            .and_then(Value::as_i64)
            .and_then(weekday_name)
            .ok_or(RecurrenceDescriptionError::InvalidOffset(index))?;
        let ordinal = offset
            .get("wdo")
            .and_then(Value::as_i64)
            .and_then(weekday_ordinal)
            .ok_or(RecurrenceDescriptionError::InvalidOffset(index))?;

        return Ok(format!("{ordinal} {weekday}, {month}"));
    }

    Err(RecurrenceDescriptionError::InvalidOffset(index))
}

fn pluralized(amount: i32, singular: &str, plural: &str) -> String {
    if amount == 1 {
        format!("1 {singular}")
    } else {
        format!("{amount} {plural}")
    }
}

fn weekday_name(value: i64) -> Option<&'static str> {
    [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ]
    .get(value as usize)
    .copied()
}

fn month_name(value: i64) -> Option<&'static str> {
    [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ]
    .get(value as usize)
    .copied()
}

fn format_day(timestamp: i64) -> Option<String> {
    Utc.timestamp_opt(timestamp, 0)
        .single()
        .map(|date| date.format("%Y-%m-%d").to_string())
}

fn weekday_ordinal(value: i64) -> Option<String> {
    if value == -1 {
        return Some("last".to_string());
    }
    if !(1..=5).contains(&value) {
        return None;
    }

    Some(ordinal(value))
}

fn ordinal(value: i64) -> String {
    let suffix = match value % 100 {
        11..=13 => "th",
        _ => match value % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        },
    };

    format!("{value}{suffix}")
}

fn join_list(values: &[impl AsRef<str>]) -> String {
    match values {
        [] => String::new(),
        [value] => value.as_ref().to_string(),
        [first, second] => format!("{} and {}", first.as_ref(), second.as_ref()),
        _ => {
            let Some((last, initial)) = values.split_last() else {
                unreachable!();
            };
            let initial = initial
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(", ");

            format!("{initial}, and {}", last.as_ref())
        }
    }
}

/// Recurrence rule type (`rr.tp`).
#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    Display,
    EnumString,
    FromPrimitive,
    IntoPrimitive,
)]
#[repr(i32)]
#[serde(from = "i32", into = "i32")]
pub enum RecurrenceType {
    /// Fixed schedule cadence.
    FixedSchedule = 0,
    /// Interval anchored after completion date.
    AfterCompletion = 1,

    /// Unknown value preserved for forward compatibility.
    #[num_enum(catch_all)]
    #[strum(disabled, to_string = "{0}")]
    Unknown(i32),
}

#[allow(clippy::derivable_impls)]
impl Default for RecurrenceType {
    fn default() -> Self {
        Self::FixedSchedule
    }
}

/// Recurrence frequency unit (`rr.fu`).
#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    Display,
    EnumString,
    FromPrimitive,
    IntoPrimitive,
)]
#[repr(i32)]
#[serde(from = "i32", into = "i32")]
pub enum FrequencyUnit {
    /// Yearly bitmask value `4`.
    Yearly = 4,
    /// Monthly bitmask value `8`.
    Monthly = 8,
    /// Daily bitmask value `16`.
    Daily = 16,
    /// Weekly bitmask value `256`.
    Weekly = 256,

    /// Unknown value preserved for forward compatibility.
    #[num_enum(catch_all)]
    #[strum(disabled, to_string = "{0}")]
    Unknown(i32),
}

#[allow(clippy::derivable_impls)]
impl Default for FrequencyUnit {
    fn default() -> Self {
        Self::Weekly
    }
}

/// Default recurrence frequency unit (`rr.fu`) is weekly.
fn default_frequency_unit() -> FrequencyUnit {
    FrequencyUnit::Weekly
}

/// Default recurrence frequency amount (`rr.fa`) is every 1 unit.
const fn default_frequency_amount() -> i32 {
    1
}

/// Default recurrence end date (`rr.ed`) far in the future (~year 4001).
pub const RECURRENCE_END_NEVER: i64 = 64_092_211_200;

const fn default_recurrence_end_date() -> Option<i64> {
    Some(RECURRENCE_END_NEVER)
}

/// Sentinel used when a recurrence has no explicit start date.
const RECURRENCE_START_DATE_SENTINEL: i64 = -62_135_769_600;

/// Current observed recurrence rule version (`rrv`).
const fn default_version() -> i32 {
    4
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offset(fields: &[(&str, i64)]) -> BTreeMap<String, Value> {
        fields
            .iter()
            .map(|(key, value)| ((*key).to_string(), Value::from(*value)))
            .collect()
    }

    #[test]
    fn rust_default_matches_an_empty_wire_rule() {
        let decoded: RecurrenceRule = serde_json::from_str("{}").expect("empty recurrence rule");

        assert_eq!(RecurrenceRule::default(), decoded);
    }

    #[test]
    fn describes_fixed_daily_schedule() {
        let rule = RecurrenceRule {
            frequency_unit: FrequencyUnit::Daily,
            frequency_amount: 1,
            offsets: vec![offset(&[("dy", 0)])],
            ..Default::default()
        };

        assert_eq!(rule.human_readable().as_deref(), Ok("Repeat daily"));
    }

    #[test]
    fn describes_fixed_weekly_schedule_with_multiple_days() {
        let rule = RecurrenceRule {
            frequency_unit: FrequencyUnit::Weekly,
            frequency_amount: 2,
            offsets: vec![
                offset(&[("wd", 1)]),
                offset(&[("wd", 3)]),
                offset(&[("wd", 5)]),
            ],
            ..Default::default()
        };

        assert_eq!(
            rule.human_readable().as_deref(),
            Ok("Repeat every 2 weeks on Monday, Wednesday, and Friday")
        );
    }

    #[test]
    fn describes_fixed_monthly_day_and_weekday_schedules() {
        let day_rule = RecurrenceRule {
            frequency_unit: FrequencyUnit::Monthly,
            frequency_amount: 1,
            offsets: vec![offset(&[("dy", 14)])],
            ..Default::default()
        };
        let weekday_rule = RecurrenceRule {
            frequency_unit: FrequencyUnit::Monthly,
            frequency_amount: 1,
            offsets: vec![offset(&[("wd", 5), ("wdo", -1)])],
            ..Default::default()
        };

        assert_eq!(
            day_rule.human_readable().as_deref(),
            Ok("Repeat every month on the 15th day")
        );
        assert_eq!(
            weekday_rule.human_readable().as_deref(),
            Ok("Repeat every month on the last Friday")
        );
        let last_day_rule = RecurrenceRule {
            offsets: vec![offset(&[("dy", -1)])],
            ..day_rule
        };
        assert_eq!(
            last_day_rule.human_readable().as_deref(),
            Ok("Repeat every month on the last day")
        );
    }

    #[test]
    fn describes_fixed_yearly_schedule() {
        let exact_day_rule = RecurrenceRule {
            frequency_unit: FrequencyUnit::Yearly,
            frequency_amount: 2,
            offsets: vec![offset(&[("mo", 8), ("dy", 14)])],
            ..Default::default()
        };
        let legacy_weekday_rule = RecurrenceRule {
            frequency_unit: FrequencyUnit::Yearly,
            offsets: vec![offset(&[("mo", 10), ("wd", 4), ("wdo", -1)])],
            ..Default::default()
        };

        assert_eq!(
            exact_day_rule.human_readable().as_deref(),
            Ok("Repeat every 2 years on the 15th day, September")
        );
        assert_eq!(
            legacy_weekday_rule.human_readable().as_deref(),
            Ok("Repeat every year on the last Thursday, November")
        );
    }

    #[test]
    fn describes_after_completion_schedules() {
        for (unit, amount, expected) in [
            (
                FrequencyUnit::Daily,
                1,
                "Repeat 1 day after the previous to-do has been completed",
            ),
            (
                FrequencyUnit::Weekly,
                2,
                "Repeat 2 weeks after the previous to-do has been completed",
            ),
            (
                FrequencyUnit::Monthly,
                3,
                "Repeat 3 months after the previous to-do has been completed",
            ),
            (
                FrequencyUnit::Yearly,
                4,
                "Repeat 4 years after the previous to-do has been completed",
            ),
        ] {
            let rule = RecurrenceRule {
                recurrence_type: RecurrenceType::AfterCompletion,
                frequency_unit: unit,
                frequency_amount: amount,
                ..Default::default()
            };

            assert_eq!(rule.human_readable().as_deref(), Ok(expected));
        }
    }

    #[test]
    fn describes_start_and_end_conditions() {
        let dated_rule = RecurrenceRule {
            frequency_unit: FrequencyUnit::Daily,
            start_date: Some(1_787_443_200),
            end_date: Some(1_798_675_200),
            ..Default::default()
        };
        let counted_rule = RecurrenceRule {
            frequency_unit: FrequencyUnit::Weekly,
            start_date: Some(RECURRENCE_START_DATE_SENTINEL),
            repeat_count: 5,
            ..Default::default()
        };

        assert_eq!(
            dated_rule.human_readable().as_deref(),
            Ok("Repeat daily; starts 2026-08-23; ends 2026-12-31")
        );
        assert_eq!(
            counted_rule.human_readable().as_deref(),
            Ok("Repeat every week; ends after 5 repetitions")
        );
    }

    #[test]
    fn rejects_invalid_end_conditions_and_dates() {
        let conflicting_rule = RecurrenceRule {
            end_date: Some(1_798_675_200),
            repeat_count: 5,
            ..Default::default()
        };
        let invalid_date_rule = RecurrenceRule {
            start_date: Some(i64::MAX),
            ..Default::default()
        };
        let invalid_count_rule = RecurrenceRule {
            repeat_count: -1,
            ..Default::default()
        };

        assert_eq!(
            conflicting_rule.human_readable(),
            Err(RecurrenceDescriptionError::ConflictingEndConditions)
        );
        assert_eq!(
            invalid_date_rule.human_readable(),
            Err(RecurrenceDescriptionError::InvalidStartDate(i64::MAX))
        );
        assert_eq!(
            invalid_count_rule.human_readable(),
            Err(RecurrenceDescriptionError::InvalidRepeatCount(-1))
        );
    }

    #[test]
    fn rejects_unrecognized_offset_shapes() {
        let rule = RecurrenceRule {
            frequency_unit: FrequencyUnit::Weekly,
            frequency_amount: 1,
            offsets: vec![offset(&[("weekday", 1)])],
            ..Default::default()
        };

        assert_eq!(
            rule.human_readable(),
            Err(RecurrenceDescriptionError::InvalidOffset(0))
        );
    }
}
