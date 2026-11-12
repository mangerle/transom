//! 终端/SFTP/转发/宿主/同步/ZMODEM 等领域的命令载荷。

use crate::ids::{ConnectionId, ForwardId, TabId, TransferId};
use crate::model::forward::ForwardRule;
use crate::model::host::HostAction;
use crate::model::session::Session;
use crate::model::settings::SyncSettings;
use crate::protocol::sftp::SftpOperation;
use crate::secrets::ConnectionSecrets;
use crate::transport::TerminalSize;
use std::path::PathBuf;

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
#[derive(Clone)]
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
    /// 开关系统统计采样：界面不再消费 CPU/内存/磁盘等指标时停止远端轮询。
    SetStatsSampling {
        enabled: bool,
    },
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
    /// 只读取远端并生成本机/远端关系摘要，不落盘目录。
    CloudSyncCheck {
        settings: SyncSettings,
        catalog: crate::model::catalog::SessionCatalog,
        trigger: crate::protocol::sync::SyncTrigger,
    },
    /// 只验证端点、凭据与对象可达性。
    CloudSyncTest {
        settings: SyncSettings,
        trigger: crate::protocol::sync::SyncTrigger,
    },
    CloudSyncPush {
        settings: SyncSettings,
        catalog: crate::model::catalog::SessionCatalog,
        trigger: crate::protocol::sync::SyncTrigger,
        mode: crate::protocol::sync::CloudPushMode,
    },
    CloudSyncPull {
        settings: SyncSettings,
        trigger: crate::protocol::sync::SyncTrigger,
        mode: crate::protocol::sync::CloudPullMode,
    },
    GitSync {
        settings: SyncSettings,
        catalog: crate::model::catalog::SessionCatalog,
        trigger: crate::protocol::sync::SyncTrigger,
    },
}

/// ZMODEM 传输命令。
pub enum ZmodemCommand {
    ZmodemUpload { tab_id: TabId, files: Vec<PathBuf> },
    ZmodemCancel { tab_id: TabId },
}
