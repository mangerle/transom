//! 应用分领域命令枚举定义。
//!
//! 将全局单一的 AppCommand 按照业务领域归类为分领域子命令：
//! - TerminalCommand: Connect, CloseTab, Shutdown
//! - SftpCommand: Sftp (文件操作), SftpCancel (取消任务)
//! - ForwardCommand: ForwardStart, ForwardStop
//! - HostCommand: DeployPublicKey, RunHostActions, SampleProcesses, SampleProcessCount, KillProcess
//! - SyncCommand: CloudSyncPush, CloudSyncPull, GitSync
//! - ZmodemCommand: ZmodemUpload, ZmodemCancel

use crate::ids::{ConnectionId, ForwardId, TabId, TransferId};
use crate::model::forward::ForwardRule;
use crate::model::host::HostAction;
use crate::model::session::Session;
use crate::model::settings::SyncSettings;
use crate::protocol::sftp::SftpOperation;
use crate::secrets::ConnectionSecrets;
use crate::transport::TerminalSize;
use std::path::PathBuf;

/// 终端建连参数载荷，收敛多参数并消除枚举变体尺寸倾斜。
pub struct ConnectParams {
    pub tab_id: TabId,
    pub connection_id: ConnectionId,
    pub session: Session,
    pub jumps: Vec<crate::model::session::SshHop>,
    pub proxy: crate::protocol::proxy::OutboundProxy,
    pub size: TerminalSize,
    pub secrets: ConnectionSecrets,
    pub zmodem: crate::protocol::zmodem::ZmodemSettings,
    pub allow_sha1_publickey_auth: bool,
    /// 是否启用 Shell 集成（提示符导航与退出码识别）。
    ///
    /// 随命令携带而不是让控制器回读设置：控制器不持有设置文件句柄，
    /// 与 zmodem 配置是同一套约定。本地会话据此决定要不要用启动参数预装集成脚本。
    pub shell_integration: bool,
}

/// 终端建连与生命周期命令。
pub enum TerminalCommand {
    Connect(Box<ConnectParams>),
    CloseTab { tab_id: TabId },
    Shutdown,
}

/// SFTP 远程文件操作命令。
pub enum SftpCommand {
    Sftp {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        op: SftpOperation,
    },
    SftpCancel {
        tab_id: TabId,
        op_id: TransferId,
    },
}

/// 端口转发命令。
pub enum ForwardCommand {
    ForwardStart {
        tab_id: TabId,
        connection_id: ConnectionId,
        rule: ForwardRule,
    },
    ForwardStop {
        tab_id: TabId,
        connection_id: ConnectionId,
        rule_id: ForwardId,
    },
}

/// 主机监控与运维命令。
pub enum HostCommand {
    DeployPublicKey {
        tab_id: TabId,
        connection_id: ConnectionId,
        key_name: String,
        public_key: String,
    },
    RunHostActions {
        actions: Vec<HostAction>,
    },
    SampleProcesses {
        tab_id: TabId,
        connection_id: ConnectionId,
    },
    /// 只采样进程总数：进程卡片折叠时用它维持摘要计数，避免拉取全量明细。
    SampleProcessCount {
        tab_id: TabId,
        connection_id: ConnectionId,
    },
    KillProcess {
        tab_id: TabId,
        connection_id: ConnectionId,
        pid: u32,
        force: bool,
    },
}

/// 云备份与 Git 同步命令。
pub enum SyncCommand {
    CloudSyncPush {
        settings: SyncSettings,
        catalog: crate::model::catalog::SessionCatalog,
    },
    CloudSyncPull {
        settings: SyncSettings,
    },
    GitSync {
        settings: SyncSettings,
        catalog: crate::model::catalog::SessionCatalog,
    },
}

/// ZMODEM 传输命令。
pub enum ZmodemCommand {
    ZmodemUpload { tab_id: TabId, files: Vec<PathBuf> },
    ZmodemCancel { tab_id: TabId },
}
