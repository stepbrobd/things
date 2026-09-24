use std::collections::{BTreeMap, HashMap};

use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand};
use iocraft::prelude::*;
use serde_json::json;

use crate::{
    app::Cli,
    commands::{Command, write_json},
    common::{DIM, GREEN, ICONS, colored, counted, one_line, resolve_single_tag},
    store::Tag,
    ui::{
        render_element_to_string,
        views::{json::common::build_tags_json, tags::TagsView},
    },
    wire::{
        area::AreaPatch,
        tags::{TagPatch, TagProps},
        task::TaskPatch,
        wire_object::{EntityType, WireObject},
    },
};

#[derive(Debug, Subcommand)]
pub enum TagsSubcommand {
    #[command(about = "Show all tags")]
    List(TagsListArgs),
    #[command(about = "Create a new tag")]
    New(TagsNewArgs),
    #[command(about = "Rename or reparent a tag")]
    Edit(TagsEditArgs),
    #[command(about = "Delete a tag")]
    Delete(TagsDeleteArgs),
}

#[derive(Debug, Args)]
#[command(about = "Show, create, edit, or delete tags")]
pub struct TagsArgs {
    #[command(subcommand)]
    pub command: Option<TagsSubcommand>,
}

#[derive(Debug, Default, Args)]
pub struct TagsListArgs {}

#[derive(Debug, Args)]
pub struct TagsNewArgs {
    /// Tag title
    pub name: String,
    #[arg(long, short = 'p', help = "Parent tag title or UUID/prefix")]
    pub parent: Option<String>,
}

#[derive(Debug, Args)]
pub struct TagsEditArgs {
    /// Tag title or UUID/prefix
    pub tag_id: String,
    #[arg(long, short = 'n', help = "Replace tag title")]
    pub name: Option<String>,
    #[arg(long = "move", short = 'm', help = "Move under another tag or clear")]
    pub move_target: Option<String>,
}

#[derive(Debug, Args)]
pub struct TagsDeleteArgs {
    /// Tag title or UUID/prefix
    pub tag_id: String,
}

#[derive(Debug, Clone)]
struct TagsEditPlan {
    tag: crate::store::Tag,
    update: TagPatch,
    labels: Vec<String>,
}

/// the tag's delete and, in the same commit as the app writes it, the tag taken off every to-do and area that carries it
fn build_tags_delete_plan(
    identifier: &str,
    store: &crate::store::ThingsStore,
) -> std::result::Result<(Tag, BTreeMap<String, WireObject>), String> {
    let (tag, err) = resolve_single_tag(store, identifier);
    let Some(tag) = tag else {
        return Err(err);
    };
    // the app's handling of child tags is not captured
    // child tags are left to the user
    if store
        .tags_by_uuid
        .values()
        .any(|child| child.parent_uuid.as_ref() == Some(&tag.uuid))
    {
        return Err(format!(
            "{} has child tags, move or delete them first.",
            one_line(&tag.title)
        ));
    }
    let without = |tags: &[crate::ids::ThingsId]| {
        tags.iter()
            .filter(|id| **id != tag.uuid)
            .cloned()
            .collect::<Vec<_>>()
    };
    let mut changes = BTreeMap::new();
    changes.insert(tag.uuid.to_string(), WireObject::delete(EntityType::Tag4));
    for task in store
        .tasks_by_uuid
        .values()
        .filter(|task| task.tags.contains(&tag.uuid))
    {
        changes.insert(
            task.uuid.to_string(),
            WireObject::update(
                EntityType::Task7,
                TaskPatch {
                    tag_ids: Some(without(&task.tags)),
                    ..Default::default()
                },
            ),
        );
    }
    for area in store
        .areas_by_uuid
        .values()
        .filter(|area| area.tags.contains(&tag.uuid))
    {
        changes.insert(
            area.uuid.to_string(),
            WireObject::update(
                EntityType::Area3,
                AreaPatch {
                    tag_ids: Some(without(&area.tags)),
                    ..Default::default()
                },
            ),
        );
    }
    Ok((tag, changes))
}

