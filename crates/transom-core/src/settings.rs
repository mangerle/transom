use crate::ids::SnippetId;
use crate::layout::{ActivityBarLayout, StatusBarLayout};
use crate::sftp_filter::DEFAULT_TRANSFER_EXCLUDES;
use crate::sync::SyncSettings;
use crate::terminal::{TerminalInteraction, TerminalProfile};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 终端提示音响铃模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BellStyle {
    /// 关闭
    None,
    /// 屏幕可视化闪烁
    #[default]
    Visual,
    /// 播放提示音
    Audible,
}

/// 窗口与标签页设置
/// 工具面板（SFTP/转发）停靠在主面板的哪一侧。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PanelSide {
    /// 主面板右侧（默认）。
    #[default]
    Right,
    /// 主面板左侧，介于侧栏与主面板之间。
    Left,
    /// 反序列化兜底：未知值按右侧处理。
    #[serde(other)]
    Unknown,
}

impl PanelSide {
    /// 是否停靠左侧；兜底值视同右侧。
    pub fn is_left(self) -> bool {
        matches!(self, PanelSide::Left)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowSettings {
    /// 是否在标签页上显示关闭按钮
    #[serde(default = "default_true")]
    pub show_tab_close_button: bool,
    /// 所有标签页关闭时是否退出主程序
    #[serde(default)]
    pub close_window_on_all_tabs_closed: bool,
    /// 终端响铃模式
    #[serde(default)]
    pub bell_style: BellStyle,
    /// 是否允许弹系统桌面通知：既管"长时间命令完成"，也管终端程序主动发的 OSC 777 通知。
    #[serde(default = "default_true")]
    pub desktop_notifications: bool,
    /// 命令耗时达到多少秒才值得通知；太短的阈值会让 `ls` 也弹窗。
    #[serde(default = "default_notify_threshold_secs")]
    pub notify_threshold_secs: u64,
    /// 预设或记忆的窗口宽度
    #[serde(default = "default_window_width")]
    pub window_width: f32,
    /// 预设或记忆的窗口高度
    #[serde(default = "default_window_height")]
    pub window_height: f32,
    /// 记忆的窗口水平坐标
    #[serde(default)]
    pub window_x: Option<f32>,
    /// 记忆的窗口垂直坐标
    #[serde(default)]
    pub window_y: Option<f32>,
    /// 是否记忆窗口大小与位置
    #[serde(default = "default_true")]
    pub remember_window_bounds: bool,
    /// 启动时标签区开什么。取代早期的 `restore_previous_tabs` 布尔（已废弃：文件里
    /// 残留该键会被忽略，下次保存自然消失）。
    #[serde(default)]
    pub startup_tabs: StartupTabs,
    /// 启动时会话列表（侧栏）的展开策略。
    #[serde(default)]
    pub sidebar_startup: SidebarStartup,
    /// 上次退出时会话列表是否展开。
    ///
    /// 只在 `sidebar_startup = RememberLast` 时参与决策，由退出流程与窗口几何、
    /// 标签组同批写入：它属于"上次状态"而不是"配置意图"，所以不随每次点击落盘，
    /// 丢失边界也与标签组一致（崩溃后一起退到上次正常退出）。
    #[serde(default = "default_true")]
    pub last_sidebar_visible: bool,
    /// 上次打开的会话 ID 列表
    #[serde(default)]
    pub previous_tab_sessions: Vec<String>,
    /// 上次打开的标签组与分屏布局；优先于扁平的会话列表。
    #[serde(default)]
    pub previous_tab_groups: Vec<SavedTabGroup>,
    /// 上次退出时的双标签并排视图；`None` 表示单标签视图。
    #[serde(default)]
    pub tab_tiling: Option<SavedTabTiling>,
    /// 工具面板（SFTP/转发）停靠方向；旧配置缺省按右侧。
    #[serde(default)]
    pub sftp_panel_side: PanelSide,
    /// 是否把 transom 注册为系统默认终端（仅 Linux 生效）：接管桌面快捷键
    /// 与文件管理器"在终端打开"。属于显式 opt-in，默认关闭，
    /// 与 `.desktop` 模板"不静默抢占默认程序"的设计一致。
    #[serde(default)]
    pub default_terminal: bool,
}

/// 启动时标签区的内容。
///
/// 为什么是枚举而不是"恢复上次"+"开本地终端"两个布尔：两个旋钮会出现
/// "恢复出三个标签后再叠一个本地终端"这种没人想要的组合；三档互斥后每个取值
/// 都是一句能说清的话，也不存在优先级要解释。
///
/// 反序列化沿用 [`LogLevel`] 的口径：设置文件属外部输入，认不出的值退回默认档，
/// 而不是让整个 `settings.json` 加载失败、把所有设置一起丢掉。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupTabs {
    /// 恢复上次退出时打开的标签组（默认，等价于早期布尔 `true`）。
    #[default]
    RestoreLast,
    /// 启动后留空工作台（等价于早期布尔 `false`）。
    EmptyWorkspace,
    /// 每次启动新建一个本地终端标签，不恢复上次标签。
    NewLocalTerminal,
}

impl StartupTabs {
    /// 全部档位，供设置页分段按钮遍历。
    pub const ALL: [Self; 3] = [
        Self::RestoreLast,
        Self::EmptyWorkspace,
        Self::NewLocalTerminal,
    ];

