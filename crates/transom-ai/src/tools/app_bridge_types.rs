//! 软件操作桥接器的强类型动作契约与前置反序列化解析器。
//!
//! 设计原理（Why）：
//! - 工具入口（`app_tools`）与控制平面（`controller/ai_ops/app_bridge`）原先通过无类型
//!   `action: &str` + `serde_json::Value` 传递参数，两侧容易发生字段名与动作枚举漂移；
//! - 本模块在 `transom-ai` 中集中定义各领域的强类型 `#[serde(tag = "action")]` 枚举，
//!   工具入口在调用桥接器前先执行“Parse, Don't Validate”解析校验，控制平面处理器
//!   亦可直接反序列化为强类型枚举进行穷举 `match`。

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::AppBridgeDomain;
use crate::error::ToolError;

/// 将 `(action, args)` 合并反序列化为强类型领域动作枚举。
///
/// # Errors
/// 动作未知、必填字段缺失或字段类型不匹配时返回 [`ToolError::InvalidArguments`]。
pub fn parse_app_action<T: DeserializeOwned>(
    tool: &'static str,
    action: &str,
    args: &Value,
) -> Result<T, ToolError> {
    let mut map = match args {
        Value::Object(obj) => obj.clone(),
        Value::Null => Map::new(),
        _ => {
            return Err(ToolError::InvalidArguments {
                tool,
                reason: "参数必须为 JSON 对象".to_owned(),
            });
        }
    };
    map.insert("action".to_owned(), Value::String(action.to_owned()));
    serde_json::from_value(Value::Object(map)).map_err(|err| ToolError::InvalidArguments {
        tool,
        reason: format!("动作 `{action}` 参数解析失败: {err}"),
    })
}

/// 在工具入口前置校验软件领域动作名称与基础结构合法性（Parse, Don't Validate）。
///
/// # Errors
/// 当 `action` 未在对应领域枚举中注册或字段类型不合法时返回 [`ToolError::InvalidArguments`]。
pub(crate) fn validate_software_action(
    domain: AppBridgeDomain,
    action: &str,
    args: &Value,
) -> Result<(), ToolError> {
    match domain {
        AppBridgeDomain::SessionCatalog => {
            parse_app_action::<SessionCatalogAction>(domain.as_str(), action, args).map(|_| ())
        }
        AppBridgeDomain::WorkspaceControl => {
            parse_app_action::<WorkspaceControlAction>(domain.as_str(), action, args).map(|_| ())
        }
        AppBridgeDomain::SftpTransfer => {
            parse_app_action::<SftpTransferAction>(domain.as_str(), action, args).map(|_| ())
        }
        AppBridgeDomain::HostMonitor => {
            parse_app_action::<HostMonitorAction>(domain.as_str(), action, args).map(|_| ())
        }
        AppBridgeDomain::PortForwardOps => {
            parse_app_action::<PortForwardOpsAction>(domain.as_str(), action, args).map(|_| ())
        }
        AppBridgeDomain::ContainerWorkspace => {
            parse_app_action::<ContainerWorkspaceAction>(domain.as_str(), action, args).map(|_| ())
        }
        AppBridgeDomain::GitWorkspace => {
            parse_app_action::<GitWorkspaceAction>(domain.as_str(), action, args).map(|_| ())
        }
        AppBridgeDomain::ApiClient => {
            parse_app_action::<ApiClientAction>(domain.as_str(), action, args).map(|_| ())
        }
        AppBridgeDomain::AppSettingsOps => {
            parse_app_action::<AppSettingsOpsAction>(domain.as_str(), action, args).map(|_| ())
        }
        AppBridgeDomain::AiMemorySkills => {
            parse_app_action::<AiMemorySkillsAction>(domain.as_str(), action, args).map(|_| ())
        }
        AppBridgeDomain::RemoteExec | AppBridgeDomain::RemoteFs | AppBridgeDomain::TransferFile => {
            Ok(())
        }
    }
}

