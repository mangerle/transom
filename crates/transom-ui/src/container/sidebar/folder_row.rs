//! 引擎分组行：折叠、计数与拖拽落点。

use super::*;

impl AppShell {
    pub(crate) fn render_engine_folder_row(
        &mut self,
        row: &EngineFolderRow,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let folder_id = row.id;
        let app = cx.entity().clone();
        let label = row.name.clone();
        let chevron = if row.collapsed {
            IconName::ChevronRight
        } else {
            IconName::ChevronDown
        };
        let element = div()
            .id(ElementId::Name(
                format!("container-engine-folder-{folder_id}").into(),
            ))
            .w_full()
            .h(ROW_HEIGHT)
            .pl(indent_padding(row.depth))
            .pr(px(4.0))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_engine_folder(folder_id, cx);
            }))
            .on_drag(
                ContainerSidebarDrag::Folder(folder_id),
                move |_, _, _, cx| {
                    cx.new(|_| SidebarDragPreview::new(label.clone(), IconName::Folder))
                },
            )
            .on_drop({
                let app = app.clone();
                move |drag: &ContainerSidebarDrag, window, cx| {
                    let drag = *drag;
                    app.update(cx, |this, cx| {
                        this.handle_container_sidebar_drop(
                            drag,
                            ContainerSidebarTarget::Folder(folder_id),
                            cx,
                        );
                        window.refresh();
                    });
                }
            })
            .context_menu({
                let app = app.clone();
                move |menu, _window, _cx| {
                    let app_entity = app.clone();
                    menu.item(crate::widgets::menu_item(
                        rust_i18n::t!("container.sidebar.new_engine_here").to_string(),
                        IconName::Plus,
                        {
                            let app = app_entity.clone();
                            move |_, window, cx| {
                                app.update(cx, |this, cx| {
                                    this.open_new_container_in_folder(Some(folder_id), window, cx);
                                });
                            }
                        },
                    ))
                    .item(crate::widgets::menu_item(
                        rust_i18n::t!("container.sidebar.new_subfolder").to_string(),
                        IconName::FolderPlus,
                        {
                            let app = app_entity.clone();
                            move |_, window, cx| {
                                app.update(cx, |this, cx| {
                                    this.open_engine_folder_form(Some(folder_id), None, window, cx);
                                });
                            }
                        },
                    ))
                    .item(crate::widgets::menu_item(
                        rust_i18n::t!("container.sidebar.rename").to_string(),
                        IconName::FolderPen,
                        {
                            let app = app_entity.clone();
                            move |_, window, cx| {
                                app.update(cx, |this, cx| {
                                    this.open_engine_folder_form(None, Some(folder_id), window, cx);
                                });
                            }
                        },
                    ))
                    .item(PopupMenuItem::separator())
                    .item(crate::widgets::menu_item(
                        rust_i18n::t!("container.sidebar.delete_folder").to_string(),
                        IconName::Trash,
                        {
                            let app = app_entity.clone();
                            move |_, window, cx| {
                                app.update(cx, |this, cx| {
                                    this.delete_engine_folder(folder_id, window, cx);
                                });
                            }
                        },
                    ))
                }
            });

        let into_bg = p.drop_target;
        let hover_bg = p.list_head.blend(p.sidebar_border.opacity(0.6));
        let mut band = h_flex()
            .id(ElementId::Name(
                format!("container-engine-folder-band-{folder_id}").into(),
            ))
            .relative()
            .w_full()
            .h(BAND_HEIGHT)
            .px_2()
            .gap_1()
            .items_center()
            .rounded(p.radius.md)
            .bg(p.list_head)
            .border_b_1()
            .border_color(p.sidebar_border)
            .text_color(p.sidebar_foreground)
            .hover(move |style| style.bg(hover_bg))
            .drag_over::<ContainerSidebarDrag>(move |style, _, _, _| style.bg(into_bg))
            .child(Icon::new(chevron).with_size(ROW_CHEVRON_SIZE))
            .child(
                Icon::new(if row.collapsed {
                    IconName::Folder
                } else {
                    IconName::FolderOpen
                })
                .with_size(ROW_ICON_SIZE),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(row.name.clone()),
            )
            .child(div().text_xs().child(row.engine_count.to_string()));
        if row.folder_count > 0 {
            band = band.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground.opacity(0.7))
                    .child(format!("({})", row.folder_count)),
            );
        }
        let anchor = ContainerSidebarDrag::Folder(folder_id);
        band = band.child(engine_reorder_overlay(&app, anchor, true, p));
        element.child(band).into_any_element()
    }
}
