//! 活动栏拖拽交互、跟随预览与落点重序覆盖层。

use super::metrics::METRICS;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::ActivityItemId;

/// 活动栏拖拽载荷：只携带被拖动的条目标识，落点由当前布局解析。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ActivityDrag(pub(super) ActivityItemId);

/// 跟随鼠标的条目预览：只显示图标。
///
/// 设计原理（Why）：活动栏本身只有图标，预览若带名称会在窄条旁形成更宽的卡片反遮挡落点；
/// 保持同尺寸图标贴手移动，拖拽时只看落点即可。
pub(super) struct ActivityDragPreview {
    icon: IconName,
}

impl ActivityDragPreview {
    pub(super) fn new(icon: IconName) -> Self {
        Self { icon }
    }
}

impl Render for ActivityDragPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        div()
            .size(rems(METRICS.button))
            .flex()
            .items_center()
            .justify_center()
            .rounded(p.radius.md)
            .bg(p.popover)
            .border_1()
            .border_color(p.border)
            .shadow_md()
            .child(
                Icon::new(self.icon)
                    .with_size(crate::theme::tokens::rem_px(
                        METRICS.icon,
                        window.rem_size(),
                    ))
                    .text_color(p.popover_foreground),
            )
    }
}

pub(super) fn render_activity_reorder_overlay(
    app: &Entity<AppShell>,
    anchor: ActivityItemId,
    p: &Palette,
) -> AnyElement {
    v_flex()
        .absolute()
        .inset_0()
        .child(render_activity_reorder_half(app, anchor, true, p))
        .child(render_activity_reorder_half(app, anchor, false, p))
        .into_any_element()
}

/// 图标内部的上下半区：上半插到锚点之前，下半插到锚点之后。
fn render_activity_reorder_half(
    app: &Entity<AppShell>,
    anchor: ActivityItemId,
    before: bool,
    p: &Palette,
) -> AnyElement {
    let line_color = p.sidebar_accent_foreground;
    let tint = p.sidebar_accent.opacity(0.18);
    let zone = div().w_full().flex_1().border_color(line_color);
    let zone = if before {
        zone.drag_over::<ActivityDrag>(move |style, _, _, _| {
            style.border_t_2().border_color(line_color).bg(tint)
        })
    } else {
        zone.drag_over::<ActivityDrag>(move |style, _, _, _| {
            style.border_b_2().border_color(line_color).bg(tint)
        })
    };
    let app = app.clone();
    zone.on_drop(move |drag: &ActivityDrag, window, cx| {
        let dragged = drag.0;
        app.update(cx, |this, cx| {
            if this
                .settings
                .activity_bar
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

/// 两组之间的空白半区。`top_end` 为真时落到顶组末尾，否则落到底组开头。
///
/// 插入线画在靠近目标组的一侧：顶组末尾贴着上半区上沿，底组开头贴着下半区下沿。
pub(super) fn render_activity_blank_zone(
    app: &Entity<AppShell>,
    top_end: bool,
    p: &Palette,
) -> AnyElement {
    let line_color = p.sidebar_accent_foreground;
    let tint = p.sidebar_accent.opacity(0.12);
    let zone = div()
        .w(px(METRICS.button))
        .flex_1()
        .border_color(line_color);
    let zone = if top_end {
        zone.drag_over::<ActivityDrag>(move |style, _, _, _| {
            style.border_t_2().border_color(line_color).bg(tint)
        })
    } else {
        zone.drag_over::<ActivityDrag>(move |style, _, _, _| {
            style.border_b_2().border_color(line_color).bg(tint)
        })
    };
    let app = app.clone();
    zone.on_drop(move |drag: &ActivityDrag, window, cx| {
        let dragged = drag.0;
        app.update(cx, |this, cx| {
            let changed = if top_end {
                this.settings.activity_bar.move_to_top_end(dragged)
            } else {
                this.settings.activity_bar.move_to_bottom_start(dragged)
            };
            if changed {
                this.persist_layout_settings(cx);
            } else {
                cx.notify();
            }
            window.refresh();
        });
    })
    .into_any_element()
}