    /// 稳定小写名：设置文件里的取值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RestoreLast => "restore_last",
            Self::EmptyWorkspace => "empty_workspace",
            Self::NewLocalTerminal => "new_local_terminal",
        }
    }

    /// 解析设置里的档位；无法识别时退回默认档。
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "empty_workspace" | "empty" | "none" => Self::EmptyWorkspace,
            "new_local_terminal" | "local" => Self::NewLocalTerminal,
            _ => Self::RestoreLast,
        }
    }
}

impl<'de> serde::Deserialize<'de> for StartupTabs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = <String as serde::Deserialize>::deserialize(deserializer)?;
        Ok(Self::parse(&raw))
    }
}

/// 启动时会话列表（侧栏）的展开策略。
///
/// 为什么把"记住上次"做成一个档位而不是再加一个开关：单靠"记住"会让用户被自己的
/// 上一次行为绑架——某次手动收起后，每次启动都收起且无法解释；而单靠"固定默认值"
/// 又丢掉了"记住"这个常见预期。合成三档后两种诉求都是显式选择，互不屏蔽。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarStartup {
    /// 沿用上次退出时的展开状态（默认）。
    #[default]
    RememberLast,
    /// 每次启动都展开。
    AlwaysShow,
    /// 每次启动都收起。
    AlwaysHide,
}

impl SidebarStartup {
    /// 全部档位，供设置页分段按钮遍历。
    pub const ALL: [Self; 3] = [Self::RememberLast, Self::AlwaysShow, Self::AlwaysHide];

    /// 稳定小写名：设置文件里的取值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RememberLast => "remember_last",
            Self::AlwaysShow => "always_show",
            Self::AlwaysHide => "always_hide",
        }
    }

    /// 解析设置里的档位；无法识别时退回默认档。
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "always_show" | "show" => Self::AlwaysShow,
            "always_hide" | "hide" => Self::AlwaysHide,
            _ => Self::RememberLast,
        }
    }
}

impl<'de> serde::Deserialize<'de> for SidebarStartup {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = <String as serde::Deserialize>::deserialize(deserializer)?;
        Ok(Self::parse(&raw))
    }
}

impl WindowSettings {
    /// 启动时会话列表是否展开。
    ///
    /// 纯函数：三档策略与"上次状态"的组合是启动期最容易配错的地方，抽出来才能
    /// 被单测逐个钉住，而不是靠界面点上碰一次。
    pub fn sidebar_visible_at_startup(&self) -> bool {
        match self.sidebar_startup {
            SidebarStartup::AlwaysShow => true,
            SidebarStartup::AlwaysHide => false,
            SidebarStartup::RememberLast => self.last_sidebar_visible,
        }
    }
}

/// 分屏方向，同时用于运行期布局与窗口恢复。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SplitAxis {
    /// 左右并排。
    #[default]
    Horizontal,
    /// 上下堆叠。
    Vertical,
}

/// 分割树的持久化形状，与运行期布局节点一一对应，但不携带窗格标识。
///
/// 为什么只存形状：同一标签组内的窗格共享一份会话配置，恢复时按叶子个数补齐
/// 连接即可；无法从数量推断的信息是每层切分的方向与嵌套结构，这才是必须落盘的。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SavedLayout {
    /// 叶子窗格。同时是缺省形状：设置文件属外部输入，字段缺失时退化为单窗格，
    /// 而不是让整个 `AppSettings` 加载失败。
    #[default]
    Pane,
    /// 沿 `axis` 把空间分给两个子节点的内部节点。
    Split {
        axis: SplitAxis,
        first: Box<SavedLayout>,
        second: Box<SavedLayout>,
    },
}

/// 窗口恢复时重建的一个标签组。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedTabGroup {
    /// `"local"` 或持久化会话 ID；组内窗格共享同一会话配置。
    pub session: String,
    /// 分割树形状；窗格数量与每层方向都由它表达，恢复时按叶子顺序补齐连接。
    #[serde(default)]
    pub layout: SavedLayout,
}

/// 双标签并排视图的恢复快照。
///
/// 用 `previous_tab_groups` 的下标而不是标签组 ID：组 ID 是运行期生成的 UUID，
/// 不随标签组落盘，恢复时只能按下标找回刚重建的组。任一下标越界、两个下标相同，
/// 或对应标签组没有成功恢复时，整份快照作废并退回单标签视图，绝不猜测绑定关系。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTabTiling {
    /// 前侧（左/上）标签组在 `previous_tab_groups` 中的下标。
    pub first: usize,
    /// 后侧（右/下）标签组的下标。
    pub second: usize,
    /// 并排方向：左右还是上下。
    #[serde(default)]
    pub axis: SplitAxis,
    /// 退出时焦点是否落在第二格。
    #[serde(default)]
    pub focused_second: bool,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            show_tab_close_button: true,
            close_window_on_all_tabs_closed: false,
            bell_style: BellStyle::default(),
            desktop_notifications: true,
            notify_threshold_secs: default_notify_threshold_secs(),
            window_width: default_window_width(),
            window_height: default_window_height(),
            window_x: None,
            window_y: None,
            remember_window_bounds: true,
            startup_tabs: StartupTabs::default(),
            sidebar_startup: SidebarStartup::default(),
            last_sidebar_visible: true,
            previous_tab_sessions: Vec::new(),
            previous_tab_groups: Vec::new(),
            tab_tiling: None,
            sftp_panel_side: PanelSide::Right,
            default_terminal: false,
        }
    }
}

