//! GPUI 顶层应用外壳状态、快捷键动作委托与主布局容器。

use crate::UiServices;
use crate::cloud_sync::SyncFormInputs;
use crate::forward::{ForwardPanelState, PendingForwardConfirm};
use crate::keymap;
use crate::keys::KeyWizardState;
use crate::palette::{Palette, localized_error};
use crate::pane_tree::LayoutNode;
use crate::probe::ProbeBadge;
use crate::session_form::{AuthChoice, SessionForm};
use crate::sftp::{
    EditorPanelState, PendingEditorClose, SftpChmodModalState, SftpFilterModalState,
    SftpInputModalState, SftpPanelState, SftpPropertiesModalState, SftpSyncModalState,
};
use crate::snippets::SnippetFormState;
use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::{EditorState, InputState};
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use transom_core::{
    AppCommand, AppError, AppSettings, AppearanceSettings, Connection, ConnectionId, FolderId,
    ForwardId, ForwardStatus, GroupId, HostKeyRecord, NetworkTraffic, PanelSide, PromptInfo,
    SecretKind, Session, SessionCatalog, SessionId, SplitId, SystemStats, TabId, UiThemeMode,
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
    /// 当前网络的实时速率：SSH 为传输层连接速率，本地终端为本机非回环网卡
    /// 合计速率；串口与尚未开始采样时为 `None`。
    pub(crate) network_traffic: Option<NetworkTraffic>,
    /// 初始化命令的后台调度任务；重连或关标签时丢弃以取消。
    pub(crate) _startup_task: Option<Task<()>>,
    pub(crate) _subscription: Option<Subscription>,
    /// 自动重连已尝试次数；连接成功后清零，上限与退避间隔见
    /// `schedule_auto_reconnect`。
    pub(crate) reconnect_attempts: u32,
    /// 待执行的自动重连任务；用户手动重连或关标签时随任务丢弃而取消。
    pub(crate) reconnect_task: Option<Task<()>>,
    /// 活跃的 ZMODEM 传输展示快照；连接接入时清空，事件驱动更新。
    pub(crate) zmodem: Option<crate::zmodem::ZmodemState>,
}

