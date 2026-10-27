//! 会话模型、连接类型与认证配置契约。

mod connection;
mod entity;
mod ssh;

#[cfg(test)]
mod tests;

pub use connection::{
    Connection, LocalConnection, SerialConnection, SerialDataBits, SerialFlowControl, SerialParity,
    SerialStopBits, TelnetConnection, TelnetOptions, TextEncoding,
};
pub use entity::{BlockViewOverride, Session, StartupCommands};
pub use ssh::{AuthMethod, ProxyJumpConfig, SessionOptions, SshConnection, SshHop};
