//! 工具运行上下文、能力集合与进度事件发送。
//!
//! 该模块只负责运行时状态与事件背压，不定义具体工具实现。

use super::*;
use crate::ToolCallId;

/// 工具运行期挂载的可选后端能力句柄集合（全部默认 `None`，按作用域装配）。
#[derive(Clone, Default)]
pub struct ToolCapabilities {
    pub remote_exec: Option<Arc<dyn RemoteExecutor>>,
    pub remote_fs: Option<Arc<dyn RemoteFsExecutor>>,
    pub terminal: Option<Arc<dyn TerminalContextProvider>>,
    pub runtime_inspector: Option<Arc<dyn RuntimeInspector>>,
    pub forwarder: Option<Arc<dyn ForwardExecutor>>,
    pub memory_writer: Option<Arc<dyn MemoryWriter>>,
    pub knowledge_searcher: Option<Arc<dyn KnowledgeSearcher>>,
    pub app_bridge: Option<Arc<dyn AppActionBridge>>,
    pub subagent_runner: Option<Arc<dyn SubagentRunner>>,
    pub skill_loader: Option<Arc<dyn SkillLoader>>,
}

/// 工具运行上下文：汇集绑定会话、超时配置、取消令牌与各后端能力句柄。
#[derive(Clone)]
pub struct ToolRuntimeContext {
    pub scope: SessionScope,
    pub locked_session: Option<LockedSession>,
    pub bulk_hosts: Vec<BulkHostTarget>,
    pub output_store: Arc<ToolOutputStore>,
    pub command_timeout: Duration,
    pub cancel: CancellationToken,
    pub events: mpsc::Sender<AgentEvent>,
    /// 远端写操作前保存快照的本地目录。
    pub snapshot_dir: Option<std::path::PathBuf>,
    pub caps: ToolCapabilities,
}

impl ToolRuntimeContext {
    /// 从工具运行上下文构造统一的远端 I/O 截止时间与取消令牌。
    pub fn io_context(&self) -> ToolIoContext {
        ToolIoContext::new(self.command_timeout, self.cancel.clone())
    }

    /// 构造最小可用的工具运行上下文（其余可选能力均默认置空）。
    pub fn minimal(
        scope: SessionScope,
        events: mpsc::Sender<AgentEvent>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            scope,
            locked_session: None,
            bulk_hosts: Vec::new(),
            output_store: Arc::new(ToolOutputStore::new()),
            command_timeout: Duration::from_secs(60),
            cancel,
            events,
            snapshot_dir: None,
            caps: ToolCapabilities::default(),
        }
    }

    /// 将修改前的远端文件内容写入快照目录并返回快照文件路径。
    pub fn save_revert_snapshot(&self, content: &str) -> Option<String> {
        let dir = self.snapshot_dir.as_ref()?;
        if let Err(err) = std::fs::create_dir_all(dir) {
            log::warn!("创建远端快照目录失败（{}）: {err}", dir.display());
            return None;
        }
        let path = dir.join(format!("{}.bak", uuid::Uuid::new_v4()));
        match std::fs::write(&path, content.as_bytes()) {
            Ok(()) => Some(path.display().to_string()),
            Err(err) => {
                log::warn!("保存远端配置变更快照失败（{}）: {err}", path.display());
                None
            }
        }
    }

    pub fn host_label(&self) -> String {
        self.locked_session
            .as_ref()
            .map(|s| s.prompt_ctx.host_name.clone())
            .unwrap_or_else(|| "通用会话".to_owned())
    }

    pub fn protection(&self) -> HostProtection {
        self.locked_session
            .as_ref()
            .map(|s| s.prompt_ctx.protection)
            .unwrap_or(HostProtection::Normal)
    }

    /// 等待有界事件流接收工具卡片。
    ///
    /// 最终卡片不得使用 `try_send`：通道饱和时必须施加背压，否则 UI、会话 JSONL
    /// 与审计日志会静默缺失同一次工具执行记录。
    pub async fn record_card(&self, mut card: ToolExecutionCard) {
        if card.host_label.is_empty() {
            card.host_label = self.host_label();
        }
        if let Err(err) = self.events.send(AgentEvent::ToolCard(card)).await {
            log::warn!("发送 AI 工具执行卡片失败: {err}");
        }
    }

    /// 推送工具开始执行的运行态预览卡片，返回用于后续原地刷新的 `call_id`。
    pub async fn emit_tool_started(
        &self,
        tool_name: &str,
        host_label: &str,
        risk: ToolRisk,
        summary: &str,
    ) -> String {
        let call_id = uuid::Uuid::new_v4().to_string();
        self.record_card(ToolExecutionCard {
            call_id: ToolCallId::new(call_id.clone()),
            running: true,
            tool_name: tool_name.to_owned(),
            host_label: host_label.to_owned(),
            risk,
            summary: summary.to_owned(),
            output: String::new(),
            ok: true,
            approved: true,
            duration_ms: 0,
            diff_preview: None,
            ..Default::default()
        })
        .await;
        call_id
    }
}

/// 工具执行过程中的实时进度推送参数。
///
/// 设计原理：收敛超过 5 个的平铺参数，避免触发 Clippy 参数数量告警并保持调用端可读性。
pub struct ToolProgressParams<'a> {
    pub call_id: &'a str,
    pub tool_name: &'a str,
    pub host_label: &'a str,
    pub risk: ToolRisk,
    pub summary: &'a str,
    pub tail_output: &'a str,
    pub elapsed_ms: u64,
}

impl ToolRuntimeContext {
    /// 推送工具执行过程中的实时末尾输出预览与已耗时（原地更新同 `call_id` 卡片）。
    pub fn emit_tool_progress(&self, params: ToolProgressParams<'_>) {
        let preview = clip_tail_lines(params.tail_output, 5, 160);
        let card = ToolExecutionCard {
            call_id: ToolCallId::from(params.call_id),
            running: true,
            tool_name: params.tool_name.to_owned(),
            host_label: params.host_label.to_owned(),
            risk: params.risk,
            summary: params.summary.to_owned(),
            output: preview,
            ok: true,
            approved: true,
            duration_ms: params.elapsed_ms,
            diff_preview: None,
            ..Default::default()
        };
        // 进度预览是可合并的尽力事件；通道饱和时允许丢弃，但最终卡片始终通过异步背压发送。
        match self.events.try_send(AgentEvent::ToolCard(card)) {
            Ok(()) | Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Closed(_)) => {
                log::warn!("AI 工具进度事件通道已关闭");
            }
        }
    }

    /// 推送最新计划快照；与工具卡片、轮次终态共用有序事件流。
    ///
    /// 使用异步发送等待背压，避免最终计划在通道拥堵时被静默丢弃。
    pub async fn emit_plan(&self, plan: PlanSnapshot) {
        if let Err(err) = self.events.send(AgentEvent::Plan(plan)).await {
            log::warn!("发送 AI 计划更新失败: {err}");
        }
    }

    /// 清洗 ANSI 并通过会话级输出句柄池生成预览（超限则保留 `output_id`）。
    pub fn format_tool_output(&self, tool_name: &str, raw: &str) -> String {
        self.output_store
            .store_and_preview(tool_name, raw, DEFAULT_TOOL_OUTPUT_LIMIT)
    }

    /// 构造仅用于导出工具 Schema 的空上下文（不关联任何连接或任务）。
    pub(crate) fn dummy_for_schema() -> Self {
        let (events, _event_rx) = mpsc::channel(1);
        Self::minimal(SessionScope::Global, events, CancellationToken::new())
    }
}
