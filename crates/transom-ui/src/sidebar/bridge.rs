//! 会话侧栏与外壳之间的唯一事件与弹窗桥接。
//!
//! 设计原理（Why）：
//! - `SessionSidebarView` 与全部行渲染、表单定义都已脱离 `AppShell`；
//! - 本文件集中承接侧栏意图事件（连接、目录增删改、拖放排序、批量选择、拨测）、
//!   文件夹表单的打开/保存/渲染以及拖放持久化，避免其他侧栏文件反向依赖外壳。

use super::entity::SessionSidebarEvent;
use super::folders::FolderForm;
use super::tree::{SidebarDrag, SidebarTarget};
use crate::app::{AppShell, app_settings};
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::{CatalogError, FolderId};

/// 处理会话侧栏发往外壳的意图事件。
pub(crate) fn on_session_sidebar_event(
    shell: &mut AppShell,
    event: &SessionSidebarEvent,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    match event {
        SessionSidebarEvent::ToggleSelectionMode => {
            crate::batch::toggle_selection_mode(shell, cx);
        }
        SessionSidebarEvent::SelectAllSessions => {
            crate::batch::select_all_sessions(shell, cx);
        }
        SessionSidebarEvent::OpenBatchEdit => {
            crate::batch::open_batch_edit(shell, window, cx);
        }
        SessionSidebarEvent::OpenNewFolder(parent) => {
            open_new_folder(shell, *parent, window, cx);
        }
        SessionSidebarEvent::OpenRenameFolder(folder_id) => {
            open_rename_folder(shell, *folder_id, window, cx);
        }
        SessionSidebarEvent::DeleteFolder(folder_id) => {
            delete_folder(shell, *folder_id, cx);
        }
        SessionSidebarEvent::ToggleFolderCollapsed(folder_id) => {
            toggle_folder_collapsed(shell, *folder_id, cx);
        }
        SessionSidebarEvent::OpenNewSession(None) => {
            shell.open_new_session(window, cx);
        }
        SessionSidebarEvent::OpenNewSession(Some(folder_id)) => {
            open_new_session_in_folder(shell, *folder_id, window, cx);
        }
        SessionSidebarEvent::ProbeFolder(folder_id) => {
            crate::probe::probe_folder(shell, *folder_id, cx);
        }
        SessionSidebarEvent::ProbeSession(session_id) => {
            crate::probe::probe_session(shell, *session_id, cx);
        }
        SessionSidebarEvent::Drop(drag, target) => {
            handle_sidebar_drop(shell, *drag, *target, cx);
        }
        SessionSidebarEvent::ToggleSessionSelection {
            session_id,
            selected,
        } => {
            crate::batch::toggle_session_selection(shell, *session_id, *selected, cx);
        }
        SessionSidebarEvent::SelectTreeSession(session_id) => {
            shell.state.sidebar.tree_selection = Some(*session_id);
            cx.notify();
        }
        SessionSidebarEvent::ConnectSession(session_id) => {
            crate::connection::connect_session(shell, *session_id, window, cx);
            shell.reveal_vertical_tabs_panel(cx);
        }
        SessionSidebarEvent::OpenContainerWorkspace(session_id) => {
            crate::container::open_container_workspace_for_session(shell, *session_id, window, cx);
        }
        SessionSidebarEvent::DuplicateSession(session_id) => {
            shell.duplicate_session(*session_id, cx);
        }
        SessionSidebarEvent::EditSession(session_id) => {
            shell.open_edit_session(*session_id, window, cx);
        }
        SessionSidebarEvent::OpenAuditViewer(session_id) => {
            crate::audit_viewer::open_audit_viewer_for_session(shell, *session_id, window, cx);
        }
        SessionSidebarEvent::CopyHost(host) => {
            shell.state.status =
                Some(rust_i18n::t!("sidebar.host_copied", host = host).to_string());
            cx.notify();
        }
        SessionSidebarEvent::DeleteSession(session_id) => {
            shell.delete_session_by_id(*session_id, cx);
        }
    }
}

/// 折叠或展开文件夹，并持久化收纳状态。
fn toggle_folder_collapsed(shell: &mut AppShell, id: FolderId, cx: &mut Context<AppShell>) {
    let collapsed = shell
        .state
        .catalog
        .folder(id)
        .map(|folder| !folder.collapsed);
    if let Some(collapsed) = collapsed {
        if let Err(error) = shell.state.catalog.set_folder_collapsed(id, collapsed) {
            shell.state.set_error(error.to_string());
            cx.notify();
            return;
        }
        shell.state.persist_catalog();
    }
    cx.notify();
}

/// 处理会话/文件夹拖放：换父级或调整同级顺序。
fn handle_sidebar_drop(
    shell: &mut AppShell,
    drag: SidebarDrag,
    target: SidebarTarget,
    cx: &mut Context<AppShell>,
) {
    match apply_sidebar_drop(shell, drag, target) {
        Ok(landed) => {
            if let Some(folder_id) = landed
                && let Some(folder) = shell
                    .state
                    .catalog
                    .folders
                    .iter_mut()
                    .find(|f| f.id == folder_id)
            {
                folder.collapsed = false;
            }
            shell.state.persist_catalog();
        }
        Err(error) => shell.state.set_error(error.to_string()),
    }
    cx.notify();
}

