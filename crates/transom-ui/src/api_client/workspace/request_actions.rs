//! API 接口树中的请求选择与生命周期动作。

use super::{ApiWorkspace, ApiWorkspaceEvent};
use gpui_kit::*;
use transom_api::{
    ApiFolderId, ApiImportFormat, ApiProtocol, ApiRequest, ApiRequestId, import_projects,
    import_projects_auto,
};

impl ApiWorkspace {
    /// 当前选中请求的 clone；没有选中请求时返回 `None`。
    pub(crate) fn selected_request(&self) -> Option<&ApiRequest> {
        self.selected_request
            .and_then(|request_id| self.project.request(request_id))
    }

    /// 选择项目树中的请求，并把当前草稿先写回项目。
    pub(crate) fn select_request(
        &mut self,
        request_id: ApiRequestId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_request == Some(request_id) {
            return;
        }
        self.capture_editor(cx);
        if self.project.request(request_id).is_none() {
            return;
        }
        self.selected_request = Some(request_id);
        self.load_selected(window, cx);
        cx.notify();
    }

    /// 打开新建/导入请求弹窗。
    pub(crate) fn open_new_request_modal(&mut self, cx: &mut Context<Self>) {
        self.ui.new_request_modal_open = true;
        cx.notify();
    }

    /// 关闭新建/导入请求弹窗。
    pub(crate) fn close_new_request_modal(&mut self, cx: &mut Context<Self>) {
        self.ui.new_request_modal_open = false;
        cx.notify();
    }

    /// 新建请求并立即选中；保存动作仍由用户显式触发。
    pub(crate) fn new_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_request_with_protocol(ApiProtocol::Http, window, cx);
    }

    /// 按指定协议新建请求并立即选中。
    pub(crate) fn new_request_with_protocol(
        &mut self,
        protocol: ApiProtocol,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_editor(cx);
        let folder_id = self
            .selected_request()
            .and_then(|request| request.folder_id);
        let mut request = ApiRequest::new(
            rust_i18n::t!("api.request.default_name").to_string(),
            folder_id,
        );
        request.protocol = protocol;
        let request_id = request.id;
        self.project.upsert_request(request);
        self.selected_request = Some(request_id);
        self.ui.new_request_modal_open = false;
        self.load_selected(window, cx);
        self.emit_project_sync(cx);
        cx.notify();
    }

    /// 从剪贴板导入 cURL 或通用接口文本到当前项目。
    pub(crate) fn import_from_clipboard(
        &mut self,
        curl_only: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
        else {
            self.exec.error = Some(rust_i18n::t!("api.request.clipboard_empty").to_string());
            self.ui.new_request_modal_open = false;
            cx.notify();
            return;
        };
        let result = if curl_only {
            import_projects(&text, ApiImportFormat::Curl)
        } else {
            import_projects_auto(&text)
        };
        match result {
            Ok(projects) => {
                let mut first_id = None;
                for project in projects {
                    for request in project.requests {
                        if first_id.is_none() {
                            first_id = Some(request.id);
                        }
                        self.project.upsert_request(request);
                    }
                }
                if let Some(request_id) = first_id {
                    self.selected_request = Some(request_id);
                    self.load_selected(window, cx);
                    self.emit_project_sync(cx);
                }
                self.exec.error = None;
            }
            Err(error) => {
                self.exec.error = Some(crate::palette::localized_error(&error));
            }
        }
        self.ui.new_request_modal_open = false;
        cx.notify();
    }

    /// 删除当前请求并选中下一个可用请求。
    pub(crate) fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request_id) = self.selected_request else {
            return;
        };
        if !self.project.remove_request(request_id) {
            return;
        }
        self.selected_request = self.project.requests.first().map(|request| request.id);
        self.load_selected(window, cx);
        self.emit_project_sync(cx);
        cx.notify();
    }

    /// 复制当前请求，并立即选中副本继续编辑。
    pub(crate) fn duplicate_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.capture_editor(cx) {
            cx.notify();
            return;
        }
        let Some(request) = self.selected_request().cloned() else {
            return;
        };
        let duplicate = request.duplicate();
        let request_id = duplicate.id;
        self.project.upsert_request(duplicate);
        self.selected_request = Some(request_id);
        self.load_selected(window, cx);
        self.emit_project_sync(cx);
        cx.notify();
    }

    /// 把当前请求移动到指定目录，传 `None` 表示移回项目根层。
    pub(crate) fn move_selected_to_folder(
        &mut self,
        folder_id: Option<ApiFolderId>,
        cx: &mut Context<Self>,
    ) {
        if !self.capture_editor(cx) {
            cx.notify();
            return;
        }
        let Some(request_id) = self.selected_request else {
            return;
        };
        if folder_id.is_some_and(|id| !self.project.folders.iter().any(|folder| folder.id == id)) {
            return;
        }
        let Some(request) = self
            .project
            .requests
            .iter_mut()
            .find(|request| request.id == request_id)
        else {
            return;
        };
        request.folder_id = folder_id;
        self.project.touch();
        self.emit_project_sync(cx);
        cx.notify();
    }

    fn emit_project_sync(&self, cx: &mut Context<Self>) {
        cx.emit(ApiWorkspaceEvent::SyncProject {
            project: self.project.clone(),
        });
    }
}