fn default_notify_threshold_secs() -> u64 {
    10
}

fn default_window_width() -> f32 {
    1280.0
}

fn default_window_height() -> f32 {
    800.0
}

fn default_true() -> bool {
    true
}

/// 界面外观的明暗模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UiThemeMode {
    /// 跟随系统明暗设置；依赖桌面 portal 上报，极简窗口管理器下可能恒为浅色。
    #[default]
    System,
    /// 固定浅色。
    Light,
    /// 固定深色。
    Dark,
    /// 反序列化兜底：设置文件属外部输入，未知值按跟随系统处理，避免一个非法
    /// 字段让整份设置回退默认。
    #[serde(other)]
    Unknown,
}

impl UiThemeMode {
    /// 归一化兜底值：写盘前洗成合法状态，保证持久化文件永远可回读。
    pub(crate) fn normalized(self) -> Self {
        match self {
            UiThemeMode::Unknown => UiThemeMode::System,
            mode => mode,
        }
    }
}

/// 界面密度档位。
///
/// 为什么只有两个数值：gpui-kit 的根插件每帧把窗口 rem 基准设为全局主题字号，
/// 间距、文字与图标全部随动，因此密度只需定 sans 与 mono 两个字号。mono 与
/// sans 的差距随档位保持在 2 至 3px，标准档即 gpui 默认值（16/13）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UiDensity {
    /// 紧凑：全局 13px、等宽 11px。
    Compact,
    /// 偏紧：全局 14px、等宽 12px。
    Snug,
    /// 适中：全局 15px、等宽 12px。
    Medium,
    /// 标准：全局 16px、等宽 13px，与 gpui 默认一致。
    #[default]
    Standard,
    /// 宽松：全局 18px、等宽 15px。
    Roomy,
    /// 反序列化兜底：未知值按标准档处理。
    #[serde(other)]
    Unknown,
}

impl UiDensity {
    /// 全部可选档位，供设置页按固定顺序枚举。
    pub const ALL: [UiDensity; 5] = [
        UiDensity::Compact,
        UiDensity::Snug,
        UiDensity::Medium,
        UiDensity::Standard,
        UiDensity::Roomy,
    ];

    /// 界面全局字号（rem 基准），单位 px。
    pub fn font_size(self) -> f32 {
        match self {
            UiDensity::Compact => 13.0,
            UiDensity::Snug => 14.0,
            UiDensity::Medium => 15.0,
            UiDensity::Standard | UiDensity::Unknown => 16.0,
            UiDensity::Roomy => 18.0,
        }
    }

    /// 界面等宽字号（SFTP 预览、指纹展示等场景），单位 px。
    pub fn mono_font_size(self) -> f32 {
        match self {
            UiDensity::Compact => 11.0,
            UiDensity::Snug | UiDensity::Medium => 12.0,
            UiDensity::Standard | UiDensity::Unknown => 13.0,
            UiDensity::Roomy => 15.0,
        }
    }

    /// 设置页展示用的本地化键。
    pub fn label_key(self) -> &'static str {
        match self {
            UiDensity::Compact => "settings.density_compact",
            UiDensity::Snug => "settings.density_snug",
            UiDensity::Medium => "settings.density_medium",
            UiDensity::Standard | UiDensity::Unknown => "settings.density_standard",
            UiDensity::Roomy => "settings.density_roomy",
        }
    }

    /// 归一化兜底值：写盘前洗成合法状态，保证持久化文件永远可回读。
    pub(crate) fn normalized(self) -> Self {
        match self {
            UiDensity::Unknown => UiDensity::Standard,
            density => density,
        }
    }
}

/// 界面外观偏好：只描述外壳皮肤（明暗、主题、密度），不触碰终端 ANSI 配色
/// 与会话渲染 profile——那两部分由 `TerminalProfile` 承担，两套体系互不覆盖。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    /// 明暗模式；`System` 跟随操作系统。
    pub mode: UiThemeMode,
    /// 用户选择的 UI 主题名；空串表示用内置默认。明暗切换时若该主题的明暗
    /// 属性与生效模式不符，运行期回退到对应明暗的默认主题，但选择值保留，
    /// 切回匹配的明暗后自动恢复用户所选主题。
    pub ui_theme: String,
    /// 界面密度档位（驱动全局字号与 rem 基准）。
    pub density: UiDensity,
    /// 界面字体族覆盖；`None` 时跟随主题自带或系统默认字体。
    ///
    /// 与字号的分工：字号归密度档（整体缩放），字体族是个体偏好——中文用户
    /// 换更纱黑体/思源黑体的诉求只有这一条入口能表达。
    pub ui_font_family: Option<String>,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            mode: UiThemeMode::System,
            ui_theme: String::new(),
            density: UiDensity::Standard,
            ui_font_family: None,
        }
    }
}

