//! 侧边面板的统一标题栏。
//!
//! 设计原理（Why）：SFTP、转发与监控面板并排出现，标题栏高度、左内边距与
//! 底部横线必须对齐，否则三条横线在视觉上互相错位。此前三个面板各写一套
//! `h_flex`，高度分别取标签栏令牌与裸像素常量，密度切换后差异更
//! 明显。这里把结构收敛为“可选图标 + 标题 + 右侧动作”，尺寸全部取设计令牌。
//!
//! 代价与局限：编辑器面板的标题栏带拖拽、目录副标题与折叠/缩放按钮，结构
//! 更复杂，暂不进本组件；需要时在本组件上增量扩展，而不是再复制一份。

use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::theme::tokens::{size, space, typography};
use gpui_kit::base::h_flex;
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;

/// 面板标题栏参数。
pub(crate) struct PanelHeader {
    title: SharedString,
    icon: Option<AppIcon>,
    border: Hsla,
    actions: Vec<AnyElement>,
}

impl PanelHeader {
    /// 以标题与当前调色板创建标题栏；默认使用侧栏边框色。
    pub(crate) fn new(title: impl Into<SharedString>, p: &Palette) -> Self {
        Self {
            title: title.into(),
            icon: None,
            border: p.sidebar_border,
            actions: Vec::new(),
        }
    }

    /// 标题前的语义图标。
    pub(crate) fn icon(mut self, icon: AppIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// 覆盖底部分割线颜色（监控面板与主区共线，用普通边框色）。
    pub(crate) fn border(mut self, border: Hsla) -> Self {
        self.border = border;
        self
    }

    /// 追加一个右侧动作；调用顺序即显示顺序。
    pub(crate) fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    /// 渲染标题栏。
    pub(crate) fn render(self, p: &Palette) -> AnyElement {
        let title = self.title;
        let icon = self.icon;
        let mut left = h_flex()
            .gap(rems(space::SM))
            .items_center()
            .min_w_0()
            .flex_shrink_1();
        if let Some(icon) = icon {
            left = left.child(AppIconElement::new(icon).small().text_color(p.primary));
        }
        left = left.child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .truncate()
                .text_size(rems(typography::BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        );

        h_flex()
            .h(rems(size::TAB_BAR_HEIGHT))
            .flex_none()
            .w_full()
            .px(rems(space::MD))
            .items_center()
            .justify_between()
            .border_color(p.divider(self.border))
            .divider_edge(DividerEdge::Bottom, p.divider_width)
            .child(left)
            .child(
                h_flex()
                    .gap(rems(space::XS))
                    .items_center()
                    .flex_shrink_0()
                    .children(self.actions),
            )
            .into_any_element()
    }
}
