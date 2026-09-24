use iocraft::prelude::*;

use crate::{
    common::{ICONS, shown_title},
    ids::ThingsId,
    store::Task,
    ui::components::{
        empty_text::EmptyText,
        id::Id,
        tasks::{TaskList, TaskOptions},
    },
};

pub struct ProjectsAreaGroup {
    pub area_uuid: ThingsId,
    pub area_title: String,
    pub projects: Vec<Task>,
}

#[derive(Default, Props)]
pub struct ProjectsViewProps {
    pub projects_count: usize,
    pub no_area_projects: Vec<Task>,
    pub area_groups: Vec<ProjectsAreaGroup>,
    pub detailed: bool,
    pub id_prefix_len: usize,
}

#[component]
pub fn ProjectsView<'a>(props: &'a ProjectsViewProps) -> impl Into<AnyElement<'a>> {
    if props.projects_count == 0 {
        return element! { EmptyText(content: "No incomplete projects.") }.into_any();
    }

    let options = TaskOptions {
        detailed: props.detailed,
        show_project: false,
        show_area: false,
        show_today_markers: true,
        show_staged_today_marker: false,
    };

    let free_projects = if !props.no_area_projects.is_empty() {
        Some(element! {
            View(flex_direction: FlexDirection::Column, padding_left: 2) {
                TaskList(
                    items: props.no_area_projects.iter().collect::<Vec<_>>(),
                    id_prefix_len: props.id_prefix_len,
                    options,
                )
            }
        })
    } else {
        None
    };

    let project_areas = props.area_groups.iter().map(|group| {
        element! {
            ProjectsAreaSection(group, id_prefix_len: props.id_prefix_len, options)
        }
    });

    element! {
        View(flex_direction: FlexDirection::Column, gap: 1) {
            Text(
                content: format!("● Projects  ({})", props.projects_count),
                color: Color::Green,
                weight: Weight::Bold,
                wrap: TextWrap::NoWrap,
            )
            #(free_projects)
            #(project_areas)
        }
    }
    .into_any()
}

#[derive(Default, Props)]
struct ProjectsAreaSectionProps<'a> {
    pub group: Option<&'a ProjectsAreaGroup>,
    pub id_prefix_len: usize,
    pub options: TaskOptions,
}

#[component]
fn ProjectsAreaSection<'a>(props: &ProjectsAreaSectionProps<'a>) -> impl Into<AnyElement<'a>> {
    let Some(group) = props.group else {
        return element!(Fragment).into_any();
    };

    element! {
        View(flex_direction: FlexDirection::Column, padding_left: 2) {
            View(flex_direction: FlexDirection::Row, gap: 1) {
                Id(id: &group.area_uuid, length: props.id_prefix_len)
                Text(content: ICONS.area, color: Color::DarkGrey)
                Text(content: shown_title(&group.area_title), wrap: TextWrap::NoWrap, weight: Weight::Bold)
            }
            View(flex_direction: FlexDirection::Column, padding_left: 2) {
                TaskList(
                    items: group.projects.iter().collect::<Vec<_>>(),
                    id_prefix_len: props.id_prefix_len,
                    options: props.options,
                )
            }
        }
    }
    .into_any()
}
