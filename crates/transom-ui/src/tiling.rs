//! 多标签并排：N 槽位网格、分组栈布局与标签动作。
//!
//! 并排视图仍以独立标签为最小单位：每个槽位绑定一个标签组，空格位用 `None`
//! 表示。同一方向的一组格子放进一个可拖拽容器（`TilingNode::Stack`），初始
//! 等分；宽网格先分列、再在每列内分行，窄网格反过来，因此主方向的线贯穿
//! 整张网格，次方向的线在各列/行内独立。
//!
//! 与标签栏的关系：整张网格在标签栏合并成一个条目，标题跟随焦点槽位的标签；
//! 合并条目只是视觉容器，关闭菜单仍让用户选择只关焦点标签还是关掉整组。

use crate::app::AppShell;
use crate::pane_tree::PaneDirection;
use crate::sftp::PendingEditorClose;
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use transom_core::{GroupId, SavedTabTiling, SplitAxis, SplitId, TabId};

/// 单个并排视图允许的最大格子数。
///
/// 上限同时约束了网格分组规模与每个终端的最小可读面积；当前取 4×4 表格的
/// 满格数 16。继续增加只会让紧凑窗口里的终端不可读，也会放大连接数。
pub(crate) const MAX_TILING_SLOTS: usize = 16;

/// 并排网格中的一个节点。
///
/// 同一方向的一组格子用 `Stack` 承载：一个可拖拽容器直接装 N 个面板，
/// 初始化时自然等分；叶子记录槽位下标。不要用嵌套二叉切分表达 N 格，
/// 否则每一刀默认 50%，四列会变成 50/25/12.5/12.5。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TilingNode {
    Slot(usize),
    Stack {
        id: SplitId,
        axis: SplitAxis,
        children: Vec<TilingNode>,
    },
}

impl TilingNode {
    /// 全部分割节点标识，供渲染侧清理已消失的拖拽状态。
    pub(crate) fn split_ids(&self) -> Vec<SplitId> {
        let mut out = Vec::with_capacity(MAX_TILING_SLOTS);
        let mut stack = vec![self];
        while let Some(node) = stack.pop() {
            if let Self::Stack { id, children, .. } = node {
                out.push(*id);
                stack.extend(children.iter());
            }
        }
        out
    }
}

/// 并排组的运行期标识。
///
/// 标签组 ID 只标识单个标签，无法表达"这几个标签属于同一张网格"；多个并排组
/// 并存后，菜单、关闭与恢复必须能精确落到其中一张，因此并排组需要独立身份。
/// 只在内存中使用，不落盘：持久化按标签组下标引用成员。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TilingId(u64);

impl TilingId {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

impl fmt::Display for TilingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// 标签栏里的一个可见条目。
///
/// 并排组在标签栏只占一个条目，组内其余成员不占位；所有邻接与操作都按条目
/// 计算，避免把隐藏成员误当成左邻。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StripEntry {
    /// 独立标签。
    Single(GroupId),
    /// 并排组的合并条目：`id` 定位并排组，`focus` 是条目上展示的焦点标签。
    Tiled { id: TilingId, focus: GroupId },
}

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
    ///
    /// 优先填已有空槽；空槽不够时按窗口比例自适应扩容，但只选"行数不减、
    /// 列数不减"的网格，并把旧标签按原行列位置搬到新网格，保证原有布局不散。
    /// 超过 `MAX_TILING_SLOTS` 或没有可用标签时返回 `None`。
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

/// 布局选择：自适应或手选网格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TilingLayoutChoice {
    Auto,
    Grid { rows: usize, cols: usize },
}

/// 可用的手动网格：容量至少覆盖 `count`，且不超过槽位上限。
pub(crate) fn tiling_layout_options(count: usize) -> Vec<(usize, usize)> {
    let mut options = Vec::new();
    for rows in 1..=4 {
        for cols in 1..=4 {
            let capacity = rows * cols;
            if capacity >= count && capacity <= MAX_TILING_SLOTS {
                options.push((rows, cols));
            }
        }
    }
    options.sort_by_key(|(rows, cols)| (rows * cols, *rows, *cols));
    options
}

/// 自适应布局：在容量足够的网格里挑单元格长宽比最接近终端、空格最少的一个。
pub(crate) fn auto_tiling_grid(count: usize, window: &Window) -> (usize, usize) {
    best_grid(count, window_aspect(window))
}