fn build_tags_edit_plan(
    args: &TagsEditArgs,
    store: &crate::store::ThingsStore,
    now: f64,
) -> std::result::Result<TagsEditPlan, String> {
    let (tag, err) = resolve_single_tag(store, &args.tag_id);
    let Some(tag) = tag else {
        return Err(err);
    };

    let mut update = TagPatch::default();
    let mut labels = Vec::new();

    if let Some(name) = &args.name {
        let name = name.trim();
        if name.is_empty() {
            return Err("Tag name cannot be empty.".to_string());
        }
        // a title is how a tag is named on the command line
        // two that differ in case alone name neither
        if store
            .tags_by_uuid
            .values()
            .any(|other| other.uuid != tag.uuid && other.title.eq_ignore_ascii_case(name))
        {
            return Err(format!("A tag named {name} exists already."));
        }
        update.title = Some(name.to_string());
        labels.push("name".to_string());
    }

    if let Some(move_target) = &args.move_target {
        let move_raw = move_target.trim();
        if move_raw.eq_ignore_ascii_case("clear") {
            update.parent_ids = Some(vec![]);
            labels.push("move=clear".to_string());
        } else {
            let (parent, err) = resolve_single_tag(store, move_raw);
            let Some(parent) = parent else {
                return Err(err);
            };
            if parent.uuid == tag.uuid {
                return Err("A tag cannot be its own parent.".to_string());
            }
            // a parent below the tag would close a cycle that no walk from the roots reaches
            if store.tag_ancestors(&parent.uuid).contains(&tag.uuid) {
                return Err(format!(
                    "Cannot move {} under {}, which is below it.",
                    one_line(&tag.title),
                    one_line(&parent.title)
                ));
            }
            let parent_id = parent.uuid;
            update.parent_ids = Some(vec![parent_id]);
            labels.push(format!("move={move_raw}"));
        }
    }

    if update.is_empty() {
        return Err("No edit changes requested.".to_string());
    }

    update.modification_date = Some(now);

    Ok(TagsEditPlan {
        tag,
        update,
        labels,
    })
}

