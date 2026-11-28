//! 控制平面软件操作统一桥接层（覆盖 10 大软件领域）。

mod api_client;
mod container_deploy;
mod dev;
mod file_io;
pub(in crate::controller::ai_ops) mod local_ops;
mod memory;
mod sessions;
mod system;
mod target_ops;
mod target_resolver;
mod workspace;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
pub(super) use memory::{StoreKnowledgeBridge, StoreMemoryBridge, StoreSkillLoader};
use parking_lot::Mutex;
use serde_json::Value;
pub(in crate::controller) use target_resolver::ControllerTargetResolver;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use transom_ai::{AppActionBridge, AppBridgeDomain, ToolError, ToolIoContext};
use transom_api::{ApiHttpService, ApiProjectStore};
use transom_core::{
    AiAppEvent, AppEvent, BackupService, GitProjectStore, KnownHostsStore, SecretStore,
    SessionStore, SettingsStore, TabId,
};

/// 待 UI 回传软件级动作执行结果的异步回调映射表。
pub(in crate::controller) type AppActionReqMap =
    Arc<Mutex<HashMap<String, oneshot::Sender<Result<String, String>>>>>;
use transom_git::GitConfig;
use transom_net::SshRemote;
use transom_platform::system::ProcessSampler;
use transom_store::AiStore;

use crate::controller::ai_ops::AiConfigService;
use uuid::Uuid;

use super::super::state::{ControllerCtx, ControllerState};

/// 桥接层依赖组合根，按能力域隔离存储、项目服务、运行时与当前目标。
#[derive(Clone)]
pub(in crate::controller) struct AppBridgeDeps {
    pub(in crate::controller) stores: AppBridgeStores,
    pub(in crate::controller) projects: AppBridgeProjects,
    pub(in crate::controller) runtime: AppBridgeRuntime,
    pub(in crate::controller) target: AppBridgeTarget,
}

/// 持久化与凭证服务集合。
#[derive(Clone)]
pub(in crate::controller) struct AppBridgeStores {
    pub(in crate::controller) known_hosts: Arc<dyn KnownHostsStore>,
    pub(in crate::controller) secret_store: Arc<dyn SecretStore>,
    pub(in crate::controller) session_store: Arc<dyn SessionStore>,
    pub(in crate::controller) settings_store: Arc<dyn SettingsStore>,
    pub(in crate::controller) backup: Arc<dyn BackupService>,
    pub(in crate::controller) ai_store: Arc<AiStore>,
    pub(in crate::controller) ai_config: Arc<AiConfigService>,
}

/// Git/API 项目目录服务集合。
#[derive(Clone)]
pub(in crate::controller) struct AppBridgeProjects {
    pub(in crate::controller) git_projects: Arc<dyn GitProjectStore>,
    pub(in crate::controller) api_projects: Arc<dyn ApiProjectStore>,
    pub(in crate::controller) api_http: ApiHttpService,
    pub(in crate::controller) git_config: Arc<parking_lot::RwLock<GitConfig>>,
}

/// 执行期事件、取消、任务与本地采样依赖。
#[derive(Clone)]
pub(in crate::controller) struct AppBridgeRuntime {
    pub(in crate::controller) local_processes: Arc<Mutex<ProcessSampler>>,
    pub(in crate::controller) event_tx: flume::Sender<AppEvent>,
    pub(in crate::controller) internal_tx: flume::Sender<crate::controller::Internal>,
    pub(in crate::controller) app_action_reqs: AppActionReqMap,
    pub(in crate::controller) command_timeout: Duration,
    pub(in crate::controller) cancel: CancellationToken,
    pub(in crate::controller) tasks:
        Arc<crate::controller::ai_ops::task_runner::ManagedTaskRegistry>,
}

/// 当前工具调用绑定的标签、连接与对话上下文。
#[derive(Clone)]
pub(in crate::controller) struct AppBridgeTarget {
    pub(in crate::controller) active_tab_id: Option<TabId>,
    pub(in crate::controller) active_remote: Option<SshRemote>,
    pub(in crate::controller) active_sftp: Option<transom_net::SftpClient>,
    pub(in crate::controller) conversation_id: Option<String>,
    pub(in crate::controller) current_turn_entry_id: Arc<parking_lot::RwLock<Option<String>>>,
    pub(in crate::controller) headless_remotes: crate::controller::state::AiHeadlessRemoteMap,
}

