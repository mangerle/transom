//! GPUI 顶层应用外壳状态、快捷键动作委托与主布局容器。

use crate::UiServices;
use crate::forward::{ForwardPanelState, PendingForwardConfirm};
use crate::keymap;
use crate::keys::KeyWizardState;
use crate::palette::{Palette, localized_error};
use crate::pane_tree::LayoutNode;
use crate::session_form::{AuthChoice, SessionForm};
use crate::sftp::{
    SftpChmodModalState, SftpEditorModalState, SftpFilterModalState, SftpInputModalState,
    SftpPanelState, SftpPropertiesModalState, SftpSyncModalState,
};
use crate::snippets::SnippetEditor;
use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::InputState;
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use transom_core::{
    AppError, AppSettings, Connection, ConnectionId, FolderId, ForwardId, ForwardStatus, GroupId,
    HostKeyRecord, NetworkTraffic, PromptInfo, SecretKind, Session, SessionCatalog, SessionId,
    SplitId, SystemStats, TabId,
};
use transom_terminal::TerminalView;
use zeroize::Zeroizing;

pub(crate) enum TabStatus {
    Connecting,
    Connected,
    Failed(AppError),
    Disconnected,
}

/// 标签组：一个标签栏条目下挂载的一棵分割树。
///
/// 布局交给 [`LayoutNode`] 二叉分割树，方向因此是"每一刀"的属性而不是整组的
/// 属性：切分焦点窗格不会重排兄弟，摘除窗格会收起退化的父节点。代价是焦点、
/// 生命周期都要沿树查询，这部分复杂度由 `pane_tree` 与 `workspace` 吸收。
pub(crate) struct TabGroup {
    pub(crate) id: GroupId,
    /// 组内所有窗格共用的会话配置，用于给同会话的并列标签分配序号。
    pub(crate) session_id: SessionId,
    /// 同会话的额外标签序号：0 表示该会话的原始标签（不显示后缀），之后依次 1、2...
    /// 创建时定下、永不回改已有标签，见 `AppShell::create_group`。
    pub(crate) serial: usize,
    /// 分割树根节点，至少含一个窗格叶子。
    pub(crate) root: LayoutNode,
    pub(crate) active_pane: TabId,
    /// 单窗格最大化目标；`None` 表示按分割树铺满整个内容区。
    ///
    /// 用独立字段而不是改写分割树，是为了让"恢复分屏"零成本：布局与用户
    /// 拖出的比例都原样保留，只是渲染时暂时只看一个叶子。
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

    /// 组内全部窗格，按视觉顺序（从左到右、从上到下）。
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
    /// 运行期标题：初值取会话名，连接建立后会被远端 shell 的 OSC 序列覆盖。
    /// 只供 SFTP 目录联动等内部解析使用，界面显示一律走 `display_name`。
    pub(crate) title: String,
    pub(crate) status: TabStatus,
    pub(crate) terminal: Option<Entity<TerminalView>>,
    pub(crate) sftp: Option<SftpPanelState>,
    pub(crate) forwards_panel: Option<ForwardPanelState>,
    pub(crate) forward_statuses: HashMap<ForwardId, ForwardStatus>,
    pub(crate) system_stats: Option<SystemStats>,
    /// 当前 SSH 连接的传输层实时速率；本地终端为 `None`。
    pub(crate) network_traffic: Option<NetworkTraffic>,
    /// 初始化命令的后台调度任务；重连或关标签时丢弃以取消。
    pub(crate) _startup_task: Option<Task<()>>,
    pub(crate) _subscription: Option<Subscription>,
}

impl TerminalTab {
    /// 标签栏、窗格头、拖拽预览与状态栏显示的会话名称。
    ///
    /// 为什么不复用 `title`：`title` 是运行期标题，会被远端 `PS1` 里的 OSC 序列改写，
    /// 拿它当标签名会出现"标签跟着 cd 变名"，而且同会话多标签也无法靠它区分。
    /// 会话名在表单里是必填项，因此兜底分支只对手工编辑的目录文件生效：
    /// SSH 用 `用户@主机`，串口用设备名，本地终端用 shell 名。
    pub(crate) fn display_name(&self) -> String {
        let name = self.session.name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        match &self.session.connection {
            Connection::Ssh(ssh) => format!("{}@{}", ssh.username, ssh.host),
            Connection::Serial(serial) => serial.port.clone(),
            Connection::Local(local) => local.shell.clone().unwrap_or_else(|| "shell".to_string()),
            // `Connection` 标记为 non-exhaustive：将来新增连接类型时退回运行期标题，
            // 总比标签显示空白好。
            _ => self.title.clone(),
        }
    }

