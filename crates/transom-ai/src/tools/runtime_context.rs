//! 工具运行上下文、能力集合与进度事件发送。
//!
//! 该模块只负责运行时状态与事件背压，不定义具体工具实现。

use std::time::Duration;

use tokio::sync::mpsc::{self, error::TrySendError};
use tokio_util::sync::CancellationToken;

use super::target::{ResolvedToolTarget, ToolTargetResolver};
use super::*;
use crate::ToolCallId;
use crate::output_store::ToolOutputStore;
use crate::runner::AgentEvent;
use crate::session::SessionScope;
use crate::tools::plan::PlanSnapshot;

/// 按场景按需激活低频软件领域工具的可见性开关（抑制 Tool Schema 上下文膨胀）。
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct AppToolVisibility {
    pub container: bool,
    pub git: bool,
    pub api_client: bool,
    pub settings_ops: bool,
    pub memory_skills: bool,
}

impl Default for AppToolVisibility {
    fn default() -> Self {
        Self::all()
    }
}

impl AppToolVisibility {
    /// 全部开启（适用于全局作用域或全量能力测试）。
    pub const fn all() -> Self {
        Self {
            container: true,
            git: true,
            api_client: true,
            settings_ops: true,
            memory_skills: true,
        }
    }

    /// 全部关闭（仅保留高频核心软件工具）。
    pub const fn none() -> Self {
        Self {
            container: false,
            git: false,
            api_client: false,
            settings_ops: false,
            memory_skills: false,
        }
    }

    /// 根据当前会话作用域与用户意图动态推导需要装配的低频软件工具。
    ///
    /// 全局会话保持全部可用；绑定主机的运维会话默认只挂载核心远端与工作区工具，
    /// 仅当提示词或激活技能提及对应领域关键词时按需激活，避免无关 Schema 挤占上下文。
    pub fn for_prompt(
        scope: &SessionScope,
        prompt: &str,
        active_skill_prompt: Option<&str>,
    ) -> Self {
        if matches!(scope, SessionScope::Global) {
            return Self::all();
        }
        let combined = match active_skill_prompt {
            Some(skill) if !skill.trim().is_empty() => format!("{prompt}\n{skill}").to_lowercase(),
            _ => prompt.to_lowercase(),
        };
        let has_any = |keywords: &[&str]| keywords.iter().any(|kw| combined.contains(kw));
        Self {
            container: has_any(&["container", "docker", "podman", "compose", "容器", "镜像"]),
            git: has_any(&["git", "commit", "branch", "merge", "rebase", "仓库", "分支"]),
            api_client: has_any(&["api", "http", "endpoint", "curl", "接口", "请求"]),
            settings_ops: has_any(&["setting", "config", "sync", "cloud", "设置", "配置", "同步"]),
            memory_skills: has_any(&["memory", "skill", "runbook", "记忆", "技能", "知识库"]),
        }
    }
}

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
    /// 低频软件工具的按需可见性开关。
    pub app_tool_visibility: AppToolVisibility,
}

