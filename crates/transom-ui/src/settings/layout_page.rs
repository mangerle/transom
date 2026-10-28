//! 偏好设置中的布局页：活动栏与状态栏的条目显隐与恢复默认。

use crate::activity_bar::{activity_item_icon, activity_item_label};
use crate::app::AppShell;
use crate::palette::Palette;
use crate::status_bar::{status_item_icon, status_item_label};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::button::Button;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{ActivityItemId, StatusItemId};

/// 单个布局条目的显隐行参数。
struct VisibilityRowOptions {
    key: String,
    label: String,
    icon: IconName,
    visible: bool,
    pinned: bool,
}

impl AppShell {
    pub(crate) fn render_settings_activity_bar(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let items: Vec<ActivityItemId> = self.settings.activity_bar.order().to_vec();
        let mut cards = Vec::with_capacity(items.len());
        for id in items {
            cards.push(Self::layout_visibility_card(
                VisibilityRowOptions {
                    key: format!("activity-{}", id.key()),
                    label: activity_item_label(id),
                    icon: activity_item_icon(id),
                    visible: !self.settings.activity_bar.is_hidden(id),
                    pinned: id.is_pinned(),
                },
                move |this, checked, cx| {
                    if this.settings.activity_bar.set_visible(id, checked) {
                        this.persist_layout_settings(cx);
                    } else {
                        cx.notify();
                    }
                },
                p,
                cx,
            ));
        }

        v_flex()
            .gap_2p5()
            .child(
                v_flex()
                    .gap_2()
                    .children(Self::render_two_column_grid(cards)),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .pt_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("settings.activity_bar_note").to_string()),
                    )
                    .child(
                        Button::new("activity-restore-default")
                            .small()
                            .label(rust_i18n::t!("activity.restore_default").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.restore_activity_bar(cx))),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn render_settings_status_bar(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let items: Vec<StatusItemId> = self.settings.status_bar.order().to_vec();
        let mut cards = Vec::with_capacity(items.len());
        for id in items {
            cards.push(Self::layout_visibility_card(
                VisibilityRowOptions {
                    key: format!("status-{}", id.key()),
                    label: status_item_label(id),
                    icon: status_item_icon(id),
                    visible: !self.settings.status_bar.is_hidden(id),
                    pinned: false,
                },
                move |this, checked, cx| {
                    if this.settings.status_bar.set_visible(id, checked) {
                        this.persist_layout_settings(cx);
                    } else {
                        cx.notify();
                    }
                },
                p,
                cx,
            ));
        }

        v_flex()
            .gap_2p5()
            .child(
                v_flex()
                    .gap_2()
                    .children(Self::render_two_column_grid(cards)),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .pt_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("settings.status_bar_note").to_string()),
                    )
                    .child(
                        Button::new("status-restore-default")
                            .small()
                            .label(rust_i18n::t!("activity.restore_default").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.restore_status_bar(cx))),
                    ),
            )
            .into_any_element()
    }

    /// 双列并排重排：将单列表项两两拼装成一行双列网格。
    fn render_two_column_grid(items: Vec<AnyElement>) -> Vec<AnyElement> {
        let mut rows = Vec::with_capacity(items.len().div_ceil(2));
        let mut iter = items.into_iter();
        while let Some(first) = iter.next() {
            if let Some(second) = iter.next() {
                rows.push(
                    h_flex()
                        .w_full()
                        .gap_3()
                        .child(first)
                        .child(second)
                        .into_any_element(),
                );
            } else {
                rows.push(
                    h_flex()
                        .w_full()
                        .gap_3()
                        .child(first)
                        .child(div().flex_1().min_w_0())
                        .into_any_element(),
                );
            }
        }
        rows
    }

    /// 活动栏/状态栏共用的紧凑显隐卡片。
    fn layout_visibility_card(
        options: VisibilityRowOptions,
        on_toggle: impl Fn(&mut Self, bool, &mut Context<Self>) + 'static,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let VisibilityRowOptions {
            key,
            label,
            icon,
            visible,
            pinned,
        } = options;
        let mut trailing = h_flex().items_center().gap_1p5().flex_none();
        if pinned {
            trailing = trailing.child(
                Icon::new(IconName::Lock)
                    .xsmall()
                    .text_color(p.muted_foreground),
            );
        }
        trailing = trailing.child(
            Switch::new(ElementId::Name(format!("layout-visible-{key}").into()))
                .small()
                .checked(visible)
                .disabled(pinned)
                .on_click(cx.listener(move |this, &checked, _, cx| on_toggle(this, checked, cx))),
        );
        h_flex()
            .flex_1()
            .min_w_0()
            .justify_between()
            .items_center()
            .py_1p5()
            .px_2p5()
            .rounded(p.radius.md)
            .border_1()
            .border_color(p.border)
            .bg(p.popover)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(icon).small().text_color(p.muted_foreground))
                    .child(div().text_sm().truncate().child(label)),
            )
            .child(trailing)
            .into_any_element()
    }
}
