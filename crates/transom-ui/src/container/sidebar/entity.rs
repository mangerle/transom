//! 容器引擎侧栏实体：搜索、选中、分组表单与发现面板状态。
//!
//! 设计原理（Why）：侧栏状态随应用窗口存活，与标签页无关，因此单独成为
//! `Entity<ContainerSidebarView>`；渲染只读快照，目录（引擎、分组）由外壳在
//! 渲染期注入，实体不反查 `AppShell`。跨领域动作（打开工作区、落目录、删除
//! 连接、弹窗）通过 [`ContainerSidebarEvent`] 交外壳，事件自带全部数据。
//!
//! 代价与局限：侧栏是单例实体，目录变更后需要外壳显式注入新快照；事件处理
//! 在下一渲染帧统一执行，因此点击后到目录真正变化之间有一帧延迟。

use super::*;
use crate::container::types::EngineFolderForm;
use gpui_kit::component::input::InputEvent;
use transom_core::SessionId;

/// 侧栏发给外壳的动作意图。
///
/// 设计原理（Why）：侧栏不持有外壳句柄，也不读目录；所有需要目录写入、
/// 窗口或跨标签协调的动作都以事件上交，外壳处理时不回读侧栏实体。
#[derive(Debug, Clone)]
pub(crate) enum ContainerSidebarEvent {
    /// 打开（或聚焦）引擎工作区；profile 随事件携带。
    OpenEngine(Box<ContainerEngineProfile>),
    /// 打开引擎连接表单；`Some` 为编辑，`None` 为新建。
    OpenEngineForm(Option<Box<ContainerEngineProfile>>),
    /// 新建连接并放入指定分组。
    OpenNewEngineForm(Option<FolderId>),
    /// 删除引擎连接。
    DeleteEngine(EngineProfileId),
    /// 把引擎连接移动到根层级。
    MoveEngineToRoot(EngineProfileId),
    /// 折叠/展开引擎分组。
    ToggleFolder(FolderId),
    /// 提交分组新建/重命名表单。
    SubmitFolderForm {
        /// 父分组；`None` 为根层级。
        parent: Option<FolderId>,
        /// 正在重命名的分组；`None` 表示新建。
        editing: Option<FolderId>,
        /// 用户输入的名称（已去除首尾空白）。
        name: String,
    },
    /// 删除分组；其中连接与子分组提升到父层级。
    DeleteFolder(FolderId),
    /// 执行一次侧栏拖放。
    Drop {
        drag: ContainerSidebarDrag,
        target: ContainerSidebarTarget,
    },
    /// 发起本机引擎发现。
    DiscoverEngines,
    /// 添加一个发现到的引擎。
    AddDiscoveredEngine(Box<ContainerEngineProfile>),
    /// 为既有 SSH 会话打开容器工作区。
    OpenSessionWorkspace(SessionId),
    /// 一次性提示：实体拿不到窗口，由外壳在渲染期弹出。
    ShowToast {
        /// 已本地化的提示文案。
        message: String,
        /// 是否为成功提示。
        success: bool,
    },
}

impl EventEmitter<ContainerSidebarEvent> for ContainerSidebarView {}

/// 容器引擎侧栏的界面状态。
pub(crate) struct ContainerSidebarViewState {
    /// 侧栏搜索框；随实体创建，整窗复用。
    pub(crate) search: Entity<InputState>,
    /// 当前单选高亮的引擎连接。
    pub(crate) selected: Option<EngineProfileId>,
    /// 引擎分组的新建/重命名弹窗。
    pub(crate) engine_folder_form: Option<EngineFolderForm>,
    /// 最近一次「发现本机引擎」的结果面板。
    pub(crate) discovery: Option<ContainerDiscovery>,
    /// 正在进行的发现请求，避免连点重复探测。
    pub(crate) discovering: bool,
    /// 搜索框变化时通知侧栏实体，外壳再经观察器整壳重绘。
    _search_subscription: Subscription,
}

/// 容器引擎侧栏实体。
pub(crate) struct ContainerSidebarView {
    pub(crate) state: ContainerSidebarViewState,
}

