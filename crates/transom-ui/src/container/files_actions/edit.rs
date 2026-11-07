//! 容器内文件就地编辑。

use super::super::*;
use gpui_kit::component::input::TextareaState;

impl AppShell {
    /// 进入文件就地编辑模式。
    pub(crate) fn begin_edit_container_file(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        if workspace.files.preview_binary {
            return;
        }
        let Some((_, content, _)) = workspace.files.preview.as_ref() else {
            return;
        };
        let initial_text = content.clone();
        let editor = cx.new(|cx| TextareaState::new(window, cx).default_value(initial_text));
        workspace.files.preview_editor = Some(editor);
        cx.notify();
    }

    /// 取消文件就地编辑模式。
    pub(crate) fn cancel_edit_container_file(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        workspace.files.preview_editor = None;
        workspace.files.preview_saving = false;
        cx.notify();
    }

    /// 关闭当前文件预览与编辑。
    pub(crate) fn close_preview_container_file(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        workspace.files.preview = None;
        workspace.files.preview_editor = None;
        workspace.files.preview_saving = false;
        workspace.files.preview_binary = false;
        cx.notify();
    }

    /// 保存就地编辑的文件内容。
    pub(crate) fn save_edit_container_file(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        if workspace.files.preview_binary {
            return;
        }
        let Some((path, _, _)) = workspace.files.preview.as_ref().cloned() else {
            return;
        };
        let Some(editor) = workspace.files.preview_editor.as_ref() else {
            return;
        };
        let content = editor.read(cx).value().to_string();
        let is_volume = workspace.section == ContainerSection::Volumes;
        let target = if is_volume {
            workspace.selected_volume_name()
        } else {
            workspace.selected_container_name()
        };
        let Some(target) = target else {
            return;
        };

        workspace.files.preview_saving = true;
        let request_id = ContainerRequestId::new();
        let command = if is_volume {
            ContainerCommand::SaveVolumeFile {
                tab_id,
                request_id,
                volume: target,
                path,
                content,
            }
        } else {
            ContainerCommand::SaveFile {
                tab_id,
                request_id,
                container: target,
                path,
                content,
            }
        };
        self.send_tracked(tab_id, request_id, command);
        cx.notify();
    }
}
