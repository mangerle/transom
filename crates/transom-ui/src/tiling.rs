//! 双标签并排视图：两个标签组同屏显示的状态机与标签动作。
//!
//! 设计原理（Why）：只做左右/上下两格，而不是任意分割树。两格已经覆盖
//! "对照两台机器"的主场景，也不需要为第三格及以后定义插入点与跨格焦点遍历；
//! 标签组内部仍可自由分屏，两个能力叠加。
//!
//! 与标签内分屏的边界：这里操作的对象是标签组，`LayoutNode` 一行不改。
//! 代价是同一时刻只能并排两个标签组；收益是关闭标签、删除会话、恢复快照
//! 都继续沿用"一个标签一个目标"的原语义，不需要为混合目标补规则。

use crate::app::AppShell;
use crate::pane_tree::PaneDirection;
use gpui_kit::*;
use transom_core::{GroupId, SavedTabTiling, SplitAxis};

/// 并排视图中的格位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TileSide {
    /// 前侧：左右并排时在左，上下堆叠时在上。
    First,
    /// 后侧：左右并排时在右，上下堆叠时在下。
    Second,
}

/// 当前窗口的双标签并排绑定。
///
/// 两个格位始终各自绑定一个标签组，不存在"空格子"：进入并排时两格同时绑定，
/// 关闭其中任一标签后只剩一个可见组，视图随即退回单标签，避免留下无内容的
/// 空位或让用户误以为旁边的标签也被关闭。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TiledTabs {
    /// 前侧标签组。
    pub(crate) first: GroupId,
    /// 后侧标签组。
    pub(crate) second: GroupId,
    /// 两格的排列方向。
    pub(crate) axis: SplitAxis,
    /// 当前焦点所在格。
    pub(crate) focused: TileSide,
}

impl TiledTabs {
    /// 标签组是否正在并排视图中显示。
    pub(crate) fn contains(self, group: GroupId) -> bool {
        self.side_of(group).is_some()
    }

    /// 标签组所在的格位；未显示时返回 `None`。
    pub(crate) fn side_of(self, group: GroupId) -> Option<TileSide> {
        if group == self.first {
            Some(TileSide::First)
        } else if group == self.second {
            Some(TileSide::Second)
        } else {
            None
        }
    }

    /// 取某个格位绑定的标签组。
    pub(crate) fn group(self, side: TileSide) -> GroupId {
        match side {
            TileSide::First => self.first,
            TileSide::Second => self.second,
        }
    }

    /// 另一格绑定的标签组；`group` 不在视图中时返回 `None`。
    pub(crate) fn other_group(self, group: GroupId) -> Option<GroupId> {
        match self.side_of(group)? {
            TileSide::First => Some(self.second),
            TileSide::Second => Some(self.first),
        }
    }
}

impl AppShell {
    /// 把右键的标签与当前活动标签并排，右键的标签去 `axis` 指定的那一侧。
    ///
    /// 配对规则始终是"当前活动标签 + 右键的标签"：
    /// - 右键的不是活动标签：活动标签留前格，右键的标签进后格（右/下）；
    /// - 右键的就是活动标签：另一方取并排对里的另一个标签；未并排时取标签栏里
    ///   紧随其后的标签（末位回到首位），它留前格，当前标签按方向进后格。
    ///
    /// 方向描述的是"右键的标签去哪一边"，不是"活动标签去哪一边"；焦点跟随
    /// 右键的标签，用户刚点的那个标签不会在另一侧亮起。
    pub(crate) fn tile_tab(
        &mut self,
        target: GroupId,
        axis: SplitAxis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target_index) = self.groups.iter().position(|group| group.id == target) else {
            return;
        };
        let target_pane = self.groups[target_index].active_pane;
        let Some(active_group) = self
            .active_tab
            .and_then(|tab| self.group_index_for_tab(tab))
            .map(|index| self.groups[index].id)
        else {
            self.activate_tab(target_pane, window, cx);
            return;
        };

