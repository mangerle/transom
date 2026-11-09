//! 容器动作/传输结果与非传输失败：统一收尾工作区状态并给出右上角提示。
//!
//! 设计原理（Why）：收尾分支集中到 [`ContainerWorkspace`] 的具名方法，
//! 外壳只负责取提示文案、定位标签页与在需要时刷新列表。

use super::*;

impl AppShell {
    pub(super) fn on_container_action_finished(
        &mut self,
        request_id: ContainerRequestId,
        error: Option<AppError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = self.state.container_request_label(request_id, cx);
        let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
            return;
        };
        let mut refresh = false;
        let toast = match &error {
            Some(error) => {
                label.map(|label| (toast_failed(Some(&label), &localized_error(error)), false))
            }
            None => {
                refresh = true;
                label.map(|label| (toast_succeeded(&label), true))
            }
        };
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return;
        };
        let editor = workspace.update(cx, |workspace, _| {
            workspace.apply_action_finished(request_id, error.as_ref())
        });
        if let Some(editor) = editor {
            let new_content = editor.read(cx).value().to_string();
            workspace.update(cx, |workspace, _| {
                workspace.apply_preview_saved(new_content)
            });
        }
        if let Some((message, success)) = toast {
            self.show_toast(message, success, window, cx);
        }
        let (pods_open, quadlets_open) = {
            let state = &workspace.read(cx).state;
            (state.pods_dialog_open, state.quadlets_dialog_open)
        };
        if pods_open {
            workspace.update(cx, |workspace, cx| workspace.refresh_pods(cx));
        }
        if quadlets_open {
            workspace.update(cx, |workspace, cx| workspace.refresh_quadlets(cx));
        }
        if refresh {
            workspace.update(cx, |workspace, _| workspace.refresh_section());
        }
    }

    pub(super) fn on_container_transfer_finished(
        &mut self,
        request_id: ContainerRequestId,
        error: Option<AppError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = self.state.container_request_label(request_id, cx);
        let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
            return;
        };
        let toast = match &error {
            Some(error) => Some((
                toast_failed(label.as_deref(), &localized_error(error)),
                false,
            )),
            None => label.map(|label| (toast_succeeded(&label), true)),
        };
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return;
        };
        let (refresh_images, refresh_files) = workspace.update(cx, |workspace, _| {
            workspace.apply_transfer_finished(error.as_ref())
        });
        if let Some((message, success)) = toast {
            self.show_toast(message, success, window, cx);
        }
        if refresh_images {
            workspace.update(cx, |workspace, _| workspace.refresh_section());
        } else if let Some(path) = refresh_files
            && !path.is_empty()
        {
            workspace.update(cx, |workspace, cx| workspace.load_container_dir(path, cx));
        }
    }

    pub(super) fn on_container_failed(
        &mut self,
        request_id: ContainerRequestId,
        error: AppError,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = self.state.container_request_label(request_id, cx);
        let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
            return;
        };
        let err_desc = localized_error(&error);
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.apply_failed(request_id, &err_desc)
            });
        }
        // 只有明确具备动作意图的用户操作（label 为 Some）失败时才向右上角弹提示；
        // 后台静默采样、列表查询等只读请求失败只记录内部状态，杜绝弹窗轰炸。
        if let Some(label) = label {
            let message = toast_failed(Some(&label), &err_desc);
            self.show_toast(message, false, window, cx);
        }
    }

    pub(super) fn handle_prune_report(
        &mut self,
        request_id: ContainerRequestId,
        deleted_count: usize,
        target: ContainerPruneTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.state.take_container_request(request_id, cx) else {
            return;
        };
        let message = if deleted_count == 0 {
            rust_i18n::t!(target.none_key()).to_string()
        } else {
            rust_i18n::t!(target.done_key(), count = deleted_count).to_string()
        };
        self.show_toast(message, true, window, cx);
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| workspace.refresh_section());
        }
    }
}
