//! 文件夹的新建、重命名、删除与模态表单渲染。

use crate::app::{AppShell, FolderForm};
use crate::palette::Palette;
use crate::session_form::SessionForm;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::FolderId;

impl AppShell {
    pub(crate) fn open_new_folder(
        &mut self,
        parent: Option<FolderId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("folder.name_placeholder").to_string())
        });
        name.update(cx, |state, cx| state.focus(window, cx));
        self.folder_form = Some(FolderForm {
            base: None,
            parent,
            name,
            error: None,
        });
        cx.notify();
    }

    pub(crate) fn open_rename_folder(
        &mut self,
        folder_id: FolderId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(folder) = self.catalog.folder(folder_id) else {
            return;
        };
        let name = folder.name.clone();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(name)
                .placeholder(rust_i18n::t!("folder.name_placeholder").to_string())
        });
        input.update(cx, |state, cx| state.focus(window, cx));
        self.folder_form = Some(FolderForm {
            base: Some(folder_id),
            parent: None,
            name: input,
            error: None,
        });
        cx.notify();
    }

    /// 在指定文件夹下新建会话，表单预填该文件夹路径。
    pub(crate) fn open_new_session_in_folder(
        &mut self,
        folder_id: FolderId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = self.catalog.folder_path(folder_id);
        self.form = Some(SessionForm::new(
            window,
            cx,
            None,
            path,
            self.identity_options(),
        ));
        cx.notify();
    }

    pub(crate) fn save_folder_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.folder_form.as_ref() else {
            return;
        };
        let name = form.name.read(cx).value().trim().to_string();
        let base = form.base;
        let parent = form.parent;
        let result = match base {
            Some(folder_id) => self.catalog.rename_folder(folder_id, &name),
            None => self.catalog.add_folder(&name, parent).map(|_| {
                if let Some(parent_id) = parent
                    && let Some(folder) = self
                        .catalog
                        .folders
                        .iter_mut()
                        .find(|folder| folder.id == parent_id)
                {
                    folder.collapsed = false;
                }
            }),
        };
        match result {
            Ok(()) => {
                self.folder_form = None;
                self.persist_catalog();
                self.status = Some(if base.is_some() {
                    rust_i18n::t!("folder.renamed").to_string()
                } else {
                    rust_i18n::t!("folder.created").to_string()
                });
            }
            Err(error) => {
                if let Some(form) = self.folder_form.as_mut() {
                    form.error = Some(error.to_string());
                }
            }
        }
        cx.notify();
    }

    /// 删除文件夹但保留全部内容，子项提升到父层级。
    pub(crate) fn delete_folder(&mut self, folder_id: FolderId, cx: &mut Context<Self>) {
        match self.catalog.delete_folder(folder_id) {
            Ok(name) => {
                self.persist_catalog();
                self.status = Some(rust_i18n::t!("folder.deleted", name = name).to_string());
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    pub(crate) fn render_folder_form(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(form) = self.folder_form.as_ref() else {
            return div().into_any_element();
        };
        let editing = form.base.is_some();
        let title = if editing {
            rust_i18n::t!("folder.rename_title").to_string()
        } else {
            rust_i18n::t!("folder.new_title").to_string()
        };
        let description = form
            .parent
            .and_then(|parent| self.catalog.folder_path(parent))
            .map(|path| rust_i18n::t!("folder.parent_hint", path = path).to_string())
            .unwrap_or_else(|| rust_i18n::t!("folder.root_hint").to_string());

        let mut content = v_flex().gap_2().child(Self::form_field(
            rust_i18n::t!("folder.name").to_string(),
            form.name.clone(),
            p,
        ));
        if let Some(error) = &form.error {
            content = content.child(div().text_sm().text_color(p.danger).child(error.clone()));
        }

        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("cancel-folder")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.folder_form = None;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("save-folder")
                    .primary()
                    .label(rust_i18n::t!("common.save").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.save_folder_form(cx))),
            );

        let dialog = Self::dialog_frame(
            "folder-form-modal",
            title,
            Some(description),
            content,
            Some(footer),
            px(460.0),
            p,
        );
        Self::overlay(dialog, p)
    }
}
