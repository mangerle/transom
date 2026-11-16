//! 文件夹表单弹窗的打开、保存与渲染逻辑。

use super::super::FolderForm;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::FolderId;

pub(crate) fn open_new_folder(
    shell: &mut AppShell,
    parent: Option<FolderId>,
    window: &mut Window,
    cx: &mut Context<AppShell>,
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
    shell.state.ui.modals.session.folder_form = Some(FolderForm {
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

pub(super) fn open_rename_folder(
    shell: &mut AppShell,
    folder_id: FolderId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(folder) = shell.state.session.catalog.folder(folder_id) else {
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
    shell.state.ui.modals.session.folder_form = Some(FolderForm {
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

fn pick_folder_default_key_path(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(form) = shell.state.ui.modals.session.folder_form.as_mut() else {
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

pub(crate) fn save_folder_form(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    let Some(form) = shell.state.ui.modals.session.folder_form.as_ref() else {
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
        Some(folder_id) => shell.state.session.catalog.update_folder(
            folder_id,
            &name,
            default_port,
            default_username,
            default_key_path,
        ),
        None => shell
            .state
            .session
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
                    && let Some(folder) = shell
                        .state
                        .session
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
            shell.state.ui.modals.session.folder_form = None;
            shell.state.persist_catalog();
            shell.state.ui.status = Some(if base.is_some() {
                rust_i18n::t!("folder.renamed").to_string()
            } else {
                rust_i18n::t!("folder.created").to_string()
            });
        }
        Err(error) => {
            if let Some(form) = shell.state.ui.modals.session.folder_form.as_mut() {
                form.error = Some(error.to_string());
            }
        }
    }
    cx.notify();
}

fn render_folder_defaults(form: &FolderForm, app: &Entity<AppShell>, p: &Palette) -> AnyElement {
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
                        pick_folder_default_key_path(this, window, cx);
                    },
                )),
        );

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
                .child(div().flex_1().child(crate::widgets::form_field(
                    rust_i18n::t!("folder.default_port").to_string(),
                    form.default_port.clone(),
                    p,
                )))
                .child(div().flex_1().child(crate::widgets::form_field(
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
        )
        .into_any_element()
}

pub(crate) fn render_folder_form_body(
    shell: &AppShell,
    app: &Entity<AppShell>,
    _window: &mut Window,
    _cx: &App,
    p: &Palette,
) -> AnyElement {
    let Some(form) = shell.state.ui.modals.session.folder_form.as_ref() else {
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
        .and_then(|parent| shell.state.session.catalog.folder_path(parent))
        .map(|path| rust_i18n::t!("folder.parent_hint", path = path).to_string())
        .unwrap_or_else(|| rust_i18n::t!("folder.root_hint").to_string());

    let defaults = render_folder_defaults(form, app, p);
    let mut content = v_flex()
        .gap_3()
        .child(crate::widgets::form_field(
            rust_i18n::t!("folder.name").to_string(),
            form.name.clone(),
            p,
        ))
        .child(defaults);
    if let Some(error) = &form.error {
        content = content.child(div().text_sm().text_color(p.danger).child(error.clone()));
    }

    let footer = h_flex()
        .gap_2()
        .child(crate::widgets::dialog_cancel_button(
            "cancel-folder",
            rust_i18n::t!("common.cancel").to_string(),
            crate::dialog_host::on_event(app, |this, _: &ClickEvent, _, cx| {
                this.state.ui.modals.session.folder_form = None;
                cx.notify();
            }),
        ))
        .child(crate::widgets::dialog_confirm_button(
            "save-folder",
            rust_i18n::t!("common.save").to_string(),
            None,
            crate::widgets::DialogConfirmKind::Primary,
            crate::dialog_host::on_event(app, |this, _: &ClickEvent, _, cx| {
                save_folder_form(this, cx);
            }),
        ));

    crate::widgets::dialog_body(
        "folder-form-modal",
        title,
        Some(description),
        content,
        Some(footer),
    )
}