/// 窗口宽高比；纯计算版本与 UI 共用，避免尺寸口径分叉。
fn window_aspect(window: &Window) -> f32 {
    let size = window.bounds().size;
    let width = size.width / px(1.0);
    let height = size.height / px(1.0);
    if height > 0.0 {
        (width / height).max(0.25)
    } else {
        1.0
    }
}

const TARGET_CELL_ASPECT: f32 = 1.6;

/// 网格评分：单元格越接近终端比例越好，空格位越少越好。
fn grid_score(rows: usize, cols: usize, count: usize, aspect: f32) -> f32 {
    let cell_aspect = aspect * rows as f32 / cols as f32;
    let empty = rows.saturating_mul(cols).saturating_sub(count) as f32;
    (cell_aspect - TARGET_CELL_ASPECT).abs() + empty * 0.8
}

/// 纯计算版本，便于单测覆盖不同窗口形状下的自适应结果。
fn best_grid(count: usize, aspect: f32) -> (usize, usize) {
    let mut best: Option<(f32, usize, usize)> = None;
    for rows in 1..=4 {
        for cols in 1..=4 {
            let capacity = rows * cols;
            if capacity < count || capacity > MAX_TILING_SLOTS {
                continue;
            }
            let score = grid_score(rows, cols, count, aspect);
            if best.is_none_or(|(best_score, _, _)| score < best_score) {
                best = Some((score, rows, cols));
            }
        }
    }
    best.map(|(_, rows, cols)| (rows, cols))
        .unwrap_or((1, count.clamp(1, MAX_TILING_SLOTS)))
}

/// 扩容时只在"行数不减、列数不减"的网格里选，保证原有格子的行列位置不变。
fn expand_grid(
    old_rows: usize,
    old_cols: usize,
    needed: usize,
    aspect: f32,
) -> Option<(usize, usize)> {
    let mut best: Option<(f32, usize, usize)> = None;
    for rows in old_rows..=4 {
        for cols in old_cols..=4 {
            let capacity = rows * cols;
            if capacity < needed || capacity > MAX_TILING_SLOTS {
                continue;
            }
            let score = grid_score(rows, cols, needed, aspect);
            if best.is_none_or(|(best_score, _, _)| score < best_score) {
                best = Some((score, rows, cols));
            }
        }
    }
    best.map(|(_, rows, cols)| (rows, cols))
}

/// 从行列数构建网格。
///
/// 宽网格先分列、再在每列内分行；窄网格反过来。同一方向的一组格子放进一个
/// `Stack`，由可拖拽容器等分，主方向的线贯穿整张网格，次方向的线在各列/行
/// 内独立。
fn build_tiling_grid(rows: usize, cols: usize) -> TilingNode {
    let cell = |row: usize, col: usize| TilingNode::Slot(row * cols + col);
    if cols >= rows {
        let columns = (0..cols)
            .map(|col| {
                if rows == 1 {
                    cell(0, col)
                } else {
                    stack(
                        SplitAxis::Vertical,
                        (0..rows).map(|row| cell(row, col)).collect(),
                    )
                }
            })
            .collect();
        stack(SplitAxis::Horizontal, columns)
    } else {
        let row_nodes = (0..rows)
            .map(|row| {
                if cols == 1 {
                    cell(row, 0)
                } else {
                    stack(
                        SplitAxis::Horizontal,
                        (0..cols).map(|col| cell(row, col)).collect(),
                    )
                }
            })
            .collect();
        stack(SplitAxis::Vertical, row_nodes)
    }
}

fn stack(axis: SplitAxis, mut children: Vec<TilingNode>) -> TilingNode {
    if children.len() == 1 {
        return children.remove(0);
    }
    TilingNode::Stack {
        id: SplitId::new(),
        axis,
        children,
    }
}

impl AppShell {
    /// 当前焦点标签所在的标签组；没有活动标签时为 `None`。
    pub(crate) fn active_group_id(&self) -> Option<GroupId> {
        self.active_tab
            .and_then(|tab| self.group_index_for_tab(tab))
            .map(|index| self.groups[index].id)
    }

    /// 当前活动标签所属的并排组；活动标签独立显示时为 `None`。
    pub(crate) fn active_tiling(&self) -> Option<&TiledTabs> {
        let active = self.active_group_id()?;
        self.tiling_of_group(active)
    }

