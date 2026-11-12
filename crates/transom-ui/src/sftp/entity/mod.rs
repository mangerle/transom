//! SFTP 面板实体：每个持有 SFTP 的窗格一份，拥有该窗格的全部分栏状态。
//!
//! # 设计原理（Why）
//! - SFTP 状态挂在窗格上（`SessionPaneState`），随连接建立、断开、标签关闭
//!   而生死。若做成窗口级单实体，某个标签的传输进度变化会放大成整个实体
//!   重绘，等于回到整壳重绘，不符合"状态按生命周期归属"。
//! - 每窗格一份 `Entity<SftpPanelView>` 由窗格强持有；面板不持有外壳句柄，
//!   数据由外壳注入、意图用事件上报，不会形成强引用环。
//! - 状态只有一份：`SftpPanelView::state` 是面板状态的唯一事实源，
//!   `SessionPaneState` 只保存实体句柄，不保留任何影子副本。
//!
//! # 调用约束
//! 实体回调执行期间本实体已被 GPUI 租借。面板向外壳上报
//! [`super::SftpPanelEvent`] 时，外壳处理函数**不得**再 `read`/`update` 本实体，
//! 否则触发 “cannot update while it is already being updated” 崩溃。正确切分是：
//! 状态变更留在实体内完成，需要窗口的动作排进 `window_requests`，由外壳
//! 在渲染期统一执行。
//!
//! # 代价与局限
//! 外部事件处理（控制器回传的目录列表、进度、完成）需要先按标签找到实体
//! 再 `update`，比直接改字段多一次调度。

mod chmod;
mod clipboard;
mod delete;
mod drop_upload;
mod file_ops;
mod local_diff;
mod queue;
mod side_remote;
mod status;
mod task_control;

use super::SftpPanelState;
use crate::app::{AppShell, AppState};
use crate::keymap;
use gpui_kit::base::v_flex;
use gpui_kit::*;
use transom_core::{ConnectionId, TabId};

/// SFTP 面板视图实体。
pub(crate) struct SftpPanelView {
    /// 面板状态的唯一事实源。
    pub(crate) state: SftpPanelState,
    /// 所属窗格 id；命令投递与事件路由都按它寻址。
    pub(super) tab_id: TabId,
}

impl EventEmitter<super::SftpPanelEvent> for SftpPanelView {}

impl SftpPanelView {
    /// 接收外壳推送的并列远端候选；标签变化时调用。
    pub(crate) fn set_mount_candidates(&mut self, candidates: Vec<super::SftpMountCandidate>) {
        self.state.mount_candidates = candidates;
    }

    /// 用给定状态创建面板实体；创建后即可通过事件向外壳上报意图。
    pub(crate) fn new(state: SftpPanelState, tab_id: TabId) -> Self {
        Self { state, tab_id }
    }

    /// 当前挂载候选；外壳推送前用它做去抖比较。
    pub(crate) fn mount_candidates(&self) -> &[crate::sftp::SftpMountCandidate] {
        &self.state.mount_candidates
    }

    /// 面板状态只读访问。
    pub(crate) fn state(&self) -> &SftpPanelState {
        &self.state
    }
}

/// 实体渲染入口：把本实体的状态交给外壳的视图代码构建元素。
///
/// 渲染期间本实体已被 GPUI 租借，因此外壳拿到的只是 `&mut SftpPanelState`，
/// 不允许再按句柄回读本实体；状态与交互仍只有一份。
impl Render for SftpPanelView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.render_panel_content(window.rem_size(), cx);

        // 面板级键盘与鼠标监听挂在实体上：状态变更直接落在本实体，
        // 跨域动作经面板事件交给外壳执行。
        v_flex()
            .id(ElementId::Name("sftp-panel-container".into()))
            .track_focus(&self.state.focus_handle)
            .key_context(keymap::SFTP_CONTEXT)
            .on_action(cx.listener(Self::on_focus_filter))
            .on_action(cx.listener(Self::on_toggle_hidden))
            .on_action(cx.listener(Self::on_delete_selection))
            .on_key_down(cx.listener(Self::on_panel_key_down))
            .on_mouse_move(cx.listener(Self::on_panel_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_panel_mouse_up))
            .size_full()
            .overflow_hidden()
            .child(content)
    }
}

