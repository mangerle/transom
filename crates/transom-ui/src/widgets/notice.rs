//! 内联提示条：成功、错误、警告与信息四种语义。
//!
//! 设计原理（Why）：设置/密钥/身份等页面各自拼"底色 + 图标 + 文案"的提示条，
//! 危险色、内边距与圆角有的取令牌、有的取裸值，同一条错误在不同页面看起来
//! 像两种东西。组件只负责语义与排版，触发时机仍由调用方决定。
//!
//! 代价与局限：不包含标题/详情两段式布局；监控 OOM 告警这类结构化提示仍由
//! 页面自行组装，避免为单一形态过度抽象。

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;

/// 提示条语义。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoticeKind {
    Success,
    Error,
    Warning,
}

impl NoticeKind {
    fn icon(self) -> IconName {
        match self {
            Self::Success => IconName::CircleCheck,
            Self::Error => IconName::CircleX,
            Self::Warning => IconName::TriangleAlert,
        }
    }
}

/// 渲染一条内联提示。
pub(crate) fn notice(
    kind: NoticeKind,
    message: impl Into<SharedString>,
    p: &Palette,
) -> AnyElement {
    let (accent, background, text) = match kind {
        NoticeKind::Success => (p.success, p.muted, p.foreground),
        NoticeKind::Error => (p.danger, p.danger.opacity(0.12), p.danger),
        NoticeKind::Warning => (p.warning, p.warning.opacity(0.12), p.foreground),
    };
    h_flex()
        .w_full()
        .gap(rems(space::SM))
        .items_center()
        .px(rems(space::MD))
        .py(rems(space::SM))
        .rounded(p.radius.md)
        .bg(background)
        .child(Icon::new(kind.icon()).small().text_color(accent))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(rems(typography::CAPTION))
                .text_color(text)
                .child(message.into()),
        )
        .into_any_element()
}
