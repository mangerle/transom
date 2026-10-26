//! 并排布局二级菜单：自适应一行，下面是一张 4×4 布局表。
//!
//! 表格本身是布局预览：鼠标悬浮在第 r 行第 c 列时，左上角 r×c 的矩形整块
//! 染成主色，表示将要应用的 r 行 c 列布局；点击立即应用并关闭菜单。
//! 容量不足或超过上限的格子置灰不可点。
//!
//! 表格用普通元素加共享状态实现，不挂自定义 View 实体：gpui 会在菜单项内部
//! 重复渲染自定义实体，第二次取租约直接 panic（"already being updated"）。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::tiling::{TilingLayoutChoice, auto_tiling_grid, tiling_layout_options};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use std::cell::Cell;
use std::rc::Rc;
use transom_core::GroupId;

/// 二级菜单内容宽度：4 个 22px 的格子加间距约 100px，留一点内边距即可，
/// 不要把菜单撑到父级标签菜单那么宽。
const MENU_CONTENT_WIDTH: f32 = 132.0;

/// 表格当前的悬浮格；菜单项内容每次渲染都读它，悬浮变化时通知菜单重绘。
type HoveredCell = Rc<Cell<Option<(usize, usize)>>>;

/// 表格渲染共享的上下文：待并排的标签组、主题、悬浮状态与用于重绘的菜单。
struct TableContext<'a> {
    groups: &'a [GroupId],
    app: &'a Entity<AppShell>,
    palette: &'a Palette,
    hovered: HoveredCell,
    menu_entity: Entity<PopupMenu>,
}

/// 构建"并排选中标签"的二级菜单；点击任一项立即应用。
pub(super) fn build_tiling_layout_submenu(
    menu: PopupMenu,
    groups: Vec<GroupId>,
    app: Entity<AppShell>,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let count = groups.len();
    let (auto_rows, auto_cols) = auto_tiling_grid(count, window);
    let mut menu = menu;

    let auto_groups = groups.clone();
    let auto_app = app.clone();
    let auto_palette = p.clone();
    menu = menu.item(
        PopupMenuItem::element(move |_, _| {
            render_auto_row(auto_rows, auto_cols, count, &auto_palette)
        })
        .on_click(move |_, window, cx| {
            auto_app.update(cx, |this, cx| {
                this.apply_tiling_layout_for(
                    auto_groups.clone(),
                    TilingLayoutChoice::Auto,
                    window,
                    cx,
                );
            });
        }),
    );

    // 悬浮格放在共享单元格里：菜单项内容是普通元素，渲染时读当前值即可。
    // 子菜单实体在这里捕获，悬浮变化时通知它重绘，高亮矩形与标题才会跟着动。
    let hovered: HoveredCell = Rc::new(Cell::new(None));
    let table_hovered = hovered.clone();
    let table_menu = cx.entity().clone();
    let table_groups = groups.clone();
    let table_app = app.clone();
    let table_palette = p.clone();
    menu = menu
        .item(PopupMenuItem::separator())
        .item(PopupMenuItem::label(
            rust_i18n::t!("tiling.fixed").to_string(),
        ))
        .item(PopupMenuItem::element(move |_, _| {
            let ctx = TableContext {
                groups: &table_groups,
                app: &table_app,
                palette: &table_palette,
                hovered: table_hovered.clone(),
                menu_entity: table_menu.clone(),
            };
            render_tiling_table(&ctx)
        }));
    menu
}

/// 自适应项：左侧名称，右侧显示本次按窗口比例算出的网格。
fn render_auto_row(rows: usize, cols: usize, count: usize, p: &Palette) -> AnyElement {
    h_flex()
        .min_w(px(MENU_CONTENT_WIDTH))
        .gap_3()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_sm()
                .child(rust_i18n::t!("tiling.auto").to_string()),
        )
        .child(preview_grid(rows, cols, count, px(7.0), px(1.0), p))
        .into_any_element()
}

