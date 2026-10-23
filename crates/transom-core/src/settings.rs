use crate::sftp_filter::DEFAULT_TRANSFER_EXCLUDES;
use crate::terminal::{TerminalInteraction, TerminalProfile};
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
    /// 是否在启动时恢复上次会话标签页
    #[serde(default = "default_true")]
    pub restore_previous_tabs: bool,
    /// 上次打开的会话 ID 列表
    #[serde(default)]
    pub previous_tab_sessions: Vec<String>,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            show_tab_close_button: true,
            close_window_on_all_tabs_closed: false,
            bell_style: BellStyle::default(),
            window_width: default_window_width(),
            window_height: default_window_height(),
            window_x: None,
            window_y: None,
            remember_window_bounds: true,
            restore_previous_tabs: true,
            previous_tab_sessions: Vec::new(),
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub terminal_interaction: TerminalInteraction,
    #[serde(default)]
    pub terminal_profile: TerminalProfile,
    #[serde(default)]
    pub window: WindowSettings,
    #[serde(default)]
    pub sftp: SftpSettings,
}

/// SFTP 传输偏好：排除规则位于设置中，跨会话与重启保持生效。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpSettings {
    #[serde(default = "default_transfer_filter")]
    pub transfer_filter: String,
    /// 传输完成后是否计算并比对两端 SHA256；默认关闭以避免大文件额外读盘。
    #[serde(default)]
    pub verify_after_transfer: bool,
}

impl Default for SftpSettings {
    fn default() -> Self {
        Self {
            transfer_filter: default_transfer_filter(),
            verify_after_transfer: false,
        }
    }
}

fn default_transfer_filter() -> String {
    DEFAULT_TRANSFER_EXCLUDES.to_string()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: default_language(),
            terminal_interaction: TerminalInteraction::default(),
            terminal_profile: TerminalProfile::default(),
            window: WindowSettings::default(),
            sftp: SftpSettings::default(),
        }
    }
}

fn default_language() -> String {
    "zh-CN".to_string()
}