    /// 某个标签组所在的并排组；未并排时为 `None`。
    pub(crate) fn tiling_of_group(&self, group: GroupId) -> Option<&TiledTabs> {
        self.tiled_sets.iter().find(|tiled| tiled.contains(group))
    }

    /// 按并排组身份查找。
    pub(crate) fn tiling_by_id(&self, id: TilingId) -> Option<&TiledTabs> {
        self.tiled_sets.iter().find(|tiled| tiled.id == id)
    }

    /// 标签栏的可见条目：每个并排组占一条，其余标签各占一条。
    ///
    /// 条目顺序是标签栏的唯一顺序，渲染、邻接、选择与快捷并排全部以它为准。
    pub(crate) fn strip_entries(&self) -> Vec<StripEntry> {
        let groups: Vec<GroupId> = self.groups.iter().map(|group| group.id).collect();
        build_strip_entries(&groups, &self.tiled_sets)
    }

    /// 条目是否包含指定标签组；并排条目按整组成员判断。
    pub(crate) fn entry_contains(&self, entry: StripEntry, group: GroupId) -> bool {
        match entry {
            StripEntry::Single(item) => item == group,
            StripEntry::Tiled { id, .. } => self
                .tiling_by_id(id)
                .is_some_and(|tiled| tiled.contains(group)),
        }
    }

