use std::sync::Arc;

use iocraft::prelude::*;

use crate::{
    common::ICONS,
    store::{Task, ThingsStore},
    ui::components::{
        empty_text::EmptyText,
        project_item::ProjectItem,
        task_item::TaskItem,
        tasks::{TaskList, TaskOptions},
    },
};

const LIST_INDENT: u32 = 4;

#[derive(Default, Props)]
pub struct TrashViewProps<'a> {
    /// each entry of the Trash with the to-dos a trashed project took along
    pub entries: Vec<(&'a Task, Vec<&'a Task>)>,
    pub detailed: bool,
}

#[component]
pub fn TrashView<'a>(hooks: Hooks, props: &TrashViewProps<'a>) -> impl Into<AnyElement<'a>> {
    let store = hooks.use_context::<Arc<ThingsStore>>().clone();

    if props.entries.is_empty() {
        return element! { EmptyText(content: "Trash is empty.") }.into_any();
    }

    let ids = props
        .entries
        .iter()
        .flat_map(|(entry, held)| std::iter::once(*entry).chain(held.iter().copied()))
        .map(|task| task.uuid.clone())
        .collect::<Vec<_>>();
    let id_prefix_len = store.unique_prefix_length(&ids);
    let count = ids.len();
    let label = if count == 1 { "item" } else { "items" };

    let options = TaskOptions {
        detailed: props.detailed,
        show_project: true,
        show_area: false,
        show_today_markers: false,
        show_staged_today_marker: false,
    };
    let held_options = TaskOptions {
        show_project: false,
        ..options
    };

    let mut body: Vec<AnyElement<'a>> = Vec::new();
    for (entry, held) in &props.entries {
        let line = if entry.is_project() {
            element! {
                View(flex_direction: FlexDirection::Column, padding_left: 2) {
                    ProjectItem(project: *entry, options, id_prefix_len)
                    View(flex_direction: FlexDirection::Column, padding_left: LIST_INDENT) {
                        TaskList(items: held.clone(), id_prefix_len, options: held_options)
                    }
                }
            }
            .into_any()
        } else {
            element! {
                View(flex_direction: FlexDirection::Column, padding_left: 2) {
                    TaskItem(task: *entry, options, id_prefix_len)
                }
            }
            .into_any()
        };
        body.push(line);
    }

    element! {
        View(flex_direction: FlexDirection::Column) {
            Text(
                content: format!("{} Trash  ({} {})", ICONS.deleted, count, label),
                wrap: TextWrap::NoWrap,
                color: Color::DarkGrey,
                weight: Weight::Bold,
            )
            Text(content: "", wrap: TextWrap::NoWrap)
            #(body)
        }
    }
    .into_any()
}
