//! 容器动作/传输结果与非传输失败：统一收尾工作区状态并给出右上角提示。

use super::*;

impl AppShell {
    pub(super) fn on_container_action_finished(
        &mut self,
        request_id: ContainerRequestId,
        error: Option<AppError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = self.container_request_label(request_id, cx);
        let Some(tab_id) = self.take_container_request(request_id, cx) else {
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
        let mut editor_to_read = None;
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.busy = false;
            workspace.confirm = None;
            workspace.rename_container = None;
            workspace.notice = None;
            if workspace.pods_action_request == Some(request_id) {
                workspace.pods_action_request = None;
                workspace.pods_loading = false;
                if let Some(err) = &error {
                    workspace.pods_error = Some(localized_error(err));
                }
            }
            if workspace.quadlets_action_request == Some(request_id) {
                workspace.quadlets_action_request = None;
                workspace.quadlets_loading = false;
                if let Some(err) = &error {
                    workspace.quadlets_error = Some(localized_error(err));
                }
            }
            if let Some(build) = workspace.build_dialog.as_mut()
                && build.request_id == Some(request_id)
            {
                build.building = false;
                if let Some(err) = &error {
                    build.error = Some(localized_error(err));
                } else {
                    build.success = true;
                }
            }
            if workspace.files.preview_saving {
                workspace.files.preview_saving = false;
                if error.is_none() {
                    editor_to_read = workspace.files.preview_editor.take();
                }
            }
        });
        if let Some(editor) = editor_to_read {
            let new_content = editor.read(cx).value().to_string();
            self.with_container_mut(tab_id, cx, |workspace| {
                if let Some((path, _, _)) = workspace.files.preview.as_ref() {
                    let path = path.clone();
                    workspace.files.preview = Some((path, new_content, false));
                }
            });
        }
        if let Some((message, success)) = toast {
            self.show_toast(message, success, window, cx);
        }
        if self
            .with_container(tab_id, cx, |workspace| workspace.pods_dialog_open)
            .unwrap_or(false)
        {
            self.refresh_container_pods(tab_id, cx);
        }
        if self
            .with_container(tab_id, cx, |workspace| workspace.quadlets_dialog_open)
            .unwrap_or(false)
        {
            self.refresh_container_quadlets(tab_id, cx);
        }
        if refresh && let Some(workspace) = self.state.container_workspace(tab_id) {
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
        let label = self.container_request_label(request_id, cx);
        let Some(tab_id) = self.take_container_request(request_id, cx) else {
            return;
        };
        let toast = match &error {
            Some(error) => Some((
                toast_failed(label.as_deref(), &localized_error(error)),
                false,
            )),
            None => label.map(|label| (toast_succeeded(&label), true)),
        };
        let mut refresh_files = None;
        let mut refresh_images = false;
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.notice = None;
            if error.is_none() {
                if workspace.section == ContainerSection::Images {
                    refresh_images = true;
                } else {
                    refresh_files = Some(workspace.files.path.clone());
                }
            }
        });
        if let Some((message, success)) = toast {
            self.show_toast(message, success, window, cx);
        }
        if refresh_images {
            if let Some(workspace) = self.state.container_workspace(tab_id) {
                workspace.update(cx, |workspace, _| workspace.refresh_section());
            }
        } else if let Some(path) = refresh_files
            && !path.is_empty()
            && let Some(workspace) = self.state.container_workspace(tab_id)
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
        let label = self.container_request_label(request_id, cx);
        let Some(tab_id) = self.take_container_request(request_id, cx) else {
            return;
        };
        let err_desc = localized_error(&error);
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.busy = false;
            workspace.files.loading = false;
            workspace.notice = None;
            if workspace.logs_request == Some(request_id) {
                workspace.logs_request = None;
                workspace.logs_follow = false;
            }
            if workspace.stats_request == Some(request_id) {
                workspace.stats_request = None;
                workspace.stats_error = Some(err_desc.clone());
            }
            if workspace.stats_processes_request == Some(request_id) {
                workspace.stats_processes_request = None;
                workspace.stats_processes_loading = false;
                let friendly_err = if err_desc.to_lowercase().contains("is not running") {
                    rust_i18n::t!("container.stats.container_stopped").to_string()
                } else {
                    err_desc.clone()
                };
                workspace.stats_processes_error = Some(friendly_err);
            }
            if workspace.files.request == Some(request_id) {
                workspace.files.request = None;
                workspace.files.loading = false;
            }
            if workspace.pods_request == Some(request_id) {
                workspace.pods_request = None;
                workspace.pods_loading = false;
                workspace.pods_error = Some(err_desc.clone());
            }
            if workspace.pods_action_request == Some(request_id) {
                workspace.pods_action_request = None;
                workspace.pods_loading = false;
                workspace.pods_error = Some(err_desc.clone());
            }
            if workspace.quadlets_request == Some(request_id) {
                workspace.quadlets_request = None;
                workspace.quadlets_loading = false;
                workspace.quadlets_error = Some(err_desc.clone());
            }
            if workspace.quadlets_action_request == Some(request_id) {
                workspace.quadlets_action_request = None;
                workspace.quadlets_loading = false;
                workspace.quadlets_error = Some(err_desc.clone());
            }
            if workspace.changes_request == Some(request_id) {
                workspace.changes_request = None;
                workspace.changes_loading = false;
            }
            if let Some(hub) = workspace.hub_dialog.as_mut() {
                if hub.search_request == Some(request_id) {
                    hub.searching = false;
                    hub.search_request = None;
                    hub.search_error = Some(err_desc.clone());
                }
                if hub.tags_request == Some(request_id) {
                    hub.tags_loading = false;
                    hub.tags_request = None;
                    hub.tags_error = Some(err_desc.clone());
                }
            }
            if workspace
                .terminal_request
                .as_ref()
                .is_some_and(|(pending, ..)| *pending == request_id)
            {
                workspace.terminal_request = None;
            }
        });
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
        let Some(tab_id) = self.take_container_request(request_id, cx) else {
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