fn apply_sidebar_drop(
    shell: &mut AppShell,
    drag: SidebarDrag,
    target: SidebarTarget,
) -> Result<Option<FolderId>, CatalogError> {
    match (drag, target) {
        (SidebarDrag::Session(session_id), SidebarTarget::Root) => {
            shell.state.catalog.move_session(session_id, None)?;
            Ok(None)
        }
        (SidebarDrag::Folder(folder_id), SidebarTarget::Root) => {
            shell.state.catalog.move_folder(folder_id, None)?;
            Ok(None)
        }
        (SidebarDrag::Session(session_id), SidebarTarget::Folder(folder_id)) => {
            shell
                .state
                .catalog
                .move_session(session_id, Some(folder_id))?;
            Ok(Some(folder_id))
        }
        (SidebarDrag::Folder(moved_id), SidebarTarget::Folder(parent_id)) => {
            if moved_id == parent_id {
                return Ok(Some(parent_id));
            }
            shell.state.catalog.move_folder(moved_id, Some(parent_id))?;
            Ok(Some(parent_id))
        }
        (drag, SidebarTarget::Relative { anchor, side }) => match (drag, anchor) {
            (SidebarDrag::Session(session_id), SidebarDrag::Session(anchor_id)) => {
                shell
                    .state
                    .catalog
                    .reorder_session(session_id, anchor_id, side)?;
                Ok(shell
                    .state
                    .catalog
                    .folder_of_session(anchor_id)
                    .map(|folder| folder.id))
            }
            (SidebarDrag::Folder(folder_id), SidebarDrag::Folder(anchor_id)) => {
                shell
                    .state
                    .catalog
                    .reorder_folder(folder_id, anchor_id, side)?;
                Ok(shell
                    .state
                    .catalog
                    .folder(folder_id)
                    .and_then(|folder| folder.parent))
            }
            (SidebarDrag::Session(session_id), SidebarDrag::Folder(anchor_id)) => {
                shell
                    .state
                    .catalog
                    .move_session(session_id, Some(anchor_id))?;
                Ok(Some(anchor_id))
            }
            (SidebarDrag::Folder(folder_id), SidebarDrag::Session(anchor_id)) => {
                let parent = shell
                    .state
                    .catalog
                    .folder_of_session(anchor_id)
                    .map(|folder| folder.id);
                shell.state.catalog.move_folder(folder_id, parent)?;
                Ok(parent)
            }
        },
    }
}

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
    shell.state.modals.session.folder_form = Some(FolderForm {
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

fn open_rename_folder(
    shell: &mut AppShell,
    folder_id: FolderId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(folder) = shell.state.catalog.folder(folder_id) else {
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
    shell.state.modals.session.folder_form = Some(FolderForm {
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
fn open_new_session_in_folder(
    shell: &mut AppShell,
    folder_id: FolderId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let path = shell.state.catalog.folder_path(folder_id);
    let defaults = shell.state.catalog.inherited_folder_defaults(folder_id);
    let init = crate::session_form::SessionFormInit {
        base: None,
        folder_path: path.clone(),
        identities: shell.state.identity_options(),
        jump_options: shell.state.jump_options(None),
        folder_options: shell.state.folder_paths(),
        global_proxy: app_settings(cx).proxy.clone(),
        block_view: transom_core::BlockViewOverride::FollowGlobal,
    };
    let view = shell.state.modals.session.form.clone();
    view.update(cx, |view, cx| {
        view.open(init, window, cx);
        if let Some(folder_path) = path {
            view.apply_folder_selection(&folder_path, &defaults, window, cx);
        }
    });
    cx.notify();
}

fn pick_folder_default_key_path(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(form) = shell.state.modals.session.folder_form.as_mut() else {
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
    let Some(form) = shell.state.modals.session.folder_form.as_ref() else {
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
        Some(folder_id) => shell.state.catalog.update_folder(
            folder_id,
            &name,
            default_port,
            default_username,
            default_key_path,
        ),
        None => shell
            .state
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
            shell.state.modals.session.folder_form = None;
            shell.state.persist_catalog();
            shell.state.status = Some(if base.is_some() {
                rust_i18n::t!("folder.renamed").to_string()
            } else {
                rust_i18n::t!("folder.created").to_string()
            });
        }
        Err(error) => {
            if let Some(form) = shell.state.modals.session.folder_form.as_mut() {
                form.error = Some(error.to_string());
            }
        }
    }
    cx.notify();
}

/// 删除文件夹但保留全部内容，子项提升到父层级。
fn delete_folder(shell: &mut AppShell, folder_id: FolderId, cx: &mut Context<AppShell>) {
    match shell.state.catalog.delete_folder(folder_id) {
        Ok(name) => {
            shell.state.persist_catalog();
            shell.state.status = Some(rust_i18n::t!("folder.deleted", name = name).to_string());
        }
        Err(error) => shell.state.set_error(error.to_string()),
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
    let Some(form) = shell.state.modals.session.folder_form.as_ref() else {
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
        .and_then(|parent| shell.state.catalog.folder_path(parent))
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
                this.state.modals.session.folder_form = None;
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
