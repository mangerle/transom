//! 标签条目的多选与并排可用性计算。
//!
//! 设计原理（Why）：横向标签栏与纵向标签面板此前各复制一份“选中组集合 →
//! 可加入并排 / 可新建并排”的判断，两处条件一旦漂移，同一多选在两个入口
//! 就会给出不同动作。这里用纯函数统一计算，页面只消费结果。
//!
//! 代价与局限：只计算结果，不执行动作；动作仍由各入口按自身交互发起。

use super::state::WorkspaceState;
use crate::tiling::{MAX_TILING_SLOTS, TiledTabs};
use transom_core::GroupId;

/// 一个标签条目在当前多选状态下的并排能力。
pub(crate) struct TabSelectionModel {
    /// 该条目覆盖的分组：普通条目是自身，并排条目是整组。
    pub(crate) selection_groups: Vec<GroupId>,
    /// 条目内所有分组都被选中。
    pub(crate) entry_selected: bool,
    /// 未进入并排的已选分组。
    pub(crate) selected_singles: Vec<GroupId>,
    /// 已选分组触及的并排集合。
    pub(crate) touched_sets: Vec<crate::tiling::TilingId>,
    /// 可以加入当前并排集合：只选中了这一个集合，且还有未并排的已选条目。
    pub(crate) show_add_to_tiling: bool,
    /// 可以新建并排：没有触及任何并排集合，且已选独立条目数在 2..=上限。
    pub(crate) show_multi_tile: bool,
}

/// 计算条目在当前多选状态下的并排能力。
pub(crate) fn selection_model(
    workspace: &WorkspaceState,
    group_id: GroupId,
    tiled: Option<&TiledTabs>,
) -> TabSelectionModel {
    let selection_groups = match tiled {
        Some(tiled) => tiled.occupied_groups(),
        None => vec![group_id],
    };
    let entry_selected = !selection_groups.is_empty()
        && selection_groups
            .iter()
            .all(|group| workspace.tab_selection.contains(group));
    let selected_singles: Vec<GroupId> = workspace
        .groups
        .iter()
        .filter(|group| {
            workspace.tab_selection.contains(&group.id)
                && !workspace
                    .tiled_sets
                    .iter()
                    .any(|tiled| tiled.contains(group.id))
        })
        .map(|group| group.id)
        .collect();
    let touched_sets: Vec<crate::tiling::TilingId> = workspace
        .tiled_sets
        .iter()
        .filter(|tiled| {
            tiled
                .occupied_groups()
                .iter()
                .any(|group| workspace.tab_selection.contains(group))
        })
        .map(|tiled| tiled.id)
        .collect();
    let this_set_id = tiled.map(|tiled| tiled.id);
    let show_add_to_tiling = match (this_set_id, touched_sets.as_slice()) {
        (Some(id), [only]) if *only == id => !selected_singles.is_empty(),
        _ => false,
    };
    let show_multi_tile = entry_selected
        && touched_sets.is_empty()
        && (2..=MAX_TILING_SLOTS).contains(&selected_singles.len());
    TabSelectionModel {
        selection_groups,
        entry_selected,
        selected_singles,
        touched_sets,
        show_add_to_tiling,
        show_multi_tile,
    }
}

#[cfg(test)]
mod tests {
    use super::selection_model;
    use crate::app::TabGroup;
    use crate::app::workspace::WorkspaceState;
    use transom_core::{GroupId, SessionId, TabId};

    fn workspace_with_groups(count: usize) -> (WorkspaceState, Vec<GroupId>) {
        let mut workspace = WorkspaceState::new();
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            let pane = TabId::new();
            let group = TabGroup::new(pane, SessionId::new(), 0);
            ids.push(group.id);
            workspace.groups.push(group);
        }
        (workspace, ids)
    }

    /// 单组入选只标记选中，不出现“新建并排”。
    #[test]
    fn single_selection_is_not_tileable() {
        let (mut workspace, ids) = workspace_with_groups(2);
        workspace.tab_selection.insert(ids[0]);
        let model = selection_model(&workspace, ids[0], None);
        assert!(model.entry_selected);
        assert_eq!(model.selected_singles, vec![ids[0]]);
        assert!(!model.show_multi_tile);
        assert!(!model.show_add_to_tiling);
    }

    /// 两个独立分组入选时允许新建并排。
    #[test]
    fn two_singles_enable_multi_tile() {
        let (mut workspace, ids) = workspace_with_groups(2);
        workspace.tab_selection.insert(ids[0]);
        workspace.tab_selection.insert(ids[1]);
        let model = selection_model(&workspace, ids[0], None);
        assert_eq!(model.selected_singles.len(), 2);
        assert!(model.show_multi_tile);
    }
}