impl Command for TagsArgs {
    fn run_with_ctx(
        &self,
        cli: &Cli,
        out: &mut dyn std::io::Write,
        ctx: &mut dyn crate::cmd_ctx::CmdCtx,
    ) -> Result<()> {
        match self
            .command
            .as_ref()
            .unwrap_or(&TagsSubcommand::List(TagsListArgs::default()))
        {
            TagsSubcommand::List(_) => {
                let store = cli.load_store()?;
                let tags = store.tags();

                if cli.json {
                    write_json(out, &build_tags_json(&tags, &store))?;
                    return Ok(());
                }

                let by_uuid: HashMap<_, _> =
                    tags.iter().map(|t| (t.uuid.clone(), t.clone())).collect();
                let mut children: BTreeMap<_, Vec<_>> = BTreeMap::new();
                let mut top_level = Vec::new();

                for tag in tags {
                    // a tag whose parent chain comes back to itself, its own parent included, shows at the top level rather than nowhere
                    let attached = tag.parent_uuid.as_ref().filter(|parent| {
                        **parent != tag.uuid
                            && by_uuid.contains_key(*parent)
                            && !store.tag_ancestors(parent).contains(&tag.uuid)
                    });
                    match attached {
                        Some(parent_uuid) => {
                            children.entry(parent_uuid.clone()).or_default().push(tag);
                        }
                        None => top_level.push(tag),
                    }
                }

                let mut ui = element! {
                    TagsView(tags_count: by_uuid.len(), top_level, children)
                };
                let rendered = render_element_to_string(&mut ui, cli.no_color());
                writeln!(out, "{}", rendered)?;
            }
            TagsSubcommand::New(args) => {
                let name = args.name.trim();
                if name.is_empty() {
                    bail!("Tag name cannot be empty.");
                }

                let store = cli.load_store()?;
                if store
                    .tags_by_uuid
                    .values()
                    .any(|tag| tag.title.eq_ignore_ascii_case(name))
                {
                    bail!("A tag named {name} exists already.");
                }
                let mut props = TagProps {
                    title: name.to_string(),
                    sort_index: 0,
                    conflict_overrides: Some(json!({"_t": "oo", "sn": {}})),
                    ..Default::default()
                };

                if let Some(parent_raw) = &args.parent {
                    let (parent, err) = resolve_single_tag(&store, parent_raw);
                    let Some(parent) = parent else {
                        bail!("{err}");
                    };
                    props.parent_ids = vec![parent.uuid];
                }

                let uuid = ctx.next_id();
                let mut changes = BTreeMap::new();
                changes.insert(uuid.clone(), WireObject::create(EntityType::Tag4, props));
                ctx.commit_changes(changes)
                    .with_context(|| "Failed to create tag")?;

                writeln!(
                    out,
                    "{} {}  {}",
                    colored(format!("{} Created", ICONS.done), &[GREEN], cli.no_color()),
                    one_line(name),
                    colored(&uuid, &[DIM], cli.no_color())
                )?;
            }
            TagsSubcommand::Edit(args) => {
                let store = cli.load_store()?;
                let plan = build_tags_edit_plan(args, &store, ctx.now_timestamp())
                    .map_err(anyhow::Error::msg)?;

                let mut changes = BTreeMap::new();
                changes.insert(
                    plan.tag.uuid.to_string(),
                    WireObject::update(EntityType::Tag4, plan.update.clone()),
                );
                ctx.commit_changes(changes)
                    .with_context(|| "Failed to edit tag")?;

                let name = plan.update.title.as_deref().unwrap_or(&plan.tag.title);
                writeln!(
                    out,
                    "{} {}  {} {}",
                    colored(format!("{} Edited", ICONS.done), &[GREEN], cli.no_color()),
                    one_line(name),
                    colored(&plan.tag.uuid, &[DIM], cli.no_color()),
                    colored(
                        format!("({})", plan.labels.join(", ")),
                        &[DIM],
                        cli.no_color()
                    )
                )?;
            }
            TagsSubcommand::Delete(args) => {
                let store = cli.load_store()?;
                let (tag, changes) =
                    build_tags_delete_plan(&args.tag_id, &store).map_err(anyhow::Error::msg)?;
                let carriers = changes.len() - 1;
                ctx.commit_changes(changes)
                    .with_context(|| "Failed to delete tag")?;

                let from = if carriers > 0 {
                    colored(
                        format!("  (from {})", counted(carriers, "item")),
                        &[DIM],
                        cli.no_color(),
                    )
                } else {
                    String::new()
                };
                writeln!(
                    out,
                    "{} {}  {}{}",
                    colored(
                        format!("{} Deleted", ICONS.deleted),
                        &[GREEN],
                        cli.no_color()
                    ),
                    one_line(&tag.title),
                    colored(&tag.uuid, &[DIM], cli.no_color()),
                    from
                )?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        ids::ThingsId,
        store::{ThingsStore, fold_items},
        wire::{
            tags::TagProps,
            wire_object::{EntityType, WireItem, WireObject},
        },
    };

    const NOW: f64 = 1_700_000_222.0;
    const TAG_UUID: &str = "WukwpDdL5Z88nX3okGMKTC";
    const CHILD_UUID: &str = "JiqwiDaS3CAyjCmHihBDnB";

    fn build_store(entries: Vec<(String, WireObject)>) -> ThingsStore {
        let mut item: WireItem = BTreeMap::new();
        for (uuid, obj) in entries {
            item.insert(uuid, obj);
        }
        ThingsStore::from_raw_state(&fold_items([item]))
    }

    fn tag(uuid: &str, title: &str, parent: Option<&str>) -> (String, WireObject) {
        (
            uuid.to_string(),
            WireObject::create(
                EntityType::Tag4,
                TagProps {
                    title: title.to_string(),
                    sort_index: 0,
                    parent_ids: parent
                        .map(|p| {
                            vec![
                                p.parse::<ThingsId>()
                                    .expect("test parent id should parse as ThingsId"),
                            ]
                        })
                        .unwrap_or_default(),
                    ..Default::default()
                },
            ),
        )
    }

    #[test]
    fn a_deleted_tag_comes_off_everything_that_carries_it() {
        const TASK: &str = "A7h5eCi24RvAWKC3Hv3muf";
        const AREA: &str = "MpkEei6ybkFS2n6SXvwfLf";
        const OTHER: &str = "Bt11111111111111111111";
        let tagged = |ids: &[&str]| {
            ids.iter()
                .map(|id| id.parse::<ThingsId>().expect("id"))
                .collect::<Vec<_>>()
        };
        let store = build_store(vec![
            tag(TAG_UUID, "Errand", None),
            tag(OTHER, "Home", None),
            (
                TASK.to_string(),
                WireObject::create(
                    EntityType::Task7,
                    crate::wire::task::TaskProps {
                        title: "Buy milk".to_string(),
                        tag_ids: tagged(&[TAG_UUID, OTHER]),
                        ..Default::default()
                    },
                ),
            ),
            (
                AREA.to_string(),
                WireObject::create(
                    EntityType::Area3,
                    crate::wire::area::AreaProps {
                        title: "Chores".to_string(),
                        tag_ids: tagged(&[TAG_UUID]),
                        ..Default::default()
                    },
                ),
            ),
        ]);
        let (_, changes) = build_tags_delete_plan("Errand", &store).expect("plan");
        assert_eq!(
            serde_json::to_value(&changes).expect("json"),
            json!({
                TAG_UUID: {"t": 2, "e": "Tag4", "p": {}},
                TASK: {"t": 1, "e": "Task7", "p": {"tg": [OTHER]}},
                AREA: {"t": 1, "e": "Area3", "p": {"tg": []}}
            })
        );

        let parent = build_store(vec![
            tag(TAG_UUID, "Work", None),
            tag(CHILD_UUID, "Meetings", Some(TAG_UUID)),
        ]);
        let err = build_tags_delete_plan("Work", &parent).expect_err("a child tag");
        assert!(err.contains("has child tags"), "{err}");
    }

    #[test]
    fn tags_edit_payloads_and_errors() {
        let store = build_store(vec![
            tag(TAG_UUID, "Work", None),
            tag(CHILD_UUID, "Meetings", Some(TAG_UUID)),
        ]);

        let rename = build_tags_edit_plan(
            &TagsEditArgs {
                tag_id: TAG_UUID.to_string(),
                name: Some("Work Stuff".to_string()),
                move_target: None,
            },
            &store,
            NOW,
        )
        .expect("rename");
        let p = serde_json::to_value(&rename.update).expect("patch");
        assert_eq!(p.get("tt"), Some(&json!("Work Stuff")));
        assert_eq!(p.get("md"), Some(&json!(NOW)));

        // another tag's title in other case would leave both unnamed on the command line
        let clash = build_tags_edit_plan(
            &TagsEditArgs {
                tag_id: CHILD_UUID.to_string(),
                name: Some("work".to_string()),
                move_target: None,
            },
            &store,
            NOW,
        )
        .expect_err("a clash");
        assert!(clash.contains("exists already"), "{clash}");

        let reparent = build_tags_edit_plan(
            &TagsEditArgs {
                tag_id: CHILD_UUID.to_string(),
                name: None,
                move_target: Some(TAG_UUID.to_string()),
            },
            &store,
            NOW,
        )
        .expect("reparent");
        assert_eq!(
            serde_json::to_value(&reparent.update)
                .expect("patch")
                .get("pn"),
            Some(&json!([TAG_UUID]))
        );

        let clear = build_tags_edit_plan(
            &TagsEditArgs {
                tag_id: CHILD_UUID.to_string(),
                name: None,
                move_target: Some("clear".to_string()),
            },
            &store,
            NOW,
        )
        .expect("clear");
        assert_eq!(
            serde_json::to_value(&clear.update)
                .expect("patch")
                .get("pn"),
            Some(&json!([]))
        );

        let no_change = build_tags_edit_plan(
            &TagsEditArgs {
                tag_id: TAG_UUID.to_string(),
                name: None,
                move_target: None,
            },
            &store,
            NOW,
        )
        .expect_err("no changes");
        assert_eq!(no_change, "No edit changes requested.");

        let self_parent = build_tags_edit_plan(
            &TagsEditArgs {
                tag_id: TAG_UUID.to_string(),
                name: None,
                move_target: Some(TAG_UUID.to_string()),
            },
            &store,
            NOW,
        )
        .expect_err("self parent");
        assert_eq!(self_parent, "A tag cannot be its own parent.");

        // Meetings is below Work
        // that keeps Work from going under Meetings
        let cycle = build_tags_edit_plan(
            &TagsEditArgs {
                tag_id: TAG_UUID.to_string(),
                name: None,
                move_target: Some(CHILD_UUID.to_string()),
            },
            &store,
            NOW,
        )
        .expect_err("cycle");
        assert_eq!(cycle, "Cannot move Work under Meetings, which is below it.");
    }

    #[test]
    fn a_cycle_two_tags_already_form_is_rejected_and_bounded() {
        let store = build_store(vec![
            tag(TAG_UUID, "Work", Some(CHILD_UUID)),
            tag(CHILD_UUID, "Meetings", Some(TAG_UUID)),
        ]);
        assert_eq!(
            store.tag_ancestors(&TAG_UUID.parse().expect("id")),
            vec![CHILD_UUID.parse().expect("id")]
        );
        let err = build_tags_edit_plan(
            &TagsEditArgs {
                tag_id: TAG_UUID.to_string(),
                name: None,
                move_target: Some(CHILD_UUID.to_string()),
            },
            &store,
            NOW,
        )
        .expect_err("still a cycle");
        assert_eq!(err, "Cannot move Work under Meetings, which is below it.");
    }
}