/// 领域 1：`session_catalog` 强类型动作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SessionCatalogAction {
    List,
    Create {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        host: Option<String>,
        #[serde(default)]
        port: Option<u16>,
        #[serde(default)]
        username: Option<String>,
        #[serde(default)]
        private_key_path: Option<String>,
        #[serde(default)]
        folder_id: Option<String>,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        protection: Option<String>,
        #[serde(default)]
        jump_session_id: Option<String>,
    },
    Update {
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        host: Option<String>,
        #[serde(default)]
        port: Option<u16>,
        #[serde(default)]
        username: Option<String>,
        #[serde(default)]
        protection: Option<String>,
        #[serde(default)]
        folder_id: Option<String>,
        #[serde(default)]
        tags: Option<Vec<String>>,
    },
    Delete {
        #[serde(default)]
        session_id: Option<String>,
    },
    CreateFolder {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        parent_id: Option<String>,
    },
    DeleteFolder {
        #[serde(default)]
        folder_id: Option<String>,
    },
    ListKnownHosts,
    RemoveKnownHost {
        #[serde(default)]
        host: Option<String>,
        #[serde(default)]
        port: Option<u16>,
    },
    DeployPublicKey {
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        public_key_path: Option<String>,
    },
}

/// 领域 2：`workspace_control` 强类型动作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum WorkspaceControlAction {
    ListTabs,
    ConnectSession,
    OpenLocalTerminal,
    SwitchTab,
    CloseTab,
    SplitPane,
    OpenPanel,
    SendTerminalInput,
}

/// 领域 3：`sftp_transfer` 强类型动作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SftpTransferAction {
    ListTransfers,
    CancelTransfer,
    SyncPreview {
        #[serde(default)]
        local_dir: Option<String>,
        #[serde(default)]
        remote_dir: Option<String>,
    },
    SyncApply {
        #[serde(default)]
        local_dir: Option<String>,
        #[serde(default)]
        remote_dir: Option<String>,
        #[serde(default)]
        direction: Option<String>,
    },
}

/// 领域 4：`host_monitor` 强类型动作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum HostMonitorAction {
    Snapshot,
    ListProcesses,
    KillProcess {
        #[serde(default)]
        pid: Option<u32>,
        #[serde(default)]
        signal: Option<String>,
    },
    ToggleSampling {
        #[serde(default)]
        enabled: Option<bool>,
    },
    ProbeHost {
        #[serde(default)]
        host: Option<String>,
        #[serde(default)]
        port: Option<u16>,
    },
}

/// 领域 5：`port_forward_ops` 强类型动作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum PortForwardOpsAction {
    List,
    Create,
    Start,
    Stop,
    Delete,
    Probe,
}

/// 领域 6：`container_workspace` 强类型动作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ContainerWorkspaceAction {
    DiscoverEngines,
    ListContainers,
    InspectContainer,
    ContainerLogs,
    StartContainer,
    StopContainer,
    RestartContainer,
    RemoveContainer,
    CreateAndStartContainer,
    DeployComposeYaml,
    ListImages,
    PullImage,
    RemoveImage,
    ListNetworks,
    ListVolumes,
    ComposeUp,
    ComposeDown,
    OpenExecTerminal,
}

/// 领域 7：`git_workspace` 强类型动作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum GitWorkspaceAction {
    ListProjects,
    Status,
    Log,
    Diff,
    Branches,
    Stage,
    StageAll,
    Unstage,
    Discard,
    DeleteUntracked,
    Commit,
    Checkout,
    CreateBranch,
    RenameBranch,
    Pull,
    Fetch,
    Push,
    StashList,
    StashSave,
    StashPop,
}

/// 领域 8：`api_client` 强类型动作。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ApiClientAction {
    ListProjects,
    ListRequests {
        #[serde(default)]
        project_id: Option<String>,
    },
    GetRequest {
        #[serde(default)]
        request_id: Option<String>,
    },
    CreateRequest {
        #[serde(default)]
        project_id: Option<String>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        method: Option<String>,
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        headers: Option<Map<String, Value>>,
        #[serde(default)]
        body: Option<String>,
    },
    DeleteRequest {
        #[serde(default)]
        request_id: Option<String>,
    },
    SendRequest {
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        environment_id: Option<String>,
    },
}

/// 领域 9：`app_settings_ops` 强类型动作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AppSettingsOpsAction {
    ListSnippets,
    SaveSnippet,
    DeleteSnippet,
    GetSettings,
    UpdateSettings,
    SyncStatus,
    SyncPush,
    SyncPull,
    ExportBackup,
}

/// 领域 10：`ai_memory_skills` 强类型动作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AiMemorySkillsAction {
    ReadMemory,
    AppendMemory,
    ReplaceMemory,
    ListSkills,
    SaveSkill,
    DeleteSkill,
}
