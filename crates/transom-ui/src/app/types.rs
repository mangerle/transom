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

/// 终端标签窗格的具体内容形态。
///
/// 遵循“非法状态不可表示 (Make Illegal States Unrepresentable)”设计原则：
/// 标签页要么是承载实时通信连接的会话窗格（终端视图、SFTP、端口转发等），
/// 要么是独立的录像回放播放器，二者在类型系统中严格互斥。
pub(crate) enum TabContent {
    /// 实时通信会话窗格（包含终端引擎与伴随子面板）。
    Session(Box<SessionPaneState>),
    /// 录像回放播放器。
    Player(Entity<crate::cast_player::CastPlayerView>),
}

/// 实时通信会话窗格关联的所有运行期状态。
#[derive(Default)]
pub(crate) struct SessionPaneState {
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
}

pub(crate) struct TerminalTab {
    pub(crate) id: TabId,
    pub(crate) session_id: SessionId,
    pub(crate) session: Session,
    pub(crate) connection_id: Option<ConnectionId>,
    pub(crate) title: String,
    pub(crate) status: TabStatus,
    pub(crate) content: TabContent,
}

impl TerminalTab {
    /// 是否为录像播放器标签。
    pub(crate) fn is_player(&self) -> bool {
        matches!(self.content, TabContent::Player(_))
    }

    /// 获取关联的录像播放器视图句柄（若存在）。
    pub(crate) fn player(&self) -> Option<Entity<crate::cast_player::CastPlayerView>> {
        match &self.content {
            TabContent::Player(player) => Some(player.clone()),
            TabContent::Session(_) => None,
        }
    }

    /// 获取会话状态不可变引用（若当前为会话标签）。
    pub(crate) fn session_pane(&self) -> Option<&SessionPaneState> {
        match &self.content {
            TabContent::Session(pane) => Some(pane),
            TabContent::Player(_) => None,
        }
    }

    /// 获取会话状态可变引用（若当前为会话标签）。
    pub(crate) fn session_pane_mut(&mut self) -> Option<&mut SessionPaneState> {
        match &mut self.content {
            TabContent::Session(pane) => Some(pane),
            TabContent::Player(_) => None,
        }
    }

    /// 快捷访问终端视图引用。
    pub(crate) fn terminal(&self) -> Option<Entity<TerminalView>> {
        self.session_pane().and_then(|p| p.terminal.clone())
    }

    /// 快捷访问 SFTP 面板引用。
    pub(crate) fn sftp(&self) -> Option<&SftpPanelState> {
        self.session_pane().and_then(|p| p.sftp.as_ref())
    }

    /// 快捷访问 SFTP 面板可变引用。
    pub(crate) fn sftp_mut(&mut self) -> Option<&mut SftpPanelState> {
        self.session_pane_mut().and_then(|p| p.sftp.as_mut())
    }

    /// 快捷访问端口转发面板引用。
    pub(crate) fn forwards_panel(&self) -> Option<&ForwardPanelState> {
        self.session_pane().and_then(|p| p.forwards_panel.as_ref())
    }

    /// 快捷访问端口转发面板可变引用。
    pub(crate) fn forwards_panel_mut(&mut self) -> Option<&mut ForwardPanelState> {
        self.session_pane_mut()
            .and_then(|p| p.forwards_panel.as_mut())
    }

    /// 快捷访问 Zmodem 传输状态。
    pub(crate) fn zmodem(&self) -> Option<&crate::zmodem::ZmodemState> {
        self.session_pane().and_then(|p| p.zmodem.as_ref())
    }

    /// 快捷访问系统资源监控数据。
    pub(crate) fn system_stats(&self) -> Option<&SystemStats> {
        self.session_pane().and_then(|p| p.system_stats.as_ref())
    }

    /// 快捷访问网络流量监控数据。
    pub(crate) fn network_traffic(&self) -> Option<&NetworkTraffic> {
        self.session_pane().and_then(|p| p.network_traffic.as_ref())
    }

    pub(crate) fn display_name(&self) -> String {
        if self.is_player() {
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

    /// 构造处于连接等待中的实时通信终端标签。
    pub(crate) fn pending(id: TabId, session: Session) -> Self {
        let title = session.name.clone();
        Self {
            id,
            session_id: session.id,
            session,
            connection_id: None,
            title,
            status: TabStatus::Connecting,
            content: TabContent::Session(Box::default()),
        }
    }

    /// 构造已就绪的录像回放播放器标签。
    pub(crate) fn player_tab(
        id: TabId,
        session: Session,
        title: String,
        player: Entity<crate::cast_player::CastPlayerView>,
    ) -> Self {
        Self {
            id,
            session_id: session.id,
            session,
            connection_id: None,
            title,
            status: TabStatus::Connected,
            content: TabContent::Player(player),
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
