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
    /// 纵向终端标签面板；与会话树共用左侧槽位，同一时间只显示一个。
    Tabs,
    QuickOpen,
    Forwards,
    Sftp,
    /// 主机监控看板与进程管理器（Agentless，经 SSH exec 采样）。
    Monitor,
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
    ///
    /// 快捷命令（snippets）刻意不在此列：它的入口收敛在底部状态栏的常驻按钮与
    /// `ToggleSnippets` 快捷键上，活动栏再放一个是重复入口。旧设置文件里残留的
    /// `"snippets"` 条目会经 `#[serde(other)]` 归为 `Unknown` 并被 `normalize`
    /// 剔除，因此不需要数据迁移。
    pub const DEFAULT_ORDER: [Self; 11] = [
        Self::Sessions,
        Self::Tabs,
        Self::QuickOpen,
        Self::Forwards,
        Self::Sftp,
        Self::Monitor,
        Self::Language,
        Self::ImportSshConfig,
        Self::KnownHosts,
        Self::Keys,
        Self::Settings,
    ];

    /// 默认顶部条目数量，也是 `split_at` 的默认值。
    pub const DEFAULT_SPLIT_AT: usize = 6;

    /// 供界面元素 ID 与设置项复用的稳定键名。
    ///
    /// 与 serde 的小写蛇形名保持一致；改名会让已有元素状态与手工配置失配，
    /// 因此这里是公开契约的一部分，不能随中文文案调整。
    pub const fn key(self) -> &'static str {
        match self {
            Self::Sessions => "sessions",
            Self::Tabs => "tabs",
            Self::QuickOpen => "quick_open",
            Self::Forwards => "forwards",
            Self::Sftp => "sftp",
            Self::Monitor => "monitor",
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
    pub(crate) order: Vec<ActivityItemId>,
    #[serde(default)]
    pub(crate) hidden: Vec<ActivityItemId>,
    #[serde(default = "default_split_at")]
    pub(crate) split_at: usize,
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
