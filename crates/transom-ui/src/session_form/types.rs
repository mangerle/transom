//! 会话表单核心数据模型与选项类型。

use gpui_kit::Entity;
use gpui_kit::component::input::{InputState, TextareaState};
use transom_core::{
    BlockViewOverride, Connection, IdentityId, OutboundProxy, SerialDataBits, SerialFlowControl,
    SerialParity, SerialPortDescriptor, SerialStopBits, Session, SessionColor, SessionIcon,
    SessionId, TextEncoding,
};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthChoice {
    Password,
    PrivateKey,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectionChoice {
    Ssh,
    Serial,
    Telnet,
}

/// 会话表单里的出站代理档位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProxyChoice {
    /// 跟随全局偏好设置（会话不落配置）。
    FollowGlobal,
    Direct,
    Socks5,
    Http,
}

/// 会话表单的构造输入。
///
/// 表单需要的目录快照（凭据模板、跳板候选、文件夹）与全局代理说明一起
/// 传入，保证打开表单后不因目录变化而悄悄换选项。
pub(crate) struct SessionFormInit {
    pub base: Option<Session>,
    pub folder_path: Option<String>,
    pub identities: Vec<IdentityOption>,
    pub jump_options: Vec<JumpOption>,
    pub folder_options: Vec<String>,
    /// 全局出站代理，仅用于"跟随全局"的说明文字。
    pub global_proxy: OutboundProxy,
    /// 语义块模式覆盖。
    pub block_view: BlockViewOverride,
}

pub(crate) struct SessionForm {
    pub base: Option<Session>,
    pub name: Entity<InputState>,
    /// 以 `/` 分隔的文件夹路径，保存时解析为稳定的文件夹节点。
    pub folder_path: Entity<InputState>,
    /// 可选的文件夹路径清单，供下拉快速选择。
    pub folder_options: Vec<String>,
    pub connection: ConnectionChoice,
    pub host: Entity<InputState>,
    pub port: Entity<InputState>,
    pub username: Entity<InputState>,
    pub password: Entity<InputState>,
    pub key_path: Entity<InputState>,
    /// CA 签发的用户证书路径（`*-cert.pub`），仅密钥认证有意义。
    pub cert_path: Entity<InputState>,
    pub passphrase: Entity<InputState>,
    pub serial_port: Entity<InputState>,
    pub serial_baud: Entity<InputState>,
    pub serial_data_bits: SerialDataBits,
    pub serial_parity: SerialParity,
    pub serial_stop_bits: SerialStopBits,
    pub serial_flow_control: SerialFlowControl,
    pub serial_ports: Vec<SerialPortDescriptor>,
    pub serial_ports_error: Option<String>,
    /// Telnet 字符编码；远端为老设备或 BBS 时选择 GBK/Big5。
    pub telnet_encoding: TextEncoding,
    pub startup_commands: Entity<TextareaState>,
    /// SSH 连接超时（秒）；留空或非法值回退会话存量/默认。
    pub connect_timeout: Entity<InputState>,
    /// SSH keepalive 间隔（秒），0 表示禁用。
    pub keepalive: Entity<InputState>,
    pub auth: AuthChoice,
    pub save_password: bool,
    pub save_passphrase: bool,
    /// 意外断线后自动重连（见 AppShell::schedule_auto_reconnect）。
    pub auto_reconnect: bool,
    /// 关机主机的网卡 MAC，供 Wake-on-LAN 使用；留空表示不启用唤醒。
    pub wol_mac: Entity<InputState>,
    /// 可选的凭据模板清单（打开表单时从目录快照而来）。
    pub identities: Vec<IdentityOption>,
    /// 当前选用的凭据模板；`None` 表示用本会话自己的用户名与认证方式。
    pub identity_choice: Option<IdentityId>,
    /// 可选的跳板机会话清单（仅 SSH，已排除自身与会成环的候选）。
    pub jump_options: Vec<JumpOption>,
    /// 当前选用的跳板机；`None` 表示直连。
    pub jump_choice: Option<SessionId>,
    /// 出站代理档位；跟随全局时会话不落代理配置。
    pub proxy_choice: ProxyChoice,
    pub proxy_host: Entity<InputState>,
    pub proxy_port: Entity<InputState>,
    pub proxy_username: Entity<InputState>,
    pub proxy_password: Entity<InputState>,
    /// 全局代理快照，用于"跟随全局"的说明文字。
    pub global_proxy: OutboundProxy,
    /// 语义块模式覆盖：跟随全局/强制开启/强制关闭。
    pub block_view: BlockViewOverride,
    /// 会话专属图标；`Auto` 表示按连接类型自动取图标。
    pub icon: SessionIcon,
    /// 会话环境色；`None` 表示不染色。
    pub color: SessionColor,
    pub confirm_delete: bool,
    pub error: Option<String>,
}

/// 会话表单里的一个凭据模板选项。
///
/// 带上用户名是因为选中模板后要用它填进连接参数：表单无法访问目录，
/// 让调用方把需要的两个字段一起给全，比在表单里再查一次目录更直接。
pub(crate) struct IdentityOption {
    pub id: IdentityId,
    pub name: String,
    pub username: String,
}

/// 会话表单里的一个跳板机选项。
///
/// 带上主机、端口与自身的跳板引用，是为了在表单里就地渲染完整拓扑链
/// （`本机 -> 堡垒机 -> 网关 -> 目标`）；表单不回查目录。
#[derive(Clone)]
pub(crate) struct JumpOption {
    pub id: SessionId,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub jump_session_id: Option<SessionId>,
}

pub(crate) struct BuiltSession {
    pub session: Session,
    pub folder_path: Option<String>,
    pub password: Option<Zeroizing<String>>,
    pub key_passphrase: Option<Zeroizing<String>>,
}

/// SSH 连接构建结果，包含需要单独写入密钥环的敏感值。
pub(crate) struct SshConnectionBuild {
    pub connection: Connection,
    pub password: Option<Zeroizing<String>>,
    pub key_passphrase: Option<Zeroizing<String>>,
}

/// 表单初始值聚合，避免构造函数平铺过多参数。
pub(crate) struct SessionFormSeed {
    pub base: Option<Session>,
    pub connection: ConnectionChoice,
    pub auth: AuthChoice,
    pub save_password: bool,
    pub save_passphrase: bool,
    pub key_path: String,
    pub cert_path: String,
    pub folder_path: String,
    pub startup_commands: String,
    pub connect_timeout: String,
    pub keepalive: String,
    pub serial_port: String,
    pub serial_baud: String,
    pub serial_data_bits: SerialDataBits,
    pub serial_parity: SerialParity,
    pub serial_stop_bits: SerialStopBits,
    pub serial_flow_control: SerialFlowControl,
    pub telnet_encoding: TextEncoding,
    /// 可选凭据模板清单；表单只认这份快照，不回查目录。
    pub identities: Vec<IdentityOption>,
    /// 可选跳板机清单；同样只认快照。
    pub jump_options: Vec<JumpOption>,
    pub folder_options: Vec<String>,
    /// 全局代理快照。
    pub global_proxy: OutboundProxy,
    /// 语义块模式覆盖。
    pub block_view: BlockViewOverride,
}