        let partner = if target != active_group {
            Some(active_group)
        } else if let Some(other) = self.tiled_tabs.and_then(|tiled| tiled.other_group(target)) {
            Some(other)
        } else {
            self.groups
                .get(target_index + 1)
                .or_else(|| self.groups.first())
                .map(|group| group.id)
        };
        let Some(partner) = partner else {
            return;
        };
        if partner == target {
            return;
        }
        // 前格放搭档，后格放右键的标签；焦点跟随后者。
        self.tiled_tabs = Some(TiledTabs {
            first: partner,
            second: target,
            axis,
            focused: TileSide::Second,
        });
        self.activate_tab(target_pane, window, cx);
    }

    /// 退出并排视图，只改变显示方式：两个标签组都保持打开与连接。
    pub(crate) fn untile_tabs(&mut self, cx: &mut Context<Self>) {
        if self.tiled_tabs.take().is_some() {
            cx.notify();
        }
    }

    /// 激活某个标签组（通常来自并排格标题栏或点击格内容）。
    pub(crate) fn activate_group(
        &mut self,
        group: GroupId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.groups.iter().position(|item| item.id == group) else {
            return;
        };
        let pane = self.groups[index].active_pane;
        self.activate_tab(pane, window, cx);
    }

    /// 同步活动标签与并排对的焦点关系，不改变配对本身。
    ///
    /// 并排对是持久的：新建会话、点开一个不在对里的标签时，活动标签单屏显示，
    /// 配对原样保留；再点回对里的任一标签就恢复两格同屏。新建连接、重连等路径
    /// 直接写 `active_tab` 而不经过 `activate_tab`，写完调用本方法补齐焦点格。
    pub(crate) fn present_active_group(&mut self) {
        let Some(tiled) = self.tiled_tabs else {
            return;
        };
        let Some(group) = self
            .active_tab
            .and_then(|tab| self.group_index_for_tab(tab))
            .map(|index| self.groups[index].id)
        else {
            return;
        };
        if let Some(side) = tiled.side_of(group)
            && tiled.focused != side
        {
            self.tiled_tabs = Some(TiledTabs {
                focused: side,
                ..tiled
            });
        }
    }

    /// 校验并排绑定是否仍然有效；标签组被删除或重复绑定时解除配对。
    ///
    /// 活动标签不在配对里不代表配对失效：那只是暂时单屏查看别的标签。
    /// 因此这里只清理失效的组，不把焦点强行拉回并排格。
    pub(crate) fn prune_tiling(&mut self) {
        let Some(tiled) = self.tiled_tabs else {
            return;
        };
        let first_alive = self.groups.iter().any(|group| group.id == tiled.first);
        let second_alive = self.groups.iter().any(|group| group.id == tiled.second);
        if !first_alive || !second_alive || tiled.first == tiled.second {
            self.tiled_tabs = None;
            return;
        }

        let active_group = self
            .active_tab
            .and_then(|tab| self.group_index_for_tab(tab))
            .map(|index| self.groups[index].id);
        if let Some(side) = active_group.and_then(|group| tiled.side_of(group))
            && tiled.focused != side
        {
            self.tiled_tabs = Some(TiledTabs {
                focused: side,
                ..tiled
            });
        }
    }

    /// 当前活动标签是否属于并排对，决定内容区渲染两格还是单屏。
    pub(crate) fn active_in_tiling(&self, tiled: TiledTabs) -> bool {
        self.active_tab
            .and_then(|tab| self.group_index_for_tab(tab))
            .map(|index| self.groups[index].id)
            .is_some_and(|group| tiled.contains(group))
    }

    /// 窗格方向键在组内已无邻居时，沿并排方向跨到另一格。
    ///
    /// 只有方向与排列轴一致、且当前组确实位于对应一侧时才跨格；否则保持不动，
    /// 与单标签视图里"到边就不动"的语义一致。
    pub(crate) fn focus_neighbor_tile(
        &mut self,
        direction: PaneDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tiled) = self.tiled_tabs else {
            return;
        };
        let Some(group_index) = self.active_group_index() else {
            return;
        };
        let group = self.groups[group_index].id;
        let next = match (tiled.axis, direction) {
            (SplitAxis::Horizontal, PaneDirection::Right) if group == tiled.first => {
                Some(TileSide::Second)
            }
            (SplitAxis::Horizontal, PaneDirection::Left) if group == tiled.second => {
                Some(TileSide::First)
            }
            (SplitAxis::Vertical, PaneDirection::Down) if group == tiled.first => {
                Some(TileSide::Second)
            }
            (SplitAxis::Vertical, PaneDirection::Up) if group == tiled.second => {
                Some(TileSide::First)
            }
            _ => None,
        };
        if let Some(side) = next {
            self.activate_group(tiled.group(side), window, cx);
        }
    }

    /// 按持久化快照恢复双标签并排视图。
    ///
    /// `restored` 与 `previous_tab_groups` 一一对应，元素为实际重建出的标签组；
    /// 任一下标越界、两个下标相同或对应标签组没有成功恢复时静默退回单标签，
    /// 不在启动阶段为一份损坏的视图快照额外拉起连接。
    pub(crate) fn restore_tab_tiling(
        &mut self,
        saved: SavedTabTiling,
        restored: &[Option<GroupId>],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if saved.first == saved.second {
            return;
        }
        let (Some(Some(first)), Some(Some(second))) =
            (restored.get(saved.first), restored.get(saved.second))
        else {
            return;
        };
        if !self.groups.iter().any(|group| group.id == *first)
            || !self.groups.iter().any(|group| group.id == *second)
        {
            return;
        }
        let tiled = TiledTabs {
            first: *first,
            second: *second,
            axis: saved.axis,
            focused: if saved.focused_second {
                TileSide::Second
            } else {
                TileSide::First
            },
        };
        let Some(pane) = self
            .groups
            .iter()
            .find(|group| group.id == tiled.group(tiled.focused))
            .map(|group| group.active_pane)
        else {
            return;
        };
        self.tiled_tabs = Some(tiled);
        self.activate_tab(pane, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{TileSide, TiledTabs};
    use transom_core::{GroupId, SplitAxis};

    fn sample() -> (TiledTabs, GroupId, GroupId) {
        let (first, second) = (GroupId::new(), GroupId::new());
        (
            TiledTabs {
                first,
                second,
                axis: SplitAxis::Horizontal,
                focused: TileSide::First,
            },
            first,
            second,
        )
    }

    #[test]
    fn side_of_maps_both_slots_and_rejects_background_groups() {
        let (tiled, first, second) = sample();
        assert_eq!(tiled.side_of(first), Some(TileSide::First));
        assert_eq!(tiled.side_of(second), Some(TileSide::Second));
        assert_eq!(tiled.side_of(GroupId::new()), None);
        assert!(tiled.contains(first));
        assert!(!tiled.contains(GroupId::new()));
    }

    #[test]
    fn group_maps_each_side() {
        let (tiled, first, second) = sample();
        assert_eq!(tiled.group(TileSide::First), first);
        assert_eq!(tiled.group(TileSide::Second), second);
    }

    #[test]
    fn other_group_returns_counterpart_only_for_visible_groups() {
        let (tiled, first, second) = sample();
        assert_eq!(tiled.other_group(first), Some(second));
        assert_eq!(tiled.other_group(second), Some(first));
        assert_eq!(tiled.other_group(GroupId::new()), None);
    }
}
