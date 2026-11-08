//! 容器内文件就地编辑。

use super::super::*;
use gpui_kit::component::input::TextareaState;

impl ContainerWorkspace {
    /// 进入文件就地编辑模式。
    pub(crate) fn begin_edit_container_file(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.files.preview_binary {
            return;
        }
        let Some(initial_text) = self
            .state
            .files
            .preview
            .as_ref()
            .map(|(_, content, _)| content.clone())
        else {
            return;
        };
        let editor = cx.new(|cx| TextareaState::new(window, cx).default_value(initial_text));
        self.state.files.preview_editor = Some(editor);
        cx.notify();
    }

    /// 取消文件就地编辑模式。
    pub(crate) fn cancel_edit_container_file(&mut self, cx: &mut Context<Self>) {
        self.state.files.preview_editor = None;
        self.state.files.preview_saving = false;
        cx.notify();
    }

    /// 关闭当前文件预览与编辑。
    pub(crate) fn close_preview_container_file(&mut self, cx: &mut Context<Self>) {
        self.state.files.preview = None;
        self.state.files.preview_editor = None;
        self.state.files.preview_saving = false;
        self.state.files.preview_binary = false;
        cx.notify();
    }

    /// 保存就地编辑的文件内容。
    pub(crate) fn save_edit_container_file(&mut self, cx: &mut Context<Self>) {
        if self.state.files.preview_binary {
            return;
        }
        let Some((path, _, _)) = self.state.files.preview.clone() else {
            return;
        };
        let Some(editor) = self.state.files.preview_editor.clone() else {
            return;
        };
        let Some((is_volume, target)) = self.file_target() else {
            return;
        };
        let content = editor.read(cx).value().to_string();
        self.state.files.preview_saving = true;
        let request_id = ContainerRequestId::new();
        let command = if is_volume {
            ContainerCommand::SaveVolumeFile {
                tab_id: self.tab_id,
                request_id,
                volume: target,
                path,
                content,
            }
        } else {
            ContainerCommand::SaveFile {
                tab_id: self.tab_id,
                request_id,
                container: target,
                path,
                content,
            }
        };
        self.send_tracked(request_id, command);
        cx.notify();
    }
}