impl TerminalTab {
    /// 标签栏、窗格头、拖拽预览与状态栏显示的会话名称。
    ///
    /// 为什么不复用 `title`：`title` 是运行期标题，会被远端 `PS1` 里的 OSC 序列改写，
    /// 拿它当标签名会出现"标签跟着 cd 变名"，而且同会话多标签也无法靠它区分。
    /// 会话名在表单里是必填项，因此兜底分支只对手工编辑的目录文件生效：
    /// SSH 用 `用户@主机`，Telnet 用 `主机:端口`，串口用设备名，本地终端用 shell 名。
    pub(crate) fn display_name(&self) -> String {
        let name = self.session.name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        match &self.session.connection {
            Connection::Ssh(ssh) => format!("{}@{}", ssh.username, ssh.host),
            Connection::Telnet(telnet) => format!("{}:{}", telnet.host, telnet.port),
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
            reconnect_attempts: 0,
            reconnect_task: None,
            zmodem: None,
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
    /// 口令实际落位的归属：会话引用凭据模板时为模板，保证"改一处全生效"。
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
    /// 待回答凭据属于哪个会话，保存时需要它作为钥匙串条目的键。
    pub(crate) session_id: SessionId,
    /// 可写入钥匙串的凭据类型。`None` 表示服务器下发的一次性挑战，
    /// 保存没有意义，因此界面不显示保存开关。
    pub(crate) secret_kind: Option<SecretKind>,
    /// "保存到系统钥匙串"勾选状态。默认不勾选：建会话时用户已表达过是否保存，
    /// 这里不该替他改主意。
    pub(crate) save: bool,
}

/// TOFU 主机指纹确认弹窗状态。
///
/// 设计原理（Why）：首次连接必须让用户核对指纹后才写入 known_hosts，
/// 首连被中间人劫持时用户才有感知与拦截机会；`responder` 析构（标签页
/// 关闭、弹窗被顶掉）即视为取消，认证方按失败处理并断开连接。
pub(crate) struct HostKeyConfirmState {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) key_type: String,
    pub(crate) fingerprint: String,
    pub(crate) responder: tokio::sync::mpsc::Sender<bool>,
}

/// known_hosts 管理弹窗的聚合状态（开关、记录与搜索框）。
///
/// 设计原理（Why）：AppShell 此前把十余种模态的开关与状态平铺成 50+ 字段，
/// 每加一个功能就加几个 Option；按域聚合成子结构后，字段数与心智负担
/// 随域隔离，模态之间互不可见彼此内部状态。
pub(crate) struct KnownHostsModalState {
    pub(crate) open: bool,
    pub(crate) records: Vec<HostKeyRecord>,
    pub(crate) search: Entity<InputState>,
}

/// 设置弹窗的聚合状态：开关、密钥向导与全部输入框实体。
pub(crate) struct SettingsModalState {
    pub(crate) open: bool,
    pub(crate) key_wizard: Option<KeyWizardState>,
    /// 终端字体族输入框。
    pub(crate) font_family: Entity<InputState>,
    /// 界面字体族输入框（外观覆盖，留空跟随主题）。
    pub(crate) ui_font_family: Entity<InputState>,
    /// 文件面板显示排除规则的输入框。
    pub(crate) panel_hidden_patterns: Entity<InputState>,
    /// 并发传输上限的输入框。
    pub(crate) max_concurrent_transfers: Entity<InputState>,
    pub(crate) cjk_font: Entity<InputState>,
    pub(crate) font_size: Entity<InputState>,
    pub(crate) line_height: Entity<InputState>,
    pub(crate) scrollback: Entity<InputState>,
    pub(crate) word_separators: Entity<InputState>,
    pub(crate) notify_threshold: Entity<InputState>,
    pub(crate) highlight_keywords: Entity<InputState>,
    /// 打开弹窗瞬间的外观偏好快照。
    ///
    /// 界面主题与密度在弹窗内即时预览（改一档立刻全局生效），取消时凭它恢复，
    /// 与"保存才落盘"的单点纪律对齐；其余设置项维持原有不回滚行为。
    pub(crate) appearance_snapshot: Option<AppearanceSettings>,
    /// 云同步端点与凭据输入框（凭据为掩码框，不回填钥匙串已有值）。
    pub(crate) sync_form: SyncFormInputs,
    /// 凭据模板编辑器状态；`None` 表示当前未在编辑模板。
    pub(crate) identity_edit: Option<crate::identity_panel::IdentityEditState>,
}

/// SFTP 五类弹窗（权限、输入、属性、过滤、同步）与编辑器跳转行的聚合状态。
pub(crate) struct SftpModals {
    pub(crate) chmod: Option<SftpChmodModalState>,
    pub(crate) input: Option<SftpInputModalState>,
    pub(crate) properties: Option<SftpPropertiesModalState>,
    pub(crate) filter: Option<SftpFilterModalState>,
    pub(crate) sync: Option<SftpSyncModalState>,
    pub(crate) goto_line: Option<EditorGotoLineModalState>,
}

/// 编辑器"跳转到行"输入模态。
///
/// `editor` 在打开瞬间捕获：模态输入框会抢走焦点，确认时再按焦点找面板
/// 会命中错误的编辑器；订阅句柄随模态存亡，关闭即退订回车事件。
pub(crate) struct EditorGotoLineModalState {
    pub(crate) editor: Entity<EditorState>,
    pub(crate) input: Entity<InputState>,
    pub(crate) error: Option<String>,
    pub(crate) _enter_sub: Subscription,
}

/// 快捷命令面板的聚合状态：卡片展开开关与新增/编辑小弹窗。
pub(crate) struct SnippetPanelState {
    pub(crate) open: bool,
    pub(crate) form: Option<SnippetFormState>,
}

/// 快捷键域的聚合状态：生效键位过滤器、已应用键位串与改键捕获。
pub(crate) struct ShortcutRecordingState {
    pub(crate) filter: Arc<parking_lot::RwLock<Vec<KeybindingKeystroke>>>,
    pub(crate) applied_keys: Vec<String>,
    pub(crate) capture: Option<ShortcutCapture>,
}

/// 命令发送失败的界面文案：队列满提示稍后重试，控制器退出提示重启应用。
pub(crate) fn command_send_error_text(error: flume::TrySendError<AppCommand>) -> String {
    match error {
        flume::TrySendError::Full(_) => rust_i18n::t!("error.command_queue_full").to_string(),
        flume::TrySendError::Disconnected(_) => {
            rust_i18n::t!("error.controller_stopped").to_string()
        }
    }
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
    /// 工作区分组（侧栏、主区、SFTP、转发）的列宽状态。
    ///
    /// 由外壳持有而不是交给 gpui 的 keyed state：状态栏分界线要读同一份侧栏
    /// 宽度，而 keyed state 在沉浸模式切换后可能被回收，侧栏会回到初始宽度。
    pub(crate) workspace_panels: Entity<ResizableState>,
    /// 停靠左/右各自独立的列宽状态：面板顺序随停靠方向变化，共用一份状态
    /// 会把按索引记录的尺寸错位到别的面板上，因此两个方向各留一份。
    pub(crate) workspace_panels_left: Entity<ResizableState>,
    pub(crate) active_tab: Option<TabId>,
    pub(crate) search: Entity<InputState>,
    pub(crate) form: Option<SessionForm>,
    pub(crate) folder_form: Option<FolderForm>,
    /// 认证阶段由 net 层发起的凭据询问；连接前不再拦截。
    pub(crate) auth_prompt: Option<AuthPromptState>,
    /// TOFU 首连主机指纹确认弹窗；同一时刻最多一个在等用户作答。
    pub(crate) host_key_confirm: Option<HostKeyConfirmState>,
    /// 认证成功后才落盘的凭据，见 [`PendingCredential`]。
    pub(crate) pending_credential: Option<PendingCredential>,
    /// known_hosts 管理弹窗的聚合状态（开关、记录与搜索框）。
    pub(crate) known_hosts_modal: KnownHostsModalState,
    /// 命令历史内存快照：终端视图的查询回调共享它，避免每次按键都读盘。
    pub(crate) history: Arc<parking_lot::RwLock<Vec<String>>>,
    /// 共享给每个终端视图的历史查询回调（同一份快照，不各自复制）。
    pub(crate) history_provider: transom_terminal::HistoryProvider,
    /// 会话列表行尾的拨测状态点，按会话 id 索引；结果点到期后自动移除，
    /// 因此这里只存"当前还该画出来"的点，不承担历史记录职责。
    pub(crate) probe_badges: HashMap<SessionId, ProbeBadge>,
    /// 每个状态点的到期清理任务。写入新状态时覆盖旧句柄即取消旧定时器，
    /// 保证上一批结果的到期不会抹掉这一批结果。
    pub(crate) probe_expiry_tasks: HashMap<SessionId, Task<()>>,
    pub(crate) show_quick_open: bool,
    pub(crate) quick_open_state: Entity<CommandState>,
    /// 设置弹窗的聚合状态：开关、密钥向导与全部输入框实体。
    pub(crate) settings_modal: SettingsModalState,
    pub(crate) confirm_forward_any: Option<PendingForwardConfirm>,
    /// 已打开的远端文本编辑面板，Vec 顺序即叠放次序，末尾为最上层。
    pub(crate) editor_panels: Vec<EditorPanelState>,
    /// 被脏编辑面板拦下的标签/会话关闭意图；面板全部处理完毕后续行执行。
    pub(crate) pending_editor_close: Option<PendingEditorClose>,
    /// 工具栏"已打开编辑器"浮层的受控显隐（与快捷命令卡片同口径）。
    pub(crate) editor_list_open: bool,
    /// SFTP 五类弹窗（权限、重命名等输入、属性、过滤、同步）的聚合状态。
    pub(crate) sftp_modals: SftpModals,
    /// 快捷命令面板的聚合状态：卡片展开开关与新增/编辑小弹窗。
    pub(crate) snippet_panel: SnippetPanelState,
    /// 底栏右段的兜底状态文字（没有指标徽标时显示）。
    pub(crate) status: Option<String>,
    /// 全局错误缓冲：渲染时取走并转成应用内通知，界面不再有独立错误横幅。
    pub(crate) error: Option<String>,
    pub(crate) sidebar_visible: bool,
    /// 沉浸式全屏：隐藏全部应用外框，仅保留终端视口。
    pub(crate) immersive: bool,
    /// 快捷键域的聚合状态：生效键位过滤器、已应用键位串与改键捕获。
    pub(crate) shortcut_recording: ShortcutRecordingState,
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
    /// 云同步下载合并的后台任务；合并完成后置空。
    pub(crate) _sync_pull_task: Option<Task<()>>,
    /// 系统明暗变化订阅；随 AppShell 存活，drop 即退订。
    pub(crate) _appearance_subscription: Subscription,
    pub(crate) _event_task: Task<()>,
}

impl AppShell {
    /// 向控制器提交命令的统一入口：非阻塞 try_send，队列满时快速失败。
    ///
    /// 设计原理（Why）：命令通道有界（256），控制器侧以 send_async 反压；
    /// UI 侧若用阻塞 send，控制器因事件队列满而等待 UI 消费时会形成互等，
    /// 界面线程被冻结。失败不静默：错误进入全局错误栏提示用户。
    /// 返回 `false` 表示命令未入队，调用方可据此做回滚（如撤销标签删除）。
    /// 设置工具面板（SFTP/转发）停靠方向：切换到对应方向的列宽状态，
    /// 尺寸互相独立、互不错位。方向变化即时预览，落盘仍由保存承担。
    pub(crate) fn set_sftp_panel_side(&mut self, side: PanelSide, cx: &mut Context<Self>) {
        if self.settings.window.sftp_panel_side == side {
            return;
        }
        self.settings.window.sftp_panel_side = side;
        std::mem::swap(&mut self.workspace_panels, &mut self.workspace_panels_left);
        self.persist_layout_settings(cx);
    }