    /// 建立一个尚未连接的窗格：连接、终端视图与事件订阅由 `start_connection` 补齐。
    ///
    /// 字段多且初始值一律为空，集中一处构造可避免新建、分屏、窗口恢复三条路径漏项。
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
        }
    }
}

/// 新建或重命名文件夹的轻量模态表单。
pub(crate) struct FolderForm {
    /// 存在时表示重命名该文件夹，否则为新建。
    pub(crate) base: Option<FolderId>,
    /// 新建时的父文件夹；`None` 表示根层级。
    pub(crate) parent: Option<FolderId>,
    pub(crate) name: Entity<InputState>,
    pub(crate) error: Option<String>,
}

/// 备份导入或导出目标。
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

/// 备份弹窗状态。
pub(crate) struct BackupPromptState {
    pub(crate) action: BackupAction,
    pub(crate) passphrase: Entity<InputState>,
    /// 确认口令；仅导出加密备份时存在。
    pub(crate) confirm: Option<Entity<InputState>>,
    /// 导出时是否加密备份。
    pub(crate) encrypt: bool,
    /// 导出时是否把系统钥匙串里的口令一并写入。
    pub(crate) include_secrets: bool,
    /// 导入时用户声明该文件已加密。
    pub(crate) encrypted_file: bool,
    pub(crate) error: Option<String>,
    pub(crate) busy: bool,
}

/// 会话批量编辑弹窗状态。
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

/// 快捷键录制弹窗状态。
pub(crate) struct ShortcutCapture {
    pub(crate) id: &'static str,
    pub(crate) error: Option<String>,
    pub(crate) focus: FocusHandle,
}

/// 用户勾选"保存到系统钥匙串"、但认证结果尚未揭晓的凭据。
///
/// 为什么不在提交时就写：此刻还不知道服务器是否接受这个密码。先写下去会把打错的口令
/// 持久化，下次连接拿同一份错密码去撞服务器，用户既没有纠正机会，又会出现"明明输
/// 对了还是失败"的现象。
pub(crate) struct PendingCredential {
    pub(crate) session_id: SessionId,
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
    pub(crate) responder: flume::Sender<Vec<String>>,
    pub(crate) error: Option<String>,
    /// 待回答凭据属于哪个会话，保存时需要它作为钥匙串条目的键。
    pub(crate) session_id: SessionId,
    /// 可写入钥匙串的凭据类型。`None` 表示服务器下发的一次性挑战，
    /// 保存没有意义，因此界面不显示保存开关。
    pub(crate) secret_kind: Option<SecretKind>,
    /// "保存到系统钥匙串"勾选状态。默认不勾选：建会话时用户已表达过是否保存，
    /// 这里不该替他改主意。
    pub(crate) save: bool,
}

