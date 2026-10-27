//! TiledTabs 结构体及其槽位管理方法。

use super::layout::{build_tiling_grid, expand_grid, window_aspect};
use super::types::{MAX_TILING_SLOTS, TilingId, TilingNode};
use crate::pane_tree::PaneDirection;
use gpui_kit::*;
use std::collections::HashSet;
use transom_core::GroupId;

/// 当前窗口的一张并排网格。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TiledTabs {
    /// 并排组身份；同一窗口可并存多张网格。
    pub(crate) id: TilingId,
    /// 网格布局树；叶子是 `slots` 的下标，`Stack` 是同一方向的一组格子。
    pub(crate) root: TilingNode,
    /// 按行优先顺序排列的槽位；`None` 为空槽。
    pub(crate) slots: Vec<Option<GroupId>>,
    /// 焦点槽位。
    pub(crate) focused: usize,
    pub(crate) rows: usize,
    pub(crate) cols: usize,
    /// 布局是否由自适应算法选出。
    pub(crate) auto: bool,
}

impl TiledTabs {
    /// 按"标签组顺序填前 N 个槽位"构造网格。
    pub(crate) fn new(groups: Vec<GroupId>, rows: usize, cols: usize, auto: bool) -> Option<Self> {
        if rows == 0 || cols == 0 || groups.len() < 2 {
            return None;
        }
        let capacity = rows.saturating_mul(cols);
        if capacity < groups.len() || capacity > MAX_TILING_SLOTS {
            return None;
        }
        let mut seen = HashSet::with_capacity(groups.len());
        if groups.iter().any(|group| !seen.insert(*group)) {
            return None;
        }
        let mut slots = vec![None; capacity];
        for (index, group) in groups.into_iter().enumerate() {
            slots[index] = Some(group);
        }
        Some(Self {
            id: TilingId::new(),
            root: build_tiling_grid(rows, cols),
            slots,
            focused: 0,
            rows,
            cols,
            auto,
        })
    }

    /// 按持久化槽位重建；重复组或有效槽位少于两个时整份拒绝。
    pub(crate) fn from_saved_slots(
        slots: Vec<Option<GroupId>>,
        rows: usize,
        cols: usize,
        focused: usize,
        auto: bool,
    ) -> Option<Self> {
        if rows == 0 || cols == 0 {
            return None;
        }
        let capacity = rows.saturating_mul(cols);
        if capacity > MAX_TILING_SLOTS || slots.len() != capacity {
            return None;
        }
        let mut seen = HashSet::with_capacity(slots.len());
        for group in slots.iter().flatten() {
            if !seen.insert(*group) {
                return None;
            }
        }
        if seen.len() < 2 {
            return None;
        }
        let focused = if focused < slots.len() && slots[focused].is_some() {
            focused
        } else {
            slots.iter().position(|slot| slot.is_some())?
        };
        Some(Self {
            id: TilingId::new(),
            root: build_tiling_grid(rows, cols),
            slots,
            focused,
            rows,
            cols,
            auto,
        })
    }

