//! 容器内路径删除与新建目录。

use super::super::*;

impl ContainerWorkspace {
    /// 请求删除容器内路径；目录按递归删除走二次确认。
    pub(crate) fn request_remove_container_path(
        &mut self,
        path: String,
        recursive: bool,
        cx: &mut Context<Self>,
    ) {
        self.state.confirm = Some(ContainerConfirm::FilePath { path, recursive });
        cx.notify();
    }

    /// 展开或收起「新建目录」条；展开时才创建输入框，避免常驻实体。
    pub(crate) fn toggle_container_mkdir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open = !self.state.files.new_dir_open;
        self.state.files.new_dir_open = open;
        if open && self.state.files.new_dir_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(rust_i18n::t!("container.files.new_dir_placeholder").to_string())
            });
            self.state.files.new_dir_input = Some(input);
        }
        cx.notify();
    }

    /// 在容器当前目录里新建子目录：上传与整理文件都需要先有目录。
    pub(crate) fn submit_container_mkdir(&mut self, cx: &mut Context<Self>) {
        let Some(container) = self.state.selected_container_name() else {
            return;
        };
        let Some(input) = self.state.files.new_dir_input.clone() else {
            return;
        };
        let parent = if self.state.files.path.is_empty() {
            "/".to_string()
        } else {
            self.state.files.path.clone()
        };
        let name = input.read(cx).value().trim().to_string();
        if name.is_empty() || name.contains('/') || name.contains("..") {
            // 对话框由实体状态驱动，提示走既有 notice 链路，不额外弹错误框。
            self.state.notice = Some(rust_i18n::t!("container.files.new_dir_invalid").to_string());
            cx.notify();
            return;
        }
        let path = if parent.ends_with('/') {
            format!("{parent}{name}")
        } else {
            format!("{parent}/{name}")
        };
        let request_id = ContainerRequestId::new();
        self.state.files.new_dir_open = false;
        // 输入框随本次提交释放：下次展开重新创建，避免保留一份已消费的输入实体。
        self.state.files.new_dir_input = None;
        self.state.busy = true;
        self.send_tracked(
            request_id,
            ContainerCommand::CreateDir {
                tab_id: self.tab_id,
                request_id,
                container,
                path,
            },
        );
        cx.notify();
    }
}
