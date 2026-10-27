//! AppShell 依赖的各类模态弹窗与输入状态契约定义。

use crate::cloud_sync::SyncFormInputs;
use crate::forward::ForwardPanelState;
use crate::keys::KeyWizardState;
use crate::pane_tree::LayoutNode;
use crate::session_form::AuthChoice;
use crate::sftp::{
    SftpChmodModalState, SftpFilterModalState, SftpInputModalState, SftpPanelState,
    SftpPropertiesModalState, SftpSyncModalState,
};
use crate::sidebar::SidebarRow;
use crate::snippets::SnippetFormState;
use gpui_kit::component::input::{EditorState, InputState, TextareaState};
use gpui_kit::component::slider::SliderState;
use gpui_kit::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use transom_core::{
    AppearanceSettings, Connection, ConnectionId, FolderId, ForwardId, ForwardStatus, GroupId,
    HostKeyRecord, NetworkTraffic, PromptInfo, SecretKind, Session, SessionId, SystemStats, TabId,
    WindowBackgroundMode,
};
use transom_terminal::TerminalView;
use zeroize::Zeroizing;

pub(crate) enum TabStatus {
    Connecting,
    Connected,
    Failed(transom_core::AppError),
    Disconnected,
}

/// 标签组：一个标签栏条目下挂载的一棵分割树。
pub(crate) struct TabGroup {
    pub(crate) id: GroupId,
    pub(crate) session_id: SessionId,
    pub(crate) serial: usize,
    pub(crate) root: LayoutNode,
    pub(crate) active_pane: TabId,
    pub(crate) zoomed: Option<TabId>,
}

impl TabGroup {
    pub(crate) fn new(pane: TabId, session_id: SessionId, serial: usize) -> Self {
        Self {
            id: GroupId::new(),
            session_id,
            serial,
            root: LayoutNode::pane(pane),
            active_pane: pane,
            zoomed: None,
        }
    }

    pub(crate) fn pane_ids(&self) -> Vec<TabId> {
        self.root.pane_ids()
    }

    pub(crate) fn contains(&self, tab: TabId) -> bool {
        self.root.contains(tab)
    }

    pub(crate) fn pane_count(&self) -> usize {
        self.root.pane_count()
    }
}

pub(crate) struct TerminalTab {
    pub(crate) id: TabId,
    pub(crate) session_id: SessionId,
    pub(crate) session: Session,
    pub(crate) connection_id: Option<ConnectionId>,
    pub(crate) title: String,
    pub(crate) status: TabStatus,
    pub(crate) terminal: Option<Entity<TerminalView>>,
    pub(crate) sftp: Option<SftpPanelState>,
    pub(crate) forwards_panel: Option<ForwardPanelState>,
    pub(crate) forward_statuses: HashMap<ForwardId, ForwardStatus>,
    pub(crate) system_stats: Option<SystemStats>,
    pub(crate) network_traffic: Option<NetworkTraffic>,
    pub(crate) _startup_task: Option<Task<()>>,
    pub(crate) _subscription: Option<Subscription>,
    pub(crate) reconnect_attempts: u32,
    pub(crate) reconnect_task: Option<Task<()>>,
    pub(crate) zmodem: Option<crate::zmodem::ZmodemState>,
    pub(crate) player: Option<Entity<crate::cast_player::CastPlayerView>>,
}

impl TerminalTab {
    pub(crate) fn display_name(&self) -> String {
        if self.player.is_some() {
            return self.title.clone();
        }
        let name = self.session.name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        match &self.session.connection {
            Connection::Ssh(ssh) => format!("{}@{}", ssh.username, ssh.host),
            Connection::Telnet(telnet) => format!("{}:{}", telnet.host, telnet.port),
            Connection::Serial(serial) => serial.port.clone(),
            Connection::Local(local) => local.shell.clone().unwrap_or_else(|| "shell".to_string()),
            _ => self.title.clone(),
        }
    }

    pub(crate) fn pending(id: TabId, session: Session) -> Self {
        let title = session.name.clone();
        Self {
            id,
            session_id: session.id,
            session,
            connection_id: None,
            title,
            status: TabStatus::Connecting,
            terminal: None,
            sftp: None,
            forwards_panel: None,
            forward_statuses: HashMap::new(),
            system_stats: None,
            network_traffic: None,
            _startup_task: None,
            _subscription: None,
            reconnect_attempts: 0,
            reconnect_task: None,
            zmodem: None,
            player: None,
        }
    }
}

pub(crate) struct FolderForm {
    pub(crate) base: Option<FolderId>,
    pub(crate) parent: Option<FolderId>,
    pub(crate) name: Entity<InputState>,
    pub(crate) default_port: Entity<InputState>,
    pub(crate) default_username: Entity<InputState>,
    pub(crate) default_key_path: Entity<InputState>,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum BackupAction {
    Export(PathBuf),
    Import(PathBuf),
}

impl BackupAction {
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Export(path) | Self::Import(path) => path,
        }
    }

    pub(crate) fn is_export(&self) -> bool {
        matches!(self, Self::Export(_))
    }
}

pub(crate) struct BackupPromptState {
    pub(crate) action: BackupAction,
    pub(crate) passphrase: Entity<InputState>,
    pub(crate) confirm: Option<Entity<InputState>>,
    pub(crate) encrypt: bool,
    pub(crate) include_secrets: bool,
    pub(crate) encrypted_file: bool,
    pub(crate) error: Option<String>,
    pub(crate) busy: bool,
}

