//! 引擎分组行：折叠、计数与拖拽落点。

use super::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 渲染一条引擎分组行。
///
/// 设计原理（Why）：折叠、改名、新建子项与删除都以侧栏事件上交外壳；行只读
/// 目录快照，不持有 `AppShell`。重命名需要当前名称，直接取自行快照。
pub(crate) fn render_engine_folder_row(
    view: &Entity<ContainerSidebarView>,
    row: &EngineFolderRow,
    p: &Palette,
) -> AnyElement {
    let folder_id = row.id;
    let label = row.name.clone();
    let current_name = row.name.clone();
    let chevron = if row.collapsed {
        AppIcon::Ui(UiIcon::CHEVRON_RIGHT)
    } else {
        AppIcon::Ui(UiIcon::CHEVRON_DOWN)
    };
    let element = div()
        .id(ElementId::Name(
            format!("container-engine-folder-{folder_id}").into(),
        ))
        .w_full()
        .h(rems(ROW_HEIGHT_REM))
        .pl(indent_padding(row.depth))
        .pr(px(4.0))
        .flex()
        .items_center()
        .cursor_pointer()
        .on_click({
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |sidebar, cx| {
                    sidebar.request_toggle_folder(folder_id, cx)
                });
            }
        })
        .on_drag(
            ContainerSidebarDrag::Folder(folder_id),
            move |_, _, _, cx| {
                cx.new(|_| SidebarDragPreview::new(label.clone(), AppIcon::Ui(UiIcon::FOLDER)))
            },
        )
        .on_drop({
            let view = view.clone();
            move |drag: &ContainerSidebarDrag, window, cx| {
                let drag = *drag;
                view.update(cx, |sidebar, cx| {
                    sidebar.request_drop(drag, ContainerSidebarTarget::Folder(folder_id), cx);
                });
                window.refresh();
            }
        })
        .context_menu({
            let view = view.clone();
            move |menu, _window, _cx| {
                let view_new_engine = view.clone();
                let view_new_folder = view.clone();
                let view_rename = view.clone();
                let view_delete = view.clone();
                let name_for_rename = current_name.clone();
                menu.item(crate::widgets::menu_item(
                    rust_i18n::t!("container.sidebar.new_engine_here").to_string(),
                    AppIcon::Ui(UiIcon::ADD),
                    move |_, _, cx| {
                        view_new_engine.update(cx, |sidebar, cx| {
                            sidebar.request_new_engine(Some(folder_id), cx);
                        });
                    },
                ))
                .item(crate::widgets::menu_item(
                    rust_i18n::t!("container.sidebar.new_subfolder").to_string(),
                    AppIcon::Ui(UiIcon::NEW_FOLDER),
                    move |_, window, cx| {
                        view_new_folder.update(cx, |sidebar, cx| {
                            sidebar.open_folder_form(
                                Some(folder_id),
                                None,
                                String::new(),
                                window,
                                cx,
                            );
                        });
                    },
                ))
                .item(crate::widgets::menu_item(
                    rust_i18n::t!("container.sidebar.rename").to_string(),
                    AppIcon::Ui(UiIcon::EDIT),
                    move |_, window, cx| {
                        view_rename.update(cx, |sidebar, cx| {
                            sidebar.open_folder_form(
                                None,
                                Some(folder_id),
                                name_for_rename.clone(),
                                window,
                                cx,
                            );
                        });
                    },
                ))
                .item(PopupMenuItem::separator())
                .item(crate::widgets::menu_item(
                    rust_i18n::t!("container.sidebar.delete_folder").to_string(),
                    AppIcon::Ui(UiIcon::DELETE),
                    move |_, _, cx| {
                        view_delete.update(cx, |sidebar, cx| {
                            sidebar.request_delete_folder(folder_id, cx);
                        });
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
        .h(rems(BAND_HEIGHT_REM))
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
        .child(AppIconElement::new(chevron).with_size(ROW_CHEVRON_SIZE))
        .child(AppIconElement::new(AppIcon::Ui(UiIcon::FOLDER)).with_size(ROW_ICON_SIZE))
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
                .text_color(p.text.muted)
                .child(format!("({})", row.folder_count)),
        );
    }
    let anchor = ContainerSidebarDrag::Folder(folder_id);
    band = band.child(engine_reorder_overlay(view, anchor, true, p));
    element.child(band).into_any_element()
}
