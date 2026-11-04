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
        let label = self.state.containers.label_of(request_id);
        let Some(tab_id) = self.state.containers.take_tab(request_id) else {
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
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.busy = false;
            workspace.confirm = None;
            workspace.rename_container = None;
            workspace.notice = None;
            if let Some(build) = workspace.build_dialog.as_mut() {
                if build.request_id == Some(request_id) {
                    build.building = false;
                    if let Some(err) = &error {
                        build.error = Some(localized_error(err));
                    } else {
                        build.success = true;
                    }
                }
            }
            if workspace.files.preview_saving {
                workspace.files.preview_saving = false;
                if error.is_none() {
                    if let Some(editor) = workspace.files.preview_editor.take() {
                        let new_content = editor.read(cx).value().to_string();
                        if let Some((path, _, _)) = workspace.files.preview.as_mut() {
                            let p = path.clone();
                            workspace.files.preview = Some((p, new_content, false));
                        }
                    }
                }
            }
        }
        if let Some((message, success)) = toast {
            self.show_toast(message, success, window, cx);
        }
        if refresh {
            self.refresh_container_section(tab_id);
        }
    }

    pub(super) fn on_container_transfer_finished(
        &mut self,
        request_id: ContainerRequestId,
        error: Option<AppError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = self.state.containers.label_of(request_id);
        let Some(tab_id) = self.state.containers.take_tab(request_id) else {
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
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.notice = None;
            if error.is_none() {
                if workspace.section == ContainerSection::Images {
                    refresh_images = true;
                } else {
                    refresh_files = Some(workspace.files.path.clone());
                }
            }
        }
        if let Some((message, success)) = toast {
            self.show_toast(message, success, window, cx);
        }
        if refresh_images {
            self.refresh_container_section(tab_id);
        } else if let Some(path) = refresh_files
            && !path.is_empty()
        {
            self.load_container_dir(tab_id, path);
        }
    }

    pub(super) fn on_container_failed(
        &mut self,
        request_id: ContainerRequestId,
        error: AppError,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = self.state.containers.label_of(request_id);
        let Some(tab_id) = self.state.containers.take_tab(request_id) else {
            return;
        };
        let err_desc = localized_error(&error);
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
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
            if workspace.changes_request == Some(request_id) {
                workspace.changes_request = None;
                workspace.changes_loading = false;
            }
            if workspace
                .terminal_request
                .as_ref()
                .is_some_and(|(pending, ..)| *pending == request_id)
            {
                workspace.terminal_request = None;
            }
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
        let Some(tab_id) = self.state.containers.take_tab(request_id) else {
            return;
        };
        let message = if deleted_count == 0 {
            rust_i18n::t!(target.none_key()).to_string()
        } else {
            rust_i18n::t!(target.done_key(), count = deleted_count).to_string()
        };
        self.show_toast(message, true, window, cx);
        self.refresh_container_section(tab_id);
    }
}