impl AppearanceSettings {
    /// 写盘前归一化兜底值，保证持久化状态合法。
    pub fn normalize(&mut self) {
        self.mode = self.mode.normalized();
        self.density = self.density.normalized();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default = "default_language")]
    pub language: String,
    /// 界面外观（明暗、主题、密度）；旧配置缺少该字段时使用内置默认。
    #[serde(default)]
    pub appearance: AppearanceSettings,
    /// 本地终端启动的 shell；`None` 时跟随系统默认。
    ///
    /// 为什么是全局而不是会话级：本地终端从侧栏一键创建、没有会话表单，
    /// shell 是这台机器的属性而非某个会话的属性。
    #[serde(default)]
    pub local_shell: Option<String>,
    #[serde(default)]
    pub terminal_interaction: TerminalInteraction,
    #[serde(default)]
    pub terminal_profile: TerminalProfile,
    #[serde(default)]
    pub window: WindowSettings,
    #[serde(default)]
    pub sftp: SftpSettings,
    #[serde(default)]
    pub shortcuts: ShortcutSettings,
    #[serde(default)]
    pub snippets: SnippetSettings,
    /// 活动栏顺序、隐藏项与上下分区；旧配置缺少该字段时使用默认布局。
    #[serde(default)]
    pub activity_bar: ActivityBarLayout,
    /// 状态栏顺序与隐藏项；旧配置缺少该字段时使用默认布局。
    #[serde(default)]
    pub status_bar: StatusBarLayout,
    /// 云端同步端点与最近同步时间；旧配置缺少该字段时按未启用降级。
    #[serde(default)]
    pub sync: SyncSettings,
    /// ZMODEM 下载目录与自动接收开关；旧配置缺少该字段时使用默认。
    #[serde(default)]
    pub zmodem: crate::zmodem::ZmodemSettings,
    /// 会话输出审计落盘；旧配置缺少该字段时按关闭降级。
    #[serde(default)]
    pub audit: AuditSettings,
    /// 命令历史与建议；旧配置缺少该字段时按默认开启降级。
    #[serde(default)]
    pub history: HistorySettings,
    /// 运行日志级别；旧配置缺少该字段时按默认 `info` 降级。
    #[serde(default)]
    pub logs: LogSettings,
}

/// 运行日志设置。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LogSettings {
    /// 全局记录级别。改完立即生效：`log::set_max_level` 本身就是运行期可写的原子量。
    #[serde(default)]
    pub level: LogLevel,
}

/// 日志级别档位。
///
/// 为什么用枚举而不是裸字符串：级别要同时参与设置持久化、界面下拉与到
/// `log::LevelFilter` 的映射，散着写字符串必然出现"用户调了级别但没生效"这类
/// 最难查的失配。
///
/// 为什么默认 `Info`：这是给日常使用的第一档——生命周期节点与告警都在，量又不大。
/// `Debug` 留给排障（含上下文状态变迁），`Trace` 是协议与字节级数据流，
/// 生产环境常开会把磁盘打满。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Error,
    Warn,
    /// 默认档：`#[serde(default)]` 与 `Default` 都指向它。
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// 全部档位（由严到宽），供设置页下拉与遍历使用。
    pub const ALL: [Self; 5] = [
        Self::Error,
        Self::Warn,
        Self::Info,
        Self::Debug,
        Self::Trace,
    ];

    /// 稳定小写名：同时是设置文件的取值与 i18n key 的后缀（`settings.log_level.{}`）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }

    /// 读取环境变量里的级别覆盖（`TRANSOM_LOG`，兼容更熟悉的 `RUST_LOG`）。
    ///
    /// 为什么留这条通道：用户报障最常见的情形是"应用一起来就来不及改设置"，
    /// 一条环境变量就能把现场捞下来。
    ///
    /// 为什么忽略 `目标=级别` 这类写法：那是过滤器而不是整体级别，硬当级别解析会
    /// 悄悄退成 `info`，用户以为调到了 debug 其实没有；返回 `None` 让设置值继续
    /// 生效更诚实。
    pub fn from_env() -> Option<Self> {
        ["TRANSOM_LOG", "RUST_LOG"]
            .iter()
            .find_map(|key| std::env::var(key).ok())
            .map(|raw| raw.trim().to_string())
            .filter(|raw| !raw.is_empty() && !raw.contains('='))
            .map(|raw| Self::parse(&raw))
    }

    /// 映射到 log 门面的过滤级别。
    pub fn to_filter(self) -> log::LevelFilter {
        match self {
            Self::Error => log::LevelFilter::Error,
            Self::Warn => log::LevelFilter::Warn,
            Self::Info => log::LevelFilter::Info,
            Self::Debug => log::LevelFilter::Debug,
            Self::Trace => log::LevelFilter::Trace,
        }
    }

    /// 解析设置里的级别。
    ///
    /// 设置文件属于外部输入：认不出来的值一律退回默认档 `Info`，绝不允许因为
    /// 一个手写的错值把整个日志关掉（那样反而没有日志可排查）。
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "error" => Self::Error,
            "warn" | "warning" => Self::Warn,
            "debug" => Self::Debug,
            "trace" | "verbose" => Self::Trace,
            _ => Self::Info,
        }
    }
}

