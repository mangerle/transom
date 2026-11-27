//! 会话模型、连接类型与认证配置契约。

mod connection;
mod entity;
mod ssh;

#[cfg(test)]
mod tests;

pub use connection::{
    Connection, FtpConnection, FtpSecurity, LocalConnection, MoshConnection, RdpColorDepth,
    RdpConnection, RdpDesktopSize, RdpPerformance, RdpSecurity, SerialConnection, SerialDataBits,
    SerialFlowControl, SerialParity, SerialStopBits, TelnetConnection, TelnetOptions, TextEncoding,
};
pub use entity::{BlockViewOverride, Session, StartupCommands};
pub use ssh::{AuthMethod, HostProtection, ProxyJumpConfig, SessionOptions, SshConnection, SshHop};
