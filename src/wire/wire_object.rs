use std::collections::BTreeMap;

use num_enum::{FromPrimitive, IntoPrimitive};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned, ser::SerializeStruct,
};
use serde_json::Value;
use strum::{Display, EnumString};

use crate::wire::{
    area::{AreaPatch, AreaProps},
    checklist::{ChecklistItemPatch, ChecklistItemProps},
    tags::{TagPatch, TagProps},
    task::{TaskPatch, TaskProps},
    tombstone::{CommandProps, TombstoneProps},
};

pub type WireItem = BTreeMap<String, WireObject>;

/// one wire object of a history item
///
/// keyed by its id there
#[derive(Debug, Clone)]
pub struct WireObject {
    pub operation_type: OperationType,
    pub entity_type: Option<EntityType>,
    pub payload: Properties,
}

#[derive(Debug, Clone)]
pub enum Properties {
    TaskCreate(Box<TaskProps>),
    TaskUpdate(Box<TaskPatch>),
    ChecklistCreate(ChecklistItemProps),
    ChecklistUpdate(ChecklistItemPatch),
    TagCreate(TagProps),
    TagUpdate(TagPatch),
    AreaCreate(AreaProps),
    AreaUpdate(AreaPatch),
    TombstoneCreate(TombstoneProps),
    CommandCreate(CommandProps),
    Delete,
    /// known entity families we intentionally skip materializing in store state
    Ignored(BTreeMap<String, Value>),
    /// a payload the CLI holds untyped
    ///
    /// that of an unknown kind, of a known kind that did not parse, of an operation the CLI does not know, or of an envelope that did not read
    Unknown(BTreeMap<String, Value>),
}

macro_rules! impl_properties_from {
    ($($source:ty => $variant:ident),+ $(,)?) => {
        $(
            impl From<$source> for Properties {
                fn from(value: $source) -> Self {
                    Self::$variant(value)
                }
            }
        )+
    };
}

impl From<TaskProps> for Properties {
    fn from(value: TaskProps) -> Self {
        Self::TaskCreate(Box::new(value))
    }
}

impl From<TaskPatch> for Properties {
    fn from(value: TaskPatch) -> Self {
        Self::TaskUpdate(Box::new(value))
    }
}

impl_properties_from!(
    ChecklistItemProps => ChecklistCreate,
    ChecklistItemPatch => ChecklistUpdate,
    TagProps => TagCreate,
    TagPatch => TagUpdate,
    AreaProps => AreaCreate,
    AreaPatch => AreaUpdate,

);

impl WireObject {
    pub fn properties(&self) -> Result<Properties, serde_json::Error> {
        match &self.payload {
            Properties::Unknown(map) => WireObject::properties_from(
                self.operation_type,
                self.entity_type.as_ref(),
                map.clone(),
            ),
            other => Ok(other.clone()),
        }
    }

    /// the payload without the fields that do not parse on their own, for an object that is kept in view and marked
    pub fn readable_properties(&self) -> Result<Properties, serde_json::Error> {
        let Properties::Unknown(map) = &self.payload else {
            return self.properties();
        };
        let parses = |key: &String, value: &Value| {
            let single = BTreeMap::from([(key.clone(), value.clone())]);
            Self::properties_from(self.operation_type, self.entity_type.as_ref(), single).is_ok()
        };
        let readable = map
            .iter()
            .filter(|(key, value)| parses(key, value))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        Self::properties_from(self.operation_type, self.entity_type.as_ref(), readable)
    }

    pub fn properties_map(&self) -> BTreeMap<String, Value> {
        match &self.payload {
            Properties::TaskCreate(props) => to_map(props),
            Properties::TaskUpdate(patch) => to_map(patch),
            Properties::ChecklistCreate(props) => to_map(props),
            Properties::ChecklistUpdate(patch) => to_map(patch),
            Properties::TagCreate(props) => to_map(props),
            Properties::TagUpdate(patch) => to_map(patch),
            Properties::AreaCreate(props) => to_map(props),
            Properties::AreaUpdate(patch) => to_map(patch),
            Properties::TombstoneCreate(props) => to_map(props),
            Properties::CommandCreate(props) => to_map(props),
            Properties::Delete => BTreeMap::new(),
            Properties::Ignored(map) => map.clone(),
            Properties::Unknown(map) => map.clone(),
        }
    }

    pub fn create(entity_type: EntityType, payload: impl Into<Properties>) -> Self {
        let payload =
            Self::coerce_known_payload(OperationType::Create, &entity_type, payload.into());
        Self {
            operation_type: OperationType::Create,
            entity_type: Some(entity_type),
            payload,
        }
    }

