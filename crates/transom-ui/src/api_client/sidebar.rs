//! API 项目侧栏实体：项目登记表、搜索与内联新建/重命名/删除确认状态。

use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_api::{ApiExportFormat, ApiProject, ApiProjectId, ApiRegistry};

/// 侧栏发往外壳的意图。
#[derive(Clone)]
pub(crate) enum ApiSidebarEvent {
    /// 打开或聚焦项目标签；事件携带完整项目，避免外壳回读刚派发事件的实体。
    OpenProject(ApiProject),
    /// 项目登记表已变化，由外壳异步落盘并协调已打开标签。
    PersistRegistry {
        registry: ApiRegistry,
        changed_project: Option<ApiProject>,
        removed_project: Option<ApiProjectId>,
        notice: Option<String>,
    },
    /// 选择外部 JSON 文件并导入一个或多个项目。
    ImportProject,
    /// 将指定项目导出为外部格式。
    ExportProject {
        project: ApiProject,
        format: ApiExportFormat,
    },
}

/// 项目名称编辑器的两种模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApiProjectEditorMode {
    Create,
    Rename(ApiProjectId),
}

/// 侧栏内联编辑器；输入框、错误与目标项目都随侧栏实体存活。
pub(crate) struct ApiProjectEditor {
    mode: ApiProjectEditorMode,
    pub(crate) input: Entity<InputState>,
    pub(crate) error: Option<String>,
}

/// API 侧栏状态。
///
/// 只保存应用级项目列表和侧栏自身交互状态；接口树、请求编辑器和响应状态
/// 全部属于各自 [`crate::api_client::ApiWorkspace`]。
pub(crate) struct ApiSidebarState {
    pub(crate) registry: ApiRegistry,
    pub(crate) search: Entity<InputState>,
    pub(crate) editor: Option<ApiProjectEditor>,
    pub(crate) pending_delete: Option<ApiProjectId>,
    pub(crate) load_error: Option<String>,
}

/// 单例 API 侧栏实体。
pub(crate) struct ApiSidebarView {
    pub(crate) state: ApiSidebarState,
}

impl EventEmitter<ApiSidebarEvent> for ApiSidebarView {}

