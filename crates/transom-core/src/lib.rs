//! Domain models and application-facing contracts shared by the workspace.

pub mod color;
pub mod commands;
pub mod error;
pub mod forward;
pub mod ids;
pub mod known_hosts;
pub mod prompt;
pub mod secrets;
pub mod session;
pub mod settings;
pub mod sftp;
pub mod ssh_config;
pub mod storage;
pub mod system_stats;
pub mod terminal;
pub mod theme;

pub use color::Rgb;
pub use commands::{AppCommand, AppEvent, SystemStats};
pub use error::{AppError, ErrorCode};
pub use forward::{
    BindAddress, ForwardKind, ForwardRule, ForwardState, ForwardStatus, SocksAuth, SocksOptions,
    TargetAddress,
};
pub use ids::{ConnectionId, ForwardId, SessionId, TabId, TransferId};
pub use known_hosts::{HostKeyInfo, HostKeyRecord, HostKeyStatus, KnownHostsStore, normalize_host};
pub use prompt::PromptInfo;
pub use secrets::{ConnectionSecrets, SecretKey, SecretKind};
pub use session::{
    AuthMethod, Connection, LocalConnection, Session, SessionOptions, SshConnection,
};
pub use settings::{AppSettings, BellStyle, WindowSettings};
pub use sftp::{
    RemoteFile, SftpOperation, TransferDirection, TransferMode, TransferStatus, TransferTask,
    format_octal_permissions, format_rwx_permissions, parse_octal_permissions,
};
pub use ssh_config::{parse_ssh_config, parse_ssh_config_with_home};
pub use storage::{SecretStore, SessionStore, SettingsStore};
pub use terminal::{
    TerminalControl, TerminalControlHandle, TerminalInteraction, TerminalIo, TerminalPalette,
    TerminalProfile, TerminalSize,
};
pub use theme::{PresetTheme, preset_themes};
