//! 引擎连接行：图标、名称、状态点与右键菜单。

use super::*;
use crate::container::types::ContainerWorkspaceStatus;
use crate::widgets::{SidebarRowState, sidebar_row_band};
use gpui_kit::component::menu::PopupMenu;
use std::collections::HashMap;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 引擎行的渲染参数集合，避免六个平铺参数造成顺序错误。
pub(crate) struct EngineRowParams<'a> {
    pub view: &'a Entity<ContainerSidebarView>,
    pub row: &'a EngineRow,
    pub active: Option<EngineProfileId>,
    pub selected: Option<EngineProfileId>,
    pub states: &'a HashMap<EngineProfileId, ContainerWorkspaceStatus>,
    pub palette: &'a Palette,
}

/// 渲染一条引擎连接行。
///
/// 设计原理（Why）：行只读目录快照与连接状态；双击打开、右键菜单、复制与
/// 删除都以侧栏事件上交外壳，行本身不持有 `AppShell`。
pub(crate) fn render_engine_row(params: EngineRowParams<'_>) -> AnyElement {
    let outline = engine_row_outline(&params);
    let row = engine_row_shell(&params, &outline);
    let menu_ctx = EngineMenuTarget {
        view: params.view.clone(),
        id: outline.id,
        profile: outline.profile.clone(),
        folder_id: outline.folder_id,
        name: outline.name.clone(),
        endpoint: outline.description.clone(),
    };
    let row = row.context_menu(move |menu, _window, _cx| engine_context_menu(menu, &menu_ctx));
    row.child(engine_row_band(&params, &outline))
        .into_any_element()
}

/// 行渲染所需的只读推导结果。
struct EngineRowOutline {
    id: EngineProfileId,
    name: String,
    description: String,
    icon: AppIcon,
    profile_color: Option<gpui_kit::Hsla>,
    folder_id: Option<FolderId>,
    depth: usize,
    row_state: SidebarRowState,
    status_color: gpui_kit::Hsla,
    action_color: gpui_kit::Hsla,
    profile: ContainerEngineProfile,
}

fn engine_row_outline(params: &EngineRowParams<'_>) -> EngineRowOutline {
    let row = params.row;
    let profile = &row.profile;
    let id = profile.id;
    let name = profile.display_name();
    let description = profile.engine.describe();
    let icon = crate::session_appearance::session_icon(profile.icon)
        .unwrap_or_else(|| crate::session_appearance::container_engine_icon(profile.engine.kind));
    let profile_color = crate::session_appearance::session_color(profile.color);
    let is_active = params.active == Some(id);
    let is_selected = params.selected == Some(id);
    let row_state = if is_active {
        SidebarRowState::Current
    } else if is_selected {
        SidebarRowState::Selected
    } else {
        SidebarRowState::Normal
    };
    let p = params.palette;
    let status_color = match params.states.get(&id) {
        Some(ContainerWorkspaceStatus::Connected) => p.success,
        Some(ContainerWorkspaceStatus::Connecting) => p.warning,
        Some(ContainerWorkspaceStatus::Failed(_))
        | Some(ContainerWorkspaceStatus::Disconnected) => p.danger,
        None => p.border,
    };
    let action_color = if row_state.is_highlighted() {
        p.sidebar_accent_foreground
    } else {
        p.muted_foreground
    };
    EngineRowOutline {
        id,
        name,
        description,
        icon,
        profile_color,
        folder_id: profile.folder_id,
        depth: row.depth,
        row_state,
        status_color,
        action_color,
        profile: (**profile).clone(),
    }
}

fn engine_row_shell(params: &EngineRowParams<'_>, outline: &EngineRowOutline) -> Stateful<Div> {
    let view = params.view;
    let id = outline.id;
    let name = outline.name.clone();
    let icon = outline.icon;
    let folder_id = outline.folder_id;
    let profile_for_open = outline.profile.clone();
    let click_view = view.clone();
    let drag_label = name.clone();
    let drop_view = view.clone();
    div()
        .id(ElementId::Name(format!("container-engine-{id}").into()))
        .w_full()
        .h(rems(ROW_HEIGHT_REM))
        .pl(indent_padding(outline.depth))
        .pr(px(4.0))
        .flex()
        .items_center()
        .cursor_pointer()
        .on_click(move |event: &ClickEvent, _, cx| {
            if event.click_count() >= 2 {
                log::debug!("双击打开引擎工作区，连接: {id}");
                click_view.update(cx, |sidebar, cx| {
                    sidebar.request_open_engine(profile_for_open.clone(), cx);
                });
            } else {
                click_view.update(cx, |sidebar, cx| sidebar.select_engine(id, cx));
            }
        })
        .on_drag(ContainerSidebarDrag::Engine(id), move |_, _, _, cx| {
            cx.new(|_| SidebarDragPreview::new(drag_label.clone(), icon))
        })
        .on_drop(move |drag: &ContainerSidebarDrag, window, cx| {
            let drag = *drag;
            let target = match folder_id {
                Some(folder_id) => ContainerSidebarTarget::Folder(folder_id),
                None => ContainerSidebarTarget::Root,
            };
            drop_view.update(cx, |sidebar, cx| {
                sidebar.request_drop(drag, target, cx);
            });
            window.refresh();
        })
}

/// 右键菜单所需的全部上下文，避免七个平铺参数。
struct EngineMenuTarget {
    view: Entity<ContainerSidebarView>,
    id: EngineProfileId,
    profile: ContainerEngineProfile,
    folder_id: Option<FolderId>,
    name: String,
    endpoint: String,
}

