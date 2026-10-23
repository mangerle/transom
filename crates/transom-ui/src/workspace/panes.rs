//! 标签内分屏窗格渲染。

use crate::app::{AppShell, TabStatus};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::SplitAxis;

impl AppShell {
    pub(super) fn render_active_content(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(group_index) = self.active_group_index() else {
            return self.render_empty_state(p, cx);
        };
        let pane_ids = self.groups[group_index].panes.clone();
        let axis = self.groups[group_index].axis;
        let active_pane = self.groups[group_index].active_pane;
        if pane_ids.len() == 1 {
            let Some(index) = self.tabs.iter().position(|tab| tab.id == pane_ids[0]) else {
                return self.render_empty_state(p, cx);
            };
            return self.render_pane_body(index, window, cx, p);
        }

        let mut layout = match axis {
            SplitAxis::Horizontal => h_flex(),
            SplitAxis::Vertical => v_flex(),
        }
        .flex_1()
        .size_full()
        .overflow_hidden();
        for (position, pane_id) in pane_ids.into_iter().enumerate() {
            if position > 0 {
                layout = layout.child(match axis {
                    SplitAxis::Horizontal => {
                        div().w(px(1.0)).h_full().bg(p.border).into_any_element()
                    }
                    SplitAxis::Vertical => {
                        div().h(px(1.0)).w_full().bg(p.border).into_any_element()
                    }
                });
            }
            let Some(index) = self.tabs.iter().position(|tab| tab.id == pane_id) else {
                continue;
            };
            let pane = self.render_split_pane(index, pane_id == active_pane, window, cx, p);
            layout = layout.child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.activate_tab(pane_id, window, cx);
                        }),
                    )
                    .child(pane),
            );
        }
        div()
            .flex_1()
            .size_full()
            .overflow_hidden()
            .child(layout)
            .into_any_element()
    }

    fn render_pane_body(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        if let Some(terminal) = self.tabs[index].terminal.clone() {
            div()
                .flex_1()
                .overflow_hidden()
                .child(terminal)
                .into_any_element()
        } else {
            self.render_tab_status(index, window, cx, p)
        }
    }

    fn render_split_pane(
        &mut self,
        index: usize,
        active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let tab_id = self.tabs[index].id;
        let title = self.tabs[index].title.clone();
        let (dot_color, icon_name) = match &self.tabs[index].status {
            TabStatus::Connecting => (p.muted_foreground, IconName::Loader),
            TabStatus::Connected => (p.success, IconName::Terminal),
            TabStatus::Failed(_) | TabStatus::Disconnected => (p.danger, IconName::CircleAlert),
        };
        let header = h_flex()
            .id(ElementId::Name(format!("pane-header-{tab_id}").into()))
            .h(px(28.0))
            .flex_none()
            .px_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(p.border)
            .bg(if active { p.tab_active } else { p.tab_bar })
            .text_color(if active {
                p.tab_active_foreground
            } else {
                p.tab_foreground
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate_tab(tab_id, window, cx);
            }))
            .child(div().size(px(7.0)).rounded_full().flex_none().bg(dot_color))
            .child(Icon::new(icon_name).xsmall())
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .truncate()
                    .child(title),
            )
            .child(
                div()
                    .id(ElementId::Name(format!("close-pane-{tab_id}").into()))
                    .size(px(18.0))
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| {
                        s.bg(p.muted_foreground.opacity(0.2))
                            .text_color(p.foreground)
                    })
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_pane(tab_id, window, cx);
                    }))
                    .child(Icon::new(IconName::X).xsmall()),
            );
        let body = self.render_pane_body(index, window, cx, p);
        v_flex()
            .size_full()
            .overflow_hidden()
            .child(header)
            .child(body)
            .into_any_element()
    }
}
