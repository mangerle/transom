//! 工作区共享的领域模型、协议契约与跨平台抽象。
//!
//! 目录组织（Why）：模块按子域分治——`model`（领域模型与算法）、
//! `protocol`（线格式与同步）、`platform`（平台路径与权限）、`crypto`
//! （加解密信封）；根模块只保留最基础的应用契约（命令/事件、错误、ID、
//! 存储 trait、备份文档）。历史路径（`transom_core::session` 等）由别名
//! re-export 保持可用，调用方与 crate 内引用都不需要跟随目录调整。

pub mod backup;
pub mod commands;
pub mod crypto;
pub mod error;
mod highlight;
pub mod ids;
pub mod model;
pub mod platform;
pub mod prompt;
pub mod protocol;
pub mod secrets;
pub mod storage;

// 历史路径别名：`crate::session`、`transom_core::sftp` 等全部继续解析到
// 迁移后的模块；这是纯目录整理，不改调用方的任何一行。
pub use async_trait::async_trait;
pub use model::{
    catalog, color, forward, history, host, identity, keys, known_hosts, layout, mac, serial,
    session, session_appearance, settings, terminal, theme, trigger,
};
pub use platform::{paths, private_fs};
pub use protocol::{
    merge, proxy, sftp, sftp_filter, sftp_sync, ssh_config, ssh_url, sync, system_stats, traffic,
    vfs, zmodem,
};
pub use vfs::{VfsEntry, VirtualFileSystem, get_parent_path};

pub use backup::{
    BackupDocument, BackupExportOptions, BackupExportReport, BackupImportOptions, BackupSecret,
    SecretRestoreReport, collect_catalog_secrets,
};
pub use catalog::{
    CatalogError, CatalogMergeSummary, DropSide, FolderInheritedDefaults, MAX_PROXY_JUMP_DEPTH,
    SessionCatalog, SessionFolder,
};
pub use color::Rgb;
pub use commands::{
    AppCommand, AppEvent, ConnectParams, DeepLinkRequest, ForwardAppEvent, ForwardCommand,
    HostAppEvent, HostCommand, SftpAppEvent, SftpCommand, SyncAppEvent, SyncCommand, SystemStats,
    TerminalAppEvent, TerminalCommand, ZmodemAppEvent, ZmodemCommand,
};
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
    format_octal_permissions, format_rwx_permissions, is_safe_file_name, parse_octal_permissions,
    safe_local_join,
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
