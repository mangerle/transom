//! 标签栏辅助徽标组件与辅助结构体。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::tiling::TiledTabs;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{GroupId, TabId};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 窗格列表里的一行预渲染数据。
///
/// 下拉内容是点开时才构建的，因此这里先把标题、状态标识等取成拥有值，
/// 闭包内不再借用 `AppShell`。
pub(super) struct PaneRow {
    pub(super) tab_id: TabId,
    pub(super) ordinal: usize,
    pub(super) title: String,
    pub(super) dot_color: Hsla,
    pub(super) icon: AppIcon,
    pub(super) active: bool,
}

/// 合并条目下拉里的一行：一个并排标签组的预渲染数据。
pub(super) struct TiledRow {
    pub(super) group_id: GroupId,
    pub(super) ordinal: usize,
    pub(super) title: String,
    pub(super) dot_color: Hsla,
    pub(super) icon: AppIcon,
    pub(super) focused: bool,
}

/// 构建并排组徽标元素。
pub(super) fn build_tiled_group_badge(
    shell: &AppShell,
    tiled: &TiledTabs,
    app_entity: &Entity<AppShell>,
    p: &Palette,
) -> AnyElement {
    let mut rows = Vec::with_capacity(tiled.occupied_count());
    for (slot, group_id) in tiled.slots.iter().enumerate() {
        let Some(group_id) = *group_id else {
            continue;
        };
        let Some(group) = shell.state.workspace.group(group_id) else {
            continue;
        };
        let Some(tab) = shell
            .state
            .workspace
            .tabs
            .iter()
            .find(|tab| tab.id == group.active_pane)
        else {
            continue;
        };
        let (dot_color, _) = crate::workspace::pane_status_indicator(&tab.status, p);
        let mut title = tab.tab_label();
        if group.serial > 0 {
            title = format!("{title} #{}", group.serial);
        }
        rows.push(TiledRow {
            group_id,
            ordinal: slot + 1,
            title,
            dot_color,
            icon: crate::connection::icon(&tab.session.connection),
            focused: slot == tiled.focused,
        });
    }
    let layout_icon = match (tiled.rows, tiled.cols) {
        (1, 2) => AppIcon::Ui(UiIcon::EDITOR_PREVIEW),
        (2, 1) => AppIcon::Ui(UiIcon::EDITOR_ONLY),
        (1, 3) => AppIcon::Ui(UiIcon::EDITOR_PREVIEW),
        (3, 1) => AppIcon::Ui(UiIcon::EDITOR_ONLY),
        (2, 2) => AppIcon::Ui(UiIcon::EDITOR_PREVIEW),
        (2, 3) | (3, 2) => AppIcon::Ui(UiIcon::EDITOR_PREVIEW),
        (3, 3) => AppIcon::Ui(UiIcon::EDITOR_PREVIEW),
        _ => AppIcon::Ui(UiIcon::EDITOR_PREVIEW),
    };
    let count = rows.len();
    let occupied_groups = tiled.occupied_groups();
    let badge_palette = p.clone();
    let muted = p.muted_foreground;
    let foreground = p.foreground;
    let tiling_id = tiled.id;

    Button::new(ElementId::Name(format!("tiled-list-{tiling_id}").into()))
        .ghost()
        .xsmall()
        .icon(layout_icon)
        .label(count.to_string())
        .dropdown_menu({
            let app_entity = app_entity.clone();
            move |menu, _window, _cx| {
                let mut menu = menu;
                for row in rows.iter() {
                    let group_id = row.group_id;
                    let title = row.title.clone();
                    let (dot_color, icon, focused, ordinal) =
                        (row.dot_color, row.icon, row.focused, row.ordinal);
                    menu = menu.item(
                        PopupMenuItem::element(move |_window, _cx| {
                            let mut row = h_flex()
                                .gap_2()
                                .items_center()
                                .child(crate::widgets::status_dot_bare(dot_color))
                                .child(AppIconElement::new(icon).xsmall().text_color(muted))
                                .child(
                                    div()
                                        .text_xs()
                                        .flex_none()
                                        .text_color(muted)
                                        .child(ordinal.to_string()),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .flex_1()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .truncate()
                                        .child(title.clone()),
                                );
                            if focused {
                                row = row.child(
                                    AppIconElement::new(AppIcon::Ui(UiIcon::CHECK))
                                        .xsmall()
                                        .text_color(foreground),
                                );
                            }
                            row
                        })
                        .on_click({
                            let app = app_entity.clone();
                            move |_, window, cx| {
                                app.update(cx, |this, cx| {
                                    this.activate_group(group_id, window, cx);
                                });
                            }
                        }),
                    );
                }
                menu.item(PopupMenuItem::separator())
                    .submenu_with_icon(
                        Some(Icon::new(AppIcon::Ui(UiIcon::EDITOR_PREVIEW))),
                        rust_i18n::t!("tiling.adjust").to_string(),
                        _window,
                        _cx,
                        {
                            let groups = occupied_groups.clone();
                            let app = app_entity.clone();
                            let palette = badge_palette.clone();
                            move |menu, window, cx| {
                                crate::workspace::tiling_menu::build_tiling_layout_submenu(
                                    menu,
                                    groups.clone(),
                                    app.clone(),
                                    palette.clone(),
                                    window,
                                    cx,
                                )
                            }
                        },
                    )
                    .item(crate::widgets::menu_item(
                        rust_i18n::t!("menu.untile").to_string(),
                        AppIcon::Ui(UiIcon::RESTORE),
                        {
                            let app = app_entity.clone();
                            move |_, _window, cx| {
                                app.update(cx, |this, cx| {
                                    this.untile_tiling(tiling_id, cx);
                                });
                            }
                        },
                    ))
            }
        })
        .into_any_element()
}

/// 构建悬浮提示内容。
///
/// 第一行是标签可见主名（运行期标题或会话名），第二行在两者不同时补会话身份：
/// 远端程序可以把标签改成 `nvim app.rs`，但用户仍需随时确认这是哪台会话。
pub(super) fn build_tab_tooltip(
    tooltip_title: String,
    session_name: String,
    show_session: bool,
    tooltip_muted: Hsla,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_window, cx| {
        let tooltip_title = tooltip_title.clone();
        let session_name = session_name.clone();
        Tooltip::element(move |_, _| {
            let mut tip = v_flex().gap_0p5().child(
                div()
                    .text_sm()
                    .max_w(px(360.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .truncate()
                    .child(tooltip_title.clone()),
            );
            if show_session {
                tip = tip.child(
                    div()
                        .text_xs()
                        .max_w(px(360.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .truncate()
                        .text_color(tooltip_muted)
                        .child(session_name.clone()),
                );
            }
            tip
        })
        .build(_window, cx)
    }
}