/// 4×4 布局表：行数 × 列数，悬浮整块矩形染色，点击应用。
fn render_tiling_table(ctx: &TableContext) -> AnyElement {
    let count = ctx.groups.len();
    let selectable = tiling_layout_options(count);
    let current = ctx.hovered.get();
    let mut table = v_flex().gap_1();
    for rows in 1..=4 {
        let mut line = h_flex().gap_1();
        for cols in 1..=4 {
            let can_pick = selectable.contains(&(rows, cols));
            line = line.child(render_cell(ctx, rows, cols, can_pick, current));
        }
        table = table.child(line);
    }
    let caption = current.map_or_else(
        || rust_i18n::t!("tiling.table_hint").to_string(),
        |(rows, cols)| format!("{rows} × {cols}"),
    );
    v_flex()
        .min_w(px(MENU_CONTENT_WIDTH))
        .items_center()
        .gap_1()
        .child(table)
        .child(
            div()
                .text_xs()
                .text_color(ctx.palette.muted_foreground)
                .child(caption),
        )
        .into_any_element()
}

/// 单个候选布局格子：可点时悬浮更新共享状态并重绘菜单，点击应用布局。
fn render_cell(
    ctx: &TableContext,
    rows: usize,
    cols: usize,
    selectable: bool,
    current: Option<(usize, usize)>,
) -> AnyElement {
    let p = ctx.palette;
    let highlighted =
        current.is_some_and(|(hover_rows, hover_cols)| rows <= hover_rows && cols <= hover_cols);
    let hovered_here = current == Some((rows, cols));
    let background = if highlighted {
        p.primary.opacity(0.26)
    } else {
        p.tab_bar
    };
    let border = if hovered_here {
        p.primary
    } else if highlighted {
        p.primary.opacity(0.55)
    } else {
        p.border
    };
    let mut cell = div()
        .id(ElementId::Name(
            format!("tiling-table-{rows}-{cols}").into(),
        ))
        .size(px(22.0))
        .rounded(p.radius.sm)
        .border_1()
        .border_color(border)
        .bg(background);
    if selectable {
        let hovered = ctx.hovered.clone();
        let menu_entity = ctx.menu_entity.clone();
        let app = ctx.app.clone();
        let groups = ctx.groups.to_vec();
        cell = cell
            .cursor_pointer()
            .on_hover(move |is_hovered: &bool, _window, cx| {
                let next = if *is_hovered {
                    Some((rows, cols))
                } else if hovered.get() == Some((rows, cols)) {
                    None
                } else {
                    hovered.get()
                };
                if next != hovered.get() {
                    hovered.set(next);
                    // 菜单项内容不是独立视图，只能靠通知子菜单重绘来换高亮。
                    menu_entity.update(cx, |_, cx| cx.notify());
                }
            })
            .on_click(move |_, window, cx| {
                app.update(cx, |this, cx| {
                    this.apply_tiling_layout_for(
                        groups.clone(),
                        TilingLayoutChoice::Grid { rows, cols },
                        window,
                        cx,
                    );
                });
                // 自定义菜单内容不会像普通菜单项那样自动消失，主动派发
                // Escape 走 PopupMenu 自己的取消链关闭父/子菜单。
                if let Ok(escape) = Keystroke::parse("escape") {
                    window.dispatch_keystroke(escape, cx);
                }
            });
    } else if !highlighted {
        cell = cell.opacity(0.25);
    }
    cell.into_any_element()
}

/// 迷你预览：前 `filled` 格填主色，其余为空位底色。
fn preview_grid(
    rows: usize,
    cols: usize,
    filled: usize,
    cell: Pixels,
    gap: Pixels,
    p: &Palette,
) -> AnyElement {
    let mut grid = v_flex().gap(gap);
    for row in 0..rows {
        let mut line = h_flex().gap(gap);
        for col in 0..cols {
            let slot = row * cols + col;
            line = line.child(div().size(cell).rounded(px(1.0)).bg(if slot < filled {
                p.primary
            } else {
                p.border
            }));
        }
        grid = grid.child(line);
    }
    grid.into_any_element()
}