    pub fn update(entity_type: EntityType, payload: impl Into<Properties>) -> Self {
        let payload =
            Self::coerce_known_payload(OperationType::Update, &entity_type, payload.into());
        Self {
            operation_type: OperationType::Update,
            entity_type: Some(entity_type),
            payload,
        }
    }

    pub fn delete(entity_type: EntityType) -> Self {
        Self {
            operation_type: OperationType::Delete,
            entity_type: Some(entity_type),
            payload: Properties::Delete,
        }
    }

    fn properties_from(
        operation_type: OperationType,
        entity_type: Option<&EntityType>,
        properties: BTreeMap<String, Value>,
    ) -> Result<Properties, serde_json::Error> {
        use EntityType::*;
        use Properties::*;
        let p = properties;

        fn parse<T: DeserializeOwned>(
            properties: BTreeMap<String, Value>,
        ) -> Result<T, serde_json::Error> {
            parse_props_from_map(properties)
        }

        let payload = match operation_type {
            OperationType::Delete => Delete,
            OperationType::Create => match entity_type {
                Some(entity) if entity.is_task_family() => TaskCreate(Box::new(parse(p)?)),
                Some(entity) if entity.is_checklist_family() => ChecklistCreate(parse(p)?),
                Some(entity) if entity.is_tag_family() => TagCreate(parse(p)?),
                Some(entity) if entity.is_area_family() => AreaCreate(parse(p)?),
                Some(Tombstone2) => TombstoneCreate(parse(p)?),
                Some(Command) => CommandCreate(parse(p)?),
                Some(Settings3 | Settings4 | Settings5) => Ignored(p),
                _ => Properties::Unknown(p),
            },
            OperationType::Update => match entity_type {
                Some(entity) if entity.is_task_family() => TaskUpdate(Box::new(parse(p)?)),
                Some(entity) if entity.is_checklist_family() => ChecklistUpdate(parse(p)?),
                Some(entity) if entity.is_tag_family() => TagUpdate(parse(p)?),
                Some(entity) if entity.is_area_family() => AreaUpdate(parse(p)?),
                Some(Settings3 | Settings4 | Settings5) => Ignored(p),
                _ => Properties::Unknown(p),
            },
            OperationType::Unknown(_) => Properties::Unknown(p),
        };

        Ok(payload)
    }

    fn coerce_known_payload(
        operation_type: OperationType,
        entity_type: &EntityType,
        payload: Properties,
    ) -> Properties {
        match payload {
            Properties::Unknown(map) => {
                match Self::properties_from(operation_type, Some(entity_type), map.clone()) {
                    Ok(parsed) => parsed,
                    Err(_) => Properties::Unknown(map),
                }
            }
            other => other,
        }
    }
}

#[derive(Deserialize)]
struct RawWireObject {
    #[serde(rename = "t")]
    operation_type: OperationType,
    #[serde(rename = "e")]
    entity_type: Option<EntityType>,
    #[serde(
        rename = "p",
        default,
        deserialize_with = "crate::wire::deserialize_default_on_null"
    )]
    properties: BTreeMap<String, Value>,
}

impl Serialize for WireObject {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("WireObject", 3)?;
        state.serialize_field("t", &self.operation_type)?;
        state.serialize_field("e", &self.entity_type)?;
        state.serialize_field("p", &self.properties_map())?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for WireObject {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        // an envelope this CLI cannot read is an operation it does not know
        // that marks its object rather than failing the whole line
        let Ok(raw) = serde_json::from_value::<RawWireObject>(value) else {
            return Ok(Self {
                operation_type: OperationType::Unknown(-1),
                entity_type: None,
                payload: Properties::Unknown(BTreeMap::new()),
            });
        };
        let properties = raw.properties;
        let parsed = WireObject::properties_from(
            raw.operation_type,
            raw.entity_type.as_ref(),
            properties.clone(),
        );
        // a payload of a known kind that does not parse is kept opaque
        // the fold marks its object rather than failing whole
        let payload = match parsed {
            Ok(payload) => payload,
            Err(_) => Properties::Unknown(properties),
        };
        Ok(Self {
            operation_type: raw.operation_type,
            entity_type: raw.entity_type,
            payload,
        })
    }
}

fn parse_props_from_map<T: DeserializeOwned>(
    properties: BTreeMap<String, Value>,
) -> Result<T, serde_json::Error> {
    serde_json::from_value(Value::Object(
        properties
            .into_iter()
            .collect::<serde_json::Map<String, Value>>(),
    ))
}

