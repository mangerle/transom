//! AI 面板运行状态机与事件归属判断。
//!
//! 设计原理（Why）：运行身份必须与对话视图分离；流式展示事件只允许写入当前对话，
//! 运行终态则按运行 ID 清理，确保用户切换对话后旧运行不会污染当前消息流。

use std::time::Instant;

use transom_core::AiAppEvent;

use super::ActiveAiRun;
use super::AiPanelState;

impl AiPanelState {
    /// 当前是否存在活动运行。
    pub(crate) fn has_active_run(&self) -> bool {
        self.active_run.is_some()
    }

    /// 活动运行是否属于当前正在查看的对话。
    pub(crate) fn active_run_is_current(&self) -> bool {
        self.active_run.as_ref().is_some_and(|run| {
            run.conversation_id
                .as_deref()
                .is_none_or(|conversation_id| {
                    self.active_conversation_id.as_deref() == Some(conversation_id)
                })
        })
    }

    /// 当前对话中运行的主键。
    pub(crate) fn current_run_id(&self) -> Option<&str> {
        self.active_run
            .as_ref()
            .filter(|_| self.active_run_is_current())
            .map(|run| run.run_id.as_str())
    }

    /// 当前对话运行已耗时。
    pub(crate) fn current_run_started_at(&self) -> Option<Instant> {
        self.active_run
            .as_ref()
            .filter(|_| self.active_run_is_current())
            .map(|run| run.started_at)
    }

    /// 记录本地已成功投递的运行。
    pub(crate) fn begin_run(&mut self, run_id: String, conversation_id: Option<String>) {
        self.active_run = Some(ActiveAiRun {
            run_id,
            conversation_id,
            started_at: Instant::now(),
            cancel_requested: false,
        });
    }

    /// 将控制平面确认的会话 ID 绑定到本地运行。
    pub(crate) fn bind_run(&mut self, run_id: &str, conversation_id: &str) {
        if let Some(run) = &mut self.active_run
            && run.run_id == run_id
        {
            run.conversation_id = Some(conversation_id.to_owned());
            run.started_at = Instant::now();
            run.cancel_requested = false;
        }
    }

    /// 请求取消当前可见运行，但在收到 `RunFinished` 前保留运行身份。
    pub(crate) fn request_current_run_cancel(&mut self) -> Option<String> {
        let conversation = self.active_conversation_id.clone();
        let run = self.active_run.as_mut()?;
        if run
            .conversation_id
            .as_deref()
            .is_some_and(|id| conversation.as_deref() != Some(id))
        {
            return None;
        }
        run.cancel_requested = true;
        Some(run.run_id.clone())
    }

    /// 在收到对应终态时清除运行。
    pub(crate) fn finish_run(&mut self, run_id: &str) -> bool {
        if self
            .active_run
            .as_ref()
            .is_some_and(|run| run.run_id == run_id)
        {
            self.active_run = None;
            true
        } else {
            false
        }
    }

    pub(super) fn conversation_is_current(&self, conversation_id: &str) -> bool {
        self.active_conversation_id.as_deref() == Some(conversation_id)
    }

    fn run_is_active(&self, run_id: &str) -> bool {
        self.active_run
            .as_ref()
            .is_some_and(|run| run.run_id == run_id)
    }

    fn run_event_is_visible(&self, run_id: &str, conversation_id: &str) -> bool {
        self.run_is_active(run_id) && self.conversation_is_current(conversation_id)
    }

    /// 判断事件是否属于当前面板状态，防止旧运行或后台对话污染当前消息流。
    pub(crate) fn accepts_event(&self, event: &AiAppEvent) -> bool {
        match event {
            AiAppEvent::RunStarted { run_id, .. } => self
                .active_run
                .as_ref()
                .is_none_or(|run| run.run_id == *run_id),
            AiAppEvent::RunFinished { run_id, .. } => self.run_is_active(run_id),
            AiAppEvent::TextDelta {
                run_id,
                conversation_id,
                ..
            }
            | AiAppEvent::ReasoningDelta {
                run_id,
                conversation_id,
                ..
            }
            | AiAppEvent::TurnEnd {
                run_id,
                conversation_id,
                ..
            }
            | AiAppEvent::PlanUpdated {
                run_id,
                conversation_id,
                ..
            } => self.run_event_is_visible(run_id, conversation_id),
            AiAppEvent::ToolCard(dto) => {
                if dto.run_id.is_empty() {
                    self.conversation_is_current(&dto.conversation_id)
                } else {
                    self.run_event_is_visible(&dto.run_id, &dto.conversation_id)
                }
            }
            AiAppEvent::FocusUpdated { run_id, .. } => {
                self.active_run_is_current() && self.run_is_active(run_id)
            }
            AiAppEvent::TerminalHandoffRequested { run_id, .. } => self.run_is_active(run_id),
            AiAppEvent::ApprovalRequested(dto) => {
                dto.owner_run_id == "mcp-external"
                    || self.run_is_active(&dto.owner_run_id)
                    || self.managed_tasks.iter().any(|task| {
                        task.dto.task_id == dto.run_id
                            && task.dto.status == transom_core::AiManagedTaskStatus::Running
                    })
            }
            AiAppEvent::ContextCompacted {
                conversation_id, ..
            }
            | AiAppEvent::ConversationRolledBack {
                conversation_id, ..
            }
            | AiAppEvent::SubagentCompleted {
                conversation_id, ..
            } => self.conversation_is_current(conversation_id),
            AiAppEvent::InlineCommandResult { .. }
            | AiAppEvent::TerminalContextRequested { .. }
            | AiAppEvent::AppActionRequested { .. }
            | AiAppEvent::ModelsDiscovered { .. }
            | AiAppEvent::ConnectionTested { .. }
            | AiAppEvent::AcpInstallStatus { .. }
            | AiAppEvent::AcpModelsProbed { .. }
            | AiAppEvent::ManagedTaskStarted(_)
            | AiAppEvent::ManagedTaskOutput { .. }
            | AiAppEvent::ManagedTaskFinished { .. } => true,
        }
    }
}
