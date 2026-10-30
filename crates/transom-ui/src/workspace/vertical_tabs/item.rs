//! 纵向标签面板的行渲染与交互。
//!
//! 每组一行：状态点、连接图标、会话名（含 `#N` 序号）、主机副标题，
//! 以及分屏/并排角标与关闭按钮。点击激活该组的活动窗格，中键关闭组，
//! 右键复用横向标签栏的上下文菜单，拖拽上下换序由 `drag` 模块处理。

use super::drag::attach_vertical_tab_drag;
use crate::app::AppShell;
use crate::connection::session_host;
use crate::palette::Palette;
use crate::tiling::{MAX_TILING_SLOTS, TilingId};
use crate::workspace::pane_status_indicator;
use crate::workspace::tab_strip::menu::{TabContextMenuParams, build_tab_context_menu};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{Connection, GroupId, SplitAxis};

/// 行槽位高度；与侧栏会话行保持一致（两处需同步）。
const ROW_HEIGHT: Pixels = px(38.0);
/// 内层高亮条高度：留出上下间隙，背景不覆盖整行槽位。
const ROW_BAND_HEIGHT: Pixels = px(34.0);
/// 行内图标尺寸：与 34px 内容条匹配。
const ROW_ICON_SIZE: Pixels = px(18.0);

/// 渲染纵向标签面板中的单个标签组行。
pub(super) fn render_vertical_tab_row(
    shell: &AppShell,
    group_id: GroupId,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> Option<AnyElement> {
    let group = shell.workspace.group(group_id)?;
    let tab = shell.workspace.tab(group.active_pane)?;
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
        .workspace
        .active_tab
        .is_some_and(|tab_id| group.contains(tab_id));
    let title = tab.display_name();
    let labeled = if serial > 0 {
        format!("{title} #{serial}")
    } else {
        title.clone()
    };
    let subtitle = connection_detail(&tab.session.connection);
    let is_local = matches!(tab.session.connection, Connection::Local(_));
    let (dot_color, _) = pane_status_indicator(&tab.status, p);
    let appearance = shell.session_appearance(tab);
    let icon = appearance
        .icon
        .unwrap_or_else(|| connection_icon(&tab.session.connection));
    let icon_color = appearance.color.unwrap_or(if active {
        p.sidebar_accent_foreground
    } else {
        p.muted_foreground
    });
    let tiled = shell.tiling_of_group(group_id);
    let tiled_badge = tiled.is_some();
    let this_set_id = tiled.map(|tiled| tiled.id);
    let menu_host = session_host(&tab.session).map(str::to_string);
    let can_tile = shell.workspace.groups.len() >= 2;

    // 右键菜单与横向标签栏共用同一套参数：并排、多选、复制主机等动作的
    // 可用性判断必须与标签栏完全一致，避免两个入口给出不同结果。
    let selection_groups = match tiled {
        Some(tiled) => tiled.occupied_groups(),
        None => vec![group_id],
    };
    let entry_selected = !selection_groups.is_empty()
        && selection_groups
            .iter()
            .all(|group| shell.workspace.tab_selection.contains(group));
    let selected_singles: Vec<GroupId> = shell
        .workspace
        .groups
        .iter()
        .filter(|group| {
            shell.workspace.tab_selection.contains(&group.id)
                && shell.tiling_of_group(group.id).is_none()
        })
        .map(|group| group.id)
        .collect();
    let touched_sets: Vec<TilingId> = shell
        .workspace
        .tiled_sets
        .iter()
        .filter(|tiled| {
            tiled
                .occupied_groups()
                .iter()
                .any(|group| shell.workspace.tab_selection.contains(group))
        })
        .map(|tiled| tiled.id)
        .collect();
    let show_add_to_tiling = match (this_set_id, touched_sets.as_slice()) {
        (Some(id), [only]) if *only == id => !selected_singles.is_empty(),
        _ => false,
    };
    let show_multi_tile = entry_selected
        && touched_sets.is_empty()
        && (2..=MAX_TILING_SLOTS).contains(&selected_singles.len());
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
        .child(div().size(px(8.0)).rounded_full().flex_none().bg(dot_color))
        .child(
            Icon::new(icon)
                .with_size(ROW_ICON_SIZE)
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
            IconName::Columns2
        } else {
            IconName::Rows2
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
                .child(Icon::new(split_icon).xsmall())
                .child(div().text_xs().child(pane_count.to_string())),
        );
    }
    if tiled_badge {
        band = band.child(Icon::new(IconName::Grid2x2).xsmall().text_color(if active {
            p.sidebar_accent_foreground
        } else {
            p.muted_foreground
        }));
    }

    if shell.settings.window.show_tab_close_button {
        let close = div()
            .id(ElementId::Name(
                format!("vertical-tab-close-{group_id}").into(),
            ))
            .size(px(18.0))
            .rounded(p.radius.sm)
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(if active {
                p.sidebar_accent_foreground
            } else {
                p.muted_foreground
            })
            .hover(|style| {
                style
                    .bg(p.muted_foreground.opacity(0.2))
                    .text_color(p.foreground)
            })
            .active(|style| style.bg(p.muted_foreground.opacity(0.3)))
            .tooltip(|_window, cx| {
                Tooltip::new(rust_i18n::t!("app.close").to_string()).build(_window, cx)
            })
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.close_group(group_id, window, cx);
            }))
            .child(Icon::new(IconName::WindowClose).xsmall());
        band = band.child(close);
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

/// 行副标题：主机地址或本地 Shell 名，没有可展示信息时为空串。
fn connection_detail(connection: &Connection) -> String {
    match connection {
        Connection::Ssh(ssh) => format!("{}@{}", ssh.username, ssh.host),
        Connection::Mosh(mosh) => format!("{}@{}", mosh.username, mosh.host),
        Connection::Telnet(telnet) => format!("{}:{}", telnet.host, telnet.port),
        Connection::Local(local) => local.shell.clone().unwrap_or_default(),
        Connection::Serial(serial) => serial.port.clone(),
        _ => String::new(),
    }
}

/// 连接类型图标：与横向标签栏使用同一套映射。
pub(super) fn connection_icon(connection: &Connection) -> IconName {
    match connection {
        Connection::Local(_) => IconName::Terminal,
        Connection::Serial(_) => IconName::Cable,
        Connection::Telnet(_) => IconName::Network,
        Connection::Mosh(_) => IconName::Radio,
        _ => IconName::Server,
    }
}
