//! 状态栏徽标的拖拽换序、右键菜单与布局持久化。
//!
//! 状态栏是单行布局：徽标左右半区决定插到锚点之前/之后，没有上下分组。
//! 左侧连接状态固定，不参与拖拽；空白处与徽标菜单共用"恢复默认布局"。

use super::status_item_icon;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::StatusItemId;

const PREVIEW_SIZE: f32 = 28.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StatusDrag(StatusItemId);

/// 拖拽预览只显示条目图标，与活动栏保持一致的轻量反馈。
struct StatusDragPreview {
    icon: IconName,
}

impl StatusDragPreview {
    fn new(icon: IconName) -> Self {
        Self { icon }
    }
}

impl Render for StatusDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        div()
            .size(px(PREVIEW_SIZE))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .bg(p.popover)
            .border_1()
            .border_color(p.border)
            .shadow_md()
            .child(
                Icon::new(self.icon)
                    .small()
                    .text_color(p.popover_foreground),
            )
    }
}

/// 给徽标挂上拖拽源、左右落点与右键菜单。
pub(super) fn attach_status_drag(
    badge: Stateful<Div>,
    item: StatusItemId,
    app: &Entity<AppShell>,
    p: &Palette,
) -> AnyElement {
    let icon = status_item_icon(item);
    let overlay_app = app.clone();
    let menu_app = app.clone();
    badge
        .on_drag(StatusDrag(item), move |_, _, _, cx| {
            cx.new(|_| StatusDragPreview::new(icon))
        })
        .child(status_reorder_overlay(&overlay_app, item, p))
        .context_menu(move |menu, _window, _cx| status_item_context_menu(menu, item, &menu_app))
        .into_any_element()
}

fn status_reorder_overlay(app: &Entity<AppShell>, anchor: StatusItemId, p: &Palette) -> AnyElement {
    h_flex()
        .absolute()
        .inset_0()
        .child(status_reorder_half(app, anchor, true, p))
        .child(status_reorder_half(app, anchor, false, p))
        .into_any_element()
}

/// 徽标左右半区：左半插到锚点之前，右半插到锚点之后。
fn status_reorder_half(
    app: &Entity<AppShell>,
    anchor: StatusItemId,
    before: bool,
    p: &Palette,
) -> AnyElement {
    let line_color = p.sidebar_accent_foreground;
    let tint = p.foreground.opacity(0.06);
    let zone = div().h_full().flex_1().border_color(line_color);
    let zone = if before {
        zone.drag_over::<StatusDrag>(move |style, _, _, _| {
            style.border_l_2().border_color(line_color).bg(tint)
        })
    } else {
        zone.drag_over::<StatusDrag>(move |style, _, _, _| {
            style.border_r_2().border_color(line_color).bg(tint)
        })
    };
    let app = app.clone();
    zone.on_drop(move |drag: &StatusDrag, window, cx| {
        let dragged = drag.0;
        app.update(cx, |this, cx| {
            if this
                .settings
                .status_bar
                .move_relative_to(dragged, anchor, !before)
            {
                this.persist_layout_settings(cx);
            } else {
                cx.notify();
            }
            window.refresh();
        });
    })
    .into_any_element()
}

/// 右侧徽标簇左侧的空白落点：拖到这里直接移到最前。
pub(super) fn status_front_drop_zone(app: &Entity<AppShell>, p: &Palette) -> AnyElement {
    let line_color = p.sidebar_accent_foreground;
    let tint = p.foreground.opacity(0.04);
    let app = app.clone();
    div()
        .flex_1()
        .h_full()
        .border_color(line_color)
        .drag_over::<StatusDrag>(move |style, _, _, _| {
            style.border_r_2().border_color(line_color).bg(tint)
        })
        .on_drop(move |drag: &StatusDrag, window, cx| {
            let dragged = drag.0;
            app.update(cx, |this, cx| {
                if this.settings.status_bar.move_to_front(dragged) {
                    this.persist_layout_settings(cx);
                } else {
                    cx.notify();
                }
                window.refresh();
            });
        })
        .into_any_element()
}

fn status_item_context_menu(
    menu: PopupMenu,
    id: StatusItemId,
    app: &Entity<AppShell>,
) -> PopupMenu {
    let app_hide = app.clone();
    let app_restore = app.clone();
    menu.item(
        PopupMenuItem::new(rust_i18n::t!("status.hide_item").to_string())
            .icon(IconName::EyeOff)
            .on_click(move |_, _, cx| {
                app_hide.update(cx, |this, cx| this.hide_status_item(id, cx));
            }),
    )
    .item(PopupMenuItem::separator())
    .item(
        PopupMenuItem::new(rust_i18n::t!("activity.restore_default").to_string())
            .icon(IconName::RotateCcw)
            .on_click(move |_, _, cx| {
                app_restore.update(cx, |this, cx| this.restore_status_bar(cx));
            }),
    )
}

/// 空白区兜底菜单：状态栏没有可隐藏的具体条目，只保留整条栏共有的动作。
pub(super) fn status_bar_context_menu(menu: PopupMenu, app: &Entity<AppShell>) -> PopupMenu {
    let app_restore = app.clone();
    menu.item(
        PopupMenuItem::new(rust_i18n::t!("activity.restore_default").to_string())
            .icon(IconName::RotateCcw)
            .on_click(move |_, _, cx| {
                app_restore.update(cx, |this, cx| this.restore_status_bar(cx));
            }),
    )
}

impl AppShell {
    fn hide_status_item(&mut self, id: StatusItemId, cx: &mut Context<Self>) {
        if self.settings.status_bar.set_visible(id, false) {
            self.persist_layout_settings(cx);
        } else {
            cx.notify();
        }
    }

    pub(crate) fn restore_status_bar(&mut self, cx: &mut Context<Self>) {
        if self.settings.status_bar.restore_defaults() {
            self.persist_layout_settings(cx);
        } else {
            cx.notify();
        }
    }
}
