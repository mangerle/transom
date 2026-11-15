//! 会话侧栏实体与事件定义。
//!
//! 设计原理（Why）：
//! - 与 `ContainerSidebarView`、`GitSidebarView` 保持统一：侧栏视图本身只读
//!   `SidebarState`、目录行快照与拨测徽标渲染，不再直接持有或穿透应用外壳；
//! - 点击、折叠、多选、拖拽与右键菜单全部通过 [`SessionSidebarEvent`] 上报，
//!   由外层唯一桥接 `sidebar/bridge.rs` 在渲染期统一执行。

use super::tree::{SidebarDrag, SidebarTarget};
use gpui_kit::*;
use transom_core::{FolderId, SessionId};

/// 会话侧栏向外壳发出的意图事件。
#[derive(Clone)]
pub(crate) enum SessionSidebarEvent {
    ToggleSelectionMode,
    SelectAllSessions,
    OpenBatchEdit,
    OpenNewFolder(Option<FolderId>),
    OpenRenameFolder(FolderId),
    DeleteFolder(FolderId),
    ToggleFolderCollapsed(FolderId),
    OpenNewSession(Option<FolderId>),
    ProbeFolder(FolderId),
    ProbeSession(SessionId),
    Drop(SidebarDrag, SidebarTarget),
    ToggleSessionSelection {
        session_id: SessionId,
        selected: bool,
    },
    SelectTreeSession(SessionId),
    ConnectSession(SessionId),
    OpenContainerWorkspace(SessionId),
    DuplicateSession(SessionId),
    EditSession(SessionId),
    OpenAuditViewer(SessionId),
    CopyHost(String),
    DeleteSession(SessionId),
}

/// 会话侧栏实体：仅负责派发侧栏交互事件，不直接修改目录或会话状态。
pub(crate) struct SessionSidebarView;

impl SessionSidebarView {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn emit_action(&mut self, event: SessionSidebarEvent, cx: &mut Context<Self>) {
        cx.emit(event);
    }
}

impl EventEmitter<SessionSidebarEvent> for SessionSidebarView {}

impl Render for SessionSidebarView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
