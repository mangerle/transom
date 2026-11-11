//! 环境变量卡片：前 12 条键值对与一键复制全部为 `.env`。

use super::*;
use crate::container::ContainerWorkspace;
use gpui_kit::ClipboardItem;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 环境变量列表（支持一键复制全部为 .env）。
///
/// 复制结果写入工作区实体的 `notice`，由外壳渲染帧统一转成通知，渲染函数
/// 不再直接调用外壳的 toast 方法。
pub(crate) fn render_env_card(
    view: &Entity<ContainerWorkspace>,
    env: &[String],
    p: &Palette,
) -> AnyElement {
    if env.is_empty() {
        return div()
            .text_size(rems(0.75))
            .text_color(p.muted_foreground)
            .child("—")
            .into_any_element();
    }

    let all_env_text = env.join("\n");
    let copy_all_btn = Button::new("btn-copy-all-env")
        .ghost()
        .xsmall()
        .child(AppIconElement::new(AppIcon::Ui(UiIcon::COPY)).small())
        .label(rust_i18n::t!("container.detail.copy_env").to_string())
        .on_click({
            let view = view.clone();
            let copy_env_text = all_env_text.clone();
            move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy_env_text.clone()));
                let message = rust_i18n::t!("container.detail.env_copied").to_string();
                view.update(cx, |workspace, cx| {
                    workspace.state.notice = Some(message);
                    cx.notify();
                });
            }
        });

    let items = env.iter().take(12).map(|entry| {
        let (k, v) = entry.split_once('=').unwrap_or((entry.as_str(), ""));
        h_flex()
            .gap_1()
            .items_center()
            .text_size(rems(0.6875))
            .font_family(p.mono_font_family.clone())
            .truncate()
            .child(div().text_color(p.muted_foreground).child(format!("{k}=")))
            .child(div().text_color(p.foreground).child(v.to_string()))
            .into_any_element()
    });

    let count_hint = if env.len() > 12 {
        let remaining = env.len() - 12;
        Some(
            div()
                .text_size(rems(0.6875))
                .text_color(p.muted_foreground)
                .child(
                    rust_i18n::t!("container.detail.env_more_hint", count = remaining).to_string(),
                )
                .into_any_element(),
        )
    } else {
        None
    };

    let content = v_flex()
        .gap_1()
        .p_2()
        .rounded(p.radius.sm)
        .bg(p.background)
        .border_1()
        .border_color(p.stroke.muted)
        .children(items)
        .children(count_hint);

    overview_card(
        rust_i18n::t!("container.field.env").to_string(),
        AppIcon::Ui(UiIcon::DIFF),
        p,
        Some(copy_all_btn.into_any_element()),
        content,
    )
}