fn engine_context_menu(menu: PopupMenu, target: &EngineMenuTarget) -> PopupMenu {
    let mut menu = engine_menu_open(menu, &target.view, &target.profile);
    if target.folder_id.is_some() {
        menu = engine_menu_move_to_root(menu, &target.view, target.id);
    }
    let menu = engine_menu_edit(menu, &target.view, &target.profile);
    let menu = engine_menu_copy_endpoint(menu, &target.view, &target.endpoint);
    let menu = engine_menu_copy_name(menu, &target.view, &target.name);
    let menu = menu.item(PopupMenuItem::separator());
    engine_menu_delete(menu, &target.view, target.id)
}

fn engine_menu_open(
    menu: PopupMenu,
    view: &Entity<ContainerSidebarView>,
    profile: &ContainerEngineProfile,
) -> PopupMenu {
    let view = view.clone();
    let profile = profile.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("container.sidebar.open").to_string(),
        AppIcon::Ui(UiIcon::RUN),
        move |_, _, cx| {
            view.update(cx, |sidebar, cx| {
                sidebar.request_open_engine(profile.clone(), cx);
            });
        },
    ))
}

fn engine_menu_move_to_root(
    menu: PopupMenu,
    view: &Entity<ContainerSidebarView>,
    id: EngineProfileId,
) -> PopupMenu {
    let view = view.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("container.sidebar.move_to_root").to_string(),
        AppIcon::Ui(UiIcon::SHOW_AS_TREE),
        move |_, _, cx| {
            view.update(cx, |sidebar, cx| {
                sidebar.request_move_engine_to_root(id, cx);
            });
        },
    ))
}

fn engine_menu_edit(
    menu: PopupMenu,
    view: &Entity<ContainerSidebarView>,
    profile: &ContainerEngineProfile,
) -> PopupMenu {
    let view = view.clone();
    let profile = profile.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("common.edit").to_string(),
        AppIcon::Ui(UiIcon::EDIT),
        move |_, _, cx| {
            view.update(cx, |sidebar, cx| {
                sidebar.request_engine_form(Some(profile.clone()), cx);
            });
        },
    ))
}

fn engine_menu_copy_endpoint(
    menu: PopupMenu,
    view: &Entity<ContainerSidebarView>,
    endpoint: &str,
) -> PopupMenu {
    let view = view.clone();
    let endpoint = endpoint.to_string();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("container.sidebar.copy_endpoint").to_string(),
        AppIcon::Ui(UiIcon::COPY),
        move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(endpoint.clone()));
            let message = rust_i18n::t!(
                "container.sidebar.endpoint_copied",
                endpoint = endpoint.as_str()
            )
            .to_string();
            view.update(cx, |sidebar, cx| {
                sidebar.request_toast(message.clone(), true, cx);
            });
        },
    ))
}

fn engine_menu_copy_name(
    menu: PopupMenu,
    view: &Entity<ContainerSidebarView>,
    name: &str,
) -> PopupMenu {
    let view = view.clone();
    let name = name.to_string();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("container.sidebar.copy_name").to_string(),
        AppIcon::Ui(UiIcon::COPY),
        move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(name.clone()));
            let message =
                rust_i18n::t!("container.sidebar.name_copied", name = name.as_str()).to_string();
            view.update(cx, |sidebar, cx| {
                sidebar.request_toast(message.clone(), true, cx);
            });
        },
    ))
}

fn engine_menu_delete(
    menu: PopupMenu,
    view: &Entity<ContainerSidebarView>,
    id: EngineProfileId,
) -> PopupMenu {
    let view = view.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("common.delete").to_string(),
        AppIcon::Ui(UiIcon::DELETE),
        move |_, _, cx| {
            view.update(cx, |sidebar, cx| {
                sidebar.request_delete_engine(id, cx);
            });
        },
    ))
}

fn engine_row_band(params: &EngineRowParams<'_>, outline: &EngineRowOutline) -> AnyElement {
    let p = params.palette;
    let id = outline.id;
    let name = outline.name.clone();
    let description = outline.description.clone();
    let icon = outline.icon;
    let action_color = outline.action_color;
    let profile_color = outline.profile_color;
    let row_state = outline.row_state;
    let status_color = outline.status_color;
    sidebar_row_band(
        ElementId::Name(format!("container-engine-band-{id}").into()),
        row_state,
        p,
    )
    .child(
        AppIconElement::new(icon)
            .with_rems(ROW_ICON_SIZE)
            .text_color(if row_state.is_highlighted() {
                action_color
            } else {
                profile_color.unwrap_or(p.primary)
            }),
    )
    .child(
        v_flex()
            .flex_1()
            .overflow_hidden()
            .child(
                div()
                    .text_sm()
                    .line_height(px(17.0))
                    .font_weight(FontWeight::MEDIUM)
                    .truncate()
                    .child(name),
            )
            .child(
                div()
                    .text_xs()
                    .line_height(px(14.0))
                    .truncate()
                    .text_color(p.muted_foreground)
                    .child(description),
            ),
    )
    .child(
        div()
            .size(px(6.0))
            .flex_none()
            .rounded_full()
            .bg(status_color),
    )
    .child(engine_reorder_overlay(
        params.view,
        ContainerSidebarDrag::Engine(id),
        false,
        p,
    ))
    .into_any_element()
}
