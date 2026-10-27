//! Domain models and application-facing contracts shared by the workspace.

pub mod backup;
pub mod catalog;
pub mod color;
pub mod commands;
pub mod error;
pub mod forward;
mod highlight;
pub mod history;
pub mod host;
pub mod identity;
pub mod ids;
pub mod keys;
pub mod known_hosts;
pub mod layout;
pub mod mac;
pub mod merge;
pub mod paths;
pub mod prompt;
pub mod proxy;
pub mod secrets;
pub mod serial;
pub mod session;
pub mod session_appearance;
pub mod settings;
pub mod sftp;
pub mod sftp_filter;
pub mod sftp_sync;
pub mod ssh_config;
pub mod ssh_url;
pub mod storage;
pub mod sync;
pub mod system_stats;
pub mod terminal;
pub mod theme;
pub mod traffic;
pub mod trigger;
pub mod zmodem;

pub use backup::{
    BackupDocument, BackupExportOptions, BackupExportReport, BackupImportOptions, BackupSecret,
    SecretRestoreReport, collect_catalog_secrets,
};
pub use catalog::{
    CatalogError, CatalogMergeSummary, DropSide, FolderInheritedDefaults, MAX_PROXY_JUMP_DEPTH,
    SessionCatalog, SessionFolder,
};
pub use color::Rgb;
pub use commands::{AppCommand, AppEvent, DeepLinkRequest, SystemStats};
pub use error::{AppError, ErrorCode};
pub use forward::{
    BindAddress, ForwardKind, ForwardRule, ForwardState, ForwardStatus, SocksAuth, SocksOptions,
    TargetAddress,
};
pub use highlight::{CompiledHighlight, HighlightKind, HighlightSpan, TerminalHighlight};
pub use history::{
    HISTORY_LIMIT, SUGGESTION_LIMIT, record as record_command, suggestions as command_suggestions,
};
pub use host::{HostAction, HostOutcome};
pub use identity::Identity;
pub use ids::{
    ConnectionId, FolderId, ForwardId, GroupId, IdentityId, SessionId, SnippetId, SplitId, TabId,
    TransferId, TriggerId,
};
pub use keys::{GeneratedKey, KeyAlgorithm, PublicKeyInfo, generate_keypair, scan_public_keys};
pub use known_hosts::{HostKeyInfo, HostKeyRecord, HostKeyStatus, KnownHostsStore, normalize_host};
pub use layout::{ActivityBarLayout, ActivityItemId, StatusBarLayout, StatusItemId};
pub use mac::{MacAddress, MacParseError, normalize_mac};
pub use merge::{SyncMergeReport, merge_catalogs};
pub use prompt::{PromptInfo, PromptKind};
pub use proxy::{OutboundProxy, ProxyAuth};
pub use secrets::{ConnectionSecrets, SecretKey, SecretKind, SecretOwner};
pub use serial::{SerialPortDescriptor, SerialPortProvider};
pub use session::{
    AuthMethod, BlockViewOverride, Connection, LocalConnection, ProxyJumpConfig, SerialConnection,
    SerialDataBits, SerialFlowControl, SerialParity, SerialStopBits, Session, SessionOptions,
    SshConnection, SshHop, StartupCommands, TelnetConnection, TelnetOptions, TextEncoding,
};
pub use session_appearance::{SessionColor, SessionIcon};
pub use settings::{
    AppSettings, AppearanceSettings, AuditSettings, BellStyle, BlockViewSettings, BlockViewStyle,
    HistorySettings, LogLevel, LogSettings, PanelSide, SavedLayout, SavedTabGroup, SavedTabTiling,
    SftpSettings, ShortcutSettings, SidebarStartup, Snippet, SnippetSettings, SplitAxis,
    StartupTabs, UiDensity, UiThemeMode, WindowBackgroundMode, WindowSettings,
};
pub use sftp::{
    PreviewMode, RemoteDeleteTarget, RemoteFile, RemoteMetadata, RemoteTextMeta, SftpOperation,
    TransferDirection, TransferKind, TransferMode, TransferStatus, TransferSummary, TransferTask,
    format_octal_permissions, format_rwx_permissions, parse_octal_permissions,
};
pub use sftp_filter::{
    DEFAULT_TRANSFER_EXCLUDES, TransferFilter, TransferMatcher, validate_transfer_filter,
};
pub use sftp_sync::{
    DirComparison, DirDiffEntry, DirDiffStatus, DirEntryMeta, DirSnapshotEntry, SyncDirection,
    SyncItem, build_dir_comparison,
};
pub use ssh_config::{parse_ssh_config, parse_ssh_config_with_home};
pub use ssh_url::{
    DEFAULT_SSH_PORT, SshTarget, is_ssh_url, looks_like_host, parse_ssh_target,
    quick_connect_session,
};
pub use storage::{BackupService, HistoryStore, SecretStore, SessionStore, SettingsStore};
pub use sync::{
    CloudSyncTarget, GitSyncResult, GitSyncSettings, SYNC_MAX_DOCUMENT_BYTES, SYNC_OBJECT_NAME,
    SyncEndpoint, SyncPushReport, SyncSettings, is_sync_conflict,
};
pub use system_stats::{CoreUsage, FilesystemUsage, InterfaceRate, ProcessInfo, StatsSamplerState};
pub use terminal::{
    FOLLOW_UI_THEME, RightClickBehavior, TerminalControl, TerminalControlHandle,
    TerminalCursorShape, TerminalInteraction, TerminalIo, TerminalKeyBinding, TerminalPalette,
    TerminalProfile, TerminalSize,
};
pub use theme::{PaletteMode, PresetTheme, preset_themes};
pub use traffic::{NetworkTraffic, TrafficCounter};
pub use trigger::{TriggerAction, TriggerRule, TriggerSettings};
pub use zmodem::{ZmodemDirection, ZmodemEvent, ZmodemFinished, ZmodemSettings};
