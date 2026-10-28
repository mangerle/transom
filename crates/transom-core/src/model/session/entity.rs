//! 会话主体模型与启动命令序列。

use super::connection::Connection;
use super::ssh::AuthMethod;
use crate::ids::{FolderId, SessionId};
use crate::model::forward::ForwardRule;
use crate::model::session_appearance::{SessionColor, SessionIcon};
use crate::model::terminal::TerminalProfile;
use crate::secrets::SecretKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub name: String,
    /// 所属文件夹；`None` 表示位于会话列表根层级。
    #[serde(default)]
    pub folder_id: Option<FolderId>,
    pub connection: Connection,
    pub terminal: TerminalProfile,
    #[serde(default)]
    pub forwards: Vec<ForwardRule>,
    /// 连接建立后自动在交互式 Shell 中执行的命令。
    #[serde(default)]
    pub startup: StartupCommands,
    /// 引用的凭据模板；`None` 表示用本会话自己的用户名与认证方式。
    ///
    /// 引用生效点在连接时（`SessionCatalog::resolved_session` 把模板的用户名与
    /// 认证方式叠加到会话副本上），因此老会话文件缺该字段就是"未引用"，
    /// 行为与从前完全一致。
    #[serde(default)]
    pub identity_id: Option<crate::ids::IdentityId>,
    /// 会话专属图标；`Auto` 表示按连接类型自动取图标。
    ///
    /// 老会话文件没有该字段就是"跟随连接类型"，标签栏与侧栏的图标与从前一致。
    #[serde(default)]
    pub icon: SessionIcon,
    /// 会话环境色；`None` 表示不染色。
    #[serde(default)]
    pub color: SessionColor,
    /// 该会话的出站代理；`None` 表示跟随全局偏好设置。
    ///
    /// 老会话文件没有该字段，按跟随全局降级。
    #[serde(default)]
    pub proxy: Option<crate::protocol::proxy::OutboundProxy>,
    /// 语义块视图的会话级覆盖；老会话文件按跟随全局降级。
    #[serde(default)]
    pub block_view: BlockViewOverride,
    /// 最后修改时间（Unix 毫秒），供多端三路合并判定谁更新。
    ///
    /// 老会话文件没有该字段时为 0：合并遇到 0 不做时间比较，退回"基于
    /// 共同祖先的内容比较"，绝不因为缺时间戳就丢掉用户改动。
    #[serde(default)]
    pub updated_at: u64,
}

/// 当前 Unix 毫秒时间戳；时钟异常时按 0 处理。
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

impl Session {
    /// 标记会话刚刚被修改；多端合并据此判断谁更新。
    pub fn touch(&mut self) {
        self.updated_at = now_millis();
    }

    pub fn new(name: impl Into<String>, connection: Connection) -> Self {
        Self {
            id: SessionId::new(),
            name: name.into(),
            folder_id: None,
            connection,
            terminal: TerminalProfile::default(),
            forwards: Vec::new(),
            startup: StartupCommands::default(),
            identity_id: None,
            icon: SessionIcon::default(),
            color: SessionColor::default(),
            proxy: None,
            block_view: BlockViewOverride::default(),
            updated_at: now_millis(),
        }
    }

    /// 该会话实际生效的出站代理。
    ///
    /// 会话显式配置优先；未配置时跟随全局设置。解析发生在连接派发前，
    /// net 层拿到的是最终结果，不需要理解"会话/全局"的优先级。
    pub fn effective_proxy(
        &self,
        global: &crate::protocol::proxy::OutboundProxy,
    ) -> crate::protocol::proxy::OutboundProxy {
        self.proxy.clone().unwrap_or_else(|| global.clone())
    }

    /// 该会话口令在钥匙串里的归属。
    ///
    /// 引用凭据模板时归模板（一份口令、改一处生效），否则归会话自己。
    /// `identity_id` 已由 `SessionCatalog::normalize` 保证指向存在的模板，
    /// 因此这里不必再回查目录。
    pub fn secret_owner(&self) -> crate::secrets::SecretOwner {
        self.identity_id
            .map_or(crate::secrets::SecretOwner::Session(self.id), |id| {
                crate::secrets::SecretOwner::Identity(id)
            })
    }

    /// 该会话可能使用的钥匙串口令种类。
    ///
    /// 只有口令认证与私钥认证的 SSH 会话才会在系统钥匙串里留条目；本地 Shell
    /// 与串口会话没有可保存的口令，导出备份时也无须去读钥匙串。
    pub fn secret_kinds(&self) -> &'static [SecretKind] {
        match &self.connection {
            Connection::Ssh(ssh) => match &ssh.auth {
                AuthMethod::Password { .. } => &[SecretKind::Password],
                AuthMethod::PrivateKey { .. } => &[SecretKind::KeyPassphrase],
                AuthMethod::Agent => &[],
            },
            _ => &[],
        }
    }
}

/// 连接成功后自动执行的初始化命令序列。
///
/// 命令写入交互式 Shell，而不是独立的 exec 通道，这样 `cd`、环境变量导出与
/// `sudo` 等状态才能保留在当前终端会话中；代价是必须等远端 Shell 就绪，
/// 因此执行端会在连接完成后先等待一个短暂的稳定窗口。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartupCommands {
    /// 按顺序逐条执行，空行在编辑阶段即被丢弃。
    #[serde(default)]
    pub commands: Vec<String>,
}

impl StartupCommands {
    /// 从多行文本解析命令列表。
    ///
    /// 仅去除每行首尾空白并忽略空行；行内空白原样保留，避免破坏带参数的
    /// 复杂命令（如带引号的 `export A="x y"`）。
    pub fn from_multiline(text: &str) -> Self {
        let commands = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
        Self { commands }
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    pub fn to_multiline(&self) -> String {
        self.commands.join("\n")
    }
}

/// 会话级语义块模式覆盖：只影响开关，样式与折叠阈值仍跟随全局。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BlockViewOverride {
    /// 跟随全局偏好（默认，老会话文件缺字段即此档）。
    #[default]
    FollowGlobal,
    /// 强制开启，适合生产 Linux 虚拟机与构建机。
    Enabled,
    /// 强制关闭，适合交换机 Console 等纯字符设备。
    Disabled,
}
