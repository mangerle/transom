//! 底部广播输入条：把一条命令或多行脚本同时发送到选定的终端窗格。
//!
//! 与快捷命令（Snippets）的边界：Snippets 只填当前焦点窗格，目标固定且单值；
//! 这里引入独立的目标集合，默认取当前标签的窗格，用户可显式多选其他标签。
//! 目标单位是窗格而不是标签：每个窗格是一条独立连接，标签并排或标签内分屏
//! 都只是显示方式，广播逻辑不感知布局。
//!
//! 单行模式回车发送；多行模式同样回车发送，`Shift+Enter` 换行。多行文本走
//! 终端当前的括号粘贴语义，避免远端 shell 把脚本的每一行提前执行。

use crate::app::AppState;
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::*;
use std::collections::HashSet;
use transom_core::{GroupId, TabId};

mod ops;
mod rows;

/// 广播条的运行期状态。
///
/// 目标集合只活在本次打开期间：目标跟着"当前视觉标签"切换，同一视觉标签内
/// 点窗格、切焦点不会重置用户手动勾选的结果。`targets_scope` 记录当前目标是
/// 按哪一组标签组算出来的，用于区分"真的换了标签"与"同一标签内换焦点"。
pub(crate) struct BroadcastBarState {
    pub(crate) open: bool,
    pub(crate) multiline: bool,
    pub(crate) targets_open: bool,
    pub(crate) input: Entity<InputState>,
    pub(crate) textarea: Entity<TextareaState>,
    pub(crate) targets: HashSet<TabId>,
    pub(crate) targets_scope: Vec<GroupId>,
}

impl AppState {
    /// 当前"视觉标签"里的标签组，顺序固定为并排对的前后格或单标签本身。
    ///
    /// 用它作为目标作用域：并排时是两个标签组（同一个合并条目），配对挂起或
    /// 未并排时只有当前标签。顺序不跟随焦点，避免在并排的两格间点来点去时
    /// 误判成切换了标签。
    fn current_visual_groups(&self) -> Vec<GroupId> {
        let Some(active_index) = self.active_group_index() else {
            return Vec::new();
        };
        let active_group = self.workspace.groups[active_index].id;
        if let Some(tiled) = self.active_tiling() {
            return tiled.occupied_groups();
        }
        vec![active_group]
    }

    /// 标签切换、并排关系变化后，把广播目标重置到新的视觉标签。
    ///
    /// 同一视觉标签内换焦点（点窗格、切并排格）不会改变作用域，因此不会
    /// 覆盖用户手动增删的目标。
    pub(crate) fn sync_broadcast_targets(&mut self) {
        if !self.features.broadcast.open {
            return;
        }
        let scope = self.current_visual_groups();
        let same = scope.len() == self.features.broadcast.targets_scope.len()
            && scope
                .iter()
                .all(|group| self.features.broadcast.targets_scope.contains(group));
        if same {
            return;
        }
        self.features.broadcast.targets = self.default_broadcast_targets_for(&scope);
        self.features.broadcast.targets_scope = scope;
    }

    /// 按给定的视觉标签作用域收集默认目标：只取已有终端的窗格。
    fn default_broadcast_targets_for(&self, groups: &[GroupId]) -> HashSet<TabId> {
        let mut targets = HashSet::with_capacity(self.workspace.tabs.len());
        for group_id in groups {
            let Some(group) = self
                .workspace
                .groups
                .iter()
                .find(|group| group.id == *group_id)
            else {
                continue;
            };
            for pane_id in group.pane_ids() {
                if self
                    .workspace
                    .tabs
                    .iter()
                    .any(|tab| tab.id == pane_id && tab.terminal().is_some())
                {
                    targets.insert(pane_id);
                }
            }
        }
        targets
    }
}

pub(crate) use ops::BroadcastOps;
pub(crate) use ops::set_broadcast_target;
