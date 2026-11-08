//! SSH transport, authentication, SFTP, and port forwarding.

mod agent;
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

pub use auth::{AuthPrompter, AutoTrustKnownHosts, PasswordPrompter};
pub use bridge::AsyncChannelStream;
pub use client::{
    CommandOutput, SshConnectParams, SshRemote, SshRemoteConnectParams, SshSession, connect_ssh,
    connect_ssh_remote,
};
pub use error::NetError;
pub use forward::{ForwardTask, start_forward};
pub use host::{probe_tcp, send_wake_on_lan};
pub use keys::deploy_public_key;
pub use mosh::{
    MoshClientCommand, MoshSessionInfo, build_mosh_server_command, local_mosh_client,
    parse_mosh_server_output,
};
pub use serial::{SerialSession, SystemSerialPortProvider, open_serial};
pub use sftp::SftpClient;
pub use sftp::check_file::{
    CHECK_FILE_HANDLE_EXT, CHECK_FILE_NAME_EXT, CheckFileRequest, CheckFileResponse,
    decode_check_file_reply,
};
pub use sftp::sudo_write::{SudoWriteReport, SudoWriteRequest, probe_sudo, sudo_write};
pub use system_stats::StatsSamplerState;
pub use telnet::{TelnetSession, connect_telnet};
