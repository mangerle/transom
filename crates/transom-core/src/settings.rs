use crate::ids::SnippetId;
use crate::sftp_filter::DEFAULT_TRANSFER_EXCLUDES;
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
            restore_previous_tabs: true,
            previous_tab_sessions: Vec::new(),
            previous_tab_groups: Vec::new(),
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
    #[serde(default)]
    pub shortcuts: ShortcutSettings,
    #[serde(default)]
    pub snippets: SnippetSettings,
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

/// 快捷命令库。命令内容跨重启保持，展开状态属运行期不在此列。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnippetSettings {
    #[serde(default)]
    pub items: Vec<Snippet>,
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
    /// 内置编辑器可打开的单文件字节上限，默认 8 MiB。
    ///
    /// 设计原理（Why）：编辑远程文本走「全量下载到临时文件 → 读入缓冲区 →
    /// 全量回传覆盖」，文件大小直接决定下载耗时、Rope 常驻内存与高亮重绘开销；
    /// 双击一个几百 MB 的日志会把整份内容拉进界面，因此在打开前按目录列表已知
    /// 的尺寸拦截，并提示改用远端编辑器。超限属于误操作而非故障，故不进设置页，
    /// 由 config.toml 单点承载，需要放宽时改一处即可。
    #[serde(default = "default_edit_max_bytes")]
    pub edit_max_bytes: u64,
}

impl Default for SftpSettings {
    fn default() -> Self {
        Self {
            transfer_filter: default_transfer_filter(),
            verify_after_transfer: false,
            max_concurrent_transfers: default_max_concurrent_transfers(),
            edit_max_bytes: default_edit_max_bytes(),
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
            terminal_interaction: TerminalInteraction::default(),
            terminal_profile: TerminalProfile::default(),
            window: WindowSettings::default(),
            sftp: SftpSettings::default(),
            shortcuts: ShortcutSettings::default(),
            snippets: SnippetSettings::default(),
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
    }

    #[test]
    fn snippets_round_trip_and_reject_invalid_entries() {
        let settings = SnippetSettings {
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
}
