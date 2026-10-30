//! SFTP 跨列剪贴板、拖拽载荷与并列列状态构造。
//!
//! 设计原理（Why）：
//! - 实现初衷：定义跨栏拖拽载荷 SftpDragItem、拖拽浮层视觉 SftpDragPreview 与剪贴板状态 SftpClipboardData；
//! - 核心优势：纯数据契约与共享列构造，供视图渲染与动作调度解耦引用。

use crate::app::AppShell;
use crate::sftp::SftpRemotePaneState;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::{Icon, Sizable, VirtualListScrollHandle};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::collections::BTreeSet;
use transom_core::{ConnectionId, TabId};

/// 跨栏剪贴板数据来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SftpClipboardSource {
    /// 来自本地文件系统
    Local,
    /// 来自远端文件系统（携带源连接 ID）
    Remote(ConnectionId),
}

/// 跨栏剪贴板中的单个条目。
///
/// 必须保留 `is_dir`：仅凭路径无法判断目录类型，粘贴近端时会把目录当作
/// 单文件传输而失败（跨远端互传曾因此丢掉目录语义）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SftpClipboardEntry {
    pub(crate) path: String,
    pub(crate) is_dir: bool,
}

/// 跨栏剪贴板数据结构（供 Ctrl+C / Ctrl+V 使用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SftpClipboardData {
    pub(crate) source: SftpClipboardSource,
    pub(crate) entries: Vec<SftpClipboardEntry>,
}

/// 跨列文件拖拽载荷。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SftpDragItem {
    pub(crate) source_tab_id: TabId,
    pub(crate) source_connection_id: Option<ConnectionId>,
    pub(crate) paths: Vec<String>,
    pub(crate) file_name: String,
    pub(crate) is_dir: bool,
}

/// 跨列文件拖拽视觉预览浮层。
pub(crate) struct SftpDragPreview {
    pub(crate) name: String,
    pub(crate) count: usize,
}

impl SftpDragPreview {
    pub(crate) fn new(name: String, count: usize) -> Self {
        Self { name, count }
    }
}

impl Render for SftpDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .px_2()
            .py_1()
            .gap_1p5()
            .rounded(px(4.0))
            .bg(hsla(0.0, 0.0, 0.15, 0.95))
            .text_color(hsla(0.0, 0.0, 0.95, 1.0))
            .text_xs()
            .items_center()
            .child(Icon::new(IconName::File).xsmall())
            .child(self.name.clone())
            .when(self.count > 1, |this| {
                this.child(
                    div()
                        .px_1()
                        .rounded(px(2.0))
                        .bg(hsla(0.0, 0.0, 0.3, 0.9))
                        .child(format!("+{}", self.count - 1)),
                )
            })
    }
}

/// SFTP 激活的焦点栏位。
///
/// 并列远端用连接 ID 而不是 `Vec` 下标标识：删除任意一列都会让后续下标整体
/// 前移，按位置记录的活动栏会静默指向另一台主机，Delete/复制等操作随之作用
/// 到错误目标。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SftpActivePane {
    Local,
    #[default]
    Remote,
    SideRemote(ConnectionId),
}

/// 删除一列并列远端后，活动栏应落到的位置。
pub(crate) fn active_pane_after_removing(
    active: SftpActivePane,
    removed: ConnectionId,
) -> SftpActivePane {
    if active == SftpActivePane::SideRemote(removed) {
        SftpActivePane::Remote
    } else {
        active
    }
}

/// 跨远端互传任务参数对象。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SftpRemoteCopyParams {
    pub(crate) source_connection_id: ConnectionId,
    pub(crate) source_path: String,
    pub(crate) target_connection_id: ConnectionId,
    pub(crate) target_path: String,
    pub(crate) is_dir: bool,
}

impl SftpRemotePaneState {
    /// 构造一个并列远端列。
    ///
    /// 与主远端共用同一份列状态：连接与标题在这里落成 `Some`，其余字段
    /// 与主远端完全一致，渲染与文件操作不需要再区分类型。
    pub(crate) fn new_side(
        connection_id: ConnectionId,
        title: String,
        path: String,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> Self {
        let path_input = crate::palette::sftp_text_input(
            window,
            cx,
            &path,
            rust_i18n::t!("sftp.sidebar.remote_path_placeholder").as_ref(),
        );
        let filter_input = crate::palette::sftp_text_input(
            window,
            cx,
            "",
            rust_i18n::t!("sftp.sidebar.filter_placeholder").as_ref(),
        );
        let name_input =
            crate::palette::sftp_text_input(window, cx, "", rust_i18n::t!("sftp.name").as_ref());
        Self {
            connection_id: Some(connection_id),
            title: Some(title),
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
            loading: true,
            show_hidden: false,
            show_filter: false,
            dir_cache: std::collections::HashMap::new(),
            subscriptions: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SftpActivePane, active_pane_after_removing};
    use transom_core::ConnectionId;

    /// 删除非活动列不应改变活动栏指向：按连接 ID 记录不会因下标前移而漂移。
    #[test]
    fn removing_other_side_keeps_active_connection() {
        let active = ConnectionId::new();
        let removed = ConnectionId::new();
        assert_eq!(
            active_pane_after_removing(SftpActivePane::SideRemote(active), removed),
            SftpActivePane::SideRemote(active)
        );
    }

    /// 删除活动列后回落到主远端，而不是指向已经不存在的连接。
    #[test]
    fn removing_active_side_falls_back_to_main_remote() {
        let active = ConnectionId::new();
        assert_eq!(
            active_pane_after_removing(SftpActivePane::SideRemote(active), active),
            SftpActivePane::Remote
        );
    }

    /// 本地栏不参与并列列删除的回落。
    #[test]
    fn local_pane_is_untouched() {
        assert_eq!(
            active_pane_after_removing(SftpActivePane::Local, ConnectionId::new()),
            SftpActivePane::Local
        );
    }
}
