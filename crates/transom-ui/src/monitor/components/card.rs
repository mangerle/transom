//! 通用可折叠卡片组件（Collapsible Card）。

use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::collapsible::Collapsible;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;

/// 监控板块的可折叠卡片。
pub(crate) struct CollapsibleCard {
    id: ElementId,
    icon: IconName,
    title: String,
    collapsed: bool,
    summary: Option<AnyElement>,
    content: Option<AnyElement>,
}

impl CollapsibleCard {
    pub(crate) fn new(id: impl Into<ElementId>, icon: IconName, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            icon,
            title: title.into(),
            collapsed: false,
            summary: None,
            content: None,
        }
    }

    pub(crate) fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    pub(crate) fn summary(mut self, summary: impl IntoElement) -> Self {
        self.summary = Some(summary.into_any_element());
        self
    }

    pub(crate) fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }

    pub(crate) fn render<F>(self, p: &Palette, on_toggle: F) -> AnyElement
    where
        F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    {
        let arrow_icon = if self.collapsed {
            IconName::ChevronRight
        } else {
            IconName::ChevronDown
        };

        let mut card = v_flex()
            .id(self.id.clone())
            .w_full()
            .overflow_hidden()
            .rounded_md()
            .border_1()
            .border_color(p.border.opacity(0.65))
            .bg(p.muted.opacity(0.35));

        // 标题栏：点击整行可切换折叠/展开
        let mut header = h_flex()
            .id(format!("{}-header", self.id))
            .w_full()
            .h(px(32.0))
            .px_2p5()
            .justify_between()
            .items_center()
            .rounded_t_md()
            .cursor_pointer()
            .hover(|s| s.bg(p.muted.opacity(0.4)))
            .on_click(on_toggle)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .min_w_0()
                    .flex_shrink_1()
                    .child(Icon::new(self.icon).small().text_color(p.primary))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.foreground)
                            .truncate()
                            .child(self.title),
                    ),
            );

        let mut right_side = h_flex().gap_2().items_center().flex_shrink_0();
        if self.collapsed
            && let Some(summary) = self.summary
        {
            right_side = right_side.child(summary);
        }
        right_side = right_side.child(
            Icon::new(arrow_icon)
                .xsmall()
                .text_color(p.muted_foreground),
        );

        header = header.child(right_side);
        card = card.child(header);

        if let Some(content) = self.content {
            let content_box = div()
                .w_full()
                .overflow_hidden()
                .p_2p5()
                .border_t_1()
                .border_color(p.border.opacity(0.4))
                .child(content);
            card = card.child(
                Collapsible::new()
                    .open(!self.collapsed)
                    .content(content_box),
            );
        }

        card.into_any_element()
    }
}
