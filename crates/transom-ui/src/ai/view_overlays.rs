//! AI 面板辅助横幅与遮罩弹窗渲染（后台运行横幅、图片大图预览、抽屉审批提醒）。

use std::sync::Arc;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::state::AiDrawerTab;
use super::view::AiAssistantView;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

impl AiAssistantView {
    /// 渲染当前活动的审批确认卡片或悬浮弹层/二选一卡片。
    pub(super) fn render_chat_overlays(
        &self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> (Option<AnyElement>, Option<AnyElement>) {
        let current_run_id = self.state.current_run_id();
        let approval_el = self
            .state
            .run
            .pending_approvals
            .iter()
            .find(|approval| {
                approval.owner_run_id == "mcp-external"
                    || current_run_id == Some(approval.owner_run_id.as_str())
            })
            .map(|approval| {
                let req_id = approval.request_id.clone();
                let view_d = cx.entity();
                div()
                    .px(rems(space::SM))
                    .pb(rems(space::XS))
                    .child(super::approval::render_approval_card(
                        approval,
                        &self.approval_feedback_input,
                        p,
                        move |decision, w, cx| {
                            let id = req_id.clone();
                            view_d
                                .update(cx, |this, cx| this.respond_approval(&id, decision, w, cx));
                        },
                        cx,
                    ))
                    .into_any_element()
            });
        let popover_el = self
            .render_slash_floating_popover(p, cx)
            .or_else(|| self.render_mention_floating_popover(p, cx))
            .or_else(|| self.render_target_session_popover(p, cx))
            .or_else(|| self.maybe_render_choice_card(p, cx))
            .map(|el| {
                div()
                    .px(rems(space::SM))
                    .pb(rems(space::XS))
                    .child(el)
                    .into_any_element()
            });
        (approval_el, popover_el)
    }

    /// 渲染跨会话后台运行横幅。
    pub(super) fn render_background_run_banner(
        &self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let bg_conv_id = self
            .state
            .background_run_conversation_id()
            .map(str::to_owned);
        let v_stop = cx.entity();
        let mut actions = h_flex().gap(px(4.0)).flex_shrink_0();
        if let Some(cid) = bg_conv_id {
            let v_jump = cx.entity();
            actions = actions.child(
                Button::new("ai-bg-run-jump")
                    .outline()
                    .xsmall()
                    .label(rust_i18n::t!("ai.bg_run_switch").to_string())
                    .on_click(move |_, _, cx| {
                        v_jump.update(cx, |this, cx| this.select_conversation(&cid, cx));
                    }),
            );
        }
        actions = actions.child(
            Button::new("ai-bg-run-stop")
                .ghost()
                .xsmall()
                .label(rust_i18n::t!("ai.bg_run_stop").to_string())
                .on_click(move |_, _, cx| {
                    v_stop.update(cx, |this, cx| this.cancel_background_run(cx));
                }),
        );
        self.render_background_run_banner_box(actions.into_any_element(), p)
    }

    /// 渲染跨会话后台运行横幅容器。
    pub(super) fn render_background_run_banner_box(
        &self,
        actions: AnyElement,
        p: &Palette,
    ) -> AnyElement {
        div()
            .px(rems(space::SM))
            .pb(rems(space::XS))
            .child(
                h_flex()
                    .w_full()
                    .px(rems(space::SM))
                    .py(rems(space::XS))
                    .gap(rems(space::XS))
                    .justify_between()
                    .items_center()
                    .rounded(p.radius.md)
                    .bg(p.surface.floating)
                    .border_1()
                    .border_color(p.warning.opacity(0.45))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(6.0))
                            .items_center()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_color(p.warning)
                                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::STOP)).small()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rems(typography::CAPTION))
                                    .text_color(p.foreground)
                                    .child(rust_i18n::t!("ai.bg_run_banner_title").to_string()),
                            ),
                    )
                    .child(actions),
            )
            .into_any_element()
    }

    /// 解码并在当前面板弹出图片大图预览遮罩。
    pub(super) fn open_image_preview(
        &mut self,
        title: String,
        base64: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(decoded) = super::composer::image::decode_base64_render_image(&base64, 1568) {
            self.preview_image = Some((title, decoded));
            cx.notify();
        }
    }

    pub(super) fn render_image_preview_modal(
        &self,
        title: &str,
        image: &Arc<RenderImage>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let v_close_bg = cx.entity();
        let v_close_btn = cx.entity();
        div()
            .id("ai-image-preview-backdrop")
            .absolute()
            .inset_0()
            .bg(gpui_kit::black().opacity(0.72))
            .flex()
            .items_center()
            .justify_center()
            .p(rems(space::LG))
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, _, cx| {
                v_close_bg.update(cx, |this, cx| {
                    this.preview_image = None;
                    cx.notify();
                });
            })
            .child(
                v_flex()
                    .id("ai-image-preview-dialog")
                    .max_w(relative(0.94))
                    .max_h(relative(0.9))
                    .rounded(p.radius.lg)
                    .bg(p.surface.floating)
                    .border_1()
                    .border_color(p.border)
                    .shadow_lg()
                    .overflow_hidden()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .w_full()
                            .px(rems(space::SM))
                            .py(rems(space::XS))
                            .justify_between()
                            .items_center()
                            .border_b_1()
                            .border_color(p.divider(p.border))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(rems(typography::BODY))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.foreground)
                                    .child(title.to_owned()),
                            )
                            .child(
                                Button::new("ai-image-preview-close")
                                    .ghost()
                                    .xsmall()
                                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::CLOSE)).small())
                                    .on_click(move |_, _, cx| {
                                        v_close_btn.update(cx, |this, cx| {
                                            this.preview_image = None;
                                            cx.notify();
                                        });
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .p(rems(space::SM))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                img(ImageSource::Render(image.clone()))
                                    .max_w_full()
                                    .max_h_full()
                                    .object_fit(ObjectFit::Contain),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_drawer_approval_banner(
        &self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let count = self.state.run.pending_approvals.len();
        let view = cx.entity();
        h_flex()
            .w_full()
            .px(rems(space::SM))
            .py(rems(space::XS))
            .gap(rems(space::XS))
            .justify_between()
            .items_center()
            .bg(p.warning.opacity(0.12))
            .border_b_1()
            .border_color(p.warning.opacity(0.4))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(6.0))
                    .items_center()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(p.warning)
                            .child(AppIconElement::new(AppIcon::Ui(UiIcon::LOCKED)).small()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(rems(typography::CAPTION))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.foreground)
                            .child(
                                rust_i18n::t!("ai.pending_approval_banner", count = count)
                                    .to_string(),
                            ),
                    ),
            )
            .child(
                Button::new("ai-drawer-review-approval")
                    .primary()
                    .xsmall()
                    .label(rust_i18n::t!("ai.pending_approval_back").to_string())
                    .on_click(move |_, _, cx| {
                        view.update(cx, |this, cx| {
                            this.state.view.drawer = AiDrawerTab::Chat;
                            cx.notify();
                        });
                    }),
            )
            .into_any_element()
    }
}
