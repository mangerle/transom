//! 渲染期窗口动作队列。
//!
//! 设计原理（Why）：面板实体在事件派发阶段被 GPUI 租借，拿不到 `Window`，
//! 需要窗口的动作（打开弹窗、聚焦、调整布局）必须延后到渲染期。此前四个
//! 领域各有一个 `pending_*_requests` 向量，外壳每帧按固定顺序分别消费；
//! 队列合并后只有一个消费入口，新增请求类型不会漏接 flush。
//!
//! 代价与局限：合并后按入队顺序执行，跨领域动作的先后由产生顺序决定；
//! 需要严格先后的动作应在产生点排好，而不是依赖队列分组的固定次序。

use crate::api_client::{ApiSidebarEvent, ApiWorkspaceEvent};
use crate::container::ContainerWorkspaceEvent;
use crate::editor::{EditorCloseRequest, EditorPanelEvent};
use crate::git::GitWorkspaceEvent;
use crate::monitor::MonitorPanelEvent;
use crate::settings::SettingsRequest;
use crate::sftp::SftpPanelEvent;
use transom_core::{GroupId, TabId};

/// 一条需要窗口才能执行的请求。
pub(crate) enum WindowRequest {
    Sftp(SftpPanelEvent),
    Monitor(MonitorPanelEvent),
    Editor(EditorPanelEvent),
    Settings(SettingsRequest),
    /// 容器工作区动作：标签 id 在事件订阅时确定，执行时不回读实体。
    Container {
        tab_id: TabId,
        event: ContainerWorkspaceEvent,
    },
    /// 容器侧栏动作：侧栏是单例实体，事件自带全部数据。
    ContainerSidebar(crate::container::ContainerSidebarEvent),
    /// Git 侧栏发往外壳的意图动作。
    GitSidebar(crate::git::GitSidebarEvent),
    /// API 项目侧栏发往外壳的意图动作。
    ApiSidebar(ApiSidebarEvent),
    /// API 工作区发往外壳的窗口级动作。
    ApiWorkspace {
        tab_id: TabId,
        event: ApiWorkspaceEvent,
    },
    /// 会话侧栏发往外壳的意图动作。
    SessionSidebar(crate::sidebar::SessionSidebarEvent),
    /// Git 工作区窗口级动作。
    Git(GitWorkspaceEvent),
    /// 激活指定标签并同步窗口焦点。
    ActivateTab(TabId),
    /// 打开设置弹窗并定位到指定页。
    OpenSettings(Option<usize>),
    /// 编辑器脏检查完成后的关闭动作。
    EditorClose(EditorCloseRequest),
    /// 右上角操作结果提示；展示依赖窗口，延后到渲染期执行。
    Toast {
        message: String,
        success: bool,
    },
    /// 关闭包含指定标签的整组窗格。
    CloseTab(TabId),
    /// 关闭指定标签组。
    CloseGroup(GroupId),
    /// 关闭单个标签窗格。
    ClosePane(TabId),
    /// 在新标签打开本地终端（可选初始工作目录）。
    LocalTerminal(Option<std::path::PathBuf>),
    /// 底栏瞬态提示：由外壳在渲染期设置并自动清除。
    TransientStatus(String),
    /// 打开会话表单：`None` 为新建，`Some` 为编辑对应会话。
    OpenSessionForm(Option<transom_core::SessionId>),
}

/// 有界请求队列：只在当前帧内积压，渲染期一次取空。
#[derive(Default)]
pub(crate) struct WindowRequests {
    items: Vec<WindowRequest>,
}

/// 单帧窗口请求上限；超过后先淘汰可覆盖请求，再拒绝新增请求。
const WINDOW_REQUEST_LIMIT: usize = 256;

impl WindowRequests {
    /// 入队一条请求；可覆盖项只保留最新一条，队列满时优先淘汰低优先级项。
    fn push(&mut self, request: WindowRequest) {
        if Self::is_coalescible(&request) {
            self.items
                .retain(|existing| !Self::same_coalescible_kind(existing, &request));
        } else if self.items.len() >= WINDOW_REQUEST_LIMIT {
            if let Some(index) = self.items.iter().position(Self::is_coalescible) {
                self.items.remove(index);
                log::warn!("窗口请求队列已满，已淘汰一条可覆盖请求");
            } else {
                log::warn!("窗口请求队列已满，新增请求被丢弃");
                return;
            }
        }
        if self.items.len() < WINDOW_REQUEST_LIMIT {
            self.items.push(request);
        } else {
            log::warn!("窗口请求队列已满，新增请求被丢弃");
        }
    }

    fn is_coalescible(request: &WindowRequest) -> bool {
        matches!(
            request,
            WindowRequest::Toast { .. }
                | WindowRequest::TransientStatus(_)
                | WindowRequest::ActivateTab(_)
        )
    }

    fn same_coalescible_kind(left: &WindowRequest, right: &WindowRequest) -> bool {
        matches!(
            (left, right),
            (WindowRequest::Toast { .. }, WindowRequest::Toast { .. })
                | (
                    WindowRequest::TransientStatus(_),
                    WindowRequest::TransientStatus(_)
                )
                | (WindowRequest::ActivateTab(_), WindowRequest::ActivateTab(_))
        )
    }

    pub(crate) fn push_sftp(&mut self, request: SftpPanelEvent) {
        self.push(WindowRequest::Sftp(request));
    }

