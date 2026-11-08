//! 容器内路径删除与新建目录。

use super::super::*;

impl AppShell {
    /// 请求删除容器内路径；目录按递归删除走二次确认。
    pub(crate) fn request_remove_container_path(
        &mut self,
        tab_id: TabId,
        path: String,
        recursive: bool,
        cx: &mut Context<Self>,
    ) {
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.confirm = Some(ContainerConfirm::FilePath { path, recursive });
        });
        cx.notify();
    }

    /// 重命名容器内路径。
    /// 展开或收起「新建目录」条；展开时才创建输入框，避免常驻实体。
    pub(crate) fn toggle_container_mkdir(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (open, has_input) = self
            .with_container(tab_id, cx, |workspace| {
                (
                    !workspace.files.new_dir_open,
                    workspace.files.new_dir_input.is_some(),
                )
            })
            .unwrap_or((false, true));
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.files.new_dir_open = open;
        });
        if open && !has_input {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(rust_i18n::t!("container.files.new_dir_placeholder").to_string())
            });
            self.with_container_mut(tab_id, cx, |workspace| {
                workspace.files.new_dir_input = Some(input);
            });
        }
        cx.notify();
    }

    /// 在容器当前目录里新建子目录：上传与整理文件都需要先有目录。
    pub(crate) fn submit_container_mkdir(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((container, input, parent)) = self
            .with_container(tab_id, cx, |workspace| {
                let container = workspace.selected_container_name()?;
                let input = workspace.files.new_dir_input.clone()?;
                let parent = if workspace.files.path.is_empty() {
                    "/".to_string()
                } else {
                    workspace.files.path.clone()
                };
                Some((container, input, parent))
            })
            .flatten()
        else {
            return;
        };
        let name = input.read(cx).value().trim().to_string();
        if name.is_empty() || name.contains('/') || name.contains("..") {
            self.show_toast(
                rust_i18n::t!("container.files.new_dir_invalid").to_string(),
                false,
                window,
                cx,
            );
            return;
        }
        let path = if parent.ends_with('/') {
            format!("{parent}{name}")
        } else {
            format!("{parent}/{name}")
        };
        let request_id = ContainerRequestId::new();
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.files.new_dir_open = false;
            workspace.busy = true;
        });
        input.update(cx, |input, cx| input.set_value("", window, cx));
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(
                    request_id,
                    ContainerCommand::CreateDir {
                        tab_id,
                        request_id,
                        container,
                        path,
                    },
                );
            });
        }
        cx.notify();
    }
}