/// 反序列化走 [`LogLevel::parse`]：未知值降级为默认档，而不是让整个设置文件报错。
impl<'de> serde::Deserialize<'de> for LogLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = <String as serde::Deserialize>::deserialize(deserializer)?;
        Ok(Self::parse(&raw))
    }
}

/// 命令历史与建议设置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistorySettings {
    /// 总开关：关闭后不再采集命令，也不再弹建议。
    ///
    /// 为什么默认开启：采集到的只是"这一行敲过什么"，与 shell 自己在
    /// `~/.bash_history` 里留的内容同性质，且不落远端；建议功能只有在
    /// 有历史时才出现，关掉它反而让多数人以为功能坏了。
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for HistorySettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// 会话输出审计落盘设置。
///
/// 为什么默认关闭：审计文件会把远端输出的全部内容（含可能出现在屏幕上的
/// 口令、令牌与业务数据）落到本地磁盘。这是合规诉求，但不是每个人都愿意
/// 在不知情时被记录，所以必须由用户显式开启。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditSettings {
    /// 总开关；关闭时不创建文件，也不产生任何额外 IO。
    #[serde(default)]
    pub enabled: bool,
}

/// 快捷命令条上的一键命令。
///
/// 为什么放在全局设置而不是会话里：这条特性的动机是"同一条长命令在几十台机器上反复敲"，
/// 命令本身与具体会话无关；会话级差异由终端里已有的 cd/别名承担。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub id: SnippetId,
    /// 按钮上显示的名字。
    pub name: String,
    /// 实际写入终端的文本。
    pub command: String,
}

/// 名字长度上限：按钮要能并排放进底栏。
const SNIPPET_NAME_MAX_CHARS: usize = 40;
/// 命令长度上限，避免把整段脚本塞进一个按钮后无法点击。
const SNIPPET_COMMAND_MAX_CHARS: usize = 4000;

impl Snippet {
    /// 命令条目的最小合法性：名字与命令都不能为空，长度要有界。
    ///
    /// 校验放在 core 而不是界面层，设置文件属外部输入（可手工编辑、可被导入），
    /// 任何一条不合格就整条丢弃，而不是让底栏画出个空白按钮。
    pub fn is_valid(&self) -> bool {
        let name = self.name.trim();
        let command = self.command.trim();
        !name.is_empty()
            && !command.is_empty()
            && name.chars().count() <= SNIPPET_NAME_MAX_CHARS
            && command.chars().count() <= SNIPPET_COMMAND_MAX_CHARS
    }
}

/// 快捷命令库。命令内容与“直接执行”开关跨重启保持，面板显隐属运行期不在此列。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnippetSettings {
    #[serde(default)]
    pub items: Vec<Snippet>,
    /// 点击命令后是否直接发送并回车。
    ///
    /// 默认关闭：终端里误执行一条危险命令的代价远高于多点一次回车，
    /// 因此开启后由界面明确提示当前处于“点击即执行”状态。
    #[serde(default)]
    pub execute_directly: bool,
}

impl SnippetSettings {
    /// 丢弃不合法条目并按名字去重（同名以后出现的那条为准）。
    pub fn sanitized(mut self) -> Self {
        self.items.retain(Snippet::is_valid);
        let mut deduped: Vec<Snippet> = Vec::with_capacity(self.items.len());
        for item in self.items {
            if let Some(position) = deduped
                .iter()
                .position(|existing| existing.name.trim() == item.name.trim())
            {
                deduped[position] = item;
            } else {
                deduped.push(item);
            }
        }
        self.items = deduped;
        self
    }
}

/// 用户覆盖的快捷键配置。
///
/// 仅保存被用户修改的动作；未出现的动作在运行时使用内置默认值。
/// 每个动作可配置多个备选键位，空列表或全部非法时回退到默认键位，
/// 避免手工编辑配置后出现“完全无法操作”的状态。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ShortcutSettings {
    #[serde(default)]
    pub bindings: BTreeMap<String, Vec<String>>,
}