    /// 快速并排：把右键的标签和它在标签栏里左边那一条并到一起。
    ///
    /// 右键不会切换活动标签，所以这里不看焦点，只看标签栏的条目邻接：
    /// - 右键的标签已在某张网格里：只聚焦它，恢复该网格；
    /// - 左边是独立标签：两者新建一张网格，右键标签按 `axis` 放到右/下；
    /// - 左边是并排合并条目：把右键标签加进那一张网格（先填空槽，满了扩格），
    ///   绝不把组里的成员抽出来重配；
    /// - 标签栏里只有它自己：只激活。
    ///
    /// 新建网格只是往 `tiled_sets` 追加，不替换其他并排组；同一窗口可以同时
    /// 存在多张网格，只有活动标签所在的那张显示在内容区。
    pub(crate) fn tile_tab(
        &mut self,
        target: GroupId,
        axis: SplitAxis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target_pane) = self
            .groups
            .iter()
            .find(|group| group.id == target)
            .map(|group| group.active_pane)
        else {
            return;
        };
        if self.tiling_of_group(target).is_some() {
            self.activate_group(target, window, cx);
            return;
        }
        let Some(previous) = self.previous_strip_entry(target) else {
            return;
        };
        match previous {
            StripEntry::Tiled { id, .. } => {
                if let Some(group) = self.add_groups_to_tiling(id, vec![target], window) {
                    self.activate_group(group, window, cx);
                } else {
                    self.error =
                        Some(rust_i18n::t!("tiling.too_many", max = MAX_TILING_SLOTS).to_string());
                    cx.notify();
                }
            }
            StripEntry::Single(partner) if partner != target => {
                let (rows, cols) = match axis {
                    SplitAxis::Horizontal => (1, 2),
                    SplitAxis::Vertical => (2, 1),
                };
                let Some(tiled) = TiledTabs::new(vec![partner, target], rows, cols, false) else {
                    return;
                };
                self.tiled_sets.push(tiled);
                self.prune_pane_split_states();
                self.activate_tab(target_pane, window, cx);
            }
            StripEntry::Single(_) => {
                self.activate_tab(target_pane, window, cx);
            }
        }
    }

    /// 标签栏里目标标签左边那一条（首位回绕到末位）。
    ///
    /// 按条目而不是按标签组下标取：并排组只占一个条目，组内其余成员不占位。
    fn previous_strip_entry(&self, target: GroupId) -> Option<StripEntry> {
        let entries = self.strip_entries();
        let position = entries
            .iter()
            .position(|entry| self.entry_contains(*entry, target))?;
        if position == 0 {
            entries.last().copied()
        } else {
            entries.get(position - 1).copied()
        }
    }

    /// 关闭指定并排组里的全部标签（合并条目关闭菜单的第二项）。
    pub(crate) fn close_tiling_group(
        &mut self,
        id: TilingId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(groups) = self.tiling_by_id(id).map(|tiled| tiled.occupied_groups()) else {
            return;
        };
        self.close_tiling_groups(groups, window, cx);
    }

    /// 关闭指定的一组并排标签：先统一过脏编辑闸门，再逐个关闭标签组。
    pub(crate) fn close_tiling_groups(
        &mut self,
        groups: Vec<GroupId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panes: Vec<TabId> = groups
            .iter()
            .filter_map(|group_id| self.groups.iter().find(|group| group.id == *group_id))
            .flat_map(|group| group.pane_ids())
            .collect();
        if self.block_close_for_dirty_editors(
            &panes,
            PendingEditorClose::Tiling(groups.clone()),
            cx,
        ) {
            return;
        }
        for group_id in groups {
            if self.groups.iter().any(|group| group.id == group_id) {
                self.close_group(group_id, window, cx);
            }
        }
    }

    /// 退出指定并排组，只改变显示方式：组内标签都保持打开与连接。
    pub(crate) fn untile_tiling(&mut self, id: TilingId, cx: &mut Context<Self>) {
        let before = self.tiled_sets.len();
        self.tiled_sets.retain(|tiled| tiled.id != id);
        if self.tiled_sets.len() != before {
            self.prune_pane_split_states();
            self.sync_broadcast_targets();
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

    /// 同步活动标签与所在并排组的焦点槽，不改变并排关系本身。
    pub(crate) fn present_active_group(&mut self) {
        if let Some(active) = self.active_group_id()
            && let Some(tiled) = self
                .tiled_sets
                .iter_mut()
                .find(|tiled| tiled.contains(active))
            && let Some(slot) = tiled.slot_of(active)
        {
            tiled.focused = slot;
        }
        self.sync_broadcast_targets();
    }

    /// 校验所有并排绑定：摘掉已消失的标签组，有效槽位少于两个时解除该组。
    ///
    /// 活动标签不在某张网格里不代表那张网格失效——那只是暂时单屏查看别的
    /// 标签，因此这里不把焦点强行拉回网格。
    pub(crate) fn prune_tilings(&mut self) {
        let alive: HashSet<GroupId> = self.groups.iter().map(|group| group.id).collect();
        let active = self.active_group_id();
        self.tiled_sets.retain_mut(|tiled| {
            let dead: Vec<GroupId> = tiled
                .occupied_groups()
                .into_iter()
                .filter(|group| !alive.contains(group))
                .collect();
            for group in dead {
                tiled.remove_group(group);
            }
            if tiled.occupied_count() < 2 {
                return false;
            }
            if let Some(active) = active
                && let Some(slot) = tiled.slot_of(active)
            {
                tiled.focused = slot;
            }
            tiled.ensure_focus();
            true
        });
        self.prune_pane_split_states();
        self.sync_broadcast_targets();
    }

    /// 窗格方向键在组内已无邻居时，沿网格方向跨到相邻槽位。
    ///
    /// 只在活动标签所属的那张网格内移动：其他并排组当前不可见。
    pub(crate) fn focus_neighbor_tile(
        &mut self,
        direction: PaneDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tiled) = self.active_tiling().cloned() else {
            return;
        };
        let Some(active) = self.active_group_id() else {
            return;
        };
        let Some(slot) = tiled.slot_of(active) else {
            return;
        };
        let Some(next) = tiled.adjacent_slot(slot, direction) else {
            return;
        };
        let Some(group) = tiled.group_at(next) else {
            return;
        };
        self.activate_group(group, window, cx);
    }

    /// 按持久化快照恢复一张并排网格，返回它的焦点标签。
    ///
    /// 快照按标签组下标引用成员；组内成员与已恢复的其它并排组重叠时整份放弃，
    /// 避免同一个标签同时出现在两张网格里。恢复只建结构，不抢焦点：多张网格
    /// 同时恢复时由调用方决定激活哪一张。
    pub(crate) fn restore_tab_tiling(
        &mut self,
        saved: &SavedTabTiling,
        restored: &[Option<GroupId>],
    ) -> Option<GroupId> {
        if saved.rows == 0
            || saved.cols == 0
            || saved.slots.len() != saved.rows.saturating_mul(saved.cols)
        {
            return None;
        }
        let slots: Vec<Option<GroupId>> = saved
            .slots
            .iter()
            .map(|index| index.and_then(|index| restored.get(index).copied().flatten()))
            .collect();
        let tiled =
            TiledTabs::from_saved_slots(slots, saved.rows, saved.cols, saved.focused, saved.auto)?;
        // 同一个标签不能同时属于两张网格。
        if tiled
            .occupied_groups()
            .iter()
            .any(|group| self.tiling_of_group(*group).is_some())
        {
            return None;
        }
        let focus = tiled.focused_group();
        self.tiled_sets.push(tiled);
        self.prune_pane_split_states();
        focus
    }

    /// Ctrl/Cmd+点击标签：整组加入或移出多选集合。
    pub(crate) fn toggle_tab_selection(&mut self, groups: &[GroupId], cx: &mut Context<Self>) {
        if groups.is_empty() {
            return;
        }
        let all_selected = groups
            .iter()
            .all(|group| self.tab_selection.contains(group));
        if all_selected {
            for group in groups {
                self.tab_selection.remove(group);
            }
        } else {
            for group in groups {
                self.tab_selection.insert(*group);
            }
        }
        cx.notify();
    }

    pub(crate) fn clear_tab_selection(&mut self, cx: &mut Context<Self>) {
        if !self.tab_selection.is_empty() {
            self.tab_selection.clear();
            cx.notify();
        }
    }

    /// 丢弃已经关闭的标签组，避免多选集合里残留死 ID。
    pub(crate) fn prune_tab_selection(&mut self) {
        self.tab_selection
            .retain(|group| self.groups.iter().any(|item| item.id == *group));
    }

    /// 按给定标签组与布局应用并排；由二级布局菜单点击后调用。
    ///
    /// 两种落点由"这些标签现在属于谁"决定，不再靠调用方区分：
    /// - 全是独立标签：新建一张网格并入列，不动其它并排组；
    /// - 全在同一张网格里：重排那张网格（徽章里的"重排并排布局"走这条路），
    ///   网格身份不变，焦点标签尽量保留。
    ///
    /// 跨网格的混合集合没有明确落点，直接拒绝：把一半成员搬走等于把那张网格
    /// 拆散，正是要避免的行为。
    pub(crate) fn apply_tiling_layout_for(
        &mut self,
        groups: Vec<GroupId>,
        choice: TilingLayoutChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let groups: Vec<GroupId> = groups
            .into_iter()
            .filter(|group| self.groups.iter().any(|item| item.id == *group))
            .collect();
        if groups.len() < 2 {
            return;
        }
        if groups.len() > MAX_TILING_SLOTS {
            self.error = Some(rust_i18n::t!("tiling.too_many", max = MAX_TILING_SLOTS).to_string());
            cx.notify();
            return;
        }
        let (rows, cols, auto) = match choice {
            TilingLayoutChoice::Auto => {
                let (rows, cols) = auto_tiling_grid(groups.len(), window);
                (rows, cols, true)
            }
            TilingLayoutChoice::Grid { rows, cols } => (rows, cols, false),
        };
        let touched: Vec<TilingId> = self
            .tiled_sets
            .iter()
            .filter(|tiled| groups.iter().any(|group| tiled.contains(*group)))
            .map(|tiled| tiled.id)
            .collect();
        match touched.as_slice() {
            // 全新的一组：追加到并排组列表，不替换任何已有网格。
            [] => {
                let Some(tiled) = TiledTabs::new(groups, rows, cols, auto) else {
                    return;
                };
                let focus = tiled.focused_group();
                self.tiled_sets.push(tiled);
                self.tab_selection.clear();
                self.prune_pane_split_states();
                self.activate_group_by_id(focus, window, cx);
            }
            // 同一张网格重排：保留网格身份与焦点标签。
            [id] => {
                let Some(index) = self.tiled_sets.iter().position(|tiled| tiled.id == *id) else {
                    return;
                };
                if !groups
                    .iter()
                    .all(|group| self.tiled_sets[index].contains(*group))
                {
                    return;
                }
                let focus = self.tiled_sets[index].focused_group();
                let Some(mut tiled) = TiledTabs::new(groups, rows, cols, auto) else {
                    return;
                };
                tiled.id = *id;
                if let Some(focus) = focus
                    && let Some(slot) = tiled.slot_of(focus)
                {
                    tiled.focused = slot;
                }
                self.tiled_sets[index] = tiled;
                self.prune_pane_split_states();
                let focus = self.tiled_sets[index].focused_group();
                self.activate_group_by_id(focus, window, cx);
            }
            // 跨网格的混合集合：没有明确落点，整份拒绝。
            _ => return,
        }
        self.sync_broadcast_targets();
        cx.notify();
    }

    /// 激活某个标签组所在的标签；标签不存在时不做任何事。
    fn activate_group_by_id(
        &mut self,
        group: Option<GroupId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(group) = group else {
            return;
        };
        self.activate_group(group, window, cx);
    }

    /// 把一组独立标签加入指定并排组，返回第一个新标签组。
    ///
    /// 布局由 `TiledTabs::add_groups` 保留原位置并自适应扩容；返回 `None`
    /// 表示没有可加入的标签，或再加入会超过格子上限。已经在别的并排组里的
    /// 标签不会被搬走——一个标签只能属于一张网格。
    pub(crate) fn add_groups_to_tiling(
        &mut self,
        id: TilingId,
        groups: Vec<GroupId>,
        window: &Window,
    ) -> Option<GroupId> {
        let groups: Vec<GroupId> = groups
            .into_iter()
            .filter(|group| self.groups.iter().any(|item| item.id == *group))
            .filter(|group| self.tiling_of_group(*group).is_none())
            .collect();
        if groups.is_empty() {
            return None;
        }
        let first_group = {
            let tiled = self.tiled_sets.iter_mut().find(|tiled| tiled.id == id)?;
            let slot = tiled.add_groups(groups, window)?;
            tiled.group_at(slot)
        };
        let first_group = first_group?;
        self.prune_pane_split_states();
        self.sync_broadcast_targets();
        Some(first_group)
    }

    /// 把一个独立标签填进指定并排组的空槽位。
    pub(crate) fn fill_tiling_slot(
        &mut self,
        id: TilingId,
        slot: usize,
        group: GroupId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tiling_of_group(group).is_some() {
            return;
        }
        let filled = self
            .tiled_sets
            .iter_mut()
            .find(|tiled| tiled.id == id)
            .is_some_and(|tiled| tiled.fill_slot(slot, group));
        if !filled {
            return;
        }
        self.activate_group(group, window, cx);
        self.sync_broadcast_targets();
    }

    /// 可填进空槽的标签组：还没有属于任何并排组的独立标签。
    pub(crate) fn available_tiling_groups(&self) -> Vec<GroupId> {
        self.groups
            .iter()
            .filter(|group| self.tiling_of_group(group.id).is_none())
            .map(|group| group.id)
            .collect()
    }
}