impl ContainerSidebarView {
    /// 创建侧栏实体并初始化搜索框。
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.sidebar.search").to_string())
        });
        let search_subscription = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        Self {
            state: ContainerSidebarViewState {
                search,
                selected: None,
                engine_folder_form: None,
                discovery: None,
                discovering: false,
                _search_subscription: search_subscription,
            },
        }
    }

    /// 高亮某个引擎连接。
    pub(crate) fn select_engine(&mut self, id: EngineProfileId, cx: &mut Context<Self>) {
        self.state.selected = Some(id);
        cx.notify();
    }

    /// 打开引擎分组的新建/重命名弹窗。
    ///
    /// 设计原理（Why）：实体不读目录，当前名称由调用方从渲染快照带入，
    /// 避免为了一个初始值回查外壳。
    pub(crate) fn open_folder_form(
        &mut self,
        parent: Option<FolderId>,
        editing: Option<FolderId>,
        current_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(current_name)
                .placeholder(rust_i18n::t!("container.sidebar.folder_name").to_string())
        });
        input.update(cx, |state, cx| state.focus(window, cx));
        self.state.engine_folder_form = Some(EngineFolderForm {
            parent,
            editing,
            input,
            error: None,
        });
        cx.notify();
    }

    /// 关闭分组表单。
    pub(crate) fn dismiss_folder_form(&mut self, cx: &mut Context<Self>) {
        self.state.engine_folder_form = None;
        cx.notify();
    }

    /// 提交分组表单：名称校验与目录写入归外壳，失败时外壳回写错误。
    pub(crate) fn submit_folder_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.state.engine_folder_form.as_ref() else {
            return;
        };
        let name = form.input.read(cx).value().trim().to_string();
        let parent = form.parent;
        let editing = form.editing;
        cx.emit(ContainerSidebarEvent::SubmitFolderForm {
            parent,
            editing,
            name,
        });
        cx.notify();
    }

    /// 回写分组表单错误并保持弹窗打开。
    pub(crate) fn set_folder_form_error(&mut self, error: String, cx: &mut Context<Self>) {
        if let Some(form) = self.state.engine_folder_form.as_mut() {
            form.error = Some(error);
        }
        cx.notify();
    }

    /// 目录写入成功后关闭分组表单。
    pub(crate) fn finish_folder_form(&mut self, cx: &mut Context<Self>) {
        self.state.engine_folder_form = None;
        cx.notify();
    }

    /// 标记发现请求进行中并清空旧结果。
    pub(crate) fn begin_discovery(&mut self, cx: &mut Context<Self>) {
        self.state.discovering = true;
        self.state.discovery = None;
        cx.notify();
    }

    /// 应用发现结果。
    pub(crate) fn apply_discovery(
        &mut self,
        discovery: ContainerDiscovery,
        cx: &mut Context<Self>,
    ) {
        self.state.discovering = false;
        self.state.discovery = Some(discovery);
        cx.notify();
    }

    /// 收起发现结果面板。
    pub(crate) fn dismiss_discovery(&mut self, cx: &mut Context<Self>) {
        self.state.discovery = None;
        cx.notify();
    }

    /// 从发现结果中移除一个已添加的候选。
    pub(crate) fn remove_discovery_candidate(&mut self, endpoint: &str, cx: &mut Context<Self>) {
        if let Some(discovery) = self.state.discovery.as_mut() {
            discovery
                .candidates
                .retain(|candidate| candidate.engine.endpoint.describe() != endpoint);
        }
        cx.notify();
    }

    /// 请求打开（或聚焦）引擎工作区。
    pub(crate) fn request_open_engine(
        &mut self,
        profile: ContainerEngineProfile,
        cx: &mut Context<Self>,
    ) {
        cx.emit(ContainerSidebarEvent::OpenEngine(Box::new(profile)));
    }

    /// 请求打开引擎连接表单；`Some` 为编辑，`None` 为新建。
    pub(crate) fn request_engine_form(
        &mut self,
        profile: Option<ContainerEngineProfile>,
        cx: &mut Context<Self>,
    ) {
        cx.emit(ContainerSidebarEvent::OpenEngineForm(profile.map(Box::new)));
    }

    /// 请求新建连接并放入指定分组。
    pub(crate) fn request_new_engine(&mut self, folder: Option<FolderId>, cx: &mut Context<Self>) {
        cx.emit(ContainerSidebarEvent::OpenNewEngineForm(folder));
    }

    /// 请求删除引擎连接。
    pub(crate) fn request_delete_engine(&mut self, id: EngineProfileId, cx: &mut Context<Self>) {
        cx.emit(ContainerSidebarEvent::DeleteEngine(id));
    }

    /// 请求把引擎连接移动到根层级。
    pub(crate) fn request_move_engine_to_root(
        &mut self,
        id: EngineProfileId,
        cx: &mut Context<Self>,
    ) {
        cx.emit(ContainerSidebarEvent::MoveEngineToRoot(id));
    }

    /// 请求折叠/展开分组。
    pub(crate) fn request_toggle_folder(&mut self, id: FolderId, cx: &mut Context<Self>) {
        cx.emit(ContainerSidebarEvent::ToggleFolder(id));
    }

    /// 请求删除分组。
    pub(crate) fn request_delete_folder(&mut self, id: FolderId, cx: &mut Context<Self>) {
        cx.emit(ContainerSidebarEvent::DeleteFolder(id));
    }

    /// 请求发起本机引擎发现。
    pub(crate) fn request_discover(&mut self, cx: &mut Context<Self>) {
        cx.emit(ContainerSidebarEvent::DiscoverEngines);
    }

    /// 请求添加一个发现到的引擎。
    pub(crate) fn request_add_discovered(
        &mut self,
        profile: ContainerEngineProfile,
        cx: &mut Context<Self>,
    ) {
        cx.emit(ContainerSidebarEvent::AddDiscoveredEngine(Box::new(
            profile,
        )));
    }

    /// 请求为 SSH 会话打开容器工作区。
    pub(crate) fn request_open_session(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        cx.emit(ContainerSidebarEvent::OpenSessionWorkspace(session_id));
    }

    /// 请求外壳弹出一条一次性提示。
    pub(crate) fn request_toast(&mut self, message: String, success: bool, cx: &mut Context<Self>) {
        cx.emit(ContainerSidebarEvent::ShowToast { message, success });
    }

    /// 请求外壳执行一次拖放。
    pub(crate) fn request_drop(
        &mut self,
        drag: ContainerSidebarDrag,
        target: ContainerSidebarTarget,
        cx: &mut Context<Self>,
    ) {
        cx.emit(ContainerSidebarEvent::Drop { drag, target });
    }
}