pub struct AppShell {
    pub(crate) services: UiServices,
    pub(crate) settings: AppSettings,
    /// 会话与文件夹共享同一份目录快照，任何一方变化都整体持久化。
    pub(crate) catalog: SessionCatalog,
    pub(crate) tabs: Vec<TerminalTab>,
    /// 标签栏条目：每项持有一棵分割树，树里的叶子才是 `tabs` 中的窗格。
    pub(crate) groups: Vec<TabGroup>,
    /// 各分割节点的拖拽尺寸状态，由外壳持有以跨过不可见的帧。
    ///
    /// 交给 gpui 的 keyed state 会在元素缺席一帧后被回收，切标签或窗格最大化
    /// 再回来时用户拖出的比例就归零；见 `AppShell::pane_split_state`。
    pub(crate) pane_split_states: HashMap<SplitId, Entity<ResizableState>>,
    pub(crate) active_tab: Option<TabId>,
    pub(crate) search: Entity<InputState>,
    pub(crate) form: Option<SessionForm>,
    pub(crate) folder_form: Option<FolderForm>,
    /// 认证阶段由 net 层发起的凭据询问；连接前不再拦截。
    pub(crate) auth_prompt: Option<AuthPromptState>,
    /// 认证成功后才落盘的凭据，见 [`PendingCredential`]。
    pub(crate) pending_credential: Option<PendingCredential>,
    pub(crate) show_known_hosts_modal: bool,
    pub(crate) known_hosts_records: Vec<HostKeyRecord>,
    pub(crate) known_hosts_search: Entity<InputState>,
    pub(crate) show_quick_open: bool,
    pub(crate) quick_open_state: Entity<CommandState>,
    pub(crate) show_settings_modal: bool,
    pub(crate) key_wizard: Option<KeyWizardState>,
    pub(crate) settings_font_family: Entity<InputState>,
    pub(crate) settings_cjk_font: Entity<InputState>,
    pub(crate) settings_font_size: Entity<InputState>,
    pub(crate) settings_line_height: Entity<InputState>,
    pub(crate) settings_scrollback: Entity<InputState>,
    pub(crate) settings_word_separators: Entity<InputState>,
    pub(crate) settings_notify_threshold: Entity<InputState>,
    pub(crate) settings_highlight_keywords: Entity<InputState>,
    pub(crate) confirm_forward_any: Option<PendingForwardConfirm>,
    pub(crate) sftp_editor_modal: Option<SftpEditorModalState>,
    pub(crate) sftp_chmod_modal: Option<SftpChmodModalState>,
    pub(crate) sftp_input_modal: Option<SftpInputModalState>,
    pub(crate) sftp_properties_modal: Option<SftpPropertiesModalState>,
    pub(crate) sftp_filter_modal: Option<SftpFilterModalState>,
    pub(crate) sftp_sync_modal: Option<SftpSyncModalState>,
    /// 底部快捷命令条是否展开（运行期状态，与侧边栏、沉浸模式同口径）。
    pub(crate) snippets_expanded: bool,
    /// 快捷命令库编辑弹窗。
    pub(crate) snippet_editor: Option<SnippetEditor>,
    pub(crate) status: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) sidebar_visible: bool,
    /// 沉浸式全屏：隐藏全部应用外框，仅保留终端视口。
    pub(crate) immersive: bool,
    /// 当前生效的应用快捷键，终端据此截留按键而不是编码为控制字符。
    pub(crate) shortcut_filter: Arc<parking_lot::RwLock<Vec<KeybindingKeystroke>>>,
    /// 上次应用到键位表的有效键位，改键时用于抑制旧绑定。
    pub(crate) applied_shortcut_keys: Vec<String>,
    pub(crate) shortcut_capture: Option<ShortcutCapture>,
    /// 侧边栏多选模式与已选会话。
    pub(crate) selection_mode: bool,
    pub(crate) selected_sessions: HashSet<SessionId>,
    /// 单击会话行留下的单选高亮。
    ///
    /// 与 `selected_sessions`（多选批量操作）分开：单击只是"看住这一行"，
    /// 既不该被当成加入批量操作的意图，也不该触发连接。
    pub(crate) tree_selection: Option<SessionId>,
    pub(crate) batch_edit: Option<BatchEditForm>,
    pub(crate) backup_prompt: Option<BackupPromptState>,
    pub(crate) _backup_task: Option<Task<()>>,
    pub(crate) _event_task: Task<()>,
}