impl AppShell {
    /// 按窗格只读面板状态。
    ///
    /// 与旧 `tab.sftp()` 的差别：状态存放在实体里，读取需要 `cx`；
    /// 返回值借用 `cx`，调用方必须在借用结束后再修改外壳。
    pub(crate) fn sftp_state<'a>(&self, tab_id: TabId, cx: &'a App) -> Option<&'a SftpPanelState> {
        let view = self.state.sftp_view(tab_id)?;
        Some(view.read(cx).state())
    }

    /// 按窗格独占借用实体（含状态与实体方法）。
    ///
    /// 借用结束（`GpuiBorrow` 析构）时自动通知实体重绘；调用期间 `cx` 被
    /// 独占，必须先结束借用再调用其它 `&mut Context<Self>` 方法。
    pub(crate) fn sftp_view_mut<'a>(
        &self,
        tab_id: TabId,
        cx: &'a mut Context<Self>,
    ) -> Option<GpuiBorrow<'a, SftpPanelView>> {
        let view = self.state.sftp_view(tab_id)?;
        Some(view.as_mut(cx))
    }

    /// 按窗格与连接只读面板状态；连接不属于该窗格时返回 `None`。
    pub(crate) fn sftp_state_for_connection<'a>(
        &self,
        tab_id: TabId,
        connection_id: ConnectionId,
        cx: &'a App,
    ) -> Option<&'a SftpPanelState> {
        let view = self.state.sftp_view(tab_id)?;
        let view_ref = view.read(cx);
        if !self
            .state
            .sftp_connection_matches(tab_id, connection_id, view_ref)
        {
            return None;
        }
        Some(&view_ref.state)
    }

    /// 按窗格与连接独占借用实体；连接必须是该窗格主连接或已挂载的并列列。
    ///
    /// 事件回传只携带连接号：连接不属于该窗格时视为迟到事件，返回 `None`
    /// 而不是静默改到别的列上。
    pub(crate) fn sftp_view_mut_for_connection<'a>(
        &self,
        tab_id: TabId,
        connection_id: ConnectionId,
        cx: &'a mut Context<Self>,
    ) -> Option<GpuiBorrow<'a, SftpPanelView>> {
        let view = self.state.sftp_view(tab_id)?;
        if !self
            .state
            .sftp_connection_matches(tab_id, connection_id, view.read(cx))
        {
            return None;
        }
        Some(view.as_mut(cx))
    }
}

impl SftpPanelView {
    fn on_focus_filter(
        &mut self,
        _: &keymap::FocusSftpFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_focus_filter_action(window, cx);
    }

    fn on_toggle_hidden(
        &mut self,
        _: &keymap::ToggleSftpHidden,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_toggle_hidden_action(cx);
    }

    fn on_delete_selection(
        &mut self,
        _: &keymap::DeleteSftpSelection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_selected(cx);
    }

    /// 面板级快捷键：Ctrl/Cmd+C 复制选中项，Ctrl/Cmd+V 粘贴。
    fn on_panel_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.modifiers.control || event.keystroke.modifiers.platform {
            if event.keystroke.key == "c" {
                self.copy_selection(cx);
            } else if event.keystroke.key == "v" {
                self.paste_clipboard(cx);
            }
        }
    }

    fn on_panel_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_resize_queue(event.position.y.into(), cx);
    }

    fn on_panel_mouse_up(
        &mut self,
        _: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop_resize_queue(cx);
    }
}
impl AppState {
    /// 按窗格查找 SFTP 实体句柄；未建立面板或窗格已关闭时返回 `None`。
    pub(crate) fn sftp_view(&self, tab_id: TabId) -> Option<Entity<SftpPanelView>> {
        self.workspace.tab(tab_id)?.sftp_view()
    }

    /// 该窗格是否持有指定连接的远端列（主连接或并列列）。
    pub(crate) fn sftp_connection_matches(
        &self,
        tab_id: TabId,
        connection_id: ConnectionId,
        view: &SftpPanelView,
    ) -> bool {
        let main_connection = self.workspace.tab(tab_id).and_then(|tab| tab.connection_id);
        main_connection == Some(connection_id)
            || view
                .state
                .side_remotes
                .iter()
                .any(|side| side.connection_id == Some(connection_id))
    }

    /// 销毁窗格上的 SFTP 实体；返回此前是否存在。
    ///
    /// 只更新窗格上的句柄，不读写实体本身，因此可在实体回调回交外壳时调用。
    pub(crate) fn close_sftp_panel(&mut self, tab_id: TabId) -> bool {
        let Some(pane) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.session_pane_mut())
        else {
            return false;
        };
        pane.sftp_view.take().is_some()
    }
}
