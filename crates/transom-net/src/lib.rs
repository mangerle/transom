//! SSH transport, authentication, SFTP, and port forwarding.

mod auth;
mod bridge;
mod client;
mod forward;
mod keys;
mod serial;
mod sftp;
mod sftp_dir;
mod sftp_hash;
mod sftp_scan;
mod sftp_sync;
mod traffic;

pub use auth::{AuthPrompter, PasswordPrompter};
pub use client::{SshRemote, SshSession, connect_ssh};
pub use forward::{ForwardTask, start_forward};
pub use keys::deploy_public_key;
pub use serial::{SerialSession, SystemSerialPortProvider, open_serial};
pub use sftp::SftpClient;
