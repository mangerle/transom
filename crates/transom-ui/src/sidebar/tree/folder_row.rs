//! 侧边栏文件夹树行渲染。

use super::super::entity::{SessionSidebarEvent, SessionSidebarView};
use super::reorder::{
    RowActionButtonProps, SidebarDragPreview, reorder_overlay, row_action_button,
};
use super::{
    FolderRow, ROW_CHEVRON_SIZE, ROW_HEIGHT_REM, ROW_ICON_SIZE, SidebarDrag, SidebarTarget,
    indent_padding,
};
use crate::palette::Palette;
use crate::widgets::sidebar_group_row_band;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::FolderId;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

/// 渲染单个文件夹树行，包含折叠切换、拖放放置、右键菜单与整组拨测入口。
pub(crate) fn render_folder_row(
    view: &Entity<SessionSidebarView>,
    row: &FolderRow,
    p: &Palette,
) -> AnyElement {
    let folder_id = row.id;
    let label = row.name.clone();
    let chevron = if row.collapsed {
        AppIcon::Ui(UiIcon::CHEVRON_RIGHT)
    } else {
        AppIcon::Ui(UiIcon::CHEVRON_DOWN)
    };

    let click_view = view.clone();
    let drop_view = view.clone();
    let menu_view = view.clone();

    let element = div()
        .id(ElementId::Name(format!("folder-{folder_id}").into()))
        .w_full()
        .h(rems(ROW_HEIGHT_REM))
        .pl(indent_padding(row.depth))
        .pr(px(4.0))
        .flex()
        .items_center()
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            click_view.update(cx, |this, cx| {
                this.emit_action(SessionSidebarEvent::ToggleFolderCollapsed(folder_id), cx);
            });
        })
        .on_drag(SidebarDrag::Folder(folder_id), move |_, _, _, cx| {
            cx.new(|_| SidebarDragPreview::new(label.clone(), AppIcon::Ui(UiIcon::FOLDER)))
        })
        .on_drop(move |drag: &SidebarDrag, window, cx| {
            let drag = *drag;
            drop_view.update(cx, |this, cx| {
                this.emit_action(
                    SessionSidebarEvent::Drop(drag, SidebarTarget::Folder(folder_id)),
                    cx,
                );
            });
            window.refresh();
        })
        .context_menu(move |menu, _window, _cx| folder_context_menu(menu, &menu_view, folder_id));

    let band = folder_band(row, chevron, view, p);
    element.child(band).into_any_element()
}

/// 文件夹行主体：折叠箭头、图标、名称、计数、拨测按钮与换序落点。
fn folder_band(
    row: &FolderRow,
    chevron: AppIcon,
    view: &Entity<SessionSidebarView>,
    p: &Palette,
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
    let probe_view = view.clone();
    // 表头这一个按钮拨测整组（含子文件夹）：分组巡检是本功能的主用法。
    band = band.child(row_action_button(
        RowActionButtonProps {
            element_id: format!("folder-probe-{folder_id}"),
            icon: AppIcon::Ui(UiIcon::EXCEPTION_ANALYZER),
            tooltip: rust_i18n::t!("probe.folder_tooltip").to_string(),
            idle_color: p.muted_foreground,
            on_click: move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                // 点按钮不该顺带折叠/展开这个分组。
                cx.stop_propagation();
                probe_view.update(cx, |this, cx| {
                    this.emit_action(SessionSidebarEvent::ProbeFolder(folder_id), cx);
                });
            },
        },
        p,
    ));
    let anchor = SidebarDrag::Folder(folder_id);
    // 文件夹行保留中段作为“放入该分组”，上下各三分之一用于同级换序。
    band = band.child(reorder_overlay(view, anchor, true, p));
    band
}

/// 文件夹行右键菜单：新建会话/子文件夹、重命名与删除。
fn folder_context_menu(
    menu: PopupMenu,
    view: &Entity<SessionSidebarView>,
    folder_id: FolderId,
) -> PopupMenu {
    let emit_item = |menu: PopupMenu, label: String, icon: AppIcon, event: SessionSidebarEvent| {
        let view = view.clone();
        menu.item(crate::widgets::menu_item(label, icon, move |_, _, cx| {
            view.update(cx, |this, cx| this.emit_action(event.clone(), cx));
        }))
    };

    let menu = emit_item(
        menu,
        rust_i18n::t!("menu.new_session_here").to_string(),
        AppIcon::Ui(UiIcon::ADD),
        SessionSidebarEvent::OpenNewSession(Some(folder_id)),
    );
    let menu = emit_item(
        menu,
        rust_i18n::t!("menu.new_folder").to_string(),
        AppIcon::Ui(UiIcon::NEW_FOLDER),
        SessionSidebarEvent::OpenNewFolder(Some(folder_id)),
    );
    let menu = emit_item(
        menu,
        rust_i18n::t!("menu.rename").to_string(),
        AppIcon::Ui(UiIcon::EDIT),
        SessionSidebarEvent::OpenRenameFolder(folder_id),
    )
    .item(PopupMenuItem::separator());
    emit_item(
        menu,
        rust_i18n::t!("menu.delete_folder").to_string(),
        AppIcon::Ui(UiIcon::DELETE),
        SessionSidebarEvent::DeleteFolder(folder_id),
    )
}
