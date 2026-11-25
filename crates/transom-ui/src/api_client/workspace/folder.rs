//! API 接口树目录的新建、重命名与删除动作。

use super::ApiWorkspace;
use crate::palette::Palette;
use crate::theme::tokens::space;
use crate::widgets::{IconButton, IconButtonSize, action_button};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::*;
use transom_api::{ApiFolder, ApiFolderId};
use transom_ui_icons::{AppIcon, UiIcon};

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
    /// 切换接口树目录的折叠状态。
    pub(crate) fn toggle_folder_collapsed(
        &mut self,
        folder_id: ApiFolderId,
        cx: &mut Context<Self>,
    ) {
        if !self.ui.collapsed_folders.remove(&folder_id) {
            self.ui.collapsed_folders.insert(folder_id);
        }
        cx.notify();
    }

    pub(crate) fn start_create_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ui.pending_folder_delete = None;
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.folder_name_placeholder").to_string())
        });
        let focus = input.read(cx).focus_handle(cx);
        self.ui.folder_editor = Some(FolderEditor {
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
        self.ui.pending_folder_delete = None;
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("api.request.folder_name_placeholder").to_string())
                .default_value(name)
        });
        let focus = input.read(cx).focus_handle(cx);
        self.ui.folder_editor = Some(FolderEditor {
            mode: FolderEditorMode::Rename(folder_id),
            input,
            error: None,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn submit_folder_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.ui.folder_editor.take() else {
            return;
        };
        let name = editor.input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.ui.folder_editor = Some(FolderEditor {
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
        self.ui.folder_editor = None;
        cx.notify();
    }

    pub(crate) fn request_delete_folder(&mut self, folder_id: ApiFolderId, cx: &mut Context<Self>) {
        self.ui.folder_editor = None;
        self.ui.pending_folder_delete = Some(folder_id);
        cx.notify();
    }

    pub(crate) fn confirm_delete_folder(&mut self, cx: &mut Context<Self>) {
        let Some(folder_id) = self.ui.pending_folder_delete.take() else {
            return;
        };
        if self.project.remove_folder(folder_id) {
            cx.notify();
        }
    }

    pub(crate) fn cancel_delete_folder(&mut self, cx: &mut Context<Self>) {
        self.ui.pending_folder_delete = None;
        cx.notify();
    }
}

pub(super) fn render_folder_editor(
    view: &Entity<ApiWorkspace>,
    editor: &FolderEditor,
    p: &Palette,
) -> AnyElement {
    let submit_view = view.clone();
    let cancel_view = view.clone();
    let row = h_flex()
        .mx(rems(space::XS))
        .my(rems(space::XXS))
        .gap(rems(space::XS))
        .items_center()
        .child(Input::new(&editor.input).flex_1().min_w_0())
        .child(
            IconButton::new(
                "api-folder-editor-submit",
                AppIcon::Ui(UiIcon::CHECK),
                rust_i18n::t!("api.request.save_folder").to_string(),
            )
            .size(IconButtonSize::Compact)
            .render()
            .on_click(move |_, _, cx| {
                submit_view.update(cx, |this, cx| this.submit_folder_editor(cx));
            }),
        )
        .child(
            IconButton::new(
                "api-folder-editor-cancel",
                AppIcon::Ui(UiIcon::CLOSE),
                rust_i18n::t!("api.project.cancel").to_string(),
            )
            .size(IconButtonSize::Compact)
            .render()
            .on_click(move |_, _, cx| {
                cancel_view.update(cx, |this, cx| this.cancel_folder_editor(cx));
            }),
        );
    let mut content = v_flex().child(row);
    if let Some(error) = &editor.error {
        content = content.child(
            div()
                .px(rems(space::SM))
                .text_color(p.danger)
                .child(error.clone()),
        );
    }
    content.into_any_element()
}

pub(super) fn render_folder_delete_confirm(
    view: &Entity<ApiWorkspace>,
    folder: &ApiFolder,
    request_count: usize,
    p: &Palette,
) -> AnyElement {
    let confirm_view = view.clone();
    let cancel_view = view.clone();
    v_flex()
        .mx(rems(space::XS))
        .my(rems(space::XXS))
        .p(rems(space::SM))
        .gap(rems(space::XS))
        .rounded(p.radius.md)
        .border_1()
        .border_color(p.danger)
        .child(
            div().text_color(p.foreground).child(
                rust_i18n::t!(
                    "api.request.delete_folder_confirm",
                    name = folder.name.as_str()
                )
                .to_string(),
            ),
        )
        .child(div().text_color(p.muted_foreground).child(
            rust_i18n::t!("api.request.delete_folder_hint", count = request_count).to_string(),
        ))
        .child(
            h_flex()
                .gap(rems(space::XS))
                .child(
                    action_button(
                        "api-folder-delete-confirm",
                        AppIcon::Ui(UiIcon::DELETE),
                        rust_i18n::t!("api.project.delete").to_string(),
                    )
                    .danger()
                    .on_click(move |_, _, cx| {
                        confirm_view.update(cx, |this, cx| this.confirm_delete_folder(cx));
                    }),
                )
                .child(
                    IconButton::new(
                        "api-folder-delete-cancel",
                        AppIcon::Ui(UiIcon::CLOSE),
                        rust_i18n::t!("api.project.cancel").to_string(),
                    )
                    .size(IconButtonSize::Compact)
                    .render()
                    .on_click(move |_, _, cx| {
                        cancel_view.update(cx, |this, cx| this.cancel_delete_folder(cx));
                    }),
                ),
        )
        .into_any_element()
}