    pub(crate) fn send_command(&mut self, command: AppCommand) -> bool {
        match self.services.command_tx.try_send(command) {
            Ok(()) => true,
            Err(error) => {
                self.error = Some(command_send_error_text(error));
                false
            }
        }
    }

    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        services: UiServices,
        mut settings: AppSettings,
    ) -> Self {
        // 设置文件是外部输入：未知条目、重复项、越界分区都在进入运行期前清洗。
        settings.activity_bar.normalize();
        settings.status_bar.normalize();
        settings.appearance.normalize();
        cx.bind_keys(keymap::bindings(&settings.shortcuts));
        let shortcut_filter = Arc::new(parking_lot::RwLock::new(keymap::reserved_keystrokes(
            &settings.shortcuts,
        )));
        let applied_shortcut_keys = keymap::effective_key_strings(&settings.shortcuts);
        // 命令历史读一次进内存，之后终端视图的查询回调只读这份快照：
        // 建议要在每次按键时同步返回，不能去碰磁盘。关闭开关时连读都不读。
        let history: Arc<parking_lot::RwLock<Vec<String>>> =
            Arc::new(parking_lot::RwLock::new(if settings.history.enabled {
                services.history.load()
            } else {
                Vec::new()
            }));
        let history_provider: transom_terminal::HistoryProvider = {
            let snapshot = Arc::clone(&history);
            Arc::new(move |prefix: &str| {
                transom_core::command_suggestions(
                    &snapshot.read(),
                    prefix,
                    transom_core::SUGGESTION_LIMIT,
                )
            })
        };
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(rust_i18n::t!("app.search").to_string())
        });
        let quick_open_state = cx.new(|cx| CommandState::new(window, cx));
        let workspace_panels = cx.new(|_| ResizableState::default());
        let workspace_panels_left = cx.new(|_| ResizableState::default());
        // 侧栏拖动与状态栏分界线共享尺寸：状态一通知就重绘外壳，分界线实时跟随。
        // 两个方向的分组状态各自订阅（未激活方向不会产生通知）。
        cx.observe(&workspace_panels, |_, _, cx| cx.notify())
            .detach();
        cx.observe(&workspace_panels_left, |_, _, cx| cx.notify())
            .detach();
        let (catalog, error) = match services.session_store.load() {
            Ok(catalog) => (catalog, None),
            Err(error) => {
                log::error!(
                    "读取会话目录失败，界面按空目录启动，原因: {}",
                    localized_error(&error)
                );
                (SessionCatalog::default(), Some(localized_error(&error)))
            }
        };
        log::info!(
            "会话目录就绪，会话数: {}，分组数: {}，凭据模板数: {}",
            catalog.sessions.len(),
            catalog.folders.len(),
            catalog.identities.len()
        );

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

        // 系统明暗变化跟随：仅在"跟随系统"档响应，手动选择的明暗不被系统切换
        // 覆盖。读取走实体而不是闭包快照，设置页改动后无需重新挂订阅。
        let app_entity = cx.entity();
        let appearance_subscription = window.observe_window_appearance(move |window, cx| {
            let follows_system =
                app_entity.read(cx).settings.appearance.mode == UiThemeMode::System;
            if follows_system {
                crate::theme::sync_with_system(window, cx);
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
        let settings_ui_font_val = settings
            .appearance
            .ui_font_family
            .clone()
            .unwrap_or_default();
        let settings_ui_font_family = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("settings.ui_font_family_hint").to_string())
                .default_value(settings_ui_font_val)
        });
        let settings_panel_hidden = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("settings.panel_hidden_hint").to_string())
                .default_value(settings.sftp.panel_hidden_patterns.clone())
        });
        let settings_max_concurrent = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("3")
                .default_value(settings.sftp.max_concurrent_transfers.to_string())
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
        // 云同步表单：端点字段回填设置值，凭据掩码框恒为空。
        let sync_form = SyncFormInputs::new(window, cx);

        let mut this = Self {
            services,
            settings,
            catalog,
            tabs: Vec::new(),
            groups: Vec::new(),
            pane_split_states: HashMap::new(),
            workspace_panels,
            workspace_panels_left,
            active_tab: None,
            search,
            form: None,
            folder_form: None,
            auth_prompt: None,
            host_key_confirm: None,
            pending_credential: None,
            history,
            history_provider,
            known_hosts_modal: KnownHostsModalState {
                open: false,
                records: known_hosts_records,
                search: known_hosts_search,
            },
            probe_badges: HashMap::new(),
            probe_expiry_tasks: HashMap::new(),
            show_quick_open: false,
            quick_open_state,
            settings_modal: SettingsModalState {
                open: false,
                key_wizard: None,
                font_family: settings_font_family,
                ui_font_family: settings_ui_font_family,
                panel_hidden_patterns: settings_panel_hidden,
                max_concurrent_transfers: settings_max_concurrent,
                cjk_font: settings_cjk_font,
                font_size: settings_font_size,
                line_height: settings_line_height,
                scrollback: settings_scrollback,
                word_separators: settings_word_separators,
                notify_threshold: settings_notify_threshold,
                highlight_keywords: settings_highlight_keywords,
                appearance_snapshot: None,
                identity_edit: None,
                sync_form,
            },
            confirm_forward_any: None,
            editor_panels: Vec::new(),
            pending_editor_close: None,
            editor_list_open: false,
            sftp_modals: SftpModals {
                chmod: None,
                input: None,
                properties: None,
                filter: None,
                sync: None,
                goto_line: None,
            },
            snippet_panel: SnippetPanelState {
                open: false,
                form: None,
            },
            status: None,
            error,
            sidebar_visible: true,
            immersive: false,
            shortcut_recording: ShortcutRecordingState {
                filter: shortcut_filter,
                applied_keys: applied_shortcut_keys,
                capture: None,
            },
            selection_mode: false,
            tree_selection: None,
            selected_sessions: HashSet::with_capacity(8),
            batch_edit: None,
            backup_prompt: None,
            _backup_task: None,
            _sync_pull_task: None,
            _appearance_subscription: appearance_subscription,
            _event_task: event_task,
        };

        // Linux 上窗口级外观上报比应用级可靠（gpui-kit issue #104）；开窗前
        // lib.rs 的首次注入读的是应用级外观，这里在首帧前用窗口读数再校准一次，
        // "跟随系统"档的用户才不会在启动瞬间看到错误的明暗。
        if this.settings.appearance.mode == UiThemeMode::System {
            crate::theme::sync_with_system(window, cx);
        }
        // 终端配色跟随模式：按启动时生效的 UI 主题解析一次调色板，
        // settings.json 里的落盘值只是上次解析的缓存。
        this.sync_terminal_palette(cx);

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
        if self.immersive {
            // 沉浸模式没有底栏，快捷命令卡片失去锚点，先收起再进入。
            self.snippet_panel.open = false;
        }
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
        // 悬停预览先绘制，保证任何模态框都压在它上面。
        if let Some(preview) = self.render_sftp_hover_preview(window, &p) {
            overlays.push(preview);
        }
        if self.form.is_some() {
            overlays.push(self.render_session_form(window, cx, &p));
        }
        if self.folder_form.is_some() {
            overlays.push(self.render_folder_form(window, cx, &p));
        }
        if self.auth_prompt.is_some() {
            overlays.push(self.render_auth_prompt(window, cx, &p));
        }
        if self.host_key_confirm.is_some() {
            overlays.push(self.render_host_key_confirm(window, cx, &p));
        }
        if self.known_hosts_modal.open {
            overlays.push(self.render_known_hosts_modal(window, cx, &p));
        }
        if self.confirm_forward_any.is_some() {
            overlays.push(self.render_forward_confirm_modal(window, cx, &p));
        }
        if self.show_quick_open {
            overlays.push(self.render_quick_open_modal(window, cx, &p));
        }
        if self.settings_modal.open {
            overlays.push(self.render_settings_modal(window, cx, &p));
        }
        if self.shortcut_recording.capture.is_some() {
            overlays.push(self.render_shortcut_capture(cx, &p));
        }
        if self.batch_edit.is_some() {
            overlays.push(self.render_batch_edit(window, cx, &p));
        }
        if self.backup_prompt.is_some() {
            overlays.push(self.render_backup_prompt(window, cx, &p));
        }
        if self.settings_modal.key_wizard.is_some() {
            overlays.push(self.render_key_wizard(window, cx, &p));
        }
        if self.sftp_modals.chmod.is_some() {
            overlays.push(self.render_sftp_chmod_modal(window, cx, &p));
        }
        if self.sftp_modals.input.is_some() {
            overlays.push(self.render_sftp_input_modal(window, cx, &p));
        }
        if self.sftp_modals.properties.is_some() {
            overlays.push(self.render_sftp_properties_modal(window, cx, &p));
        }
        if self.sftp_modals.filter.is_some() {
            overlays.push(self.render_sftp_filter_modal(window, cx, &p));
        }
        if self.sftp_modals.sync.is_some() {
            overlays.push(self.render_sftp_sync_modal(window, cx, &p));
        }
        if self.sftp_modals.goto_line.is_some() {
            overlays.push(self.render_editor_goto_line_modal(window, cx, &p));
        }
        if self.snippet_panel.form.is_some() {
            overlays.push(self.render_snippet_form(&p, cx));
        }
        overlays
    }
}
