//! the wire types of the Things Cloud sync protocol
//!
//! a history page holds items shaped `{ uuid: { "t": operation, "e": entity, "p": properties } }`, and replaying them in order yields the current state

use serde::{Deserialize, Deserializer, Serializer};

pub mod area;
pub mod checklist;
pub mod notes;
pub mod recurrence;
pub mod tags;
pub mod task;
pub mod tombstone;
pub mod wire_object;

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

pub(crate) fn deserialize_optional_field<'de, D, T>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// a day stamp as the app writes it, a whole number without a fraction
pub(crate) fn serialize_day_stamp<S>(
    value: &Option<Option<f64>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        // a day stamp is whole
        // an f64 holds every integer below 2^53 exactly
        Some(Some(day)) if day.fract() == 0.0 && day.abs() < 9.0e15 => {
            serializer.serialize_i64(*day as i64)
        }
        Some(Some(day)) => serializer.serialize_f64(*day),
        Some(None) | None => serializer.serialize_none(),
    }
}

pub(crate) fn deserialize_default_on_null<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}

/// a patch field of one or many, null leaving the field as it is
pub(crate) fn deserialize_patch_vec_or_single<'de, D, T>(
    deserializer: D,
) -> Result<Option<Vec<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<OneOrMany<T>>::deserialize(deserializer).map(|value| {
        value.map(|value| match value {
            OneOrMany::One(v) => vec![v],
            OneOrMany::Many(v) => v,
        })
    })
}

pub(crate) fn deserialize_vec_or_single<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<OneOrMany<T>>::deserialize(deserializer).map(|value| match value {
        None => Vec::new(),
        Some(OneOrMany::One(v)) => vec![v],
        Some(OneOrMany::Many(v)) => v,
    })
}
