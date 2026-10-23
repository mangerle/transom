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
    /// 上次打开的标签组与分屏布局；优先于扁平的会话列表。
    #[serde(default)]
    pub previous_tab_groups: Vec<SavedTabGroup>,
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

/// 窗口恢复时重建的一个标签组。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedTabGroup {
    /// `"local"` 或持久化会话 ID；组内窗格共享同一会话配置。
    pub session: String,
    /// 窗格数量；恢复时会被限制在合理范围内。
    #[serde(default = "default_saved_panes")]
    pub panes: usize,
    #[serde(default)]
    pub axis: SplitAxis,
}

fn default_saved_panes() -> usize {
    1
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
            previous_tab_groups: Vec::new(),
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
    /// 同时执行的传输任务数上限，默认 3。
    #[serde(default = "default_max_concurrent_transfers")]
    pub max_concurrent_transfers: usize,
}

impl Default for SftpSettings {
    fn default() -> Self {
        Self {
            transfer_filter: default_transfer_filter(),
            verify_after_transfer: false,
            max_concurrent_transfers: default_max_concurrent_transfers(),
        }
    }
}

fn default_max_concurrent_transfers() -> usize {
    3
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_without_tab_groups_still_load() {
        let settings: AppSettings = serde_json::from_str(r#"{"language":"zh-CN"}"#).unwrap();
        assert!(settings.window.previous_tab_groups.is_empty());
        assert_eq!(settings.window.previous_tab_sessions.len(), 0);
    }

    #[test]
    fn saved_tab_groups_round_trip_axis_and_panes() {
        let mut settings = AppSettings::default();
        settings.window.previous_tab_groups = vec![SavedTabGroup {
            session: "session-id".into(),
            panes: 3,
            axis: SplitAxis::Vertical,
        }];
        let json = serde_json::to_string(&settings).unwrap();
        let decoded: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.window.previous_tab_groups.len(), 1);
        assert_eq!(decoded.window.previous_tab_groups[0].panes, 3);
        assert_eq!(
            decoded.window.previous_tab_groups[0].axis,
            SplitAxis::Vertical
        );
    }
}
