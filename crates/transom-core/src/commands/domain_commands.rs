//! 应用分领域命令枚举定义。
//!
//! 将全局单一的 AppCommand 按照业务领域归类为分领域子命令：
//! - TerminalCommand: Connect, CloseTab, Shutdown
//! - SftpCommand: Sftp (文件操作), SftpCancel (取消任务)
//! - ForwardCommand: ForwardStart, ForwardStop
//! - HostCommand: DeployPublicKey, RunHostActions, SampleProcesses, KillProcess
//! - SyncCommand: CloudSyncPush, CloudSyncPull, GitSync
//! - ZmodemCommand: ZmodemUpload, ZmodemCancel

use crate::forward::ForwardRule;
use crate::host::HostAction;
use crate::ids::{ConnectionId, ForwardId, TabId, TransferId};
use crate::secrets::ConnectionSecrets;
use crate::session::Session;
use crate::sftp::SftpOperation;
use crate::sync::SyncSettings;
use crate::terminal::TerminalSize;
use std::path::PathBuf;

/// 终端建连参数载荷，收敛多参数并消除枚举变体尺寸倾斜。
pub struct ConnectParams {
    pub tab_id: TabId,
    pub connection_id: ConnectionId,
    pub session: Session,
    pub jumps: Vec<crate::session::SshHop>,
    pub proxy: crate::proxy::OutboundProxy,
    pub size: TerminalSize,
    pub secrets: ConnectionSecrets,
    pub zmodem: crate::zmodem::ZmodemSettings,
    pub allow_sha1_publickey_auth: bool,
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
        catalog: crate::catalog::SessionCatalog,
    },
    CloudSyncPull {
        settings: SyncSettings,
    },
    GitSync {
        settings: SyncSettings,
        catalog: crate::catalog::SessionCatalog,
    },
}

/// ZMODEM 传输命令。
pub enum ZmodemCommand {
    ZmodemUpload { tab_id: TabId, files: Vec<PathBuf> },
    ZmodemCancel { tab_id: TabId },
}