    pub(crate) fn occupied_count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }

    pub(crate) fn contains(&self, group: GroupId) -> bool {
        self.slots.contains(&Some(group))
    }

    pub(crate) fn slot_of(&self, group: GroupId) -> Option<usize> {
        self.slots.iter().position(|slot| *slot == Some(group))
    }

    pub(crate) fn group_at(&self, slot: usize) -> Option<GroupId> {
        self.slots.get(slot).copied().flatten()
    }

    /// 焦点槽位的标签；焦点槽为空时退回第一个有内容的槽。
    pub(crate) fn focused_group(&self) -> Option<GroupId> {
        self.group_at(self.focused)
            .or_else(|| self.slots.iter().flatten().next().copied())
    }

    pub(crate) fn occupied_groups(&self) -> Vec<GroupId> {
        self.slots.iter().flatten().copied().collect()
    }

    /// 把某个标签从槽位里摘掉，返回是否真的命中。
    pub(crate) fn remove_group(&mut self, group: GroupId) -> bool {
        let Some(slot) = self.slot_of(group) else {
            return false;
        };
        self.slots[slot] = None;
        if self.focused == slot {
            self.ensure_focus();
        }
        true
    }

    /// 焦点槽为空时向前后两侧寻找最近的有内容槽。
    pub(crate) fn ensure_focus(&mut self) {
        if self.group_at(self.focused).is_some() {
            return;
        }
        let len = self.slots.len();
        for offset in 1..len {
            let forward = (self.focused + offset) % len;
            if self.slots[forward].is_some() {
                self.focused = forward;
                return;
            }
            let backward = (self.focused + len - offset) % len;
            if self.slots[backward].is_some() {
                self.focused = backward;
                return;
            }
        }
    }

    /// 把标签填进指定空槽；槽已被占用或标签已在网格里时拒绝。
    pub(crate) fn fill_slot(&mut self, slot: usize, group: GroupId) -> bool {
        if slot >= self.slots.len() || self.slots[slot].is_some() || self.contains(group) {
            return false;
        }
        self.slots[slot] = Some(group);
        self.focused = slot;
        true
    }

    /// 把新标签加入网格，返回第一个新标签所在槽位。
    pub(crate) fn add_groups(&mut self, groups: Vec<GroupId>, window: &Window) -> Option<usize> {
        let new_groups: Vec<GroupId> = groups
            .into_iter()
            .filter(|group| !self.contains(*group))
            .collect();
        if new_groups.is_empty() {
            return None;
        }
        let needed = self.occupied_count() + new_groups.len();
        if needed > MAX_TILING_SLOTS {
            return None;
        }

        let (old_rows, old_cols, old_focused) = (self.rows, self.cols, self.focused);
        let old_slots = self.slots.clone();
        let (new_rows, new_cols) = if needed <= old_rows.saturating_mul(old_cols) {
            (old_rows, old_cols)
        } else {
            expand_grid(old_rows, old_cols, needed, window_aspect(window))?
        };
        if new_rows != old_rows || new_cols != old_cols {
            let mut slots = vec![None; new_rows.saturating_mul(new_cols)];
            for (slot, group) in old_slots.iter().enumerate() {
                let Some(group) = group else {
                    continue;
                };
                let row = slot / old_cols;
                let col = slot % old_cols;
                slots[row * new_cols + col] = Some(*group);
            }
            self.slots = slots;
            self.rows = new_rows;
            self.cols = new_cols;
            self.root = build_tiling_grid(new_rows, new_cols);
            let row = old_focused / old_cols;
            let col = old_focused % old_cols;
            self.focused = row * new_cols + col;
            self.auto = true;
        }

        let mut first_slot = None;
        for group in new_groups {
            let Some(slot) = self.slots.iter().position(|slot| slot.is_none()) else {
                break;
            };
            self.slots[slot] = Some(group);
            if first_slot.is_none() {
                first_slot = Some(slot);
            }
        }
        if let Some(slot) = first_slot {
            self.focused = slot;
        }
        self.ensure_focus();
        first_slot
    }

    /// 沿方向键寻找相邻的最近有内容槽；该方向到边时返回 `None`。
    pub(crate) fn adjacent_slot(&self, from: usize, direction: PaneDirection) -> Option<usize> {
        if from >= self.slots.len() {
            return None;
        }
        let (row, col) = (from / self.cols, from % self.cols);
        match direction {
            PaneDirection::Left => (0..col)
                .rev()
                .find(|candidate| self.slots[row * self.cols + candidate].is_some())
                .map(|candidate| row * self.cols + candidate),
            PaneDirection::Right => ((col + 1)..self.cols)
                .find(|candidate| self.slots[row * self.cols + candidate].is_some())
                .map(|candidate| row * self.cols + candidate),
            PaneDirection::Up => (0..row)
                .rev()
                .find(|candidate| self.slots[candidate * self.cols + col].is_some())
                .map(|candidate| candidate * self.cols + col),
            PaneDirection::Down => ((row + 1)..self.rows)
                .find(|candidate| self.slots[candidate * self.cols + col].is_some())
                .map(|candidate| candidate * self.cols + col),
        }
    }
}
