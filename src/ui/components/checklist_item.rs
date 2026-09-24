use iocraft::prelude::*;

use crate::{
    common::{ICONS, one_line},
    store::ChecklistItem,
    ui::components::id::Id,
};

/// one checklist item row
///
/// with an `id` the id takes a fixed-width left column
/// the connector follows it
///
/// ```text
/// M ├╴○ Confirm changelog
/// J └╴● Tag release commit   (is_last)
/// ```
///
/// without one the connector starts at the first column
///
/// ```text
/// ├╴○ title
/// └╴● title
/// ```

#[derive(Default, Props)]
pub struct CheckListRowProps<'a> {
    pub item: Option<&'a ChecklistItem>,
    pub id_prefix_len: usize,
    pub is_last: bool,
}

#[component]
pub fn CheckListRow<'a>(props: &CheckListRowProps<'a>) -> impl Into<AnyElement<'a>> {
    let Some(item) = props.item else {
        return element!(Fragment).into_any();
    };

    let connector = if props.is_last { "└╴" } else { "├╴" };

    let id = if props.id_prefix_len > 0 {
        element!(Id(id: &item.uuid, length: props.id_prefix_len)).into_any()
    } else {
        element!(Fragment).into_any()
    };

    element!(View {
        View(flex_direction: FlexDirection::Row, gap: 1) {
            #(id)
            Text(content: connector, color: Color::DarkGrey)
        }
        View(flex_direction: FlexDirection::Row, gap: 1) {
            Text(content: checklist_icon(item), color: Color::DarkGrey)
            Text(content: one_line(&item.title))
        }
    })
    .into_any()
}

fn checklist_icon(item: &ChecklistItem) -> &'static str {
    if item.is_completed() {
        ICONS.checklist_done
    } else if item.is_canceled() {
        ICONS.checklist_canceled
    } else {
        ICONS.checklist_open
    }
}