    pub(crate) fn push_monitor(&mut self, request: MonitorPanelEvent) {
        self.push(WindowRequest::Monitor(request));
    }

    pub(crate) fn push_editor(&mut self, request: EditorPanelEvent) {
        self.push(WindowRequest::Editor(request));
    }

    pub(crate) fn push_settings(&mut self, request: SettingsRequest) {
        self.push(WindowRequest::Settings(request));
    }

    /// 容器工作区的窗口动作：事件在派发阶段不持有实体句柄，直接连标签 id 入队。
    pub(crate) fn push_container(&mut self, tab_id: TabId, event: ContainerWorkspaceEvent) {
        self.push(WindowRequest::Container { tab_id, event });
    }

    pub(crate) fn push_container_sidebar(
        &mut self,
        event: crate::container::ContainerSidebarEvent,
    ) {
        self.push(WindowRequest::ContainerSidebar(event));
    }

    pub(crate) fn push_git_sidebar(&mut self, event: crate::git::GitSidebarEvent) {
        self.push(WindowRequest::GitSidebar(event));
    }

    pub(crate) fn push_api_sidebar(&mut self, event: ApiSidebarEvent) {
        self.push(WindowRequest::ApiSidebar(event));
    }

    pub(crate) fn push_api_workspace(&mut self, tab_id: TabId, event: ApiWorkspaceEvent) {
        self.push(WindowRequest::ApiWorkspace { tab_id, event });
    }

    pub(crate) fn push_session_sidebar(&mut self, event: crate::sidebar::SessionSidebarEvent) {
        self.push(WindowRequest::SessionSidebar(event));
    }

    pub(crate) fn push_git(&mut self, event: GitWorkspaceEvent) {
        self.push(WindowRequest::Git(event));
    }

    pub(crate) fn push_activate_tab(&mut self, tab_id: TabId) {
        self.push(WindowRequest::ActivateTab(tab_id));
    }

    pub(crate) fn push_editor_close(&mut self, request: EditorCloseRequest) {
        self.push(WindowRequest::EditorClose(request));
    }

    pub(crate) fn push_open_settings(&mut self, page: Option<usize>) {
        self.push(WindowRequest::OpenSettings(page));
    }

    /// 排队一条结果提示：提示需要窗口，统一交给渲染期展示。
    pub(crate) fn push_toast(&mut self, message: impl Into<String>, success: bool) {
        self.push(WindowRequest::Toast {
            message: message.into(),
            success,
        });
    }

    /// 关闭标签所属的整组窗格。
    pub(crate) fn push_close_tab(&mut self, tab_id: TabId) {
        self.push(WindowRequest::CloseTab(tab_id));
    }

    /// 关闭指定标签组。
    pub(crate) fn push_close_group(&mut self, group_id: GroupId) {
        self.push(WindowRequest::CloseGroup(group_id));
    }

    /// 关闭单个标签窗格（与整组关闭区分）。
    pub(crate) fn push_close_pane(&mut self, tab_id: TabId) {
        self.push(WindowRequest::ClosePane(tab_id));
    }

    /// 在新的标签页打开本地终端。
    pub(crate) fn push_local_terminal(&mut self, cwd: Option<std::path::PathBuf>) {
        self.push(WindowRequest::LocalTerminal(cwd));
    }

    /// 排队一条底栏瞬态提示。
    pub(crate) fn push_transient_status(&mut self, message: impl Into<String>) {
        self.push(WindowRequest::TransientStatus(message.into()));
    }

    /// 打开会话表单：`None` 为新建，`Some` 为编辑。
    pub(crate) fn push_open_session_form(&mut self, session_id: Option<transom_core::SessionId>) {
        self.push(WindowRequest::OpenSessionForm(session_id));
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 取走本轮全部请求；渲染期逐个执行。
    pub(crate) fn take(&mut self) -> Vec<WindowRequest> {
        std::mem::take(&mut self.items)
    }
}

#[cfg(test)]
mod tests {
    use super::{WINDOW_REQUEST_LIMIT, WindowRequest, WindowRequests};

    #[test]
    fn coalescible_requests_keep_latest_only() {
        let mut requests = WindowRequests::default();
        requests.push_toast("第一条", false);
        requests.push_toast("第二条", true);
        requests.push_transient_status("旧提示");
        requests.push_transient_status("新提示");
        let items = requests.take();
        assert_eq!(items.len(), 2);
        assert!(matches!(
            &items[0],
            WindowRequest::Toast { message, success }
                if message == "第二条" && *success
        ));
        assert!(matches!(
            &items[1],
            WindowRequest::TransientStatus(message) if message == "新提示"
        ));
    }

    #[test]
    fn queue_limit_evicts_coalescible_requests_before_critical_ones() {
        let mut requests = WindowRequests::default();
        for _ in 0..(WINDOW_REQUEST_LIMIT - 1) {
            requests.push_close_pane(transom_core::TabId::new());
        }
        requests.push_transient_status("低优先级提示");
        let tab_id = transom_core::TabId::new();
        requests.push_close_tab(tab_id);
        assert_eq!(requests.items.len(), WINDOW_REQUEST_LIMIT);
        assert!(
            requests
                .items
                .iter()
                .any(|request| matches!(request, WindowRequest::CloseTab(id) if *id == tab_id))
        );
        assert!(
            !requests
                .items
                .iter()
                .any(|request| matches!(request, WindowRequest::TransientStatus(_)))
        );
    }
}