impl AppBridgeDeps {
    pub(in crate::controller) fn from_controller(
        state: &ControllerState,
        ctx: &ControllerCtx,
        active_tab_id: Option<TabId>,
        command_timeout: Duration,
        cancel: CancellationToken,
    ) -> Self {
        let active_sftp = active_tab_id
            .and_then(|tid| state.sessions.get(&tid))
            .and_then(|managed| managed.sftp_client.clone());
        let active_remote = active_tab_id
            .and_then(|tid| state.sessions.get(&tid))
            .and_then(|managed| managed.remote.clone());
        Self {
            stores: AppBridgeStores {
                known_hosts: ctx.known_hosts.clone(),
                secret_store: ctx.secret_store.clone(),
                session_store: ctx.session_store.clone(),
                settings_store: ctx.settings_store.clone(),
                backup: ctx.backup.clone(),
                ai_store: ctx.ai_store.clone(),
                ai_config: ctx.ai_config.clone(),
            },
            projects: AppBridgeProjects {
                git_projects: ctx.git_projects.clone(),
                api_projects: ctx.api_projects.clone(),
                api_http: ctx.api_http.clone(),
                git_config: ctx.git_config.clone(),
            },
            runtime: AppBridgeRuntime {
                local_processes: ctx.local_processes.clone(),
                event_tx: ctx.event_tx.clone(),
                internal_tx: ctx.internal_tx.clone(),
                app_action_reqs: state.ai_app_action_reqs.clone(),
                command_timeout,
                cancel,
                tasks: state.ai_tasks.clone(),
            },
            target: AppBridgeTarget {
                active_tab_id,
                active_remote,
                active_sftp,
                conversation_id: None,
                current_turn_entry_id: Arc::new(parking_lot::RwLock::new(None)),
                headless_remotes: state.ai_headless_remotes.clone(),
            },
        }
    }

    /// 替换运行时取消令牌，供派生运行或子 Agent 隔离生命周期。
    pub(in crate::controller) fn set_cancel(&mut self, cancel: CancellationToken) {
        self.runtime.cancel = cancel;
    }

    /// 构造远端 I/O 的统一超时与取消上下文。
    pub(in crate::controller) fn io_context(&self) -> ToolIoContext {
        ToolIoContext::new(self.runtime.command_timeout, self.runtime.cancel.clone())
    }
}

/// 面向内置 AI 与外部 ACP（MCP）的统一软件操作桥接器。
pub(in crate::controller) struct ControllerAppActionBridge {
    deps: AppBridgeDeps,
}

impl ControllerAppActionBridge {
    pub(in crate::controller) fn new(deps: AppBridgeDeps) -> Self {
        Self { deps }
    }

    pub(super) fn io_context(&self) -> ToolIoContext {
        self.deps.io_context()
    }

    /// 向 GPUI 界面派发同步动作并等待其通过 `RespondAppAction` 回传结果。
    pub(super) async fn request_ui_action(
        &self,
        domain: &'static str,
        action: &str,
        args: &Value,
    ) -> Result<String, ToolError> {
        let request_id = Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.deps
            .runtime
            .app_action_reqs
            .lock()
            .insert(request_id.clone(), tx);

        let evt = AppEvent::Ai(Box::new(AiAppEvent::AppActionRequested {
            request_id: request_id.clone(),
            domain: domain.to_owned(),
            action: action.to_owned(),
            args_json: args.to_string(),
        }));
        if self.deps.runtime.event_tx.send_async(evt).await.is_err() {
            self.deps.runtime.app_action_reqs.lock().remove(&request_id);
            return Err(ToolError::ExecutionFailed {
                tool: domain,
                reason: "界面事件通道已关闭".to_owned(),
            });
        }

        match tokio::time::timeout(Duration::from_secs(10), rx).await {
            Ok(Ok(Ok(out))) => Ok(out),
            Ok(Ok(Err(reason))) => Err(ToolError::ExecutionFailed {
                tool: domain,
                reason,
            }),
            Ok(Err(_)) => {
                self.deps.runtime.app_action_reqs.lock().remove(&request_id);
                Err(ToolError::ExecutionFailed {
                    tool: domain,
                    reason: "界面未能返回操作结果".to_owned(),
                })
            }
            Err(_) => {
                self.deps.runtime.app_action_reqs.lock().remove(&request_id);
                Err(ToolError::ExecutionTimeout { timeout_secs: 10 })
            }
        }
    }
}

#[async_trait]
impl AppActionBridge for ControllerAppActionBridge {
    async fn invoke(
        &self,
        domain: AppBridgeDomain,
        action: &str,
        args: Value,
    ) -> Result<String, ToolError> {
        match domain {
            AppBridgeDomain::SessionCatalog => sessions::handle(self, action, &args).await,
            AppBridgeDomain::WorkspaceControl => {
                workspace::handle_workspace(self, action, &args).await
            }
            AppBridgeDomain::HostMonitor => workspace::handle_monitor(self, action, &args).await,
            AppBridgeDomain::PortForwardOps => workspace::handle_forward(self, action, &args).await,
            AppBridgeDomain::SftpTransfer => {
                system::handle_sftp_transfer(self, action, &args).await
            }
            AppBridgeDomain::ContainerWorkspace => dev::handle_container(self, action, &args).await,
            AppBridgeDomain::GitWorkspace => dev::handle_git(self, action, &args).await,
            AppBridgeDomain::ApiClient => api_client::handle_api_client(self, action, &args).await,
            AppBridgeDomain::AppSettingsOps => system::handle_settings(self, action, &args).await,
            AppBridgeDomain::AiMemorySkills => {
                memory::handle_memory_skills(self, action, &args).await
            }
            AppBridgeDomain::RemoteExec => target_ops::handle_remote_exec(self, &args).await,
            AppBridgeDomain::RemoteFs => target_ops::handle_remote_fs(self, action, &args).await,
            AppBridgeDomain::TransferFile => {
                target_ops::handle_transfer_file(self, action, &args).await
            }
        }
    }
}