fn to_map<T: Serialize>(value: &T) -> BTreeMap<String, Value> {
    match serde_json::to_value(value) {
        Ok(Value::Object(map)) => map.into_iter().collect(),
        _ => BTreeMap::new(),
    }
}

/// operation type for wire field `t`
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, FromPrimitive, IntoPrimitive,
)]
#[repr(i32)]
#[serde(from = "i32", into = "i32")]
pub enum OperationType {
    /// full snapshot/create
    ///
    /// replace the object's current state
    Create = 0,
    /// partial update
    ///
    /// merge `p` into existing properties
    Update = 1,
    /// deletion event
    Delete = 2,

    /// unknown operation value preserved for forward compatibility
    #[num_enum(catch_all)]
    Unknown(i32),
}

/// entity type for wire field `e`
///
/// versioned by Things, `Task6` or `Area3` for instance
///
/// a version this CLI does not know parses as unknown
/// the `Task3`, `Task4`, `Area2` and first `Tombstone` kinds of histories from before base58 ids are among them
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Display, EnumString)]
#[serde(from = "String", into = "String")]
pub enum EntityType {
    /// task/project/heading entity (previous version)
    Task6,
    /// task/project/heading entity (current version)
    Task7,

    /// checklist item entity (first version)
    ChecklistItem,
    /// checklist item entity (earlier version)
    ChecklistItem2,
    /// checklist item entity (current observed version)
    ChecklistItem3,

    /// tag entity (earlier version)
    Tag3,
    /// tag entity (current observed version)
    Tag4,

    /// area entity (current observed version)
    Area3,

    /// settings entity
    Settings3,
    Settings4,
    Settings5,

    /// tombstone marker for deleted objects
    Tombstone2,

    /// one-shot command entity
    Command,
    /// unknown entity name preserved for forward compatibility
    #[strum(default, to_string = "{0}")]
    Unknown(String),
}

impl EntityType {
    pub fn is_task(&self) -> bool {
        matches!(self, Self::Task6 | Self::Task7)
    }

    /// the kinds the store keeps as typed objects
    ///
    /// their payloads must parse for the object to be whole
    pub fn is_stored(&self) -> bool {
        matches!(
            self,
            Self::Task6
                | Self::Task7
                | Self::ChecklistItem
                | Self::ChecklistItem2
                | Self::ChecklistItem3
                | Self::Tag3
                | Self::Tag4
                | Self::Area3
        )
    }

    pub fn can_upgrade_to_task7(&self) -> bool {
        matches!(self, Self::Task6 | Self::Task7)
    }

    /// a task of any version, another one read with the latest known schema
    pub fn is_task_family(&self) -> bool {
        self.is_task() || self.is_version_of("Task")
    }

    /// a checklist item of any version, another one read with the latest known schema
    pub fn is_checklist_family(&self) -> bool {
        matches!(
            self,
            Self::ChecklistItem | Self::ChecklistItem2 | Self::ChecklistItem3
        ) || self.is_version_of("ChecklistItem")
    }

    /// a tag of any version, another one read with the latest known schema
    pub fn is_tag_family(&self) -> bool {
        matches!(self, Self::Tag3 | Self::Tag4) || self.is_version_of("Tag")
    }

    /// an area of any version, another one read with the latest known schema
    pub fn is_area_family(&self) -> bool {
        matches!(self, Self::Area3) || self.is_version_of("Area")
    }

    /// a version this CLI does not know of a kind it stores
    pub fn is_other_stored_version(&self) -> bool {
        !self.is_stored()
            && (self.is_task_family()
                || self.is_checklist_family()
                || self.is_tag_family()
                || self.is_area_family())
    }

    /// a tombstone of a version this CLI does not read, the first `Tombstone` or one after `Tombstone2`
    pub fn is_other_tombstone_version(&self) -> bool {
        matches!(self, Self::Unknown(name) if name == "Tombstone")
            || self.is_version_of("Tombstone")
    }

    /// an unknown kind named `prefix` with a version number
    fn is_version_of(&self, prefix: &str) -> bool {
        let Self::Unknown(name) = self else {
            return false;
        };
        name.strip_prefix(prefix).is_some_and(|version| {
            !version.is_empty() && version.chars().all(|c| c.is_ascii_digit())
        })
    }
}

impl From<String> for EntityType {
    fn from(value: String) -> Self {
        value.parse().unwrap_or(Self::Unknown(value))
    }
}

impl From<EntityType> for String {
    fn from(value: EntityType) -> Self {
        value.to_string()
    }
}