impl AppShell {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        services: UiServices,
        settings: AppSettings,
    ) -> Self {
        cx.bind_keys(keymap::bindings(&settings.shortcuts));
        let shortcut_filter = Arc::new(parking_lot::RwLock::new(keymap::reserved_keystrokes(
            &settings.shortcuts,
        )));
        let applied_shortcut_keys = keymap::effective_key_strings(&settings.shortcuts);
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(rust_i18n::t!("app.search").to_string())
        });
        let quick_open_state = cx.new(|cx| CommandState::new(window, cx));
        let (catalog, error) = match services.session_store.load() {
            Ok(catalog) => (catalog, None),
            Err(error) => (SessionCatalog::default(), Some(localized_error(&error))),
        };

        let event_rx = services.event_rx.clone();
        let event_task = cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = event_rx.recv_async().await {
                if this
                    .update_in(cx, |view, window, cx| {
                        view.handle_app_event(event, window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        let known_hosts_search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(rust_i18n::t!("app.search").to_string())
        });
        let known_hosts_records = services.known_hosts.list().unwrap_or_default();

        let settings_font_family_val = settings
            .terminal_profile
            .font_family
            .clone()
            .unwrap_or_default();
        let settings_font_family = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("settings.font_family_hint").to_string())
                .default_value(settings_font_family_val)
        });
        let settings_cjk_font_val = settings
            .terminal_profile
            .cjk_font_family
            .clone()
            .unwrap_or_default();
        let settings_cjk_font = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("settings.cjk_font_hint").to_string())
                .default_value(settings_cjk_font_val)
        });
        let font_size_val = settings.terminal_profile.font_size.to_string();
        let settings_font_size = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("14")
                .default_value(&font_size_val)
        });
        let line_height_val = settings.terminal_profile.line_height.to_string();
        let settings_line_height = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("1.2")
                .default_value(&line_height_val)
        });
        // 分隔符必须原样回填：空格本身就是合法分隔符，任何 trim 都会吃掉用户意图。
        let settings_word_separators = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("settings.word_separators_hint").to_string())
                .default_value(settings.terminal_profile.word_separators.clone())
        });
        let settings_highlight_keywords = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("settings.highlight_keywords_hint").to_string())
                .default_value(settings.terminal_profile.highlight.keywords_joined())
        });
        let notify_threshold_val = settings.window.notify_threshold_secs.to_string();
        let settings_notify_threshold = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("10")
                .default_value(notify_threshold_val)
        });
        let scrollback_val = settings.terminal_profile.scrollback.to_string();
        let settings_scrollback = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("10000")
                .default_value(&scrollback_val)
        });

        let mut this = Self {
            services,
            settings,
            catalog,
            tabs: Vec::new(),
            groups: Vec::new(),
            pane_split_states: HashMap::new(),
            active_tab: None,
            search,
            form: None,
            folder_form: None,
            auth_prompt: None,
            pending_credential: None,
            show_known_hosts_modal: false,
            known_hosts_records,
            known_hosts_search,
            show_quick_open: false,
            quick_open_state,
            show_settings_modal: false,
            key_wizard: None,
            settings_font_family,
            settings_cjk_font,
            settings_font_size,
            settings_line_height,
            settings_scrollback,
            settings_word_separators,
            settings_notify_threshold,
            settings_highlight_keywords,
            confirm_forward_any: None,
            sftp_editor_modal: None,
            sftp_chmod_modal: None,
            sftp_input_modal: None,
            sftp_properties_modal: None,
            sftp_filter_modal: None,
            sftp_sync_modal: None,
            snippets_expanded: false,
            snippet_editor: None,
            status: None,
            error,
            sidebar_visible: true,
            immersive: false,
            shortcut_filter,
            applied_shortcut_keys,
            shortcut_capture: None,
            selection_mode: false,
            tree_selection: None,
            selected_sessions: HashSet::with_capacity(8),
            batch_edit: None,
            backup_prompt: None,
            _backup_task: None,
            _event_task: event_task,
        };

        if this.settings.window.restore_previous_tabs {
            let saved_groups = this.settings.window.previous_tab_groups.clone();
            if !saved_groups.is_empty() {
                for group in saved_groups {
                    this.restore_tab_group(&group, window, cx);
                }
            } else if !this.settings.window.previous_tab_sessions.is_empty() {
                let session_keys = this.settings.window.previous_tab_sessions.clone();
                for key in session_keys {
                    if key == "local" {
                        this.open_local_terminal(window, cx);
                    } else if let Some(session) = this
                        .catalog
                        .sessions
                        .iter()
                        .find(|s| s.id.to_string() == key)
                        .cloned()
                    {
                        this.connect_session(session.id, window, cx);
                    }
                }
            }
        }

        this
    }

    /// 持久化会话与文件夹目录。
    ///
    /// 统一入口避免各操作点漏存文件夹表；返回是否成功，调用方决定是否
    /// 继续后续动作（如选中项清理）。
    pub(crate) fn persist_catalog(&mut self) -> bool {
        match self.services.session_store.save(&self.catalog) {
            Ok(()) => true,
            Err(error) => {
                self.error = Some(localized_error(&error));
                false
            }
        }
    }

    pub(crate) fn on_new_session(
        &mut self,
        _: &keymap::NewSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_new_session(window, cx);
    }

    pub(crate) fn on_close_tab(
        &mut self,
        _: &keymap::CloseTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab_id) = self.active_tab {
            self.close_tab(tab_id, window, cx);
        }
    }

    pub(crate) fn on_next_tab(
        &mut self,
        _: &keymap::NextTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_tab(true, window, cx);
    }

    pub(crate) fn on_prev_tab(
        &mut self,
        _: &keymap::PrevTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_tab(false, window, cx);
    }

    pub(crate) fn on_reconnect(
        &mut self,
        _: &keymap::Reconnect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab_id) = self.active_tab {
            self.reconnect_tab(tab_id, window, cx);
        }
    }

    pub(crate) fn on_toggle_sidebar(
        &mut self,
        _: &keymap::ToggleSidebar,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar_visible = !self.sidebar_visible;
        cx.notify();
    }

    /// 切换沉浸式全屏：同步系统窗口全屏状态与应用外框显隐。
    ///
    /// 侧边栏、SFTP 与转发面板只做“隐藏渲染”而不清除自身开关状态，
    /// 退出沉浸模式后能原样恢复用户此前的工作布局。
    pub(crate) fn on_toggle_immersive(
        &mut self,
        _: &keymap::ToggleImmersive,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.immersive = !self.immersive;
        window.toggle_fullscreen();
        if self.immersive
            && let Some(focus_handle) = self
                .active_tab
                .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id))
                .and_then(|tab| tab.terminal.as_ref())
                .map(|terminal| terminal.read(cx).focus_handle().clone())
        {
            window.focus(&focus_handle, cx);
        }
        cx.notify();
    }

    pub(crate) fn render_overlays(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = Palette::from(cx);
        let mut overlays = Vec::new();
        if self.form.is_some() {
            overlays.push(self.render_session_form(window, cx, &p));
        }
        if self.folder_form.is_some() {
            overlays.push(self.render_folder_form(window, cx, &p));
        }
        if self.auth_prompt.is_some() {
            overlays.push(self.render_auth_prompt(window, cx, &p));
        }
        if self.show_known_hosts_modal {
            overlays.push(self.render_known_hosts_modal(window, cx, &p));
        }
        if self.confirm_forward_any.is_some() {
            overlays.push(self.render_forward_confirm_modal(window, cx, &p));
        }
        if self.show_quick_open {
            overlays.push(self.render_quick_open_modal(window, cx, &p));
        }
        if self.show_settings_modal {
            overlays.push(self.render_settings_modal(window, cx, &p));
        }
        if self.shortcut_capture.is_some() {
            overlays.push(self.render_shortcut_capture(cx, &p));
        }
        if self.batch_edit.is_some() {
            overlays.push(self.render_batch_edit(window, cx, &p));
        }
        if self.backup_prompt.is_some() {
            overlays.push(self.render_backup_prompt(window, cx, &p));
        }
        if self.key_wizard.is_some() {
            overlays.push(self.render_key_wizard(window, cx, &p));
        }
        if self.sftp_editor_modal.is_some() {
            overlays.push(self.render_sftp_editor_modal(window, cx, &p));
        }
        if self.sftp_chmod_modal.is_some() {
            overlays.push(self.render_sftp_chmod_modal(window, cx, &p));
        }
        if self.sftp_input_modal.is_some() {
            overlays.push(self.render_sftp_input_modal(window, cx, &p));
        }
        if self.sftp_properties_modal.is_some() {
            overlays.push(self.render_sftp_properties_modal(window, cx, &p));
        }
        if self.sftp_filter_modal.is_some() {
            overlays.push(self.render_sftp_filter_modal(window, cx, &p));
        }
        if self.sftp_sync_modal.is_some() {
            overlays.push(self.render_sftp_sync_modal(window, cx, &p));
        }
        if self.snippet_editor.is_some() {
            overlays.push(self.render_snippet_editor(&p, cx));
        }
        overlays
    }
}
