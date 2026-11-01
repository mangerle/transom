//! 连接目标的统一展示口径。
//!
//! 设计原理（Why）：会话树、标签栏、纵向标签、命令面板与日志都要回答
//! “这条连接指向哪里、用什么图标”。此前每个入口各自拼 `user@host`，端口、
//! Mosh 前缀与本地 shell 的回落各不相同，改一处地址格式要改五处。
//!
//! 这里按用途分成三种口径：
//! - [`display_compact`]：不带端口的短标签，供标签/会话行回落；
//! - [`display_with_port`]：带端口的完整目标，供复制地址与命令面板；
//! - [`describe_target`]：带本地/串口本地化描述，供日志与状态文案。
//!
//! 代价与局限：图标依赖 `gpui-kit` 的 `IconName`，因此本模块留在 UI 层；
//! `transom-core` 只保留连接数据，不感知图标资产。

use gpui_kit::assets::IconName;
use transom_core::{Connection, Session};

/// 短标签：不带端口，本地会话回落 shell 名，串口回落端口名。
pub(crate) fn display_compact(connection: &Connection) -> String {
    match connection {
        Connection::Ssh(ssh) => format!("{}@{}", ssh.username, ssh.host),
        Connection::Mosh(mosh) => format!("{}@{}", mosh.username, mosh.host),
        Connection::Telnet(telnet) => format!("{}:{}", telnet.host, telnet.port),
        Connection::Local(local) => local.shell.clone().unwrap_or_else(|| "shell".to_string()),
        Connection::Serial(serial) => serial.port.clone(),
        _ => String::new(),
    }
}

/// 完整目标：带端口；本地/串口按各自口径。
pub(crate) fn display_with_port(connection: &Connection) -> String {
    match connection {
        Connection::Ssh(ssh) => format!("{}@{}:{}", ssh.username, ssh.host, ssh.port),
        Connection::Mosh(mosh) => format!("mosh {}@{}:{}", mosh.username, mosh.host, mosh.ssh_port),
        Connection::Telnet(telnet) => format!("{}:{}", telnet.host, telnet.port),
        Connection::Local(local) => rust_i18n::t!(
            "connection.local_terminal",
            shell = local.shell.as_deref().unwrap_or("sh")
        )
        .to_string(),
        Connection::Serial(serial) => rust_i18n::t!(
            "connection.serial",
            port = serial.port,
            baud = serial.baud_rate
        )
        .to_string(),
        _ => rust_i18n::t!("connection.unknown").to_string(),
    }
}

/// 日志/状态文案用的目标描述。
pub(crate) fn describe_target(session: &Session) -> String {
    display_with_port(&session.connection)
}

/// 连接类型图标：全应用共用一套映射。
pub(crate) fn icon(connection: &Connection) -> IconName {
    match connection {
        Connection::Local(_) => IconName::Terminal,
        Connection::Serial(_) => IconName::Cable,
        Connection::Telnet(_) => IconName::Network,
        Connection::Mosh(_) => IconName::Radio,
        _ => IconName::Server,
    }
}