pub(crate) struct BatchEditForm {
    pub(crate) selected_count: usize,
    pub(crate) change_port: bool,
    pub(crate) port: Entity<InputState>,
    pub(crate) change_username: bool,
    pub(crate) username: Entity<InputState>,
    pub(crate) change_auth: bool,
    pub(crate) auth: AuthChoice,
    pub(crate) key_path: Entity<InputState>,
    pub(crate) save_passphrase: bool,
    pub(crate) error: Option<String>,
}

pub(crate) struct ShortcutCapture {
    pub(crate) id: &'static str,
    pub(crate) error: Option<String>,
    pub(crate) focus: FocusHandle,
}

pub(crate) struct PendingCredential {
    pub(crate) session_id: SessionId,
    pub(crate) owner: transom_core::SecretOwner,
    pub(crate) connection_id: ConnectionId,
    pub(crate) kind: SecretKind,
    pub(crate) value: Zeroizing<String>,
}

pub(crate) struct AuthPromptState {
    pub(crate) connection_id: ConnectionId,
    pub(crate) name: String,
    pub(crate) instructions: String,
    pub(crate) prompts: Vec<PromptInfo>,
    pub(crate) inputs: Vec<Entity<InputState>>,
    pub(crate) responder: tokio::sync::mpsc::Sender<Vec<String>>,
    pub(crate) error: Option<String>,
    pub(crate) session_id: SessionId,
    pub(crate) secret_kind: Option<SecretKind>,
    pub(crate) save: bool,
}

pub(crate) struct HostKeyConfirmState {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) key_type: String,
    pub(crate) fingerprint: String,
    pub(crate) responder: tokio::sync::mpsc::Sender<bool>,
}

pub(crate) struct KnownHostsModalState {
    pub(crate) open: bool,
    pub(crate) records: Vec<HostKeyRecord>,
    pub(crate) search: Entity<InputState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowAppearanceSnapshot {
    pub(crate) transparency: u8,
    pub(crate) background: WindowBackgroundMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProxyProtocol {
    Direct,
    Socks5,
    Http,
}

pub(crate) struct ProxyFormInputs {
    pub(crate) kind: ProxyProtocol,
    pub(crate) host: Entity<InputState>,
    pub(crate) port: Entity<InputState>,
    pub(crate) username: Entity<InputState>,
    pub(crate) password: Entity<InputState>,
}

pub(crate) struct SettingsModalState {
    pub(crate) open: bool,
    pub(crate) key_wizard: Option<KeyWizardState>,
    pub(crate) font_family: Entity<InputState>,
    pub(crate) ui_font_family: Entity<InputState>,
    pub(crate) panel_hidden_patterns: Entity<InputState>,
    pub(crate) max_concurrent_transfers: Entity<InputState>,
    pub(crate) cjk_font: Entity<InputState>,
    pub(crate) font_size: Entity<InputState>,
    pub(crate) line_height: Entity<InputState>,
    pub(crate) scrollback: Entity<InputState>,
    pub(crate) word_separators: Entity<InputState>,
    pub(crate) notify_threshold: Entity<InputState>,
    pub(crate) block_collapse_rows: Entity<InputState>,
    pub(crate) terminal_key_bindings: Entity<TextareaState>,
    pub(crate) highlight_keywords: Entity<InputState>,
    pub(crate) window_transparency: Entity<SliderState>,
    pub(crate) window_appearance_snapshot: Option<WindowAppearanceSnapshot>,
    pub(crate) appearance_snapshot: Option<AppearanceSettings>,
    pub(crate) sync_form: SyncFormInputs,
    pub(crate) proxy: ProxyFormInputs,
    pub(crate) identity_edit: Option<crate::identity::IdentityEditState>,
    pub(crate) trigger_edit: Option<crate::trigger_panel::TriggerEditState>,
}

pub(crate) struct SftpModals {
    pub(crate) chmod: Option<SftpChmodModalState>,
    pub(crate) input: Option<SftpInputModalState>,
    pub(crate) properties: Option<SftpPropertiesModalState>,
    pub(crate) filter: Option<SftpFilterModalState>,
    pub(crate) sync: Option<SftpSyncModalState>,
    pub(crate) goto_line: Option<EditorGotoLineModalState>,
}

pub(crate) struct EditorGotoLineModalState {
    pub(crate) editor: Entity<EditorState>,
    pub(crate) input: Entity<InputState>,
    pub(crate) error: Option<String>,
    pub(crate) _enter_sub: Subscription,
}

pub(crate) struct SnippetPanelState {
    pub(crate) open: bool,
    pub(crate) form: Option<SnippetFormState>,
}

pub(crate) struct ShortcutRecordingState {
    pub(crate) filter: Arc<parking_lot::RwLock<Vec<KeybindingKeystroke>>>,
    pub(crate) applied_keys: Vec<String>,
    pub(crate) capture: Option<ShortcutCapture>,
}

pub(crate) struct SidebarRowsCache {
    pub(crate) revision: u64,
    pub(crate) query: String,
    pub(crate) rows: std::rc::Rc<Vec<SidebarRow>>,
}
