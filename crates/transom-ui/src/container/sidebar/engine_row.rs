//! 引擎连接行：图标、名称、状态点与右键菜单。

use super::*;
use crate::container::types::ContainerWorkspaceStatus;
use std::collections::HashMap;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 渲染一条引擎连接行。
///
/// 设计原理（Why）：行只读目录快照与连接状态；双击打开、右键菜单、复制与
/// 删除都以侧栏事件上交外壳，行本身不持有 `AppShell`。
pub(crate) fn render_engine_row(
    view: &Entity<ContainerSidebarView>,
    row: &EngineRow,
    active: Option<EngineProfileId>,
    selected: Option<EngineProfileId>,
    states: &HashMap<EngineProfileId, ContainerWorkspaceStatus>,
    p: &Palette,
) -> AnyElement {
    let profile = &row.profile;
    let id = profile.id;
    let name = profile.display_name();
    let description = profile.engine.describe();
    let icon = crate::session_appearance::session_icon(profile.icon)
        .unwrap_or_else(|| crate::session_appearance::container_engine_icon(profile.engine.kind));
    let profile_color = crate::session_appearance::session_color(profile.color);
    let folder_id = profile.folder_id;
    let is_active = active == Some(id);
    let is_selected = selected == Some(id);
    let status_color = match states.get(&id) {
        Some(ContainerWorkspaceStatus::Connected) => p.success,
        Some(ContainerWorkspaceStatus::Connecting) => p.warning,
        Some(ContainerWorkspaceStatus::Failed(_))
        | Some(ContainerWorkspaceStatus::Disconnected) => p.danger,
        None => p.border,
    };
    let view_entity = view.clone();
    let menu_name = name.clone();
    let menu_endpoint = description.clone();
    let profile_for_open = (**profile).clone();
    let profile_for_menu = (**profile).clone();
    let profile_for_edit = (**profile).clone();

    let element = div()
        .id(ElementId::Name(format!("container-engine-{id}").into()))
        .w_full()
        .h(rems(ROW_HEIGHT_REM))
        .pl(indent_padding(row.depth))
        .pr(px(4.0))
        .flex()
        .items_center()
        .cursor_pointer()
        .on_click({
            let view = view.clone();
            let profile = profile_for_open.clone();
            move |event: &ClickEvent, _, cx| {
                if event.click_count() >= 2 {
                    log::debug!("双击打开引擎工作区，连接: {id}");
                    view.update(cx, |sidebar, cx| {
                        sidebar.request_open_engine(profile.clone(), cx);
                    });
                } else {
                    view.update(cx, |sidebar, cx| sidebar.select_engine(id, cx));
                }
            }
        })
        .on_drag(ContainerSidebarDrag::Engine(id), {
            let drag_label = name.clone();
            move |_, _, _, cx| cx.new(|_| SidebarDragPreview::new(drag_label.clone(), icon))
        })
        .on_drop({
            let view = view.clone();
            move |drag: &ContainerSidebarDrag, window, cx| {
                let drag = *drag;
                let target = match folder_id {
                    Some(folder_id) => ContainerSidebarTarget::Folder(folder_id),
                    None => ContainerSidebarTarget::Root,
                };
                view.update(cx, |sidebar, cx| {
                    sidebar.request_drop(drag, target, cx);
                });
                window.refresh();
            }
        })
        .context_menu({
            let view = view.clone();
            move |menu, _window, _cx| {
                let view_open = view.clone();
                let mut menu = menu.item(crate::widgets::menu_item(
                    rust_i18n::t!("container.sidebar.open").to_string(),
                    AppIcon::Ui(UiIcon::RUN),
                    {
                        let profile = profile_for_menu.clone();
                        move |_, _, cx| {
                            view_open.update(cx, |sidebar, cx| {
                                sidebar.request_open_engine(profile.clone(), cx);
                            });
                        }
                    },
                ));
                if folder_id.is_some() {
                    let view_move = view.clone();
                    menu = menu.item(crate::widgets::menu_item(
                        rust_i18n::t!("container.sidebar.move_to_root").to_string(),
                        AppIcon::Ui(UiIcon::SHOW_AS_TREE),
                        move |_, _, cx| {
                            view_move.update(cx, |sidebar, cx| {
                                sidebar.request_move_engine_to_root(id, cx);
                            });
                        },
                    ));
                }
                let view_edit = view.clone();
                menu = menu.item(crate::widgets::menu_item(
                    rust_i18n::t!("common.edit").to_string(),
                    AppIcon::Ui(UiIcon::EDIT),
                    {
                        let profile = profile_for_edit.clone();
                        move |_, _, cx| {
                            view_edit.update(cx, |sidebar, cx| {
                                sidebar.request_engine_form(Some(profile.clone()), cx);
                            });
                        }
                    },
                ));
                // 复制项在渲染期就把文本带走，菜单回调运行时不再回查目录。
                let view_copy_endpoint = view.clone();
                let endpoint = menu_endpoint.clone();
                menu = menu.item(crate::widgets::menu_item(
                    rust_i18n::t!("container.sidebar.copy_endpoint").to_string(),
                    AppIcon::Ui(UiIcon::COPY),
                    move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(endpoint.clone()));
                        let message = rust_i18n::t!(
                            "container.sidebar.endpoint_copied",
                            endpoint = endpoint.as_str()
                        )
                        .to_string();
                        view_copy_endpoint.update(cx, |sidebar, cx| {
                            sidebar.request_toast(message.clone(), true, cx);
                        });
                    },
                ));
                let view_copy_name = view.clone();
                let copied_name = menu_name.clone();
                menu = menu.item(crate::widgets::menu_item(
                    rust_i18n::t!("container.sidebar.copy_name").to_string(),
                    AppIcon::Ui(UiIcon::COPY),
                    move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copied_name.clone()));
                        let message = rust_i18n::t!(
                            "container.sidebar.name_copied",
                            name = copied_name.as_str()
                        )
                        .to_string();
                        view_copy_name.update(cx, |sidebar, cx| {
                            sidebar.request_toast(message.clone(), true, cx);
                        });
                    },
                ));
                let view_delete = view.clone();
                menu.item(PopupMenuItem::separator())
                    .item(crate::widgets::menu_item(
                        rust_i18n::t!("common.delete").to_string(),
                        AppIcon::Ui(UiIcon::DELETE),
                        move |_, _, cx| {
                            view_delete.update(cx, |sidebar, cx| {
                                sidebar.request_delete_engine(id, cx);
                            });
                        },
                    ))
            }
        });

    let mut band = h_flex()
        .id(ElementId::Name(
            format!("container-engine-band-{id}").into(),
        ))
        .relative()
        .w_full()
        .h(rems(BAND_HEIGHT_REM))
        .px_2()
        .gap_2()
        .items_center()
        .rounded(p.radius.md)
        .bg(if is_active || is_selected {
            p.list_active
        } else {
            p.list_background
        })
        .text_color(if is_active {
            p.sidebar_accent_foreground
        } else {
            p.sidebar_foreground
        });
    if is_active {
        band = band.child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(2.0))
                .rounded_full()
                .bg(p.list_active_border),
        );
    } else if !is_selected {
        band = band.hover(|style| style.bg(p.list_hover));
    }
    let action_color = if is_active {
        p.sidebar_accent_foreground
    } else {
        p.muted_foreground
    };
    band = band
        .child(
            AppIconElement::new(icon)
                .with_rems(ROW_ICON_SIZE)
                .text_color(if is_active {
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
            &view_entity,
            ContainerSidebarDrag::Engine(id),
            false,
            p,
        ));
    element.child(band).into_any_element()
}
