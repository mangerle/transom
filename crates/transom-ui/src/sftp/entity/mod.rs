//! SFTP 面板实体：每个持有 SFTP 的窗格一份，拥有该窗格的全部分栏状态。
//!
//! # 设计原理（Why）
//! - SFTP 状态挂在窗格上（`SessionPaneState`），随连接建立、断开、标签关闭
//!   而生死。若做成窗口级单实体，某个标签的传输进度变化会放大成整个实体
//!   重绘，等于回到整壳重绘，不符合"状态按生命周期归属"。
//! - 每窗格一份 `Entity<SftpPanelView>` 由窗格强持有，实体用
//!   `WeakEntity<AppShell>` 回指外壳：外壳不会因子实体而被延长生命周期，
//!   也不会形成强引用环。
//! - 状态只有一份：`SftpPanelView::state` 是面板状态的唯一事实源，
//!   `SessionPaneState` 只保存实体句柄，不保留任何影子副本。
//!
//! # 调用约束
//! 实体回调执行期间本实体已被 GPUI 租借。回调里回交外壳（`with_shell`）时，
//! 外壳方法**不得**再 `read`/`update` 本实体，否则触发
//! “cannot update while it is already being updated” 崩溃。正确切分是：
//! 状态变更留在实体内完成，外壳只接收结果（快照、布尔值、参数）。
//!
//! # 代价与局限
//! 外部事件处理（控制器回传的目录列表、进度、完成）需要先按标签找到实体
//! 再 `update`，比直接改字段多一次调度；实体构造时外壳已存在，因此
//! `attach` 在创建后立即调用，不存在未绑定窗口期。

mod queue;
mod status;

use super::SftpPanelState;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::*;
use transom_core::{ConnectionId, TabId};

/// SFTP 面板视图实体。
pub(crate) struct SftpPanelView {
    /// 面板状态的唯一事实源。
    pub(crate) state: SftpPanelState,
    /// 回到外壳的弱引用；创建后立即绑定。
    app: Option<WeakEntity<AppShell>>,
}

impl SftpPanelView {
    /// 用已构造好的面板状态创建实体；`attach` 负责绑定外壳。
    pub(crate) fn new(state: SftpPanelState) -> Self {
        Self { state, app: None }
    }

    /// 绑定外壳弱引用；必须在实体创建后立即调用一次。
    pub(crate) fn attach(&mut self, app: WeakEntity<AppShell>) {
        self.app = Some(app);
    }

    /// 面板状态只读访问。
    pub(crate) fn state(&self) -> &SftpPanelState {
        &self.state
    }
}

/// 回到外壳执行一次跨域动作；外壳已释放时返回 `None`。
///
/// 供实体方法在 `&mut self.state` 借用仍然有效时使用：调用方先解构
/// `SftpPanelView { state, app }`，把 `app` 传进来，状态留在本地。
pub(super) fn with_shell_parts<R>(
    app: &Option<WeakEntity<AppShell>>,
    cx: &mut Context<SftpPanelView>,
    action: impl FnOnce(&mut AppShell, &mut Context<AppShell>) -> R,
) -> Option<R> {
    let app = app.as_ref()?.upgrade()?;
    Some(app.update(cx, action))
}

/// 只读外壳状态；用于取设置、连接与工作区快照。
pub(super) fn read_shell_parts<R>(
    app: &Option<WeakEntity<AppShell>>,
    cx: &App,
    read: impl FnOnce(&AppShell) -> R,
) -> Option<R> {
    let app = app.as_ref()?.upgrade()?;
    Some(read(app.read(cx)))
}

/// 视图对面板状态的透明访问。
///
/// 迁移期允许 `view.remote` 这类字段访问直接落到状态上；特意不实现反向
/// 依赖（状态不回指实体），避免两个类型生命周期纠缠。
impl std::ops::Deref for SftpPanelView {
    type Target = SftpPanelState;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl std::ops::DerefMut for SftpPanelView {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

/// 实体渲染入口：把本实体的状态交给外壳的视图代码构建元素。
///
/// 渲染期间本实体已被 GPUI 租借，因此外壳拿到的只是 `&mut SftpPanelState`，
/// 不允许再按句柄回读本实体；状态与交互仍只有一份。
impl Render for SftpPanelView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let SftpPanelView { state, app } = self;
        let element = app.as_ref().and_then(|app| app.upgrade()).map(|shell| {
            shell.update(cx, |shell, shell_cx| {
                let p = Palette::from(&*shell_cx);
                shell.render_sftp_panel_with_state(state, window, shell_cx, &p)
            })
        });
        element.unwrap_or_else(|| div().into_any_element())
    }
}

impl AppShell {
    /// 按窗格查找 SFTP 实体句柄；未建立面板或窗格已关闭时返回 `None`。
    pub(crate) fn sftp_view(&self, tab_id: TabId) -> Option<Entity<SftpPanelView>> {
        self.state.workspace.tab(tab_id)?.sftp_view()
    }

    /// 按窗格只读面板状态。
    ///
    /// 与旧 `tab.sftp()` 的差别：状态存放在实体里，读取需要 `cx`；
    /// 返回值借用 `cx`，调用方必须在借用结束后再修改外壳。
    pub(crate) fn sftp_state<'a>(&self, tab_id: TabId, cx: &'a App) -> Option<&'a SftpPanelState> {
        let view = self.sftp_view(tab_id)?;
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
        let view = self.sftp_view(tab_id)?;
        Some(view.as_mut(cx))
    }

    /// 该窗格是否持有指定连接的远端列（主连接或并列列）。
    fn sftp_connection_matches(
        &self,
        tab_id: TabId,
        connection_id: ConnectionId,
        view: &SftpPanelView,
    ) -> bool {
        let main_connection = self
            .state
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.connection_id);
        main_connection == Some(connection_id)
            || view
                .state
                .side_remotes
                .iter()
                .any(|side| side.connection_id == Some(connection_id))
    }

    /// 按窗格与连接只读面板状态；连接不属于该窗格时返回 `None`。
    pub(crate) fn sftp_state_for_connection<'a>(
        &self,
        tab_id: TabId,
        connection_id: ConnectionId,
        cx: &'a App,
    ) -> Option<&'a SftpPanelState> {
        let view = self.sftp_view(tab_id)?;
        let view_ref = view.read(cx);
        if !self.sftp_connection_matches(tab_id, connection_id, view_ref) {
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
        let view = self.sftp_view(tab_id)?;
        if !self.sftp_connection_matches(tab_id, connection_id, view.read(cx)) {
            return None;
        }
        Some(view.as_mut(cx))
    }

    /// 销毁窗格上的 SFTP 实体；返回此前是否存在。
    ///
    /// 只更新窗格上的句柄，不读写实体本身，因此可在实体回调回交外壳时调用。
    pub(crate) fn close_sftp_panel(&mut self, tab_id: TabId) -> bool {
        let Some(pane) = self
            .state
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.session_pane_mut())
        else {
            return false;
        };
        pane.sftp_view.take().is_some()
    }
}
