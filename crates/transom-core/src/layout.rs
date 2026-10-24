//! 可定制工具条的布局契约与活动栏条目模型。
//!
//! 布局只保存三件事：完整顺序、隐藏集合与上下分区的切分点。顺序保存"全集"
//! 而不是只保存可见项，这样隐藏一个图标再恢复时仍能回到原来的相对位置；
//! 分区的切分点也按全集下标计算，避免隐藏项让顶部/底部归属发生漂移。
//!
//! 代价是所有变更都必须经过本模块的方法维护不变量，界面层不能直接改字段；
//! 好处是活动栏的渲染、拖拽落点与配置清洗共享同一套顺序语义，不会出现
//! "界面看到的顺序"和"落盘顺序"不一致。

use serde::{Deserialize, Serialize};

/// 活动栏条目标识。
///
/// 使用稳定的小写蛇形名序列化，重排枚举顺序或改中文名不会破坏已有设置文件。
/// 末尾保留 `Unknown` 兜底：用户手工编辑配置或跨版本导入时，未知条目会被
/// 归一化流程丢弃，而不是让整个设置文件反序列化失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityItemId {
    Sessions,
    QuickOpen,
    Forwards,
    Snippets,
    Sftp,
    Language,
    ImportSshConfig,
    KnownHosts,
    Keys,
    Settings,
    /// 未知条目的兜底占位，归一化时会被剔除。
    #[serde(other)]
    Unknown,
}

impl ActivityItemId {
    /// 默认顺序：上半区是会话工作流，下半区是低频工具与设置。
    pub const DEFAULT_ORDER: [Self; 10] = [
        Self::Sessions,
        Self::QuickOpen,
        Self::Forwards,
        Self::Snippets,
        Self::Sftp,
        Self::Language,
        Self::ImportSshConfig,
        Self::KnownHosts,
        Self::Keys,
        Self::Settings,
    ];

    /// 默认顶部条目数量，也是 `split_at` 的默认值。
    pub const DEFAULT_SPLIT_AT: usize = 5;

    /// 供界面元素 ID 与设置项复用的稳定键名。
    ///
    /// 与 serde 的小写蛇形名保持一致；改名会让已有元素状态与手工配置失配，
    /// 因此这里是公开契约的一部分，不能随中文文案调整。
    pub const fn key(self) -> &'static str {
        match self {
            Self::Sessions => "sessions",
            Self::QuickOpen => "quick_open",
            Self::Forwards => "forwards",
            Self::Snippets => "snippets",
            Self::Sftp => "sftp",
            Self::Language => "language",
            Self::ImportSshConfig => "import_ssh_config",
            Self::KnownHosts => "known_hosts",
            Self::Keys => "keys",
            Self::Settings => "settings",
            Self::Unknown => "unknown",
        }
    }

    /// 是否固定显示。
    ///
    /// 设置入口是唯一不可隐藏的条目：隐藏其他图标后用户仍能进入设置页恢复布局，
    /// 避免把自己关在定制界面之外。固定只限制"能否隐藏"，不限制拖动位置。
    pub const fn is_pinned(self) -> bool {
        matches!(self, Self::Settings)
    }
}

/// 活动栏布局：完整顺序、隐藏集合与顶部区域大小。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityBarLayout {
    #[serde(default = "default_order")]
    order: Vec<ActivityItemId>,
    #[serde(default)]
    hidden: Vec<ActivityItemId>,
    #[serde(default = "default_split_at")]
    split_at: usize,
}

fn default_order() -> Vec<ActivityItemId> {
    ActivityItemId::DEFAULT_ORDER.to_vec()
}

fn default_split_at() -> usize {
    ActivityItemId::DEFAULT_SPLIT_AT
}

impl Default for ActivityBarLayout {
    fn default() -> Self {
        Self {
            order: default_order(),
            hidden: Vec::new(),
            split_at: default_split_at(),
        }
    }
}

impl ActivityBarLayout {
    /// 完整顺序，包含当前隐藏的条目。
    pub fn order(&self) -> &[ActivityItemId] {
        &self.order
    }

    /// 顶部区域在完整顺序中的切分下标。
    pub fn split_at(&self) -> usize {
        self.split_at.min(self.order.len())
    }