/// SFTP 传输偏好：排除规则位于设置中，跨会话与重启保持生效。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpSettings {
    #[serde(default = "default_transfer_filter")]
    pub transfer_filter: String,
    /// 传输完成后是否计算并比对两端 SHA256；默认关闭以避免大文件额外读盘。
    #[serde(default)]
    pub verify_after_transfer: bool,
    /// 同时执行的传输任务数上限，默认 3。
    #[serde(default = "default_max_concurrent_transfers")]
    pub max_concurrent_transfers: usize,
    /// 内置编辑器是否软换行：长行折行显示而不出横向滚动条。
    #[serde(default = "default_true")]
    pub editor_soft_wrap: bool,
    /// 内置编辑器是否显示行号。
    #[serde(default = "default_true")]
    pub editor_line_numbers: bool,
    /// 文件面板的显示排除规则（glob 文本，逐条匹配文件名）。
    ///
    /// 与传输过滤共用语法但语义不同：这里只影响面板可见性，不拦截传输。
    #[serde(default)]
    pub panel_hidden_patterns: String,
    /// 内置编辑器可打开的单文件字节上限，默认 8 MiB。
    ///
    /// 设计原理（Why）：编辑远程文本走「全量下载到临时文件 → 读入缓冲区 →
    /// 全量回传覆盖」，文件大小直接决定下载耗时、Rope 常驻内存与高亮重绘开销；
    /// 双击一个几百 MB 的日志会把整份内容拉进界面，因此在打开前按目录列表已知
    /// 的尺寸拦截，并提示改用远端编辑器。超限属于误操作而非故障，故不进设置页，
    /// 由 config.toml 单点承载，需要放宽时改一处即可。
    #[serde(default = "default_edit_max_bytes")]
    pub edit_max_bytes: u64,
    /// 内置编辑器回传覆盖前，是否先把远端当前内容备份为 `{路径}.transom-bak`。
    ///
    /// 设计原理（Why）：回传是"整文件覆盖"而非原子替换，写入中断会在远端留下
    /// 半份文件；备份是 SFTP 协议下零 inode 副作用的最低成本兜底（rename 原子
    /// 替换会改变 inode，可能破坏正在读取该文件的进程，故不采用）。默认关闭：
    /// 备份会在远端留下额外文件，是否可接受属于部署方策略，交给 config.toml
    /// 单点承载（与 edit_max_bytes 同理，不进设置页）。
    #[serde(default)]
    pub editor_write_backup: bool,
}

impl Default for SftpSettings {
    fn default() -> Self {
        Self {
            transfer_filter: default_transfer_filter(),
            verify_after_transfer: false,
            max_concurrent_transfers: default_max_concurrent_transfers(),
            edit_max_bytes: default_edit_max_bytes(),
            editor_write_backup: false,
            editor_soft_wrap: true,
            editor_line_numbers: true,
            panel_hidden_patterns: String::new(),
        }
    }
}

fn default_max_concurrent_transfers() -> usize {
    3
}

fn default_edit_max_bytes() -> u64 {
    8 * 1024 * 1024
}

fn default_transfer_filter() -> String {
    DEFAULT_TRANSFER_EXCLUDES.to_string()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: default_language(),
            appearance: AppearanceSettings::default(),
            local_shell: None,
            terminal_interaction: TerminalInteraction::default(),
            terminal_profile: TerminalProfile::default(),
            window: WindowSettings::default(),
            sftp: SftpSettings::default(),
            shortcuts: ShortcutSettings::default(),
            snippets: SnippetSettings::default(),
            activity_bar: ActivityBarLayout::default(),
            status_bar: StatusBarLayout::default(),
            sync: SyncSettings::default(),
            zmodem: crate::zmodem::ZmodemSettings::default(),
            audit: AuditSettings::default(),
            history: HistorySettings::default(),
            logs: LogSettings::default(),
        }
    }
}

