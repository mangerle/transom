//! 窗口顶栏（Top Bar）渲染模块。

use crate::app::AppShell;
use crate::brand;
use crate::palette::Palette;
use crate::theme::tokens::size;
use gpui_kit::base::h_flex;
use gpui_kit::component::TitleBar;
use gpui_kit::*;

/// 顶栏品牌标志边长（rem）：随密度档等比缩放。
///
/// 取 1.5rem（标准档 24px）而不是图标的 1.125rem：品牌标志是实心色块，
/// 按线性图标的尺寸给会显得比旁边的粗体应用名还弱；但顶栏只有 2.25rem（36px）高，
/// 再大就会贴上下边框，24px 留 6px 上下呼吸位。
const LOGO_SIZE: f32 = 1.5;

impl AppShell {
    pub(crate) fn render_top_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let app_entity = cx.entity().clone();

        TitleBar::new()
            .h(rems(size::TOP_BAR_HEIGHT))
            .bg(p.title_bar)
            .on_close_window(move |_, window, cx| {
                app_entity.update(cx, |this, cx| {
                    this.close_app(window, cx);
                });
            })
            .child(
                h_flex().w_full().h_full().items_center().px_3().child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(brand::logo_mark(LOGO_SIZE, window, cx))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .child(rust_i18n::t!("app.name").to_string()),
                        ),
                ),
            )
            .into_any_element()
    }
}
