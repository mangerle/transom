//! 工作区共享的领域模型、协议契约与终端传输抽象。
//!
//! 目录组织（Why）：模块按子域分治——`model`（领域模型与算法）、
//! `protocol`（线格式、同步与采样解析）、`transport`（终端字节流与控制
//! 句柄契约）；平台路径/权限与加解密信封分别在 `transom-platform` 与
//! `transom-crypto`。根模块只保留最基础的应用契约（命令/事件、错误、ID、
//! 存储 trait、备份文档）。

pub mod backup;
pub mod commands;
pub mod error;
pub mod ids;
pub mod model;
pub mod prompt;
pub mod protocol;
pub mod secrets;
pub mod storage;
pub mod transport;

pub use async_trait::async_trait;

pub use backup::{
    BackupDocument, BackupExportOptions, BackupExportReport, BackupImportOptions, BackupSecret,
    SecretRestoreReport, collect_catalog_secrets,
};
pub use commands::{
    AppCommand, AppEvent, ConnectParams, DeepLinkRequest, ForwardAppEvent, ForwardCommand,
    HostAppEvent, HostCommand, SftpAppEvent, SftpCommand, SyncAppEvent, SyncCommand, SystemStats,
    TerminalAppEvent, TerminalCommand, ZmodemAppEvent, ZmodemCommand,
};
pub use error::{AppError, ErrorCode};
pub use ids::{
    ConnectionId, FolderId, ForwardId, GroupId, IdentityId, SessionId, SnippetId, SplitId, TabId,
    TransferId, TriggerId,
};
pub use model::catalog::{
    CatalogError, DropSide, FolderInheritedDefaults, MAX_PROXY_JUMP_DEPTH, MergePreference,
    SessionCatalog, SessionFolder, SyncMergeReport, merge_catalogs,
};
pub use model::color::Rgb;
pub use model::file_diff::{DEFAULT_CONTEXT_LINES, DiffHunk, FileDiff, MergeDirection};
pub use model::forward::{
    BindAddress, ForwardKind, ForwardRule, ForwardState, ForwardStatus, SocksAuth, SocksOptions,
    TargetAddress,
};
pub use model::history::{
    HISTORY_LIMIT, SUGGESTION_LIMIT, record as record_command, suggestions as command_suggestions,
};
pub use model::host::{HostAction, HostOutcome};
pub use model::identity::Identity;
pub use model::known_hosts::{
    HostKeyInfo, HostKeyRecord, HostKeyStatus, KnownHostsStore, normalize_host,
};
pub use model::layout::{ActivityBarLayout, ActivityItemId, StatusBarLayout, StatusItemId};
pub use model::mac::{MacAddress, MacParseError, normalize_mac};
pub use model::serial::{SerialPortDescriptor, SerialPortProvider};
pub use model::session::{
    AuthMethod, BlockViewOverride, Connection, LocalConnection, MoshConnection, ProxyJumpConfig,
    SerialConnection, SerialDataBits, SerialFlowControl, SerialParity, SerialStopBits, Session,
    SessionOptions, SshConnection, SshHop, StartupCommands, TelnetConnection, TelnetOptions,
    TextEncoding,
};
pub use model::session_appearance::{SessionColor, SessionIcon};
pub use model::settings::{
    AppSettings, AppearanceSettings, AuditSettings, BellStyle, BlockViewSettings, BlockViewStyle,
    DividerColor, DividerSettings, DividerWidth, GitSyncSettings, HistorySettings, LogLevel,
    LogSettings, MonitorSettings, PanelSide, SavedLayout, SavedTabGroup, SavedTabTiling,
    SftpSettings, ShortcutSettings, SidebarStartup, Snippet, SnippetSettings, SplitAxis,
    StartupTabs, SyncEndpoint, SyncSettings, TabPosition, UiDensity, UiThemeMode,
    WindowBackgroundMode, WindowSettings,
};
pub use model::terminal::TerminalHighlight;
pub use model::terminal::{
    FOLLOW_UI_THEME, RightClickBehavior, TerminalCursorShape, TerminalInteraction,
    TerminalKeyBinding, TerminalPalette, TerminalProfile,
};
pub use model::theme::{PaletteMode, PresetTheme, preset_themes};
pub use model::trigger::{TriggerAction, TriggerRule, TriggerSettings};
pub use prompt::{PromptInfo, PromptKind};
pub use protocol::proxy::{OutboundProxy, ProxyAuth};
pub use protocol::sftp::{
    ChecksumAlgorithm, PreviewMode, RemoteDeleteTarget, RemoteFile, RemoteMetadata,
    RemoteSearchQuery, RemoteSearchResultItem, RemoteTextMeta, SftpOperation, SudoCapability,
    SudoUnavailable, TransferDirection, TransferSummary, VerifyMode, format_octal_permissions,
    format_rwx_permissions, is_safe_file_name, parse_octal_permissions, safe_local_join,
};
pub use protocol::sftp_filter::{
    DEFAULT_TRANSFER_EXCLUDES, TransferFilter, TransferMatcher, validate_transfer_filter,
};
pub use protocol::sftp_sync::{
    DirComparison, DirDiffEntry, DirDiffStatus, DirEntryMeta, DirSnapshotEntry, SyncDirection,
    SyncItem, build_dir_comparison,
};
pub use protocol::ssh_config::{parse_ssh_config, parse_ssh_config_with_home};
pub use protocol::ssh_url::{
    DEFAULT_SSH_PORT, SshTarget, is_ssh_url, looks_like_host, parse_ssh_target,
    quick_connect_session,
};
pub use protocol::sync::{
    CloudSyncTarget, GitSyncResult, SYNC_MAX_DOCUMENT_BYTES, SYNC_OBJECT_NAME, SyncPushReport,
    is_sync_conflict,
};
pub use protocol::system_stats::{
    CoreUsage, FailedServiceInfo, FilesystemUsage, InterfaceRate, OomAlert, PortListeningInfo,
    ProcessInfo, ProcessSort,
};
pub use protocol::traffic::{NetworkTraffic, TrafficCounter};
pub use protocol::zmodem::{ZmodemDirection, ZmodemEvent, ZmodemFinished, ZmodemSettings};
pub use secrets::{ConnectionSecrets, SecretKey, SecretKind, SecretOwner};
pub use storage::{BackupService, HistoryStore, SecretStore, SessionStore, SettingsStore};
pub use transport::{
    SessionCleanup, TerminalControl, TerminalControlHandle, TerminalIo, TerminalSize,
};