/// 标签栏的可见条目：每张并排网格占一条，其余标签各占一条。
///
/// 网格的位置取组内最靠前的标签；成员在标签栏里不占位，因此同一张网格的其它
/// 成员不会出现在条目序列中。有效成员少于两个的网格直接忽略，按普通标签渲染，
/// 与 `prune_tilings` 的兜底口径一致。
fn build_strip_entries(groups: &[GroupId], sets: &[TiledTabs]) -> Vec<StripEntry> {
    let mut hidden: HashSet<GroupId> = HashSet::with_capacity(groups.len());
    let mut representatives: HashMap<usize, (TilingId, GroupId)> =
        HashMap::with_capacity(sets.len());
    for set in sets {
        let alive: Vec<usize> = set
            .occupied_groups()
            .iter()
            .filter_map(|group| groups.iter().position(|item| item == group))
            .collect();
        if alive.len() < 2 {
            continue;
        }
        let Some(representative) = alive.iter().copied().min() else {
            continue;
        };
        let focus = set
            .focused_group()
            .filter(|group| groups.iter().any(|item| item == group))
            .unwrap_or(groups[representative]);
        for index in &alive {
            hidden.insert(groups[*index]);
        }
        representatives.insert(representative, (set.id, focus));
    }
    let mut entries = Vec::with_capacity(groups.len());
    for (index, group) in groups.iter().enumerate() {
        if let Some(&(id, focus)) = representatives.get(&index) {
            entries.push(StripEntry::Tiled { id, focus });
        } else if hidden.contains(group) {
            continue;
        } else {
            entries.push(StripEntry::Single(*group));
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_TILING_SLOTS, StripEntry, TiledTabs, TilingNode, best_grid, build_strip_entries,
        build_tiling_grid, expand_grid, tiling_layout_options,
    };
    use crate::pane_tree::PaneDirection;
    use transom_core::{GroupId, SplitAxis};

    fn groups(count: usize) -> Vec<GroupId> {
        (0..count).map(|_| GroupId::new()).collect()
    }

    #[test]
    fn grid_tree_covers_every_slot_and_split() {
        let tree = build_tiling_grid(2, 2);
        let mut slots = Vec::new();
        let mut stack = vec![&tree];
        while let Some(node) = stack.pop() {
            match node {
                TilingNode::Slot(slot) => slots.push(*slot),
                TilingNode::Stack { children, .. } => {
                    stack.extend(children.iter());
                }
            }
        }
        slots.sort_unstable();
        assert_eq!(slots, vec![0, 1, 2, 3]);
        assert_eq!(tree.split_ids().len(), 3);
    }

    #[test]
    fn tiled_tabs_bind_slots_and_remove_with_focus() {
        let ids = groups(4);
        let mut tiled = TiledTabs::new(ids.clone(), 2, 2, false).expect("合法 2x2");
        assert_eq!(tiled.occupied_count(), 4);
        assert_eq!(tiled.slot_of(ids[2]), Some(2));
        assert_eq!(tiled.focused_group(), Some(ids[0]));

        tiled.focused = 2;
        assert!(tiled.remove_group(ids[2]));
        assert_eq!(tiled.occupied_count(), 3);
        assert_eq!(tiled.focused_group(), Some(ids[3]));
    }

    #[test]
    fn adjacent_slot_walks_grid_and_skips_empty_cells() {
        let ids = groups(4);
        let mut tiled = TiledTabs::new(ids.clone(), 2, 2, false).expect("合法 2x2");
        assert_eq!(tiled.adjacent_slot(0, PaneDirection::Right), Some(1));
        assert_eq!(tiled.adjacent_slot(0, PaneDirection::Down), Some(2));
        assert_eq!(tiled.adjacent_slot(3, PaneDirection::Left), Some(2));
        assert_eq!(tiled.adjacent_slot(3, PaneDirection::Up), Some(1));
        assert_eq!(tiled.adjacent_slot(1, PaneDirection::Right), None);

        tiled.slots[1] = None;
        assert_eq!(tiled.adjacent_slot(0, PaneDirection::Right), None);
    }

    #[test]
    fn saved_slots_reject_duplicates_and_short_grids() {
        let ids = groups(3);
        assert!(
            TiledTabs::from_saved_slots(
                vec![Some(ids[0]), Some(ids[0]), None, None],
                2,
                2,
                0,
                false
            )
            .is_none()
        );
        assert!(TiledTabs::from_saved_slots(vec![Some(ids[0]), None], 1, 2, 0, false).is_none());
        // 外部设置里的超限网格整份拒绝，不能一次拉起过多连接。
        assert!(
            TiledTabs::from_saved_slots(vec![Some(ids[0]); MAX_TILING_SLOTS + 1], 5, 4, 0, false)
                .is_none()
        );
        // 焦点落在空槽时退回第一个有内容的槽。
        let tiled = TiledTabs::from_saved_slots(
            vec![Some(ids[0]), None, Some(ids[1]), None],
            2,
            2,
            1,
            false,
        )
        .expect("两个有效槽");
        assert_eq!(tiled.focused_group(), Some(ids[0]));
    }

    #[test]
    fn layout_options_cover_capacity_and_stay_within_limit() {
        let options = tiling_layout_options(3);
        assert!(options.iter().all(|(rows, cols)| rows * cols >= 3));
        assert!(
            options
                .iter()
                .all(|(rows, cols)| rows * cols <= MAX_TILING_SLOTS)
        );
        assert!(options.contains(&(2, 2)));
        assert!(options.contains(&(1, 3)));
        assert!(options.contains(&(4, 4)));
        assert!(!options.contains(&(1, 2)));
    }

    #[test]
    fn auto_grid_prefers_exact_or_balanced_layouts() {
        assert_eq!(best_grid(2, 1.6), (1, 2));
        assert_eq!(best_grid(2, 0.6), (2, 1));
        assert_eq!(best_grid(4, 1.6), (2, 2));
        assert_eq!(best_grid(6, 1.6), (2, 3));
        assert_eq!(best_grid(9, 1.6), (3, 3));
        // 三格：2x2 比 1x3 更接近终端比例，接受一个空位。
        assert_eq!(best_grid(3, 1.6), (2, 2));
    }

    #[test]
    fn expansion_keeps_dimensions_and_picks_balanced_grid() {
        assert_eq!(expand_grid(2, 2, 5, 1.6), Some((2, 3)));
        assert_eq!(expand_grid(2, 2, 9, 1.6), Some((3, 3)));
        // 1x4 满了再加一格：只能向下扩成 2x4，列位置不变。
        assert_eq!(expand_grid(1, 4, 5, 1.6), Some((2, 4)));
        assert_eq!(expand_grid(4, 4, 17, 1.6), None);
    }

    #[test]
    fn build_grid_keeps_slot_geometry_for_wide_and_tall_layouts() {
        // 同一方向的一组格子必须是一个 N 面板容器：嵌套二叉切分会让 1x4
        // 默认变成 50/25/12.5/12.5。
        match build_tiling_grid(1, 4) {
            TilingNode::Stack { axis, children, .. } => {
                assert_eq!(axis, SplitAxis::Horizontal);
                assert_eq!(children.len(), 4);
            }
            _ => panic!("1x4 应是一个水平容器"),
        }
        assert_eq!(build_tiling_grid(1, 3).split_ids().len(), 1);
        assert_eq!(build_tiling_grid(3, 1).split_ids().len(), 1);
        // 2x2：一个水平列容器 + 每列一个垂直容器，共三个可拖拽分组。
        assert_eq!(build_tiling_grid(2, 2).split_ids().len(), 3);
        // 方向轴与形状一致：1x2 只有一条水平切分。
        match build_tiling_grid(1, 2) {
            TilingNode::Stack { axis, .. } => assert_eq!(axis, SplitAxis::Horizontal),
            _ => panic!("1x2 应是水平容器"),
        }
    }

    #[test]
    fn strip_entries_merge_each_set_into_one_item() {
        let ids = groups(4);
        // 标签栏顺序 a、b、c、d；a 与 d 并排成一张网格，b、c 各自独立。
        let set = TiledTabs::new(vec![ids[0], ids[3]], 1, 2, false).expect("合法 1x2");
        let entries = build_strip_entries(&ids, std::slice::from_ref(&set));
        // 网格只占 a 的位置，d 不再单独出现。
        assert_eq!(entries.len(), 3);
        assert!(matches!(entries[0], StripEntry::Tiled { .. }));
        assert_eq!(entries[1], StripEntry::Single(ids[1]));
        assert_eq!(entries[2], StripEntry::Single(ids[2]));

        // 两张网格并存时各占一条，互不影响。
        let other = TiledTabs::new(vec![ids[1], ids[2]], 1, 2, false).expect("合法 1x2");
        let entries = build_strip_entries(&ids, &[set, other]);
        assert_eq!(entries.len(), 2);
        assert!(matches!(entries[0], StripEntry::Tiled { .. }));
        assert!(matches!(entries[1], StripEntry::Tiled { .. }));
    }

    #[test]
    fn strip_entries_ignore_broken_sets_and_keep_non_contiguous_members_hidden() {
        let ids = groups(4);
        // 成员不连续：a 与 c 同一张网格，b 夹在中间也不单独占条目。
        let set = TiledTabs::new(vec![ids[0], ids[2]], 1, 2, false).expect("合法 1x2");
        let entries = build_strip_entries(&ids, &[set]);
        assert_eq!(entries.len(), 3);
        assert!(matches!(entries[0], StripEntry::Tiled { .. }));
        assert_eq!(entries[1], StripEntry::Single(ids[1]));
        assert_eq!(entries[2], StripEntry::Single(ids[3]));

        // 网格只剩一个有效成员时按普通标签渲染，等同于兜底降级。
        let broken = TiledTabs::new(vec![ids[0], ids[1]], 1, 2, false).expect("合法 1x2");
        let mut broken = broken;
        broken.remove_group(ids[1]);
        let entries = build_strip_entries(&ids, &[broken]);
        assert_eq!(entries.len(), 4);
        assert!(
            entries
                .iter()
                .all(|entry| matches!(entry, StripEntry::Single(_)))
        );
    }
}
