//! 偏好设置中的布局页：活动栏条目显隐与恢复默认。

use crate::activity_bar::{activity_item_icon, activity_item_label};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::button::Button;
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::ActivityItemId;

/// 构造“布局”设置页。
pub(super) fn activity_layout_page(app: Entity<AppShell>, p: Palette) -> SettingPage {
    SettingPage::new(rust_i18n::t!("settings.tab_layout").to_string())
        .icon(IconName::LayoutDashboard)
        .group(
            SettingGroup::new().item(SettingItem::render(move |_options, _window, cx| {
                app.update(cx, |this, cx| this.render_settings_layout(&p, cx))
            })),
        )
}

impl AppShell {
    pub(crate) fn render_settings_layout(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let items: Vec<_> = self.settings.activity_bar.order().to_vec();
        let mut rows = Vec::with_capacity(items.len());
        for id in items {
            rows.push(self.render_activity_visibility_row(id, p, cx));
        }

        v_flex()
            .gap_4()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child(rust_i18n::t!("settings.activity_bar").to_string()),
            )
            .child(v_flex().gap_1().children(rows))
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
            )
            .into_any_element()
    }

    /// 单个条目的显示开关。设置入口用锁图标与禁用开关表达"固定显示"。
    fn render_activity_visibility_row(
        &mut self,
        id: ActivityItemId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let visible = !self.settings.activity_bar.is_hidden(id);
        let pinned = id.is_pinned();
        let label = activity_item_label(id);
        let icon = activity_item_icon(id);
        let mut trailing = h_flex().items_center().gap_2();
        if pinned {
            trailing = trailing.child(
                Icon::new(IconName::Lock)
                    .xsmall()
                    .text_color(p.muted_foreground),
            );
        }
        trailing = trailing.child(
            Switch::new(ElementId::Name(
                format!("activity-visible-{}", id.key()).into(),
            ))
            .small()
            .checked(visible)
            .disabled(pinned)
            .on_click(cx.listener(move |this, &checked, _, cx| {
                if this.settings.activity_bar.set_visible(id, checked) {
                    this.persist_activity_bar(cx);
                } else {
                    cx.notify();
                }
            })),
        );
        h_flex()
            .justify_between()
            .items_center()
            .py_1()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(icon).small().text_color(p.muted_foreground))
                    .child(div().text_sm().child(label)),
            )
            .child(trailing)
            .into_any_element()
    }
}
