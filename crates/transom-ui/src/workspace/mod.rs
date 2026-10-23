//! 主工作区标签栏、分屏窗格、断线重连提示与底部硬件状态监控栏渲染模块。

mod panes;
mod tab_drag;
mod tab_strip;

use crate::app::{AppShell, TabStatus};
use crate::palette::{Palette, localized_error};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::TabId;

impl AppShell {
    pub(crate) fn render_main(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let disconnected_tab = self.active_tab.and_then(|tab_id| {
            self.tabs
                .iter()
                .find(|tab| tab.id == tab_id && matches!(&tab.status, TabStatus::Disconnected))
                .map(|tab| tab.id)
        });
        let content = self.render_active_content(window, cx, p);

        let mut main = v_flex()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .bg(p.background);
        if !self.immersive {
            main = main.child(self.render_tab_strip(p, cx));
        }
        if let Some(tab_id) = disconnected_tab {
            main = main.child(self.render_reconnect_banner(tab_id, p, cx));
        }
        main.child(content).into_any_element()
    }

    /// 按标签组渲染标签栏：一个标签组只显示一个条目，标题取活动窗格。
    pub(crate) fn render_reconnect_banner(
        &mut self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .px_3()
            .py_1()
            .gap_3()
            .items_center()
            .justify_between()
            .bg(p.muted)
            .text_color(p.muted_foreground)
            .child(
                div()
                    .text_xs()
                    .child(rust_i18n::t!("tab.disconnected").to_string()),
            )
            .child(
                Button::new(ElementId::Name(format!("banner-reconnect-{tab_id}").into()))
                    .primary()
                    .xsmall()
                    .label(rust_i18n::t!("common.reconnect").to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.reconnect_tab(tab_id, window, cx)
                    })),
            )
            .into_any_element()
    }

    pub(crate) fn render_tab_status(
        &mut self,
        index: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let tab_id = self.tabs[index].id;
        let status = &self.tabs[index].status;
        let (label, detail) = match status {
            TabStatus::Connecting => (
                rust_i18n::t!("tab.connecting").to_string(),
                rust_i18n::t!("status.connecting").to_string(),
            ),
            TabStatus::Connected => (rust_i18n::t!("tab.connected").to_string(), String::new()),
            TabStatus::Disconnected => (
                rust_i18n::t!("tab.disconnected").to_string(),
                rust_i18n::t!("status.disconnected").to_string(),
            ),
            TabStatus::Failed(error) => (
                rust_i18n::t!("tab.failed").to_string(),
                localized_error(error),
            ),
        };
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_3()
            .p_6()
            .child(div().text_lg().child(label))
            .child(div().text_sm().text_color(p.muted_foreground).child(detail))
            .child(
                Button::new(ElementId::Name(format!("reconnect-{tab_id}").into()))
                    .primary()
                    .label(rust_i18n::t!("common.reconnect").to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.reconnect_tab(tab_id, window, cx)
                    })),
            )
            .into_any_element()
    }

    pub(crate) fn render_empty_state(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_3()
            .child(Icon::new(IconName::Terminal).text_color(p.muted_foreground))
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.foreground)
                    .child(rust_i18n::t!("app.no_tab").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("app.no_tab_hint").to_string()),
            )
            .child(
                Button::new("empty-new-session")
                    .primary()
                    .small()
                    .icon(IconName::Plus)
                    .label(rust_i18n::t!("app.new_session").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_new_session(window, cx);
                    })),
            )
            .into_any_element()
    }
}

/// 窗格状态的可视标识：状态灯颜色与图标。
///
/// 窗格头与标签栏的窗格列表共用同一套映射，避免两处对"什么颜色代表连接失败"
/// 的理解分叉。
pub(super) fn pane_status_indicator(status: &TabStatus, p: &Palette) -> (Hsla, IconName) {
    match status {
        TabStatus::Connecting => (p.muted_foreground, IconName::Loader),
        TabStatus::Connected => (p.success, IconName::Terminal),
        TabStatus::Failed(_) | TabStatus::Disconnected => (p.danger, IconName::CircleAlert),
    }
}