    /// 条目是否被隐藏；固定条目的隐藏请求在归一化时已被清除。
    pub fn is_hidden(&self, id: ActivityItemId) -> bool {
        self.hidden.contains(&id)
    }

    /// 顶部区域中当前可见的条目。
    pub fn top_items(&self) -> impl Iterator<Item = ActivityItemId> + '_ {
        let split = self.split_at();
        self.order[..split]
            .iter()
            .copied()
            .filter(|id| !self.is_hidden(*id))
    }

    /// 底部区域中当前可见的条目。
    pub fn bottom_items(&self) -> impl Iterator<Item = ActivityItemId> + '_ {
        let split = self.split_at();
        self.order[split..]
            .iter()
            .copied()
            .filter(|id| !self.is_hidden(*id))
    }

    /// 设置显示或隐藏一个条目，返回布局是否真的发生变化。
    ///
    /// 设置入口固定显示；未知条目与不在顺序中的条目一律忽略。返回值让调用方
    /// 只在状态真正变化时落盘，避免右键菜单重复点击触发无意义的写文件。
    pub fn set_visible(&mut self, id: ActivityItemId, visible: bool) -> bool {
        if id == ActivityItemId::Unknown || !self.order.contains(&id) {
            return false;
        }
        if !visible && id.is_pinned() {
            return false;
        }
        if visible {
            let Some(position) = self.hidden.iter().position(|hidden| *hidden == id) else {
                return false;
            };
            self.hidden.remove(position);
            true
        } else if self.hidden.contains(&id) {
            false
        } else {
            self.hidden.push(id);
            true
        }
    }

    /// 把 `dragged` 移到 `anchor` 之前或之后，返回布局是否发生变化。
    ///
    /// 目标区域由锚点在当前布局中的归属决定：锚点在顶部就把拖拽项留在顶部，
    /// 在底部就留在底部。先摘走拖拽项再插入，锚点下标会随之前移，因此必须在
    /// 移除后重新查找锚点，不能复用移除前的下标。
    pub fn move_relative_to(
        &mut self,
        dragged: ActivityItemId,
        anchor: ActivityItemId,
        after: bool,
    ) -> bool {
        if dragged == anchor
            || dragged == ActivityItemId::Unknown
            || anchor == ActivityItemId::Unknown
        {
            return false;
        }
        let Some(anchor_index) = self.order.iter().position(|id| *id == anchor) else {
            return false;
        };
        let Some(dragged_index) = self.order.iter().position(|id| *id == dragged) else {
            return false;
        };
        let target_top = anchor_index < self.split_at();
        let insertion_base = if anchor_index > dragged_index {
            anchor_index - 1
        } else {
            anchor_index
        };
        let insertion = insertion_base + usize::from(after);

        self.order.remove(dragged_index);
        let mut split = self.split_at();
        if dragged_index < split {
            split -= 1;
        }
        self.order.insert(insertion, dragged);

        let new_split = if target_top {
            (split + 1).max(insertion + 1)
        } else {
            split.min(insertion)
        }
        .min(self.order.len());
        let changed = dragged_index != insertion || self.split_at != new_split;
        self.split_at = new_split;
        changed
    }

    /// 把条目移到顶部区域的末尾，返回布局是否发生变化。
    pub fn move_to_top_end(&mut self, dragged: ActivityItemId) -> bool {
        self.move_to_boundary(dragged, true)
    }

    /// 把条目移到底部区域的开头，返回布局是否发生变化。
    pub fn move_to_bottom_start(&mut self, dragged: ActivityItemId) -> bool {
        self.move_to_boundary(dragged, false)
    }

    /// 恢复默认顺序、显示状态与分区，返回布局是否发生变化。
    pub fn restore_defaults(&mut self) -> bool {
        if *self == Self::default() {
            return false;
        }
        *self = Self::default();
        true
    }

    /// 清洗来自设置文件的布局：丢弃未知与重复项、补回缺失项、清除固定项的
    /// 隐藏标记，并把切分点夹回合法范围。
    ///
    /// 补回缺失项时按默认下标插入，尽量保留用户已有顺序的相对位置；设置文件
    /// 属于外部输入，不能因为一个旧条目被删除就让整个布局失效。
    pub fn normalize(&mut self) {
        let mut order = Vec::with_capacity(ActivityItemId::DEFAULT_ORDER.len());
        for id in self.order.drain(..) {
            if id != ActivityItemId::Unknown && !order.contains(&id) {
                order.push(id);
            }
        }
        for (index, id) in ActivityItemId::DEFAULT_ORDER.iter().enumerate() {
            if !order.contains(id) {
                let position = index.min(order.len());
                order.insert(position, *id);
            }
        }
        self.order = order;

        let mut hidden = Vec::with_capacity(self.hidden.len());
        for id in self.hidden.drain(..) {
            if id != ActivityItemId::Unknown && !id.is_pinned() && !hidden.contains(&id) {
                hidden.push(id);
            }
        }
        self.hidden = hidden;
        self.split_at = self.split_at.min(self.order.len());
    }

    fn move_to_boundary(&mut self, dragged: ActivityItemId, top: bool) -> bool {
        if dragged == ActivityItemId::Unknown {
            return false;
        }
        let Some(dragged_index) = self.order.iter().position(|id| *id == dragged) else {
            return false;
        };
        self.order.remove(dragged_index);
        let mut split = self.split_at();
        if dragged_index < split {
            split -= 1;
        }
        let insertion = split;
        self.order.insert(insertion, dragged);
        let new_split = if top { split + 1 } else { split };
        let changed = dragged_index != insertion || self.split_at != new_split;
        self.split_at = new_split.min(self.order.len());
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top(layout: &ActivityBarLayout) -> Vec<ActivityItemId> {
        layout.top_items().collect()
    }

    fn bottom(layout: &ActivityBarLayout) -> Vec<ActivityItemId> {
        layout.bottom_items().collect()
    }

    #[test]
    fn item_keys_match_serialized_names() {
        for id in ActivityItemId::DEFAULT_ORDER {
            let serialized = serde_json::to_value(id).unwrap();
            assert_eq!(serialized, serde_json::Value::String(id.key().to_string()));
        }
    }

    #[test]
    fn default_layout_splits_five_and_five() {
        let layout = ActivityBarLayout::default();
        assert_eq!(top(&layout), ActivityItemId::DEFAULT_ORDER[..5].to_vec());
        assert_eq!(bottom(&layout), ActivityItemId::DEFAULT_ORDER[5..].to_vec());
        assert_eq!(layout.split_at(), 5);
    }

    #[test]
    fn normalize_restores_missing_and_drops_unknown_and_duplicates() {
        let mut layout: ActivityBarLayout = serde_json::from_str(
            r#"{
                "order": ["sftp", "sessions", "not_a_real_item", "sftp"],
                "hidden": ["settings", "sftp", "ghost", "sftp"],
                "split_at": 99
            }"#,
        )
        .unwrap();
        layout.normalize();
        assert_eq!(layout.order().len(), ActivityItemId::DEFAULT_ORDER.len());
        for id in ActivityItemId::DEFAULT_ORDER {
            assert_eq!(
                layout.order().iter().filter(|item| **item == id).count(),
                1,
                "每个已知条目必须恰好出现一次"
            );
        }
        assert!(!layout.order().contains(&ActivityItemId::Unknown));
        assert!(!layout.is_hidden(ActivityItemId::Settings));
        assert!(layout.is_hidden(ActivityItemId::Sftp));
        assert_eq!(layout.split_at(), layout.order().len());
    }

    #[test]
    fn normalize_keeps_a_valid_split_point() {
        let mut layout = ActivityBarLayout {
            order: ActivityItemId::DEFAULT_ORDER.to_vec(),
            hidden: Vec::new(),
            split_at: 4,
        };
        layout.normalize();
        assert_eq!(layout.split_at(), 4);
        assert_eq!(top(&layout).len(), 4);
        assert_eq!(bottom(&layout).len(), 6);
    }

    #[test]
    fn legacy_settings_without_activity_bar_use_defaults() {
        let settings: crate::AppSettings = serde_json::from_str(r#"{"language":"zh-CN"}"#).unwrap();
        assert_eq!(settings.activity_bar, ActivityBarLayout::default());
    }

    #[test]
    fn unknown_item_does_not_break_deserialization() {
        let mut layout: ActivityBarLayout =
            serde_json::from_str(r#"{"order":["sessions","future_item","quick_open"]}"#).unwrap();
        layout.normalize();
        assert_eq!(layout.order().len(), ActivityItemId::DEFAULT_ORDER.len());
        assert_eq!(layout.order()[0], ActivityItemId::Sessions);
        assert_eq!(layout.order()[1], ActivityItemId::QuickOpen);
    }

    #[test]
    fn move_within_top_only_reorders() {
        let mut layout = ActivityBarLayout::default();
        assert!(layout.move_relative_to(ActivityItemId::Sftp, ActivityItemId::Sessions, false));
        assert_eq!(
            top(&layout),
            vec![
                ActivityItemId::Sftp,
                ActivityItemId::Sessions,
                ActivityItemId::QuickOpen,
                ActivityItemId::Forwards,
                ActivityItemId::Snippets,
            ]
        );
        assert_eq!(layout.split_at(), 5);
        assert_eq!(bottom(&layout), ActivityItemId::DEFAULT_ORDER[5..].to_vec());
    }

    #[test]
    fn moving_top_to_bottom_start_grows_bottom() {
        let mut layout = ActivityBarLayout::default();
        assert!(layout.move_to_bottom_start(ActivityItemId::Sessions));
        assert_eq!(layout.split_at(), 4);
        assert_eq!(
            top(&layout),
            vec![
                ActivityItemId::QuickOpen,
                ActivityItemId::Forwards,
                ActivityItemId::Snippets,
                ActivityItemId::Sftp,
            ]
        );
        assert_eq!(bottom(&layout)[0], ActivityItemId::Sessions);
    }

    #[test]
    fn moving_bottom_to_top_end_grows_top() {
        let mut layout = ActivityBarLayout::default();
        assert!(layout.move_to_top_end(ActivityItemId::Settings));
        assert_eq!(layout.split_at(), 6);
        assert_eq!(top(&layout)[5], ActivityItemId::Settings);
        assert!(!bottom(&layout).contains(&ActivityItemId::Settings));
    }

    #[test]
    fn dropping_on_top_anchor_moves_item_into_top() {
        let mut layout = ActivityBarLayout::default();
        assert!(layout.move_relative_to(ActivityItemId::Keys, ActivityItemId::Sessions, true));
        assert_eq!(layout.split_at(), 6);
        let top = top(&layout);
        assert_eq!(top[0], ActivityItemId::Sessions);
        assert_eq!(top[1], ActivityItemId::Keys);
        assert!(!bottom(&layout).contains(&ActivityItemId::Keys));
    }

    #[test]
    fn dropping_on_bottom_anchor_moves_item_into_bottom() {
        let mut layout = ActivityBarLayout::default();
        assert!(layout.move_relative_to(ActivityItemId::Sessions, ActivityItemId::Language, true));
        assert_eq!(layout.split_at(), 4);
        assert_eq!(top(&layout).len(), 4);
        let bottom = bottom(&layout);
        assert_eq!(bottom[0], ActivityItemId::Language);
        assert_eq!(bottom[1], ActivityItemId::Sessions);
    }

    #[test]
    fn moving_to_current_boundary_is_a_no_op() {
        let mut layout = ActivityBarLayout::default();
        assert!(!layout.move_to_top_end(ActivityItemId::Sftp));
        assert!(!layout.move_to_bottom_start(ActivityItemId::Language));
        assert!(!layout.move_relative_to(ActivityItemId::Sftp, ActivityItemId::Sftp, true));
        assert_eq!(layout, ActivityBarLayout::default());
    }

    #[test]
    fn pinned_item_cannot_be_hidden_but_can_move() {
        let mut layout = ActivityBarLayout::default();
        assert!(!layout.set_visible(ActivityItemId::Settings, false));
        assert!(layout.set_visible(ActivityItemId::Sftp, false));
        assert!(!layout.set_visible(ActivityItemId::Sftp, false));
        assert!(layout.set_visible(ActivityItemId::Sftp, true));
        assert!(layout.move_to_top_end(ActivityItemId::Settings));
        assert_eq!(top(&layout)[5], ActivityItemId::Settings);
    }

    #[test]
    fn restore_defaults_resets_order_hidden_and_split() {
        let mut layout = ActivityBarLayout::default();
        layout.set_visible(ActivityItemId::Sftp, false);
        layout.move_to_top_end(ActivityItemId::Settings);
        assert!(layout.restore_defaults());
        assert_eq!(layout, ActivityBarLayout::default());
        assert!(!layout.restore_defaults());
    }
}
