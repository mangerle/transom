//! AI 面板终端上下文挂载、解释与 `@主机` 协同匹配逻辑。

use gpui_kit::*;
use transom_ai_contracts::extract_host_mentions;
use transom_core::TabId;

use super::panel_events::mention_token;
use super::state::{ActiveAiSessionInfo, AiDrawerTab};
use super::view::AiAssistantView;

impl AiAssistantView {
    /// 供终端右键菜单调用：直接请求解释最近的终端命令块或屏幕输出。
    pub(crate) fn explain_terminal_output(&mut self, snippet: String, cx: &mut Context<Self>) {
        self.state.view.drawer = AiDrawerTab::Chat;
        let trimmed = snippet.trim();
        let prompt = if trimmed.is_empty() {
            rust_i18n::t!("ai.explain_prompt_empty").to_string()
        } else {
            rust_i18n::t!("ai.explain_prompt_with_output", output = trimmed).to_string()
        };
        self.send_prompt_text(prompt, cx);
    }

    /// 供终端右键菜单调用：将选区或失败命令块作为紧凑上下文芯片挂载到 AI 输入区。
    pub(crate) fn attach_terminal_context(
        &mut self,
        host: String,
        cwd: Option<String>,
        exit_code: Option<i32>,
        command: Option<String>,
        output: String,
        cx: &mut Context<Self>,
    ) {
        self.state.view.drawer = AiDrawerTab::Chat;
        self.state.composer.pending_attachments.push(
            super::attachments::ComposerAttachment::TerminalContext {
                id: uuid::Uuid::new_v4().to_string(),
                host,
                cwd,
                exit_code,
                command,
                output,
            },
        );
        cx.notify();
    }
}

/// 根据 `@token`（与补全菜单同源的 `mention_token` 规范化名）匹配应纳入协同的已连接会话。
pub(super) fn collect_mentioned_scope_tabs(
    prompt: &str,
    connected_sessions: &[ActiveAiSessionInfo],
    primary_tab: Option<TabId>,
) -> Vec<TabId> {
    let candidates: Vec<(String, TabId)> = connected_sessions
        .iter()
        .filter(|s| Some(s.tab_id) != primary_tab)
        .filter_map(|s| mention_token(&s.host_name).map(|tok| (tok, s.tab_id)))
        .collect();
    let available_tokens: Vec<&str> = candidates.iter().map(|(tok, _)| tok.as_str()).collect();
    let mut matched = Vec::new();
    for mentioned in extract_host_mentions(prompt, &available_tokens) {
        for (tok, tid) in &candidates {
            if tok == &mentioned && !matched.contains(tid) {
                matched.push(*tid);
            }
        }
    }
    matched
}

#[cfg(test)]
mod tests {
    use transom_core::{HostProtection, SessionId, TabId};

    use super::collect_mentioned_scope_tabs;
    use crate::app::ai::state::ActiveAiSessionInfo;

    #[test]
    fn matches_host_mentions_with_spaces_via_normalized_token() {
        let primary = ActiveAiSessionInfo {
            tab_id: TabId::new(),
            session_id: SessionId::new(),
            host_name: "Primary Host".to_owned(),
            login: "root@10.0.0.1:22".to_owned(),
            platform: "linux".to_owned(),
            protection: HostProtection::Normal,
            connected: true,
        };
        let spaced = ActiveAiSessionInfo {
            tab_id: TabId::new(),
            session_id: SessionId::new(),
            host_name: "My Server".to_owned(),
            login: "root@10.0.0.2:22".to_owned(),
            platform: "linux".to_owned(),
            protection: HostProtection::Normal,
            connected: true,
        };
        let tabs = collect_mentioned_scope_tabs(
            "check @My-Server disk usage",
            &[primary.clone(), spaced.clone()],
            Some(primary.tab_id),
        );
        assert_eq!(tabs, vec![spaced.tab_id]);
    }
}
