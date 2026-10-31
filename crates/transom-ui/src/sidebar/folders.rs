//! 文件夹的新建、重命名、删除与模态表单渲染。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::session_form::SessionForm;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::FolderId;

/// 新建/编辑文件夹的模态表单；`base` 为编辑目标，`parent` 为新建位置。
pub(crate) struct FolderForm {
    pub(crate) base: Option<FolderId>,
    pub(crate) parent: Option<FolderId>,
    pub(crate) name: Entity<InputState>,
    pub(crate) default_port: Entity<InputState>,
    pub(crate) default_username: Entity<InputState>,
    pub(crate) default_key_path: Entity<InputState>,
    pub(crate) error: Option<String>,
}

impl FolderForm {
    /// 表单全部文本项的当前值，用作脏判定基线（见 `SessionForm::text_values`）。
    pub(crate) fn text_values(&self, cx: &App) -> Vec<String> {
        [
            &self.name,
            &self.default_port,
            &self.default_username,
            &self.default_key_path,
        ]
        .into_iter()
        .map(|input| input.read(cx).value().to_string())
        .collect()
    }
}

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
        let default_port = cx.new(|cx| InputState::new(window, cx).placeholder("22".to_string()));
        let default_username = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("form.username_placeholder").to_string())
        });
        let default_key_path =
            cx.new(|cx| InputState::new(window, cx).placeholder("~/.ssh/id_ed25519".to_string()));
        self.modals.folder_form = Some(FolderForm {
            base: None,
            parent,
            name,
            default_port,
            default_username,
            default_key_path,
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
        let port_text = folder
            .default_port
            .map(|p| p.to_string())
            .unwrap_or_default();
        let default_port = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(port_text)
                .placeholder("22".to_string())
        });
        let user_text = folder.default_username.clone().unwrap_or_default();
        let default_username = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(user_text)
                .placeholder(rust_i18n::t!("form.username_placeholder").to_string())
        });
        let key_text = folder
            .default_key_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let default_key_path = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(key_text)
                .placeholder("~/.ssh/id_ed25519".to_string())
        });
        self.modals.folder_form = Some(FolderForm {
            base: Some(folder_id),
            parent: None,
            name: input,
            default_port,
            default_username,
            default_key_path,
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
        let mut form = SessionForm::new(
            window,
            cx,
            crate::session_form::SessionFormInit {
                base: None,
                folder_path: path.clone(),
                identities: self.identity_options(),
                jump_options: self.jump_options(None),
                folder_options: self.folder_paths(),
                global_proxy: self.settings.proxy.clone(),
                block_view: transom_core::BlockViewOverride::FollowGlobal,
            },
        );
        let defaults = self.catalog.inherited_folder_defaults(folder_id);
        if let Some(folder_path) = path {
            form.apply_folder_selection(&folder_path, &defaults, window, cx);
        }
        self.install_session_form(form, cx);
        cx.notify();
    }

    pub(crate) fn pick_folder_default_key_path(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.modals.folder_form.as_mut() else {
            return;
        };
        let title = rust_i18n::t!("form.pick_key").to_string();
        let Some(path) = rfd::FileDialog::new()
            .set_title(&title)
            .set_directory(crate::keys::default_ssh_dir())
            .pick_file()
        else {
            return;
        };
        let text = path.to_string_lossy().to_string();
        form.default_key_path
            .update(cx, |state, cx| state.set_value(&text, window, cx));
        cx.notify();
    }

    pub(crate) fn save_folder_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.modals.folder_form.as_ref() else {
            return;
        };
        let name = form.name.read(cx).value().trim().to_string();
        let default_port = form
            .default_port
            .read(cx)
            .value()
            .trim()
            .parse::<u16>()
            .ok();
        let default_username = {
            let u = form.default_username.read(cx).value().trim().to_string();
            if u.is_empty() { None } else { Some(u) }
        };
        let default_key_path = {
            let p = form.default_key_path.read(cx).value().trim().to_string();
            if p.is_empty() {
                None
            } else {
                Some(crate::keys::expand_home(&p))
            }
        };
        let base = form.base;
        let parent = form.parent;
        let result = match base {
            Some(folder_id) => self.catalog.update_folder(
                folder_id,
                &name,
                default_port,
                default_username,
                default_key_path,
            ),
            None => self
                .catalog
                .add_folder_with_defaults(
                    &name,
                    parent,
                    default_port,
                    default_username,
                    default_key_path,
                )
                .map(|_| {
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
                self.modals.folder_form = None;
                self.persist_catalog();
                self.status = Some(if base.is_some() {
                    rust_i18n::t!("folder.renamed").to_string()
                } else {
                    rust_i18n::t!("folder.created").to_string()
                });
            }
            Err(error) => {
                if let Some(form) = self.modals.folder_form.as_mut() {
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
            Err(error) => self.set_error(error.to_string()),
        }
        cx.notify();
    }

    pub(crate) fn render_folder_form_body(
        &self,
        app: &Entity<AppShell>,
        _window: &mut Window,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(form) = self.modals.folder_form.as_ref() else {
            return div().into_any_element();
        };
        let editing = form.base.is_some();
        let title = if editing {
            rust_i18n::t!("folder.edit_title").to_string()
        } else {
            rust_i18n::t!("folder.new_title").to_string()
        };
        let description = form
            .parent
            .and_then(|parent| self.catalog.folder_path(parent))
            .map(|path| rust_i18n::t!("folder.parent_hint", path = path).to_string())
            .unwrap_or_else(|| rust_i18n::t!("folder.root_hint").to_string());

        let key_picker = h_flex()
            .gap_2()
            .items_center()
            .child(div().flex_1().child(form.default_key_path.clone()))
            .child(
                Button::new("browse-folder-key")
                    .outline()
                    .label(rust_i18n::t!("form.browse").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, window, cx| {
                            this.pick_folder_default_key_path(window, cx);
                        },
                    )),
            );

        let mut content = v_flex()
            .gap_3()
            .child(Self::form_field(
                rust_i18n::t!("folder.name").to_string(),
                form.name.clone(),
                p,
            ))
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .rounded(p.radius.md)
                    .bg(p.muted)
                    .border_1()
                    .border_color(p.border)
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.foreground)
                            .child(rust_i18n::t!("folder.defaults_legend").to_string()),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().flex_1().child(Self::form_field(
                                rust_i18n::t!("folder.default_port").to_string(),
                                form.default_port.clone(),
                                p,
                            )))
                            .child(div().flex_1().child(Self::form_field(
                                rust_i18n::t!("folder.default_username").to_string(),
                                form.default_username.clone(),
                                p,
                            ))),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .child(rust_i18n::t!("folder.default_key_path").to_string()),
                            )
                            .child(key_picker),
                    ),
            );
        if let Some(error) = &form.error {
            content = content.child(div().text_sm().text_color(p.danger).child(error.clone()));
        }

        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("cancel-folder")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            this.modals.folder_form = None;
                            cx.notify();
                        },
                    )),
            )
            .child(
                Button::new("save-folder")
                    .primary()
                    .label(rust_i18n::t!("common.save").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| this.save_folder_form(cx),
                    )),
            );

        Self::dialog_body(
            "folder-form-modal",
            title,
            Some(description),
            content,
            Some(footer),
        )
    }
}
