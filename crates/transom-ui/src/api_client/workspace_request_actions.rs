//! API 接口树中的请求选择与生命周期动作。

use super::ApiWorkspace;
use gpui_kit::*;
use transom_api::{ApiFolderId, ApiRequest, ApiRequestId};

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

    /// 新建请求并立即选中；保存动作仍由用户显式触发。
    pub(crate) fn new_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.capture_editor(cx);
        let folder_id = self
            .selected_request()
            .and_then(|request| request.folder_id);
        let request = ApiRequest::new(
            rust_i18n::t!("api.request.default_name").to_string(),
            folder_id,
        );
        let request_id = request.id;
        self.project.upsert_request(request);
        self.selected_request = Some(request_id);
        self.load_selected(window, cx);
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
        cx.notify();
    }
}
