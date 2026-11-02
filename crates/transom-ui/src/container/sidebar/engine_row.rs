//! 引擎连接行：图标、名称、状态点与右键菜单。

use super::*;

impl AppShell {
    pub(crate) fn active_engine_profile_id(&self) -> Option<EngineProfileId> {
        let tab = self
            .state
            .workspace
            .active_tab
            .and_then(|tab_id| self.state.workspace.tab(tab_id))?;
        match &tab.session.connection {
            transom_core::Connection::ContainerEngine(profile) => Some(profile.id),
            _ => None,
        }
    }

    pub(crate) fn render_engine_row(
        &mut self,
        row: &EngineRow,
        active: Option<EngineProfileId>,
        selected: Option<EngineProfileId>,
        states: &HashMap<EngineProfileId, ContainerWorkspaceStatus>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let profile = &row.profile;
        let id = profile.id;
        let name = profile.display_name();
        let description = profile.engine.describe();
        let icon =
            crate::session_appearance::session_icon(profile.icon).unwrap_or(IconName::Container);
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
        let app = cx.entity().clone();
        let menu_name = name.clone();
        let menu_endpoint = description.clone();

        let element = div()
            .id(ElementId::Name(format!("container-engine-{id}").into()))
            .w_full()
            .h(ROW_HEIGHT)
            .pl(indent_padding(row.depth))
            .pr(px(4.0))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    log::debug!("双击打开引擎工作区，连接: {id}");
                    if let Some(profile) = this
                        .state
                        .catalog
                        .engines
                        .iter()
                        .find(|profile| profile.id == id)
                        .cloned()
                    {
                        this.open_engine_workspace(profile, window, cx);
                    }
                } else {
                    this.state.containers.sidebar_selected = Some(id);
                    cx.notify();
                }
            }))
            .on_drag(ContainerSidebarDrag::Engine(id), {
                let drag_label = name.clone();
                move |_, _, _, cx| cx.new(|_| SidebarDragPreview::new(drag_label.clone(), icon))
            })
            .on_drop({
                let app = app.clone();
                move |drag: &ContainerSidebarDrag, window, cx| {
                    let drag = *drag;
                    let target = match folder_id {
                        Some(folder_id) => ContainerSidebarTarget::Folder(folder_id),
                        None => ContainerSidebarTarget::Root,
                    };
                    app.update(cx, |this, cx| {
                        this.handle_container_sidebar_drop(drag, target, cx);
                        window.refresh();
                    });
                }
            })
            .context_menu({
                let app = app.clone();
                move |menu, _window, _cx| {
                    let app_entity = app.clone();
                    let mut menu = menu.item(crate::widgets::menu_item(
                        rust_i18n::t!("container.sidebar.open").to_string(),
                        IconName::Play,
                        {
                            let app = app_entity.clone();
                            move |_, window, cx| {
                                app.update(cx, |this, cx| {
                                    if let Some(profile) = this
                                        .state
                                        .catalog
                                        .engines
                                        .iter()
                                        .find(|profile| profile.id == id)
                                        .cloned()
                                    {
                                        this.open_engine_workspace(profile, window, cx);
                                    }
                                });
                            }
                        },
                    ));
                    if folder_id.is_some() {
                        menu = menu.item(crate::widgets::menu_item(
                            rust_i18n::t!("container.sidebar.move_to_root").to_string(),
                            IconName::FolderTree,
                            {
                                let app = app_entity.clone();
                                move |_, _, cx| {
                                    app.update(cx, |this, cx| {
                                        this.move_engine_to_folder(id, None, cx);
                                    });
                                }
                            },
                        ));
                    }
                    menu = menu.item(crate::widgets::menu_item(
                        rust_i18n::t!("common.edit").to_string(),
                        IconName::Pen,
                        {
                            let app = app_entity.clone();
                            move |_, window, cx| {
                                app.update(cx, |this, cx| {
                                    let profile = this
                                        .state
                                        .catalog
                                        .engines
                                        .iter()
                                        .find(|profile| profile.id == id)
                                        .cloned();
                                    this.open_container_form(profile, None, window, cx);
                                });
                            }
                        },
                    ));
                    // 复制项在渲染期就把文本带走，菜单回调运行时不再回查目录。
                    let endpoint_app = app_entity.clone();
                    let endpoint = menu_endpoint.clone();
                    menu = menu.item(crate::widgets::menu_item(
                        rust_i18n::t!("container.sidebar.copy_endpoint").to_string(),
                        IconName::Copy,
                        move |_, window, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(endpoint.clone()));
                            let message = rust_i18n::t!(
                                "container.sidebar.endpoint_copied",
                                endpoint = endpoint.as_str()
                            )
                            .to_string();
                            endpoint_app.update(cx, |this, cx| {
                                this.show_toast(message, true, window, cx);
                            });
                        },
                    ));
                    let name_app = app_entity.clone();
                    let copied_name = menu_name.clone();
                    menu = menu.item(crate::widgets::menu_item(
                        rust_i18n::t!("container.sidebar.copy_name").to_string(),
                        IconName::Copy,
                        move |_, window, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copied_name.clone()));
                            let message = rust_i18n::t!(
                                "container.sidebar.name_copied",
                                name = copied_name.as_str()
                            )
                            .to_string();
                            name_app.update(cx, |this, cx| {
                                this.show_toast(message, true, window, cx);
                            });
                        },
                    ));
                    let delete_app = app_entity.clone();
                    menu.item(PopupMenuItem::separator())
                        .item(crate::widgets::menu_item(
                            rust_i18n::t!("common.delete").to_string(),
                            IconName::Trash,
                            move |_, window, cx| {
                                delete_app.update(cx, |this, cx| {
                                    this.delete_container_profile(id, window, cx);
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
            .h(BAND_HEIGHT)
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
                Icon::new(icon)
                    .with_size(ROW_ICON_SIZE)
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
                &app,
                ContainerSidebarDrag::Engine(id),
                false,
                p,
            ));
        element.child(band).into_any_element()
    }
}
