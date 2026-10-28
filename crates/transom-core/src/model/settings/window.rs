//! 窗口尺寸、标签页生命周期、多窗格布局与平铺设置。

use crate::ids::SessionId;
use serde::{Deserialize, Serialize};

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
    /// 窗口背景模式：不透明、半透明（透出桌面）或毛玻璃（系统模糊）。
    #[serde(default)]
    pub window_background: WindowBackgroundMode,
    /// 窗口背景透明度（百分比）：0 为完全不透明，100 为完全透明。
    ///
    /// 作用在应用自身的表面色与终端背景上；透出桌面还需要窗口背景模式为
    /// 半透明或毛玻璃，否则窗口底色仍是不透明的。
    #[serde(default)]
    pub window_transparency: u8,
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
    /// 上次退出时的并排网格；每张网格在标签栏里合并成一个条目，可有多张。
    ///
    /// 空列表表示没有并排。旧版的单张快照字段 `tab_tiling` 没有对应结构，
    /// 反序列化时被忽略：升级后并排会退回单标签，其余标签照常恢复。
    #[serde(default)]
    pub tab_tilings: Vec<SavedTabTiling>,
    /// 工具面板（SFTP/转发）停靠方向；旧配置缺省按右侧。
    #[serde(default)]
    pub sftp_panel_side: PanelSide,
    /// 是否把 transom 注册为系统默认终端（仅 Linux 生效）：接管桌面快捷键
    /// 与文件管理器"在终端打开"。属于显式 opt-in，默认关闭，
    /// 与 `.desktop` 模板"不静默抢占默认程序"的设计一致。
    #[serde(default)]
    pub default_terminal: bool,
    /// 最近打开过的会话 ID 列表（按最近使用优先排序，上限 20 个）。
    #[serde(default)]
    pub recent_sessions: Vec<SessionId>,
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

/// 窗口背景模式。
///
/// 对应窗口管理器层面的背景处理：半透明让桌面内容透进窗口，毛玻璃由系统
/// 对窗口后方的内容做模糊（平台不一定支持，退化成半透明）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WindowBackgroundMode {
    /// 完全不透明：窗口由窗口管理器绘制实心底色。
    #[default]
    Opaque,
    /// 半透明：透出桌面内容。
    Transparent,
    /// 毛玻璃：系统模糊窗口后方的内容。
    Blurred,
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

/// 多标签并排视图的恢复快照。
///
/// `slots` 按视觉顺序（从左到右、从上到下）记录每个格子绑定到
/// `previous_tab_groups` 的哪个下标；`None` 表示空格子。旧版的两格快照字段
/// （first/second/axis/focused_second）在新结构里没有对应字段，反序列化时被
/// 忽略；`slots` 为空即视为无有效并排，启动时按单标签恢复。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedTabTiling {
    /// 网格行数。
    #[serde(default)]
    pub rows: usize,
    /// 网格列数。
    #[serde(default)]
    pub cols: usize,
    /// 每个格子的标签组下标；长度必须等于 `rows * cols`。
    #[serde(default)]
    pub slots: Vec<Option<usize>>,
    /// 焦点所在格的下标。
    #[serde(default)]
    pub focused: usize,
    /// 布局是否由自适应算法选出（仅用于下次展示/整理，不参与恢复校验）。
    #[serde(default)]
    pub auto: bool,
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
            window_background: WindowBackgroundMode::default(),
            window_transparency: 0,
            startup_tabs: StartupTabs::default(),
            sidebar_startup: SidebarStartup::default(),
            last_sidebar_visible: true,
            previous_tab_sessions: Vec::new(),
            previous_tab_groups: Vec::new(),
            tab_tilings: Vec::new(),
            sftp_panel_side: PanelSide::Right,
            default_terminal: false,
            recent_sessions: Vec::new(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_settings_recent_sessions_default_and_roundtrip() {
        let default_settings = WindowSettings::default();
        assert!(default_settings.recent_sessions.is_empty());

        let id1 = SessionId::new();
        let id2 = SessionId::new();
        let mut settings = default_settings.clone();
        settings.recent_sessions = vec![id1, id2];

        let json = serde_json::to_string(&settings).expect("必须成功序列化");
        let restored: WindowSettings = serde_json::from_str(&json).expect("必须成功反序列化");
        assert_eq!(restored.recent_sessions, vec![id1, id2]);

        // 测试移除已删除或指定的会话 ID
        let mut filtered = restored.recent_sessions;
        filtered.retain(|id| *id != id1);
        assert_eq!(filtered, vec![id2]);
    }
}
