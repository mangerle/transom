//! 窗格与并排标签格共用的标题条。
//!
//! 设计原理（Why）：标签组内的窗格头与并排网格的格头此前各写一份布局，
//! 高度、状态点、图标尺寸、焦点配色与关闭按钮的 hover 反馈必须手工保持一致，
//! 改一处漏一处就出现两种"看起来像但不是"的标题条。这里只收敛视觉与关闭
//! 按钮的通用交互；激活语义（切窗格 / 切标签组）与右键菜单由调用方注入。

use crate::palette::Palette;
use crate::theme::tokens::size;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

/// 标题条的渲染输入。
pub(super) struct PaneHeaderProps<'a> {
    /// 容器元素标识，必须跨帧稳定。
    pub(super) id: String,
    /// 关闭按钮的元素标识，与容器分开以免嵌套点击互相干扰。
    pub(super) close_id: String,
    /// 是否为当前焦点所在格。
    pub(super) active: bool,
    pub(super) dot_color: Hsla,
    pub(super) icon: IconName,
    pub(super) title: String,
    /// 组内窗格序号；并排格没有序号，传 `None`。
    pub(super) ordinal: Option<usize>,
    pub(super) palette: &'a Palette,
}

/// 构造标题条；点击容器与点击关闭按钮的行为由调用方提供。
pub(super) fn pane_header<F, G>(
    props: PaneHeaderProps<'_>,
    on_activate: F,
    on_close: G,
) -> Stateful<Div>
where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    G: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let p = props.palette;
    let mut header = h_flex()
        .id(ElementId::Name(props.id.into()))
        .h(rems(size::PANE_HEADER_HEIGHT))
        .flex_none()
        .px_2()
        .gap_2()
        .items_center()
        .border_b_1()
        // 焦点格只靠背景色区分太弱（激活色与标签栏底色只差几个色阶），
        // 用同款强调下划线把"焦点在哪一格"显式画出来。
        .when(props.active, |style| style.border_b_2())
        .border_color(if props.active { p.primary } else { p.border })
        .bg(if props.active {
            p.tab_active
        } else {
            p.tab_bar
        })
        .text_color(if props.active {
            p.tab_active_foreground
        } else {
            p.tab_foreground
        })
        .cursor_pointer()
        .on_click(on_activate)
        .child(
            div()
                .size(px(7.0))
                .rounded_full()
                .flex_none()
                .bg(props.dot_color),
        )
        .child(Icon::new(props.icon).xsmall());
    if let Some(ordinal) = props.ordinal {
        header = header.child(
            div()
                .text_xs()
                .flex_none()
                .text_color(p.muted_foreground)
                .child(ordinal.to_string()),
        );
    }
    header
        .child(
            div()
                .flex_1()
                .text_xs()
                .overflow_hidden()
                .whitespace_nowrap()
                .truncate()
                .child(props.title),
        )
        .child(
            div()
                .id(ElementId::Name(props.close_id.into()))
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
                .on_click(on_close)
                .child(Icon::new(IconName::X).xsmall()),
        )
}
