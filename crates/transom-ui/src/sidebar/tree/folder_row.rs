//! 侧边栏文件夹树行渲染。

use super::reorder::{
    RowActionButtonProps, SidebarDragPreview, reorder_overlay, row_action_button,
};
use super::{
    BAND_HEIGHT, FolderRow, ROW_CHEVRON_SIZE, ROW_HEIGHT, ROW_ICON_SIZE, SidebarDrag,
    SidebarTarget, indent_padding,
};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;

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
            IconName::ChevronRight
        } else {
            IconName::ChevronDown
        };

        let element = div()
            .id(ElementId::Name(format!("folder-{folder_id}").into()))
            .w_full()
            .h(ROW_HEIGHT)
            .pl(indent_padding(row.depth))
            .pr(px(4.0))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_folder_collapsed(folder_id, cx);
            }))
            .on_drag(SidebarDrag::Folder(folder_id), move |_, _, _, cx| {
                cx.new(|_| SidebarDragPreview::new(label.clone(), IconName::Folder))
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
                let app_entity = app_entity.clone();
                move |menu, _window, _cx| {
                    let app = app_entity.clone();
                    menu.item(
                        PopupMenuItem::new(rust_i18n::t!("menu.new_session_here").to_string())
                            .icon(IconName::Plus)
                            .on_click({
                                let app = app.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.open_new_session_in_folder(folder_id, window, cx);
                                    });
                                }
                            }),
                    )
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("menu.new_folder").to_string())
                            .icon(IconName::FolderPlus)
                            .on_click({
                                let app = app.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.open_new_folder(Some(folder_id), window, cx);
                                    });
                                }
                            }),
                    )
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("menu.rename").to_string())
                            .icon(IconName::FolderPen)
                            .on_click({
                                let app = app.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.open_rename_folder(folder_id, window, cx);
                                    });
                                }
                            }),
                    )
                    .item(PopupMenuItem::separator())
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("menu.delete_folder").to_string())
                            .icon(IconName::Trash)
                            .on_click({
                                let app = app.clone();
                                move |_, _, cx| {
                                    app.update(cx, |this, cx| {
                                        this.delete_folder(folder_id, cx);
                                    });
                                }
                            }),
                    )
                }
            });
        // `drag_over` 闭包要求 `'static`，先取成局部颜色再捕获。
        let into_bg = p.drop_target;
        // 文件夹头悬停单独一档：在组标题底色上压 60% 结构线，比列表激活色更深，
        // 浅色变暗、深色变亮，避免悬浮后又和侧栏底混成一片。
        let hover_bg = p.list_head.blend(p.sidebar_border.opacity(0.6));
        // 悬浮样式必须与 `.id()` 落在同一个元素上：GPUI 只给带 id 的元素分配
        // global_id 与悬浮状态，缺少 id 时鼠标移入不会触发重绘，高亮只能等
        // 别的事件顺带重绘才补上，看上去就是"悬停不跟手"。
        let mut band = h_flex()
            .id(ElementId::Name(format!("folder-band-{folder_id}").into()))
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
            .drag_over::<SidebarDrag>(move |style, _, _, _| style.bg(into_bg))
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
            .child(div().text_xs().child(row.session_count.to_string()));

        if row.folder_count > 0 {
            band = band.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground.opacity(0.7))
                    .child(format!("({})", row.folder_count)),
            );
        }
        // 表头这一个按钮拨测整组（含子文件夹）：分组巡检是本功能的主用法。
        band = band.child(row_action_button(
            RowActionButtonProps {
                element_id: format!("folder-probe-{folder_id}"),
                icon: IconName::Radar,
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
        band = band.child(reorder_overlay(&app_entity, anchor, true, p));
        element.child(band).into_any_element()
    }
}