impl ApiSidebarView {
    /// 创建侧栏实体并初始化项目搜索框。
    pub(crate) fn new(
        registry: ApiRegistry,
        load_error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(rust_i18n::t!("api.sidebar.search").to_string())
        });
        Self {
            state: ApiSidebarState {
                registry,
                search,
                editor: None,
                pending_delete: None,
                load_error,
            },
        }
    }

    /// 打开新建项目内联编辑器。
    pub(crate) fn start_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.pending_delete = None;
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.project.name_placeholder").to_string())
        });
        let focus = input.read(cx).focus_handle(cx);
        self.state.editor = Some(ApiProjectEditor {
            mode: ApiProjectEditorMode::Create,
            input: input.clone(),
            error: None,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    /// 打开重命名编辑器并预填当前项目名称。
    pub(crate) fn start_rename(
        &mut self,
        project_id: ApiProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.state.registry.project(project_id) else {
            return;
        };
        let initial = project.display_name().to_string();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.project.name_placeholder").to_string())
                .default_value(initial)
        });
        let focus = input.read(cx).focus_handle(cx);
        self.state.pending_delete = None;
        self.state.editor = Some(ApiProjectEditor {
            mode: ApiProjectEditorMode::Rename(project_id),
            input: input.clone(),
            error: None,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    /// 提交当前内联编辑器。
    pub(crate) fn submit_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.state.editor.take() else {
            return;
        };
        let name = editor.input.read(cx).value().trim().to_string();
        if name.is_empty() {
            let input = editor.input.clone();
            self.state.editor = Some(ApiProjectEditor {
                mode: editor.mode,
                input,
                error: Some(rust_i18n::t!("api.project.name_required").to_string()),
            });
            cx.notify();
            return;
        }

        match editor.mode {
            ApiProjectEditorMode::Create => self.create_project(name, cx),
            ApiProjectEditorMode::Rename(project_id) => self.rename_project(project_id, name, cx),
        }
    }

    fn create_project(&mut self, name: String, cx: &mut Context<Self>) {
        let project = ApiProject::new(name);
        self.state.registry.upsert(project.clone());
        self.state.load_error = None;
        cx.emit(ApiSidebarEvent::PersistRegistry {
            registry: self.state.registry.clone(),
            changed_project: Some(project.clone()),
            removed_project: None,
            notice: Some(rust_i18n::t!("api.project.created").to_string()),
        });
        cx.emit(ApiSidebarEvent::OpenProject(project));
        cx.notify();
    }

    fn rename_project(&mut self, project_id: ApiProjectId, name: String, cx: &mut Context<Self>) {
        let Some(project) = self
            .state
            .registry
            .projects
            .iter_mut()
            .find(|project| project.id == project_id)
        else {
            return;
        };
        project.name = name;
        project.touch();
        let changed = project.clone();
        cx.emit(ApiSidebarEvent::PersistRegistry {
            registry: self.state.registry.clone(),
            changed_project: Some(changed),
            removed_project: None,
            notice: None,
        });
        cx.notify();
    }

    /// 取消内联编辑器或删除确认。
    pub(crate) fn cancel_inline_action(&mut self, cx: &mut Context<Self>) {
        self.state.editor = None;
        self.state.pending_delete = None;
        cx.notify();
    }

    /// 打开删除确认；删除动作在确认前不改变登记表。
    pub(crate) fn request_delete(&mut self, project_id: ApiProjectId, cx: &mut Context<Self>) {
        self.state.editor = None;
        self.state.pending_delete = Some(project_id);
        cx.notify();
    }

    /// 确认删除项目。
    pub(crate) fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        let Some(project_id) = self.state.pending_delete.take() else {
            return;
        };
        if !self.state.registry.remove(project_id) {
            return;
        }
        cx.emit(ApiSidebarEvent::PersistRegistry {
            registry: self.state.registry.clone(),
            changed_project: None,
            removed_project: Some(project_id),
            notice: Some(rust_i18n::t!("api.project.deleted").to_string()),
        });
        cx.notify();
    }

    /// 合并导入项目；同一 id 已存在时以导入内容替换，否则追加。
    pub(crate) fn install_imported_projects(
        &mut self,
        projects: Vec<ApiProject>,
        cx: &mut Context<Self>,
    ) {
        if projects.is_empty() {
            return;
        }
        for project in &projects {
            self.state.registry.upsert(project.clone());
        }
        let first = projects.first().cloned();
        cx.emit(ApiSidebarEvent::PersistRegistry {
            registry: self.state.registry.clone(),
            changed_project: first.clone(),
            removed_project: None,
            notice: Some(rust_i18n::t!("api.project.imported", count = projects.len()).to_string()),
        });
        if let Some(project) = first {
            cx.emit(ApiSidebarEvent::OpenProject(project));
        }
        cx.notify();
    }

    /// 用项目最新快照更新侧栏登记表，供工作区保存事件回写。
    pub(crate) fn apply_project_snapshot(&mut self, project: ApiProject, cx: &mut Context<Self>) {
        self.state.registry.upsert(project);
        cx.notify();
    }

    /// 请求外壳打开指定项目标签。
    pub(crate) fn open_project(&self, project_id: ApiProjectId, cx: &mut Context<Self>) {
        if let Some(project) = self.state.registry.project(project_id) {
            cx.emit(ApiSidebarEvent::OpenProject(project.clone()));
        }
    }

    /// 当前搜索词。
    pub(crate) fn query(&self, cx: &App) -> String {
        self.state.search.read(cx).value().trim().to_string()
    }
}
