//! AI 面板浮层菜单装配辅助方法。

use gpui_kit::*;
use transom_ai_contracts::{AcpAgentConfig, default_acp_presets};

use super::metrics::{
    render_context_usage_popup, render_session_stats_popup, render_token_usage_popup,
};
use super::popups::{
    ModelPopupParams, render_backend_popup, render_mode_popup, render_model_popup,
};
use super::state::{AiActivePopup, AiDrawerTab};
use super::view::AiAssistantView;
use crate::palette::Palette;

impl AiAssistantView {
    pub(super) fn render_toolbar_popover(
        &self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<(AiActivePopup, AnyElement)> {
        if !self.current_slash_items(cx).is_empty() {
            return None;
        }
        let view = cx.entity();
        let kind = self.state.view.active_popup;
        let pop = match kind {
            AiActivePopup::None => return None,
            AiActivePopup::Backend => {
                let v_cfg = view.clone();
                Some(render_backend_popup(
                    &self.state.config.settings,
                    self.state.config.active_acp_agent_id.as_deref(),
                    p,
                    move |id, _, cx| {
                        view.update(cx, |this, cx| this.select_acp_agent(id, cx));
                    },
                    move |_, cx| {
                        v_cfg.update(cx, |this, cx| {
                            this.state.view.active_popup = AiActivePopup::None;
                            this.state.view.drawer = AiDrawerTab::Config;
                            cx.notify();
                        });
                    },
                ))
            }
            AiActivePopup::Mode => Some(render_mode_popup(
                self.state.config.execution_mode,
                p,
                move |mode, _, cx| {
                    view.update(cx, |this, cx| this.select_execution_mode(mode, cx));
                },
            )),
            AiActivePopup::Model => {
                let v_eff = view.clone();
                let v_cfg = view.clone();
                let active_acp = self.active_acp_agent();
                Some(render_model_popup(
                    ModelPopupParams {
                        settings: &self.state.config.settings,
                        active_acp: active_acp.as_ref(),
                        selected_model_id: &self.state.config.selected_model_id,
                        selected_effort: self.state.config.selected_effort,
                        p,
                    },
                    move |id, _, cx| {
                        view.update(cx, |this, cx| this.select_model(id, cx));
                    },
                    move |eff, _, cx| {
                        v_eff.update(cx, |this, cx| this.select_effort(eff, cx));
                    },
                    move |_, cx| {
                        v_cfg.update(cx, |this, cx| {
                            this.state.view.active_popup = AiActivePopup::None;
                            this.state.view.drawer = AiDrawerTab::Config;
                            cx.notify();
                        });
                    },
                ))
            }
            AiActivePopup::SessionStats => Some(render_session_stats_popup(
                &self.state.conversation.metrics,
                p,
            )),
            AiActivePopup::TokenUsage => Some(render_token_usage_popup(
                &self.state.conversation.metrics,
                p,
            )),
            AiActivePopup::ContextUsage => {
                let v_cmp = view;
                Some(render_context_usage_popup(
                    &self.state.conversation.metrics,
                    self.active_context_window(),
                    self.state.conversation.items.len(),
                    p,
                    move |_, cx| {
                        v_cmp.update(cx, |this, cx| this.compact_conversation(String::new(), cx));
                    },
                ))
            }
        }?;
        Some((kind, pop))
    }

    pub(super) fn active_acp_agent(&self) -> Option<AcpAgentConfig> {
        let id = self.state.config.active_acp_agent_id.as_deref()?;
        self.state
            .config
            .settings
            .acp_agents
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .or_else(|| default_acp_presets().into_iter().find(|a| a.id == id))
    }
}
