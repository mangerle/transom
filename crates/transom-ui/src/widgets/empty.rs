//! 统一空状态：图标、标题、说明与可选动作。
//!
//! 主工作区已经使用组件库 `Empty`；侧栏、SFTP 等页面此前各自拼一套。
//! 这里把结构收敛到一处，保证空状态的留白、图标尺寸与文字层级一致。

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};
use gpui_kit::component::empty::{Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle};
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;

/// 渲染一个标准空状态。
pub(crate) fn empty_state(
    icon: AppIcon,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    p: &Palette,
) -> AnyElement {
    Empty::new()
        .border_0()
        .header(
            EmptyHeader::new()
                .media(
                    EmptyMedia::new().child(
                        AppIconElement::new(icon)
                            .size(rems(space::XL))
                            .text_color(p.muted_foreground),
                    ),
                )
                .title(
                    EmptyTitle::new().child(
                        div()
                            .text_size(rems(typography::TITLE))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title.into()),
                    ),
                )
                .description(
                    EmptyDescription::new().child(
                        div()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.muted_foreground)
                            .child(description.into()),
                    ),
                ),
        )
        .into_any_element()
}
