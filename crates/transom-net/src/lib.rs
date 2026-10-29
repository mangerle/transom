//! SSH transport, authentication, SFTP, and port forwarding.

mod auth;
mod bridge;
mod client;
mod error;
mod forward;
mod host;
mod keys;
pub mod mosh;
mod proxy;
mod serial;
pub mod sftp;
pub mod system_stats;
mod telnet;
mod timeout;
mod traffic;

pub use auth::{AuthPrompter, PasswordPrompter};
pub use client::{CommandOutput, SshConnectParams, SshRemote, SshSession, connect_ssh};
pub use error::NetError;
pub use forward::{ForwardTask, start_forward};
pub use host::{probe_tcp, send_wake_on_lan};
pub use keys::deploy_public_key;
pub use mosh::{
    MoshSessionInfo, build_mosh_server_command, check_local_mosh_client, parse_mosh_server_output,
};
pub use serial::{SerialSession, SystemSerialPortProvider, open_serial};
pub use sftp::SftpClient;
pub use sftp::check_file::{
    CHECK_FILE_HANDLE_EXT, CHECK_FILE_NAME_EXT, CheckFileRequest, CheckFileResponse,
    decode_check_file_reply,
};
pub use system_stats::StatsSamplerState;
pub use telnet::{TelnetSession, connect_telnet};
