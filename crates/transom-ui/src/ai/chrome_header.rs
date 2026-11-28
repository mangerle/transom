//! AI 助手面板顶部标题栏、重命名编辑框与抽屉导航按钮组。

use gpui_kit::base::h_flex;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::Input;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::AiPanelEvent;
use super::empty_state::header_icon_btn;
use super::state::{AiActivePopup, AiDrawerTab};
use super::view::AiAssistantView;
use crate::palette::Palette;
use crate::theme::tokens::{size, space, typography};

impl AiAssistantView {
    pub(super) fn render_header(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let v_new = cx.entity();
        let v_hist = cx.entity();
        let v_mem = cx.entity();
        let v_skills = cx.entity();
        let v_cfg = cx.entity();
        let v_close = cx.entity();

        let title = self
            .state
            .conversation
            .active_conversation_title
            .as_deref()
            .filter(|t| !t.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| rust_i18n::t!("ai.title").to_string());

        let engine_icon =
            super::popups::backend_icon(self.state.config.active_acp_agent_id.as_deref());
        let has_active_conv = self.state.conversation.active_conversation_id.is_some();
        let v_title = cx.entity();
        let title_el: AnyElement = if self.editing_title {
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(&self.title_input).small())
                .into_any_element()
        } else if has_active_conv {
            div()
                .id("ai-conv-title")
                .flex_1()
                .min_w_0()
                .px(px(4.0))
                .py(px(2.0))
                .rounded(p.radius.sm)
                .cursor_pointer()
                .hover(|s| s.bg(p.surface.hover))
                .tooltip(|w, cx| {
                    Tooltip::new(rust_i18n::t!("ai.title_click_edit").to_string()).build(w, cx)
                })
                .on_click(move |_, w, cx| {
                    v_title.update(cx, |this, cx| this.start_editing_title(w, cx));
                })
                .child(
                    div()
                        .truncate()
                        .text_size(rems(typography::BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(title),
                )
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(rems(typography::BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.foreground)
                .child(title)
                .into_any_element()
        };

        h_flex()
            .w_full()
            .h(rems(size::TAB_BAR_HEIGHT))
            .px(rems(space::SM))
            .justify_between()
            .items_center()
            .gap(rems(space::XS))
            .border_b_1()
            .border_color(p.divider(p.border))
            .bg(p.background)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(6.0))
                    .items_center()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(p.primary)
                            .child(AppIconElement::new(engine_icon).small()),
                    )
                    .child(title_el),
            )
            .child(
                h_flex()
                    .gap(rems(space::XXS))
                    .flex_shrink_0()
                    .child(header_icon_btn(
                        "ai-new-chat",
                        AppIcon::Ui(UiIcon::ADD),
                        rust_i18n::t!("ai.new_chat").to_string(),
                        false,
                        p,
                        move |_, cx| v_new.update(cx, |this, cx| this.clear_conversation(cx)),
                    ))
                    .child(header_icon_btn(
                        "ai-hist-btn",
                        AppIcon::Ui(UiIcon::HISTORY),
                        rust_i18n::t!("ai.history_title").to_string(),
                        self.state.view.drawer == AiDrawerTab::History,
                        p,
                        move |_, cx| {
                            v_hist.update(cx, |this, cx| {
                                this.state.view.active_popup = AiActivePopup::None;
                                this.state.view.drawer =
                                    if this.state.view.drawer == AiDrawerTab::History {
                                        AiDrawerTab::Chat
                                    } else {
                                        AiDrawerTab::History
                                    };
                                cx.notify();
                            });
                        },
                    ))
                    .child(header_icon_btn(
                        "ai-mem-btn",
                        AppIcon::Ui(UiIcon::REAL_INTENTION_BULB),
                        rust_i18n::t!("ai.memory").to_string(),
                        self.state.view.drawer == AiDrawerTab::Memory,
                        p,
                        move |_, cx| {
                            v_mem.update(cx, |this, cx| {
                                this.state.view.active_popup = AiActivePopup::None;
                                this.state.view.drawer =
                                    if this.state.view.drawer == AiDrawerTab::Memory {
                                        AiDrawerTab::Chat
                                    } else {
                                        AiDrawerTab::Memory
                                    };
                                cx.notify();
                            });
                        },
                    ))
                    .child(header_icon_btn(
                        "ai-skills-btn",
                        AppIcon::Ui(UiIcon::LIGHTNING),
                        rust_i18n::t!("ai.skills_title").to_string(),
                        self.state.view.drawer == AiDrawerTab::Skills,
                        p,
                        move |_, cx| {
                            v_skills.update(cx, |this, cx| {
                                this.state.view.active_popup = AiActivePopup::None;
                                if this.state.view.drawer == AiDrawerTab::Skills {
                                    this.state.view.drawer = AiDrawerTab::Chat;
                                } else {
                                    let disabled =
                                        this.state.config.settings.disabled_skills.clone();
                                    this.skills_drawer.update(cx, |drawer, cx| {
                                        drawer.set_disabled_and_refresh(disabled, cx);
                                    });
                                    this.state.view.drawer = AiDrawerTab::Skills;
                                }
                                cx.notify();
                            });
                        },
                    ))
                    .child(header_icon_btn(
                        "ai-cfg-btn",
                        AppIcon::Ui(UiIcon::SETTINGS),
                        rust_i18n::t!("ai.model").to_string(),
                        self.state.view.drawer == AiDrawerTab::Config,
                        p,
                        move |_, cx| {
                            v_cfg.update(cx, |this, cx| {
                                this.state.view.active_popup = AiActivePopup::None;
                                this.state.view.drawer =
                                    if this.state.view.drawer == AiDrawerTab::Config {
                                        AiDrawerTab::Chat
                                    } else {
                                        AiDrawerTab::Config
                                    };
                                cx.notify();
                            });
                        },
                    ))
                    .child(header_icon_btn(
                        "ai-close-btn",
                        AppIcon::Ui(UiIcon::CLOSE),
                        rust_i18n::t!("ai.collapse").to_string(),
                        false,
                        p,
                        move |_, cx| {
                            v_close.update(cx, |_, cx| cx.emit(AiPanelEvent::CloseRequested));
                        },
                    )),
            )
            .into_any_element()
    }
}
