//! 双标签并排视图渲染：两个标签组共享内容区，中间是可拖拽分隔线。
//!
//! 每个格都调用 `render_group_content`，因此标签组内部的窗格分屏、最大化与
//! 拖拽比例在并排视图里原样生效；这里只负责外层的两格排列、格标题与焦点。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::theme::tokens::size;
use crate::tiling::{TileSide, TiledTabs};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::resizable::{h_resizable, resizable_panel, v_resizable};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{Connection, SplitAxis};

impl AppShell {
    /// 渲染并排的两格；绑定指向的标签组缺失时返回 `None`，由调用方退回单标签。
    pub(super) fn render_tiled_content(
        &mut self,
        tiled: TiledTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> Option<AnyElement> {
        let first = self
            .groups
            .iter()
            .position(|group| group.id == tiled.first)?;
        let second = self
            .groups
            .iter()
            .position(|group| group.id == tiled.second)?;
        if first == second {
            return None;
        }

        let first_el = self.render_tiled_zone(tiled, TileSide::First, first, window, cx, p);
        let second_el = self.render_tiled_zone(tiled, TileSide::Second, second, window, cx, p);
        // 分隔线状态由外壳持有：切换方向或短暂退回单标签后再回来，比例不丢。
        let state = self.tiling_split_state.clone();
        let split = match tiled.axis {
            SplitAxis::Horizontal => h_resizable(tiling_element_id()),
            SplitAxis::Vertical => v_resizable(tiling_element_id()),
        }
        .with_state(&state)
        .child(resizable_panel().child(first_el))
        .child(resizable_panel().child(second_el));
        Some(
            div()
                .flex_1()
                .size_full()
                .overflow_hidden()
                .child(split)
                .into_any_element(),
        )
    }

    /// 渲染并排中的一格：细条标题（状态、序号、标题、关闭）加标签组内容。
    fn render_tiled_zone(
        &mut self,
        tiled: TiledTabs,
        side: TileSide,
        group_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let group_id = tiled.group(side);
        let focused = tiled.focused == side;
        let (pane_id, serial) = {
            let group = &self.groups[group_index];
            (group.active_pane, group.serial)
        };
        let (title, dot_color, tab_icon) = match self.tabs.iter().find(|tab| tab.id == pane_id) {
            Some(tab) => {
                let (dot_color, _) = super::pane_status_indicator(&tab.status, p);
                let mut title = tab.display_name();
                if serial > 0 {
                    title = format!("{title} #{serial}");
                }
                let tab_icon = match &tab.session.connection {
                    Connection::Local(_) => IconName::Terminal,
                    Connection::Serial(_) => IconName::Cable,
                    Connection::Telnet(_) => IconName::Network,
                    _ => IconName::Server,
                };
                (title, dot_color, tab_icon)
            }
            None => (String::new(), p.muted_foreground, IconName::Terminal),
        };

        let header = h_flex()
            .id(ElementId::Name(
                format!("tiled-zone-header-{group_id}").into(),
            ))
            .h(rems(size::PANE_HEADER_HEIGHT))
            .flex_none()
            .px_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .when(focused, |style| style.border_b_2())
            .border_color(if focused { p.primary } else { p.border })
            .bg(if focused { p.tab_active } else { p.tab_bar })
            .text_color(if focused {
                p.tab_active_foreground
            } else {
                p.tab_foreground
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate_group(group_id, window, cx);
            }))
            .child(div().size(px(7.0)).rounded_full().flex_none().bg(dot_color))
            .child(Icon::new(tab_icon).xsmall())
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
                    .id(ElementId::Name(
                        format!("tiled-zone-close-{group_id}").into(),
                    ))
                    .size(px(18.0))
                    .rounded(p.radius.sm)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|style| {
                        style
                            .bg(p.muted_foreground.opacity(0.2))
                            .text_color(p.foreground)
                    })
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_group(group_id, window, cx);
                    }))
                    .child(Icon::new(IconName::X).xsmall()),
            );

        let body = self.render_group_content(group_index, window, cx, p);
        v_flex()
            .size_full()
            .overflow_hidden()
            .child(header)
            // 内容必须是这个列向容器的直接子节点：单窗格与分割树都靠 `flex_1`
            // 吃掉标题栏之外的高度，若再套一层横向容器，终端画布会量到零高度。
            .child(body)
            // 点格内任意位置接管焦点；终端自身会消费事件，冒泡到这里至少覆盖
            // 标题与空白区，单击终端时也依赖冒泡把焦点格切过来。
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.activate_group(group_id, window, cx);
                }),
            )
            .into_any_element()
    }
}

/// 并排分隔线的稳定元素标识：同一窗口只有一条，切换方向时复用同一状态。
fn tiling_element_id() -> ElementId {
    ElementId::Name("tab-tiling-split".into())
}
