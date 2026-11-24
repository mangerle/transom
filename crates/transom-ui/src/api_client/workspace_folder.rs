//! API 接口树目录的新建、重命名与删除动作。

use super::ApiWorkspace;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_api::ApiFolderId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FolderEditorMode {
    Create,
    Rename(ApiFolderId),
}

/// 接口树目录编辑器；随工作区标签存活，只影响当前项目的目录结构。
pub(crate) struct FolderEditor {
    pub(crate) mode: FolderEditorMode,
    pub(crate) input: Entity<InputState>,
    pub(crate) error: Option<String>,
}

impl ApiWorkspace {
    pub(crate) fn start_create_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_folder_delete = None;
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.folder_name_placeholder").to_string())
        });
        let focus = input.read(cx).focus_handle(cx);
        self.folder_editor = Some(FolderEditor {
            mode: FolderEditorMode::Create,
            input,
            error: None,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn start_rename_folder(
        &mut self,
        folder_id: ApiFolderId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self
            .project
            .folders
            .iter()
            .find(|folder| folder.id == folder_id)
            .map(|folder| folder.name.clone())
        else {
            return;
        };
        self.pending_folder_delete = None;
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.folder_name_placeholder").to_string())
                .default_value(name)
        });
        let focus = input.read(cx).focus_handle(cx);
        self.folder_editor = Some(FolderEditor {
            mode: FolderEditorMode::Rename(folder_id),
            input,
            error: None,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn submit_folder_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.folder_editor.take() else {
            return;
        };
        let name = editor.input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.folder_editor = Some(FolderEditor {
                mode: editor.mode,
                input: editor.input,
                error: Some(rust_i18n::t!("api.request.folder_name_required").to_string()),
            });
            cx.notify();
            return;
        }
        match editor.mode {
            FolderEditorMode::Create => {
                self.project.add_folder(name);
            }
            FolderEditorMode::Rename(folder_id) => {
                let Some(folder) = self
                    .project
                    .folders
                    .iter_mut()
                    .find(|folder| folder.id == folder_id)
                else {
                    return;
                };
                folder.name = name;
                self.project.touch();
            }
        }
        cx.notify();
    }

    pub(crate) fn cancel_folder_editor(&mut self, cx: &mut Context<Self>) {
        self.folder_editor = None;
        cx.notify();
    }

    pub(crate) fn request_delete_folder(&mut self, folder_id: ApiFolderId, cx: &mut Context<Self>) {
        self.folder_editor = None;
        self.pending_folder_delete = Some(folder_id);
        cx.notify();
    }

    pub(crate) fn confirm_delete_folder(&mut self, cx: &mut Context<Self>) {
        let Some(folder_id) = self.pending_folder_delete.take() else {
            return;
        };
        if self.project.remove_folder(folder_id) {
            cx.notify();
        }
    }

    pub(crate) fn cancel_delete_folder(&mut self, cx: &mut Context<Self>) {
        self.pending_folder_delete = None;
        cx.notify();
    }
}
