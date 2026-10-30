//! 单个远端列的状态模型与目标身份。
//!
//! 设计原理（Why）：主远端与并列远端共用同一类型，列只靠 `connection_id`
//! 区分归属；本文件把列结构与选择状态机从面板状态中拆出，避免 `state.rs`
//! 继续膨胀，也让"一列"的行为可以独立审阅。

use super::selection;
use super::side_remote::SftpActivePane;
use super::state::RemoteDirCacheEntry;
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::collections::BTreeSet;
use transom_core::{ConnectionId, RemoteFile};

/// 一个远端列（主远端或并列远端）的导航、条目与选择状态。
///
/// 设计原理（Why）：主远端与并列远端的数据结构完全一致，此前并列列单独维护
/// 一份精简状态，导致两边能力、字段与渲染长期分叉。合并为同一类型后，列只
/// 靠 `connection_id` 区分归属：`None` 表示标签页主连接，`Some` 表示并列列。
pub(crate) struct SftpRemotePaneState {
    /// 连接归属：`None` 为标签页主远端，`Some` 为并列远端列。
    pub(crate) connection_id: Option<ConnectionId>,
    /// 并列远端列的标题；主远端为 `None`。
    pub(crate) title: Option<String>,
    /// 列级错误提示（并列列使用；主远端错误仍走面板状态栏）。
    pub(crate) error: Option<String>,
    pub(crate) path: String,
    pub(crate) path_input: Entity<InputState>,
    pub(crate) name_input: Entity<InputState>,
    pub(crate) filter_input: Entity<InputState>,
    pub(crate) entries: Vec<RemoteFile>,
    pub(crate) selected: Option<String>,
    /// 多选集合；`selected` 始终指向其中最近操作的主项。
    pub(crate) selection: BTreeSet<String>,
    /// Shift 连选的锚点路径。
    pub(crate) anchor: Option<String>,
    pub(crate) scroll_handle: VirtualListScrollHandle,
    /// 是否折叠为竖条。
    pub(crate) collapsed: bool,
    pub(crate) loading: bool,
    /// 本列是否显示隐藏文件。
    pub(crate) show_hidden: bool,
    /// 本列是否展开过滤输入框。
    pub(crate) show_filter: bool,
    /// 远程目录短期内存缓存（SWR 机制），消除目录跳转顿挫感。
    pub(crate) dir_cache: std::collections::HashMap<String, RemoteDirCacheEntry>,
    /// 列自身的订阅（并列列的路径回车等）：随列关闭一起释放。
    pub(crate) subscriptions: Vec<Subscription>,
}

impl SftpRemotePaneState {
    /// 构造主远端列：连接与标题留空，其余字段与并列列一致。
    pub(crate) fn new_main(
        path: String,
        path_input: Entity<InputState>,
        name_input: Entity<InputState>,
        filter_input: Entity<InputState>,
        loading: bool,
    ) -> Self {
        Self {
            connection_id: None,
            title: None,
            error: None,
            path,
            path_input,
            name_input,
            filter_input,
            entries: Vec::new(),
            selected: None,
            selection: BTreeSet::new(),
            anchor: None,
            scroll_handle: VirtualListScrollHandle::new(),
            collapsed: false,
            loading,
            show_hidden: false,
            show_filter: false,
            dir_cache: std::collections::HashMap::new(),
            subscriptions: Vec::new(),
        }
    }

    /// 单选一个条目，同时清空多选集合并重置 Shift 锚点。
    pub(crate) fn select(&mut self, path: String) {
        selection::select(
            &mut self.selected,
            &mut self.selection,
            &mut self.anchor,
            path,
        );
        // 选中动作视为用户已在继续操作，列级错误提示随之退场。
        self.error = None;
    }

    /// 设置主项但保留已有多选集合，供右键菜单定位单项操作。
    pub(crate) fn set_primary(&mut self, path: String) {
        selection::set_primary(&mut self.selected, &mut self.selection, path);
        self.error = None;
    }

    /// Ctrl/Cmd 点击切换单个条目的选中状态。
    pub(crate) fn toggle(&mut self, path: String) {
        selection::toggle(
            &mut self.selected,
            &mut self.selection,
            &mut self.anchor,
            path,
        );
        self.error = None;
    }

    /// Shift 连选：用可见列表中的一段路径替换当前集合。
    pub(crate) fn select_range(
        &mut self,
        paths: impl IntoIterator<Item = String>,
        primary: String,
    ) {
        selection::select_range(&mut self.selected, &mut self.selection, paths, primary);
        self.error = None;
    }

    /// 返回当前需要批量操作的路径；无多选时退化为单选主项。
    pub(crate) fn selected_paths(&self) -> Vec<String> {
        selection::paths(&self.selected, &self.selection)
    }

    pub(crate) fn clear_selection(&mut self) {
        selection::clear(&mut self.selected, &mut self.selection, &mut self.anchor);
    }

    /// 刷新列表后剔除已不存在的选中项，并修正主项与锚点。
    pub(crate) fn sync_selection(&mut self, valid: &BTreeSet<String>) {
        selection::sync(
            &mut self.selected,
            &mut self.selection,
            &mut self.anchor,
            valid,
        );
    }
}

/// 一个远端列的身份：主远端或某个并列远端。
///
/// 用连接 ID 标识并列列而不是下标：删除任意一列都会让后续下标前移，
/// 渲染与动作按连接寻址，不随列序变化漂移。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SftpRemotePaneRef {
    Main,
    Side(ConnectionId),
}

impl From<SftpRemotePaneRef> for SftpActivePane {
    fn from(target: SftpRemotePaneRef) -> Self {
        match target {
            SftpRemotePaneRef::Main => SftpActivePane::Remote,
            SftpRemotePaneRef::Side(connection_id) => SftpActivePane::SideRemote(connection_id),
        }
    }
}
