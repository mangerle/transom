//! 行文本复制与请求取消：复制回查目录、停止流式任务。

use super::*;

impl AppShell {
    /// 复制行的标识到剪贴板，并给出右上角回执。
    pub(super) fn copy_row_text(
        &mut self,
        tab_id: TabId,
        target: &RowTarget,
        action: RowAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = match (target, action) {
            (RowTarget::Container { id, name }, action) => {
                if action == RowAction::CopyId {
                    id.clone()
                } else {
                    name.clone()
                }
            }
            (RowTarget::Volume(name), _) | (RowTarget::Compose(name), _) => name.clone(),
            (RowTarget::Network(id), RowAction::CopyId) => id.clone(),
            (RowTarget::Network(id), RowAction::CopyName) => self
                .state
                .container_workspace(tab_id)
                .and_then(|workspace| {
                    let state = &workspace.read(cx).state;
                    state
                        .networks
                        .iter()
                        .find(|network| &network.id == id)
                        .map(|network| network.name.clone())
                })
                .unwrap_or_else(|| id.clone()),
            (RowTarget::Image(id), action) => {
                let workspace = self.state.container_workspace(tab_id);
                match action {
                    RowAction::CopyId => id.clone(),
                    _ => workspace
                        .and_then(|workspace| {
                            let state = &workspace.read(cx).state;
                            state
                                .images
                                .iter()
                                .find(|image| &image.id == id)
                                .map(|image| image.display_name())
                        })
                        .unwrap_or_else(|| transom_core::short_id(id)),
                }
            }
            _ => return,
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        self.show_toast(
            rust_i18n::t!("container.row.copied", value = text).to_string(),
            true,
            window,
            cx,
        );
    }
}

impl ContainerWorkspace {
    /// 停止当前选中项的日志、统计与终端会话。
    pub(crate) fn stop_container_streams(&mut self) {
        let requests = [
            self.state.logs_request.take(),
            self.state.stats_request.take(),
            self.state.stats_processes_request.take(),
            self.state.files.request.take(),
            self.state.changes_request.take(),
            self.state
                .terminal_request
                .take()
                .map(|(request_id, ..)| request_id),
        ];
        self.state.files.loading = false;
        self.state.changes_loading = false;
        self.state.stats_processes_loading = false;
        self.state.terminal = None;
        for request_id in requests.into_iter().flatten() {
            self.cancel_container_request(request_id);
        }
    }

    /// 立即取消一个流式请求（切换页签时使用）。
    ///
    /// 先撤销登记再投递取消：控制器可能立刻回执，登记留着会让迟到结果
    /// 再次落到本标签页。
    pub(crate) fn cancel_container_request(&mut self, request_id: ContainerRequestId) {
        self.take_request(request_id);
        self.send(ContainerCommand::Cancel {
            tab_id: self.tab_id,
            request_id,
        });
    }
}
