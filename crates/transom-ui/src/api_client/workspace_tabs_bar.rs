//! API 工作区共用的下划线页签栏，视觉与顶部主标签栏保持一致。

use crate::palette::Palette;
use crate::theme::tokens::{size, space, typography};
use gpui_kit::base::{Tab, Tabs, h_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

/// 单个页签项的数据与点击回调。
pub(super) struct UnderlineTabItem {
    pub(super) id: &'static str,
    pub(super) label: String,
    pub(super) selected: bool,
    pub(super) on_select: Box<dyn Fn(&mut Window, &mut App) + 'static>,
}

/// 渲染与主标签栏一致的底部蓝线页签栏。
pub(super) fn render_underline_tabs(
    id: &'static str,
    items: Vec<UnderlineTabItem>,
    suffix: Option<AnyElement>,
    p: &Palette,
) -> AnyElement {
    let count = items.len();
    let tabs = items.into_iter().enumerate().map(|(index, item)| {
        let on_select = item.on_select;
        let selected = item.selected;
        let label = item.label;
        let content = h_flex()
            .relative()
            .h_full()
            .px(rems(space::SM))
            .items_center()
            .text_size(rems(typography::BODY))
            .text_color(if selected {
                p.tab_active_foreground
            } else {
                p.muted_foreground
            })
            .when(!selected, |el| el.hover(|style| style.text_color(p.foreground)))
            .child(label.clone())
            .when(selected, |el| {
                el.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(px(2.0))
                        .bg(p.primary),
                )
            });

        Tab::new(item.id)
            .h_full()
            .flex_none()
            .cursor_pointer()
            .selected(selected)
            .accessibility_label(label)
            .set_position(index, count)
            .on_click(move |_, window, cx| on_select(window, cx))
            .child(content)
    });

    h_flex()
        .relative()
        .h(rems(size::PANE_HEADER_HEIGHT))
        .w_full()
        .px(rems(space::SM))
        .items_center()
        .child(
            div()
                .absolute()
                .bottom_0()
                .left_0()
                .right_0()
                .h(px(1.0))
                .bg(p.divider(p.border)),
        )
        .child(
            Tabs::new(id)
                .h_full()
                .flex_none()
                .flex()
                .items_center()
                .children(tabs),
        )
        .child(div().flex_1())
        .when_some(suffix, |row, suffix| row.child(suffix))
        .into_any_element()
}