fn default_language() -> String {
    "zh-CN".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 启动档位互斥、档名稳定：设置页分段按钮按 `ALL` 遍历，档名同时是落盘值。
    #[test]
    fn startup_enums_have_stable_names() {
        let tabs: Vec<&str> = StartupTabs::ALL.iter().map(|t| t.as_str()).collect();
        assert_eq!(
            tabs,
            vec!["restore_last", "empty_workspace", "new_local_terminal"]
        );
        let sidebar: Vec<&str> = SidebarStartup::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(sidebar, vec!["remember_last", "always_show", "always_hide"]);
        for tab in StartupTabs::ALL {
            assert_eq!(StartupTabs::parse(tab.as_str()), tab, "档名要能原样读回");
        }
        for startup in SidebarStartup::ALL {
            assert_eq!(SidebarStartup::parse(startup.as_str()), startup);
        }
    }

    /// 侧栏初值的三档决策：显式档位必须压过记忆位，只有 RememberLast 才看上次。
    #[test]
    fn sidebar_startup_overrides_the_remembered_state() {
        let mut window = WindowSettings::default();
        assert!(
            window.sidebar_visible_at_startup(),
            "默认记住上次，且上次是展开"
        );

        window.last_sidebar_visible = false;
        assert!(
            !window.sidebar_visible_at_startup(),
            "RememberLast 应沿用上次的收起状态"
        );

        // 关键回归点：用户某次手动收起后，"始终展开"必须还能救回来，
        // 否则他每次启动都要手动展开一次。
        window.sidebar_startup = SidebarStartup::AlwaysShow;
        assert!(
            window.sidebar_visible_at_startup(),
            "AlwaysShow 要压过记忆位"
        );

        window.last_sidebar_visible = true;
        window.sidebar_startup = SidebarStartup::AlwaysHide;
        assert!(
            !window.sidebar_visible_at_startup(),
            "AlwaysHide 要压过记忆位"
        );
    }

    /// 老设置文件里残留的 `restore_previous_tabs` 布尔被忽略，新字段按默认档走。
    #[test]
    fn legacy_restore_switch_is_ignored_without_breaking_load() {
        let window: WindowSettings =
            serde_json::from_str(r#"{"restore_previous_tabs":false,"window_width":1000.0}"#)
                .expect("未知旧键不应让窗口设置加载失败");
        assert_eq!(window.startup_tabs, StartupTabs::RestoreLast);
        assert_eq!(
            window.sidebar_startup,
            SidebarStartup::RememberLast,
            "缺字段应退回默认档，而不是让整个设置加载失败"
        );
        assert!(window.last_sidebar_visible);
    }

    /// 日志级别来自设置文件（外部输入）：无法识别的值退回默认档 `info`，
    /// 既不能让整份设置加载失败，更不能因为一个错值把日志整个关掉。
    #[test]
    fn log_level_degrades_to_info_on_unknown_value() {
        let loose: AppSettings = serde_json::from_str(r#"{"logs":{"level":"VERBOSE"}}"#).unwrap();
        assert_eq!(loose.logs.level, LogLevel::Trace);

        let unknown: AppSettings = serde_json::from_str(r#"{"logs":{"level":"chatty"}}"#).unwrap();
        assert_eq!(unknown.logs.level, LogLevel::Info);

        let absent: AppSettings = serde_json::from_str(r#"{"logs":{}}"#).unwrap();
        assert_eq!(absent.logs.level, LogLevel::Info);

        let legacy: AppSettings = serde_json::from_str(r#"{"language":"zh-CN"}"#).unwrap();
        assert_eq!(legacy.logs.level, LogLevel::Info);

        // 序列化必须写回稳定小写名，否则往返一次就变成另一个值。
        assert_eq!(serde_json::to_string(&LogLevel::Warn).unwrap(), "\"warn\"");
        assert_eq!(LogLevel::Warn.to_filter(), log::LevelFilter::Warn);
    }

    #[test]
    fn old_settings_without_tab_groups_still_load() {
        let settings: AppSettings = serde_json::from_str(r#"{"language":"zh-CN"}"#).unwrap();
        assert!(settings.window.previous_tab_groups.is_empty());
        assert_eq!(settings.window.previous_tab_sessions.len(), 0);
    }

    /// 通知开关是后加的：老设置里没有它时按"开启 + 10 秒阈值"降级，
    /// 而不是让整个窗口设置加载失败。
    #[test]
    fn notification_defaults_apply_to_legacy_settings() {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        let window = value["window"].as_object_mut().unwrap();
        window.remove("desktop_notifications");
        window.remove("notify_threshold_secs");
        let settings: AppSettings = serde_json::from_value(value).unwrap();
        assert!(settings.window.desktop_notifications);
        assert_eq!(settings.window.notify_threshold_secs, 10);
    }

    /// 快捷命令库是后加的字段：老设置文件里没有它也要能加载，并按空库降级。
    #[test]
    fn settings_without_snippets_still_load() {
        let settings: AppSettings = serde_json::from_str(r#"{"language":"zh-CN"}"#).unwrap();
        assert!(settings.snippets.items.is_empty());
        assert!(!settings.snippets.execute_directly);
    }

    /// 直接执行默认关闭；显式开启后必须原样往返，避免被清洗逻辑吞掉。
    #[test]
    fn snippet_execute_mode_defaults_off_and_round_trips() {
        let decoded: SnippetSettings = serde_json::from_str(r#"{"items":[]}"#).unwrap();
        assert!(!decoded.execute_directly);

        let settings = SnippetSettings {
            execute_directly: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let decoded: SnippetSettings = serde_json::from_str(&json).unwrap();
        assert!(decoded.execute_directly);
    }

    #[test]
    fn snippets_round_trip_and_reject_invalid_entries() {
        let settings = SnippetSettings {
            execute_directly: false,
            items: vec![
                Snippet {
                    id: SnippetId::new(),
                    name: "看日志".into(),
                    command: "journalctl -f".into(),
                },
                Snippet {
                    id: SnippetId::new(),
                    name: "  ".into(),
                    command: "ls".into(),
                },
                Snippet {
                    id: SnippetId::new(),
                    name: "空命令".into(),
                    command: "".into(),
                },
            ],
        };
        let json = serde_json::to_string(&settings).unwrap();
        let decoded: SnippetSettings = serde_json::from_str(&json).unwrap();
        let cleaned = decoded.sanitized();
        assert_eq!(cleaned.items.len(), 1);
        assert_eq!(cleaned.items[0].name, "看日志");
    }

    /// 同名条目留着两个只会让底栏出现两个分不清的按钮，后出现的覆盖先出现的。
    #[test]
    fn snippets_deduplicate_by_name() {
        let settings = SnippetSettings {
            execute_directly: false,
            items: vec![
                Snippet {
                    id: SnippetId::new(),
                    name: "重启".into(),
                    command: "systemctl reboot a".into(),
                },
                Snippet {
                    id: SnippetId::new(),
                    name: "重启 ".into(),
                    command: "systemctl reboot b".into(),
                },
            ],
        };
        let cleaned = settings.sanitized();
        assert_eq!(cleaned.items.len(), 1);
        assert_eq!(cleaned.items[0].command, "systemctl reboot b");
    }

    /// 超长命令不该把底栏撑坏：超过上限的条目直接判为不合法。
    #[test]
    fn snippets_enforce_length_bounds() {
        let long = Snippet {
            id: SnippetId::new(),
            name: "n".into(),
            command: "y".repeat(5_000),
        };
        assert!(!long.is_valid());
        let long_name = Snippet {
            id: SnippetId::new(),
            name: "名".repeat(41),
            command: "ls".into(),
        };
        assert!(!long_name.is_valid());
    }

    #[test]
    fn shortcut_overrides_round_trip() {
        let mut settings = AppSettings::default();
        settings
            .shortcuts
            .bindings
            .insert("new_session".into(), vec!["ctrl-shift-q".into()]);
        let json = serde_json::to_string(&settings).unwrap();
        let decoded: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(
            decoded.shortcuts.bindings.get("new_session"),
            Some(&vec!["ctrl-shift-q".to_string()])
        );
    }

    /// 嵌套布局必须原样往返：窗格数量与每层方向都只能靠树形表达。
    #[test]
    fn saved_tab_groups_round_trip_nested_layout() {
        let mut settings = AppSettings::default();
        settings.window.previous_tab_groups = vec![SavedTabGroup {
            session: "session-id".into(),
            layout: SavedLayout::Split {
                axis: SplitAxis::Horizontal,
                first: Box::new(SavedLayout::Pane),
                second: Box::new(SavedLayout::Split {
                    axis: SplitAxis::Vertical,
                    first: Box::new(SavedLayout::Pane),
                    second: Box::new(SavedLayout::Pane),
                }),
            },
        }];
        let json = serde_json::to_string(&settings).unwrap();
        let decoded: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(
            decoded.window.previous_tab_groups,
            settings.window.previous_tab_groups
        );
    }

    /// 设置文件属外部输入：缺 `layout` 字段时退化为单窗格，而不是让整份设置加载失败。
    #[test]
    fn saved_group_without_layout_degrades_to_single_pane() {
        let decoded: SavedTabGroup = serde_json::from_str(r#"{"session":"local"}"#).unwrap();
        assert_eq!(decoded.layout, SavedLayout::Pane);
    }

    /// 并排视图是后加字段：老设置没有它时按单标签视图启动，且完整往返不丢方向与焦点。
    #[test]
    fn saved_tab_tiling_round_trip_and_missing_field() {
        let decoded: AppSettings = serde_json::from_str(r#"{"language":"zh-CN"}"#).unwrap();
        assert_eq!(decoded.window.tab_tiling, None);

        let mut settings = AppSettings::default();
        settings.window.tab_tiling = Some(SavedTabTiling {
            first: 0,
            second: 2,
            axis: SplitAxis::Vertical,
            focused_second: true,
        });
        let json = serde_json::to_string(&settings).unwrap();
        let decoded: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.window.tab_tiling, settings.window.tab_tiling);
    }

    /// 界面外观是后加的字段：老设置里没有它时按内置默认降级，而不是加载失败。
    #[test]
    fn settings_without_appearance_still_load() {
        let settings: AppSettings = serde_json::from_str(r#"{"language":"zh-CN"}"#).unwrap();
        assert_eq!(settings.appearance.mode, UiThemeMode::System);
        assert!(settings.appearance.ui_theme.is_empty());
        assert_eq!(settings.appearance.density, UiDensity::Standard);
    }

    /// 明暗与密度是外部输入：未知值必须落进兜底变体而不是让整份设置加载失败。
    #[test]
    fn appearance_unknown_values_fall_back_to_defaults() {
        let decoded: AppearanceSettings =
            serde_json::from_str(r#"{"mode":"neon","ui_theme":"Transom Dark","density":"huge"}"#)
                .unwrap();
        assert_eq!(decoded.mode, UiThemeMode::Unknown);
        assert_eq!(decoded.density, UiDensity::Unknown);
        // 兜底值的行为与默认一致，但写盘前必须洗成合法状态。
        assert_eq!(decoded.density.font_size(), UiDensity::Standard.font_size());
        let mut normalized = decoded.clone();
        normalized.normalize();
        assert_eq!(normalized.mode, UiThemeMode::System);
        assert_eq!(normalized.density, UiDensity::Standard);
        assert_eq!(normalized.ui_theme, "Transom Dark");
    }

    /// 外观偏好必须原样往返：主题选择保留在设置里，与运行期回退解耦。
    #[test]
    fn appearance_round_trips() {
        let settings = AppearanceSettings {
            mode: UiThemeMode::Dark,
            ui_theme: "Transom Dark".into(),
            density: UiDensity::Roomy,
            ui_font_family: Some("Sarasa Mono SC".into()),
        };
        let json = serde_json::to_string(&settings).unwrap();
        let decoded: AppearanceSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, settings);
        assert_eq!(decoded.ui_font_family.as_deref(), Some("Sarasa Mono SC"));
    }

    /// 密度档位是全局字号契约：mono 与 sans 的差距必须始终落在 2 至 3px，
    /// 否则等宽区域（SFTP 预览、指纹）与周边文字的比例会随档位失调。
    #[test]
    fn density_sans_mono_gap_stays_within_bounds() {
        for density in UiDensity::ALL {
            let gap = density.font_size() - density.mono_font_size();
            assert!(
                (2.0..=3.0).contains(&gap),
                "{density:?} 的字号差距 {gap} 越界"
            );
        }
    }
}
