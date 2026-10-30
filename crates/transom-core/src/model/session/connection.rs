//! 串口、Telnet、本地 Shell 连接契约与连接总枚举。

use super::ssh::{AuthMethod, SessionOptions, SshConnection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct LocalConnection {
    pub shell: Option<String>,
    pub working_dir: Option<PathBuf>,
    /// 启动后直接执行的命令（`X-TerminalArgExec` / `-e <命令>`）。
    ///
    /// `None` 表示打开交互式 Shell；`Some(argv)` 表示在 PTY 里直接执行该
    /// 程序，argv 原样传递、不经过 Shell 拼串——launcher 已完成分词，再拼
    /// 一次会破坏含空格与引号的参数。一次性启动请求不会落盘，只有会话表单
    /// 保存的本地会话才可能长期携带它。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
}

impl LocalConnection {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Connection {
    Ssh(SshConnection),
    Local(LocalConnection),
    Serial(SerialConnection),
    Telnet(TelnetConnection),
    Mosh(MoshConnection),
}

/// Mosh（Mobile Shell）连接参数。
///
/// 核心特性：
/// 1. 引导阶段：通过 SSH 连接在远端启动 `mosh-server`；
/// 2. 漫游阶段：基于 UDP 与状态同步协议（SSP），在网络漫游切换与高丢包弱网下保持终端不中断并即时回显。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MoshConnection {
    /// 目标主机地址（域名或 IP）。
    pub host: String,
    /// 用于引导 mosh-server 的 SSH 端口，默认 22。
    #[serde(default = "default_mosh_ssh_port")]
    pub ssh_port: u16,
    /// 登录用户名。
    pub username: String,
    /// 远端 mosh-server UDP 端口；若为 None 则由远端自动分配（默认 60000-61000 范围）。
    #[serde(default)]
    pub server_port: Option<u16>,
    /// 远端 mosh-server 执行路径，默认 "mosh-server"。
    #[serde(default = "default_mosh_server_path")]
    pub server_path: String,
    /// SSH 引导认证方式。
    pub auth: AuthMethod,
    /// SSH 引导连接选项。
    #[serde(default)]
    pub options: SessionOptions,
}

fn default_mosh_ssh_port() -> u16 {
    22
}

fn default_mosh_server_path() -> String {
    "mosh-server".to_string()
}

impl MoshConnection {
    pub fn new(host: impl Into<String>, username: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            ssh_port: 22,
            username: username.into(),
            server_port: None,
            server_path: default_mosh_server_path(),
            auth: AuthMethod::Agent,
            options: SessionOptions::default(),
        }
    }
}

/// 串口连接参数，默认 115200 8N1、无流控。
///
/// 数据位、校验位、停止位与流控使用枚举而非裸整数，使非法组合在反序列化
/// 阶段即被拒绝；波特率保留为 `u32`，以支持厂商自定义的非标准速率。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialConnection {
    pub port: String,
    pub baud_rate: u32,
    #[serde(default)]
    pub data_bits: SerialDataBits,
    #[serde(default)]
    pub parity: SerialParity,
    #[serde(default)]
    pub stop_bits: SerialStopBits,
    #[serde(default)]
    pub flow_control: SerialFlowControl,
}

impl SerialConnection {
    pub fn new(port: impl Into<String>, baud_rate: u32) -> Self {
        Self {
            port: port.into(),
            baud_rate,
            ..Self::default()
        }
    }
}

impl Default for SerialConnection {
    fn default() -> Self {
        Self {
            port: String::new(),
            baud_rate: 115_200,
            data_bits: SerialDataBits::Eight,
            parity: SerialParity::None,
            stop_bits: SerialStopBits::One,
            flow_control: SerialFlowControl::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SerialDataBits {
    Five,
    Six,
    Seven,
    #[default]
    Eight,
}

impl SerialDataBits {
    pub const ALL: [Self; 4] = [Self::Five, Self::Six, Self::Seven, Self::Eight];

    pub fn bits(self) -> u8 {
        match self {
            Self::Five => 5,
            Self::Six => 6,
            Self::Seven => 7,
            Self::Eight => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SerialParity {
    #[default]
    None,
    Odd,
    Even,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SerialStopBits {
    #[default]
    One,
    Two,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SerialFlowControl {
    #[default]
    None,
    Software,
    Hardware,
}

/// Telnet 字符编码；决定输出与输入两个方向的字节流转码目标。
///
/// 国内部分老网络设备与 BBS 站点使用 GBK/Big5，直连会乱码；转码在传输边界
/// 一次完成，终端仿真层始终只处理 UTF-8 字节流。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TextEncoding {
    #[default]
    Utf8,
    Gbk,
    Big5,
}

/// Telnet 连接参数；协议无内建认证，登录交由终端内交互完成。
///
/// 端口默认 23；编码与选项均可缺省，老会话文件按默认值降级。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelnetConnection {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub encoding: TextEncoding,
    #[serde(default)]
    pub options: TelnetOptions,
}

impl TelnetConnection {
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 23,
            encoding: TextEncoding::default(),
            options: TelnetOptions::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelnetOptions {
    pub connect_timeout_secs: u64,
    /// `0` disables keepalive NOP messages.
    pub keepalive_secs: u64,
    /// 意外断线后自动重连（与 SSH 相同的退避策略）。
    /// 老会话文件没有该字段，按关闭降级。
    #[serde(default)]
    pub auto_reconnect: bool,
}

impl Default for TelnetOptions {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 15,
            keepalive_secs: 0,
            auto_reconnect: false,
        }
    }
}
