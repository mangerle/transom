//! 纵向标签面板的行渲染与交互。
//!
//! 每组一行：状态点、连接图标、会话名（含 `#N` 序号）、主机副标题，
//! 以及分屏/并排角标与关闭按钮。点击激活该组的活动窗格，中键关闭组，
//! 右键复用横向标签栏的上下文菜单，拖拽上下换序由 `drag` 模块处理。

use super::drag::attach_vertical_tab_drag;
use crate::app::AppShell;
use crate::app::app_settings;
use crate::connection::session_host;
use crate::palette::Palette;
use crate::theme::tokens::size;

use crate::workspace::tab_strip::menu::{TabContextMenuParams, build_tab_context_menu};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use transom_core::{Connection, GroupId, SplitAxis};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 行槽位高度；与侧栏会话行保持一致。
const ROW_HEIGHT: Rems = rems(size::TREE_ROW_HEIGHT);
/// 内层高亮条高度：留出上下间隙，背景不覆盖整行槽位。
const ROW_BAND_HEIGHT: Rems = rems(size::TREE_BAND_HEIGHT);
/// 行内图标尺寸：与侧栏树行图标一致，随界面密度同步缩放。
const ROW_ICON_SIZE: Rems = rems(size::TREE_ROW_ICON);

/// 渲染纵向标签面板中的单个标签组行。
pub(super) fn render_vertical_tab_row(
    shell: &AppShell,
    group_id: GroupId,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> Option<AnyElement> {
    let group = shell.state.workspace.group(group_id)?;
    let tab = shell.state.workspace.tab(group.active_pane)?;
    let app_entity = cx.entity().clone();
    let palette = p.clone();
    let active_pane = group.active_pane;
    let session_id = tab.session_id;
    let pane_count = group.pane_count();
    let serial = group.serial;
    let pane_axis = group.root.root_axis();
    // 激活判定用"组包含活动窗格"而不是比较活动窗格本身：组内切窗格后，
    // 行仍然保持高亮，不会短暂熄灭。
    let active = shell
        .state
        .workspace
        .active_tab
        .is_some_and(|tab_id| group.contains(tab_id));
    // 与横向标签栏同一口径：优先运行期 OSC 标题，空标题回落到会话名。
    // 纵向面板是“切换正在运行的东西”，与顶部标签显示同一名字才不会让人
    // 以为两处指向不同会话；会话身份由副标题的连接目标承担。
    let title = tab.tab_label();
    let labeled = crate::workspace::tab_presentation::tab_title(tab, serial);
    let subtitle = crate::workspace::tab_presentation::tab_subtitle(tab);
    let is_local = matches!(tab.session.connection, Connection::Local(_));
    let dot_color = crate::workspace::tab_presentation::tab_status_dot(tab, p);
    let (icon, icon_color) =
        crate::workspace::tab_presentation::tab_icon(shell, tab, active, p, cx);
    let tiled = shell.state.tiling_of_group(group_id);
    let tiled_badge = tiled.is_some();
    let this_set_id = tiled.map(|tiled| tiled.id);
    let menu_host = session_host(&tab.session).map(str::to_string);
    let can_tile = shell.state.workspace.groups.len() >= 2;

    // 右键菜单与横向标签栏共用同一套参数：并排、多选、复制主机等动作的
    // 可用性判断必须与标签栏完全一致，避免两个入口给出不同结果。
    let crate::workspace::tab_selection::TabSelectionModel {
        selection_groups,
        entry_selected,
        selected_singles,
        touched_sets,
        show_add_to_tiling,
        show_multi_tile,
    } = crate::workspace::tab_selection::selection_model(&shell.state.workspace, group_id, tiled);
    let tile_enabled = this_set_id.is_none() && can_tile;

    let mut band = h_flex()
        .id(ElementId::Name(format!("vertical-tab-{group_id}").into()))
        .relative()
        .w_full()
        .h(ROW_BAND_HEIGHT)
        .px_2()
        .gap_2()
        .items_center()
        .rounded(p.radius.md)
        .bg(if active {
            p.sidebar_accent
        } else if entry_selected {
            p.sidebar_accent.opacity(0.42)
        } else {
            p.sidebar_accent.opacity(0.14)
        })
        .text_color(if active {
            p.sidebar_accent_foreground
        } else {
            p.sidebar_foreground
        })
        .cursor_pointer();
    if active {
        // 活动行与悬浮态同为实心强调色，靠左侧竖条区分"当前正在使用的组"。
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
    } else {
        band = band.hover(|style| style.bg(p.sidebar_accent));
    }

    band = band
        .child(crate::widgets::status_dot_bare(dot_color))
        .child(
            AppIconElement::new(icon)
                .with_rems(ROW_ICON_SIZE)
                .text_color(icon_color),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .line_height(px(17.0))
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .child(labeled.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .line_height(px(14.0))
                        .truncate()
                        .text_color(if active {
                            p.sidebar_accent_foreground
                        } else {
                            p.muted_foreground
                        })
                        .child(subtitle.clone()),
                ),
        );

    if pane_count > 1 {
        let split_icon = if pane_axis == SplitAxis::Horizontal {
            AppIcon::Ui(UiIcon::EDITOR_PREVIEW)
        } else {
            AppIcon::Ui(UiIcon::EDITOR_ONLY)
        };
        band = band.child(
            h_flex()
                .flex_none()
                .gap_0p5()
                .items_center()
                .text_color(if active {
                    p.sidebar_accent_foreground
                } else {
                    p.muted_foreground
                })
                .child(AppIconElement::new(split_icon).xsmall())
                .child(div().text_xs().child(pane_count.to_string())),
        );
    }
    if tiled_badge {
        band = band.child(
            AppIconElement::new(AppIcon::Ui(UiIcon::EDITOR_PREVIEW))
                .xsmall()
                .text_color(if active {
                    p.sidebar_accent_foreground
                } else {
                    p.muted_foreground
                }),
        );
    }

    if app_settings(cx).window.show_tab_close_button {
        band = band.child(crate::workspace::tab_strip::render_tab_close_button(
            group_id,
            active,
            tiled,
            &app_entity,
            p,
            cx,
        ));
    }

    let tooltip_label = if subtitle.is_empty() {
        labeled
    } else {
        format!("{labeled}\n{subtitle}")
    };
    let menu_selection = selected_singles.clone();
    let touched_sets_count = touched_sets.len();
    let selected_singles_count = selected_singles.len();
    let menu_app_entity = app_entity.clone();

    let row = attach_vertical_tab_drag(
        div()
            .id(ElementId::Name(
                format!("vertical-tab-row-{group_id}").into(),
            ))
            .w_full()
            .h(ROW_HEIGHT)
            .flex()
            .items_center(),
        group_id,
        title,
        icon,
        &app_entity,
        p,
    )
    .child(
        band.on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |this, _, window, cx| {
                this.close_group(group_id, window, cx);
            }),
        )
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            let modifiers = event.modifiers();
            if modifiers.control || modifiers.platform {
                this.toggle_tab_selection(&selection_groups, cx);
            } else {
                this.clear_tab_selection(cx);
                this.activate_tab(active_pane, window, cx);
            }
        }))
        .context_menu(move |menu, window, cx| {
            build_tab_context_menu(
                menu,
                window,
                cx,
                &menu_app_entity,
                TabContextMenuParams {
                    active_pane,
                    session_id,
                    is_local,
                    pane_count,
                    tile_enabled,
                    group_id,
                    this_set_id,
                    show_add_to_tiling,
                    menu_selection: menu_selection.clone(),
                    show_multi_tile,
                    palette: palette.clone(),
                    entry_selected,
                    touched_sets_count,
                    selected_singles_count,
                    host: menu_host.clone(),
                },
            )
        }),
    )
    .tooltip(move |_window, cx| Tooltip::new(tooltip_label.clone()).build(_window, cx));

    Some(row.into_any_element())
}
