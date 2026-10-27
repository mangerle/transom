//! SSH transport, authentication, SFTP, and port forwarding.

mod auth;
mod bridge;
mod client;
mod error;
mod forward;
mod host;
mod keys;
mod proxy;
mod serial;
mod sftp;
mod sftp_dir;
mod sftp_hash;
mod sftp_scan;
mod sftp_sync;
mod sftp_text;
mod telnet;
mod telnet_iac;
mod traffic;
mod zmodem;

pub use auth::{AuthPrompter, PasswordPrompter};
pub use client::{SshConnectParams, SshRemote, SshSession, connect_ssh};
pub use error::NetError;
pub use forward::{ForwardTask, start_forward};
pub use host::{probe_tcp, send_wake_on_lan};
pub use keys::deploy_public_key;
pub use serial::{SerialSession, SystemSerialPortProvider, open_serial};
pub use sftp::SftpClient;
pub use telnet::{TelnetSession, connect_telnet};
pub use zmodem::{ZmodemConfig, ZmodemControl, attach_zmodem};
