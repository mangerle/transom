//! 侧边栏文件夹树行渲染。

use super::reorder::{
    RowActionButtonProps, SidebarDragPreview, reorder_overlay, row_action_button,
};
use super::{
    FolderRow, ROW_CHEVRON_SIZE, ROW_HEIGHT_REM, ROW_ICON_SIZE, SidebarDrag, SidebarTarget,
    indent_padding,
};
use crate::app::AppShell;
use crate::palette::Palette;
use crate::widgets::sidebar_group_row_band;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

impl AppShell {
    /// 渲染单个文件夹树行，包含折叠切换、拖放放置、右键菜单与整组拨测入口。
    pub(crate) fn render_folder_row(
        &mut self,
        row: &FolderRow,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let folder_id = row.id;
        let app_entity = cx.entity().clone();
        let label = row.name.clone();
        let chevron = if row.collapsed {
            AppIcon::Ui(UiIcon::CHEVRON_RIGHT)
        } else {
            AppIcon::Ui(UiIcon::CHEVRON_DOWN)
        };

        let element = div()
            .id(ElementId::Name(format!("folder-{folder_id}").into()))
            .w_full()
            .h(rems(ROW_HEIGHT_REM))
            .pl(indent_padding(row.depth))
            .pr(px(4.0))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_folder_collapsed(folder_id, cx);
            }))
            .on_drag(SidebarDrag::Folder(folder_id), move |_, _, _, cx| {
                cx.new(|_| SidebarDragPreview::new(label.clone(), AppIcon::Ui(UiIcon::FOLDER)))
            })
            .on_drop({
                let app = app_entity.clone();
                move |drag: &SidebarDrag, window, cx| {
                    let drag = *drag;
                    app.update(cx, |this, cx| {
                        this.handle_sidebar_drop(drag, SidebarTarget::Folder(folder_id), cx);
                        window.refresh();
                    });
                }
            })
            .context_menu({
                let menu_app = app_entity.clone();
                move |menu, _window, _cx| folder_context_menu(menu, &menu_app, folder_id)
            });

        let band = folder_band(row, chevron, &app_entity, p, cx);
        element.child(band).into_any_element()
    }
}

/// 文件夹行主体：折叠箭头、图标、名称、计数、拨测按钮与换序落点。
fn folder_band(
    row: &FolderRow,
    chevron: AppIcon,
    app: &Entity<AppShell>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> Stateful<Div> {
    let folder_id = row.id;
    // `drag_over` 闭包要求 `'static`，先取成局部颜色再捕获。
    let into_bg = p.drop_target;
    let mut band = sidebar_group_row_band(
        ElementId::Name(format!("folder-band-{folder_id}").into()),
        p,
    )
    .drag_over::<SidebarDrag>(move |style, _, _, _| style.bg(into_bg))
    .child(AppIconElement::new(chevron).with_rems(ROW_CHEVRON_SIZE))
    .child(AppIconElement::new(AppIcon::Ui(UiIcon::FOLDER)).with_rems(ROW_ICON_SIZE))
    .child(
        div()
            .flex_1()
            .overflow_hidden()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .truncate()
            .child(row.name.clone()),
    )
    .child(div().text_xs().child(row.session_count.to_string()));

    if row.folder_count > 0 {
        band = band.child(
            div()
                .text_xs()
                .text_color(p.text.muted)
                .child(format!("({})", row.folder_count)),
        );
    }
    // 表头这一个按钮拨测整组（含子文件夹）：分组巡检是本功能的主用法。
    band = band.child(row_action_button(
        RowActionButtonProps {
            element_id: format!("folder-probe-{folder_id}"),
            icon: AppIcon::Ui(UiIcon::EXCEPTION_ANALYZER),
            tooltip: rust_i18n::t!("probe.folder_tooltip").to_string(),
            idle_color: p.muted_foreground,
            on_click: cx.listener(move |this, _: &ClickEvent, _window, cx| {
                // 点按钮不该顺带折叠/展开这个分组。
                cx.stop_propagation();
                this.probe_folder(folder_id, cx);
            }),
        },
        p,
    ));
    let anchor = SidebarDrag::Folder(folder_id);
    // 文件夹行保留中段作为“放入该分组”，上下各三分之一用于同级换序。
    band = band.child(reorder_overlay(app, anchor, true, p));
    band
}

/// 文件夹行右键菜单：新建会话/子文件夹、重命名与删除。
fn folder_context_menu(
    menu: PopupMenu,
    app: &Entity<AppShell>,
    folder_id: transom_core::FolderId,
) -> PopupMenu {
    let new_session = app.clone();
    let new_folder = app.clone();
    let rename = app.clone();
    let delete = app.clone();
    menu.item(crate::widgets::menu_item(
        rust_i18n::t!("menu.new_session_here").to_string(),
        AppIcon::Ui(UiIcon::ADD),
        move |_, window, cx| {
            new_session.update(cx, |this, cx| {
                this.open_new_session_in_folder(folder_id, window, cx);
            });
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("menu.new_folder").to_string(),
        AppIcon::Ui(UiIcon::NEW_FOLDER),
        move |_, window, cx| {
            new_folder.update(cx, |this, cx| {
                this.open_new_folder(Some(folder_id), window, cx);
            });
        },
    ))
    .item(crate::widgets::menu_item(
        rust_i18n::t!("menu.rename").to_string(),
        AppIcon::Ui(UiIcon::EDIT),
        move |_, window, cx| {
            rename.update(cx, |this, cx| {
                this.open_rename_folder(folder_id, window, cx);
            });
        },
    ))
    .item(PopupMenuItem::separator())
    .item(crate::widgets::menu_item(
        rust_i18n::t!("menu.delete_folder").to_string(),
        AppIcon::Ui(UiIcon::DELETE),
        move |_, _, cx| {
            delete.update(cx, |this, cx| {
                this.delete_folder(folder_id, cx);
            });
        },
    ))
}
