//! 可定制工具条的布局契约与活动栏/状态栏条目模型。
//!
//! 布局只保存三件事：完整顺序、隐藏集合与上下分区的切分点。顺序保存"全集"
//! 而不是只保存可见项，这样隐藏一个图标再恢复时仍能回到原来的相对位置；
//! 分区的切分点也按全集下标计算，避免隐藏项让顶部/底部归属发生漂移。
//! 状态栏是单行布局，只使用顺序与隐藏集合，不涉及分区。
//!
//! 代价是所有变更都必须经过本模块的方法维护不变量，界面层不能直接改字段；
//! 好处是活动栏/状态栏的渲染、拖拽落点与配置清洗共享同一套顺序语义，不会
//! 出现"界面看到的顺序"和"落盘顺序"不一致。

use serde::{Deserialize, Serialize};

/// 状态栏条目标识。
///
/// 与活动栏一样使用稳定的小写蛇形名序列化，并以 `Unknown` 兜底外部配置中的
/// 未知条目。状态栏的在场性由实时数据决定（例如没有网络采样时网络项不渲染），
/// 但顺序与隐藏状态仍按全集保存，避免数据恢复后条目跳到别的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusItemId {
    Network,
    Cpu,
    Memory,
    Disk,
    Os,
    Uptime,
    /// 未知条目的兜底占位，归一化时会被剔除。
    #[serde(other)]
    Unknown,
}

impl StatusItemId {
    /// 默认顺序：网络、CPU、内存、磁盘、系统、运行时间。
    pub const DEFAULT_ORDER: [Self; 6] = [
        Self::Network,
        Self::Cpu,
        Self::Memory,
        Self::Disk,
        Self::Os,
        Self::Uptime,
    ];

    /// 供界面元素 ID 与设置项复用的稳定键名。
    pub const fn key(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Disk => "disk",
            Self::Os => "os",
            Self::Uptime => "uptime",
            Self::Unknown => "unknown",
        }
    }
}

/// 状态栏布局：单行完整顺序与隐藏集合。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusBarLayout {
    #[serde(default = "default_status_order")]
    pub(crate) order: Vec<StatusItemId>,
    #[serde(default)]
    pub(crate) hidden: Vec<StatusItemId>,
}

fn default_status_order() -> Vec<StatusItemId> {
    StatusItemId::DEFAULT_ORDER.to_vec()
}

impl Default for StatusBarLayout {
    fn default() -> Self {
        Self {
            order: default_status_order(),
            hidden: Vec::new(),
        }
    }
}

impl StatusBarLayout {
    /// 完整顺序，包含当前隐藏或暂无数据的条目。
    pub fn order(&self) -> &[StatusItemId] {
        &self.order
    }

    /// 条目是否被隐藏。
    pub fn is_hidden(&self, id: StatusItemId) -> bool {
        self.hidden.contains(&id)
    }

    /// 当前可见的条目，按用户顺序排列。
    pub fn visible_items(&self) -> impl Iterator<Item = StatusItemId> + '_ {
        self.order.iter().copied().filter(|id| !self.is_hidden(*id))
    }

    /// 设置显示或隐藏一个条目，返回布局是否真的发生变化。
    pub fn set_visible(&mut self, id: StatusItemId, visible: bool) -> bool {
        if id == StatusItemId::Unknown || !self.order.contains(&id) {
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
    /// 状态栏是单行布局，没有上下分组；锚点只表示横向插入位置。先摘走拖拽项
    /// 再插入时，锚点下标在拖拽项之后会前移一位，因此用移除后的基准下标计算。
    pub fn move_relative_to(
        &mut self,
        dragged: StatusItemId,
        anchor: StatusItemId,
        after: bool,
    ) -> bool {
        if dragged == anchor || dragged == StatusItemId::Unknown || anchor == StatusItemId::Unknown
        {
            return false;
        }
        let Some(anchor_index) = self.order.iter().position(|id| *id == anchor) else {
            return false;
        };
        let Some(dragged_index) = self.order.iter().position(|id| *id == dragged) else {
            return false;
        };
        let insertion_base = if anchor_index > dragged_index {
            anchor_index - 1
        } else {
            anchor_index
        };
        let insertion = insertion_base + usize::from(after);
        self.order.remove(dragged_index);
        self.order.insert(insertion, dragged);
        dragged_index != insertion
    }

    /// 把条目移到最前面，返回布局是否发生变化。
    ///
    /// 状态栏右侧簇是右对齐的，簇左侧留有大块空白；拖到那里等价于插到第一位，
    /// 比要求用户精确瞄准第一个徽标的左半区更自然。
    pub fn move_to_front(&mut self, dragged: StatusItemId) -> bool {
        if dragged == StatusItemId::Unknown {
            return false;
        }
        let Some(index) = self.order.iter().position(|id| *id == dragged) else {
            return false;
        };
        if index == 0 {
            return false;
        }
        let id = self.order.remove(index);
        self.order.insert(0, id);
        true
    }

    /// 恢复默认顺序与显示状态，返回布局是否发生变化。
    pub fn restore_defaults(&mut self) -> bool {
        if *self == Self::default() {
            return false;
        }
        *self = Self::default();
        true
    }

    /// 清洗来自设置文件的布局：丢弃未知与重复项、补回缺失项，并去重隐藏标记。
    /// 设置文件属于外部输入，不能让一个过期条目破坏整条状态栏。
    pub fn normalize(&mut self) {
        let mut order = Vec::with_capacity(StatusItemId::DEFAULT_ORDER.len());
        for id in self.order.drain(..) {
            if id != StatusItemId::Unknown && !order.contains(&id) {
                order.push(id);
            }
        }
        for (index, id) in StatusItemId::DEFAULT_ORDER.iter().enumerate() {
            if !order.contains(id) {
                let position = index.min(order.len());
                order.insert(position, *id);
            }
        }
        self.order = order;

        let mut hidden = Vec::with_capacity(self.hidden.len());
        for id in self.hidden.drain(..) {
            if id != StatusItemId::Unknown && !hidden.contains(&id) {
                hidden.push(id);
            }
        }
        self.hidden = hidden;
    }
}