/// 工具运行上下文：汇集绑定会话、超时配置、取消令牌与各后端能力句柄。
#[derive(Clone)]
pub struct ToolRuntimeContext {
    pub scope: SessionScope,
    pub locked_session: Option<LockedSession>,
    /// 未显式指定 `target` 时用于策略判断的默认目标。
    pub default_target: ResolvedToolTarget,
    pub bulk_hosts: Vec<BulkHostTarget>,
    pub output_store: Arc<ToolOutputStore>,
    pub command_timeout: Duration,
    pub cancel: CancellationToken,
    pub events: mpsc::Sender<AgentEvent>,
    /// 远端写操作前保存快照的本地目录。
    pub snapshot_dir: Option<std::path::PathBuf>,
    pub caps: ToolCapabilities,
    /// 用于在策略门前解析跨主机目标的元数据解析器。
    pub target_resolver: Option<Arc<dyn ToolTargetResolver>>,
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
            default_target: ResolvedToolTarget::new(None, "通用会话", HostProtection::Normal),
            bulk_hosts: Vec::new(),
            output_store: Arc::new(ToolOutputStore::new()),
            command_timeout: Duration::from_secs(60),
            cancel,
            events,
            snapshot_dir: None,
            caps: ToolCapabilities::default(),
            target_resolver: None,
        }
    }

    /// 将修改前的远端文件内容异步写入快照目录并返回快照文件路径。
    pub async fn save_revert_snapshot(&self, content: &str) -> Option<String> {
        let dir = self.snapshot_dir.as_ref()?;
        if let Err(err) = tokio::fs::create_dir_all(dir).await {
            log::warn!("创建远端快照目录失败（{}）: {err}", dir.display());
            return None;
        }
        let path = dir.join(format!("{}.bak", uuid::Uuid::new_v4()));
        match tokio::fs::write(&path, content.as_bytes()).await {
            Ok(()) => Some(path.display().to_string()),
            Err(err) => {
                log::warn!("保存远端配置变更快照失败（{}）: {err}", path.display());
                None
            }
        }
    }

    pub fn host_label(&self) -> String {
        self.default_target.label.clone()
    }

    pub fn protection(&self) -> HostProtection {
        self.default_target.protection
    }

    /// 返回当前运行上下文自身的策略目标快照。
    pub fn current_target(&self) -> ResolvedToolTarget {
        self.default_target.clone()
    }

    /// 解析本次工具调用的策略目标。
    ///
    /// 本机目标在本地保护策略正式确定前沿用当前运行上下文的保护级别，以免跨主机
    /// 目标修复悄悄改变既有本机操作的审批语义。远端目标必须通过注入的解析器解析，
    /// 解析失败时在策略门直接失败关闭。
    ///
    /// # Errors
    /// 远端目标无法解析或运行上下文未挂载目标解析器时返回 [`ToolError`]。
    pub async fn resolve_policy_target(
        &self,
        requested: Option<&str>,
    ) -> Result<ResolvedToolTarget, ToolError> {
        let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(self.current_target());
        };
        if requested.eq_ignore_ascii_case("local") {
            return Ok(ResolvedToolTarget::new(
                None,
                "本机 (Local)",
                self.default_target.protection,
            ));
        }
        if requested.eq_ignore_ascii_case("active") {
            return Ok(self.current_target());
        }
        let resolver = self
            .target_resolver
            .as_ref()
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool: "target_resolver",
                reason: "当前运行上下文未挂载跨主机目标解析器".to_owned(),
            })?;
        resolver.resolve_remote_target(requested).await
    }

    /// 等待有界事件流接收工具卡片。
    ///
    /// 最终卡片不得使用 `try_send`：通道饱和时必须施加背压，否则 UI、会话 JSONL
    /// 与审计日志会静默缺失同一次工具执行记录。
    pub async fn record_card(&self, mut card: ToolExecutionCard) {
        if card.host_label.is_empty() {
            card.host_label = self.host_label();
        }
        if let Err(err) = self
            .events
            .send(AgentEvent::ToolCard(card.sanitized()))
            .await
        {
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
        match self.events.try_send(AgentEvent::ToolCard(card.sanitized())) {
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

    /// 统一记录工具执行结果卡片并返回格式化的模型输出（或转换后的 Rig 错误）。
    ///
    /// # Errors
    /// 当 `result` 为 `Err` 时记录失败卡片并返回 [`rig_agent::tool::ToolExecutionError`]。
    pub async fn finish_tool_result(
        &self,
        meta: ToolCardMeta<'_>,
        result: Result<String, ToolError>,
    ) -> Result<String, rig_agent::tool::ToolExecutionError> {
        match result {
            Ok(raw) => {
                let clipped = self.format_tool_output(meta.tool_name, &raw);
                self.record_outcome_card(meta, clipped.clone(), true).await;
                Ok(clipped)
            }
            Err(err) => {
                let msg = err.to_string();
                self.record_outcome_card(meta, msg, false).await;
                Err(to_rig_err(err))
            }
        }
    }

    /// 统一记录一张工具执行完成卡片（适用于调用方需自行格式化输出或仅记录既定结果）。
    pub async fn record_outcome_card(&self, meta: ToolCardMeta<'_>, output: String, ok: bool) {
        self.record_card(ToolExecutionCard {
            call_id: meta.call_id.map(ToolCallId::from).unwrap_or_default(),
            running: false,
            tool_name: meta.tool_name.to_owned(),
            host_label: meta.host_label.to_owned(),
            risk: meta.risk,
            summary: meta.summary.to_owned(),
            output,
            ok,
            approved: true,
            duration_ms: meta.duration_ms,
            diff_preview: meta.diff_preview,
            revert_target_path: meta.revert_target_path,
            revert_backup_path: meta.revert_backup_path,
        })
        .await;
    }
}

/// 工具执行完成卡片的元数据，供 [`ToolRuntimeContext::finish_tool_result`] 与
/// [`ToolRuntimeContext::record_outcome_card`] 复用。
pub struct ToolCardMeta<'a> {
    pub call_id: Option<&'a str>,
    pub tool_name: &'a str,
    pub host_label: &'a str,
    pub risk: ToolRisk,
    pub summary: &'a str,
    pub duration_ms: u64,
    pub diff_preview: Option<String>,
    pub revert_target_path: Option<String>,
    pub revert_backup_path: Option<String>,
}

impl<'a> ToolCardMeta<'a> {
    /// 构造基础工具卡片元数据。
    pub fn new(tool_name: &'a str, risk: ToolRisk, summary: &'a str, duration_ms: u64) -> Self {
        Self {
            call_id: None,
            tool_name,
            host_label: "",
            risk,
            summary,
            duration_ms,
            diff_preview: None,
            revert_target_path: None,
            revert_backup_path: None,
        }
    }

    pub fn with_host(mut self, host_label: &'a str) -> Self {
        self.host_label = host_label;
        self
    }

    pub fn with_call_id(mut self, call_id: &'a str) -> Self {
        self.call_id = Some(call_id);
        self
    }

    pub fn with_diff(mut self, diff_preview: Option<String>) -> Self {
        self.diff_preview = diff_preview;
        self
    }

    pub fn with_revert(mut self, target_path: Option<String>, backup_path: Option<String>) -> Self {
        self.revert_target_path = target_path;
        self.revert_backup_path = backup_path;
        self
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::ToolRuntimeContext;
    use crate::tools::{ResolvedToolTarget, ToolError, ToolTargetResolver};
    use transom_ai_contracts::policy::HostProtection;
    use transom_ai_contracts::session::SessionScope;

    struct FakeResolver;

    #[async_trait]
    impl ToolTargetResolver for FakeResolver {
        async fn resolve_remote_target(
            &self,
            target: &str,
        ) -> Result<ResolvedToolTarget, ToolError> {
            if target == "prod-01" {
                Ok(ResolvedToolTarget::new(
                    Some("saved-prod".to_owned()),
                    "生产主机",
                    HostProtection::Production,
                ))
            } else {
                Err(ToolError::InvalidArguments {
                    tool: "target_resolver",
                    reason: "目标不存在".to_owned(),
                })
            }
        }
    }

    fn context() -> ToolRuntimeContext {
        let (events, _receiver) = mpsc::channel(1);
        ToolRuntimeContext::minimal(SessionScope::Global, events, CancellationToken::new())
    }

    #[tokio::test]
    async fn local_target_preserves_current_protection() {
        let mut ctx = context();
        ctx.default_target = ResolvedToolTarget::new(
            Some("active".to_owned()),
            "当前主机",
            HostProtection::ReadOnly,
        );

        let target = ctx
            .resolve_policy_target(Some("local"))
            .await
            .expect("本机目标应解析成功");
        assert_eq!(target.protection, HostProtection::ReadOnly);
        assert_eq!(target.label, "本机 (Local)");
    }

    #[tokio::test]
    async fn remote_target_uses_injected_resolver() {
        let mut ctx = context();
        ctx.target_resolver = Some(std::sync::Arc::new(FakeResolver));

        let target = ctx
            .resolve_policy_target(Some("prod-01"))
            .await
            .expect("远端目标应解析成功");
        assert_eq!(target.host_id.as_deref(), Some("saved-prod"));
        assert_eq!(target.protection, HostProtection::Production);
        assert_eq!(target.label, "生产主机");
    }

    #[tokio::test]
    async fn remote_target_fails_closed_without_resolver() {
        let ctx = context();
        let error = ctx
            .resolve_policy_target(Some("prod-01"))
            .await
            .expect_err("缺少解析器时必须失败关闭");
        assert!(matches!(error, ToolError::ExecutionFailed { .. }));
    }
}
