//! 窗口顶栏（Top Bar）渲染模块。

use crate::app::{AppShell, AppState};
use crate::brand;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::theme::tokens::{control, size, space, typography};
use gpui_kit::base::h_flex;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::TitleBar;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use transom_ui_icons::{AppIconElement, UiIcon};

/// 顶栏品牌标志边长（rem）：随密度档等比缩放。
///
/// 取 1.5rem（标准档 24px）而不是图标的 1.125rem：品牌标志是实心色块，
/// 按线性图标的尺寸给会显得比旁边的粗体应用名还弱；但顶栏只有 2.25rem（36px）高，
/// 再大就会贴上下边框，24px 留 6px 上下呼吸位。
const LOGO_SIZE: f32 = 1.5;

pub(crate) fn render_top_bar(
    app_state: &mut AppState,
    window: &mut Window,
    cx: &mut Context<AppShell>,
    p: &Palette,
) -> AnyElement {
    let app_entity = cx.entity().clone();

    let title_bar_border = cx.theme().title_bar_border;
    TitleBar::new()
        .h(rems(size::TOP_BAR_HEIGHT))
        .bg(p.title_bar)
        .border_color(p.divider(title_bar_border))
        .divider_edge(DividerEdge::Bottom, p.divider_width)
        .pl_2()
        .on_close_window(move |_, window, cx| {
            app_entity.update(cx, |this, cx| {
                crate::window_state::close_app(this, window, cx);
            });
        })
        .child(
            h_flex()
                .w_full()
                .h_full()
                .items_center()
                .pl_0()
                .pr_2()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(brand::logo_mark(LOGO_SIZE, window, cx))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .child(rust_i18n::t!("app.name").to_string()),
                        )
                        .child(render_version_badge(app_state, window, p, cx)),
                ),
        )
        .into_any_element()
}

/// 标题栏版本与更新入口。
///
/// 设计原理（Why）：当前版本和更新状态属于窗口身份信息，放在应用名旁边比
/// 同时占用左下活动栏与右下状态栏更直接；两个入口合并后也只有一个点击目标。
/// 有更新时徽标使用强调底色并附带标准更新图标，平时保持低干扰的纯版本文案。
fn render_version_badge(
    app_state: &AppState,
    window: &Window,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let update = app_state.features.update.clone();
    let (has_update, latest_version) = {
        let view = update.read(cx);
        (view.has_update(), view.available_version())
    };
    let label = latest_version.as_ref().map_or_else(
        || format!("v{}", crate::APP_VERSION),
        |latest| {
            rust_i18n::t!(
                "app.version_update_available",
                current = crate::APP_VERSION,
                latest = latest.as_str()
            )
            .to_string()
        },
    );
    let tooltip = if let Some(latest) = latest_version.as_ref() {
        rust_i18n::t!("app.update_available", version = latest.as_str()).to_string()
    } else {
        rust_i18n::t!("settings.update_check_action").to_string()
    };
    let accessibility_label = tooltip.clone();
    let update_view = update.clone();

    let mut content = h_flex().items_center().gap(rems(space::XS));
    if has_update {
        content = content.child(AppIconElement::new(UiIcon::UPDATE).with_size(
            crate::theme::tokens::rem_px(control::ICON, window.rem_size()),
        ));
    }
    content = content.child(
        div()
            .text_size(rems(typography::CAPTION))
            .font_weight(FontWeight::MEDIUM)
            .text_color(if has_update {
                p.foreground
            } else {
                p.muted_foreground
            })
            .child(label),
    );

    let mut button = Button::new("titlebar-version")
        .ghost()
        .h(rems(1.5))
        .px(rems(space::SM))
        .rounded(p.radius.md)
        .tooltip(tooltip)
        .accessibility_label(accessibility_label)
        .on_click(move |_, _, cx| {
            update_view.update(cx, |view, cx| view.open_and_check(cx));
        })
        .child(content);
    if has_update {
        button = button.bg(p.tone.primary_subtle);
    }
    button.into_any_element()
}
