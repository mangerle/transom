//! Git 版本控制设置（参考 openidea / IntelliJ IDEA 设置体系）。
//!
//! 设计原理（Why）：
//! - 将版本控制相关偏好独立归档，集中管理 Git 可执行程序路径、提交推送安全与差异对比偏好；
//! - 各字段均提供合理的内置默认值（`#[serde(default)]`），旧配置文件缺少相应字段时能平滑升降级。

use serde::{Deserialize, Serialize};

fn default_executable_path() -> String {
    "git".to_string()
}

fn default_true() -> bool {
    true
}

fn default_diff_mode() -> String {
    "unified".to_string()
}

fn default_protected_branches() -> String {
    "main, master".to_string()
}

fn default_follow_tags() -> String {
    "follow".to_string()
}

fn default_auto_fetch() -> u32 {
    15
}

fn default_diff_highlight() -> String {
    "word".to_string()
}

/// Git 版本控制偏好设置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSettings {
    /// Git 可执行文件路径（支持自定义完整路径或系统 PATH 查找）。
    #[serde(default = "default_executable_path")]
    pub executable_path: String,
    /// 窗口聚焦时自动刷新仓库状态（`git status`）。
    #[serde(default = "default_true")]
    pub auto_update_on_focus: bool,
    /// 定时自动抓取远端更新的间隔分钟数（0 表示关闭）。
    #[serde(default = "default_auto_fetch")]
    pub auto_fetch_interval: u32,
    /// 执行推送前始终弹出推送提交核对对话框（Push Commits Dialog）。
    #[serde(default = "default_true")]
    pub push_dialog: bool,
    /// 提交成功后自动清理提交消息输入框草稿。
    #[serde(default = "default_true")]
    pub clear_message_on_commit: bool,
    /// 提交前若存在未跟踪改动则发出告警提示。
    #[serde(default = "default_true")]
    pub warn_uncommitted: bool,
    /// 默认推送标签策略（"none"、"follow" 或 "all"）。
    #[serde(default = "default_follow_tags")]
    pub default_push_tags: String,
    /// 默认差异对比视图形态（"unified" 单栏合并 或 "split" 双栏并排）。
    #[serde(default = "default_diff_mode")]
    pub default_diff_mode: String,
    /// 差异对比中默认忽略空白字符改动。
    #[serde(default)]
    pub ignore_whitespace: bool,
    /// 差异视图精细空白忽略档位（"none" / "trim" / "all" / "blank"）。
    #[serde(default)]
    pub diff_ignore_whitespace_level: Option<String>,
    /// 差异视图显示行号。
    #[serde(default = "default_true")]
    pub diff_show_line_numbers: bool,
    /// 差异视图长行自动换行。
    #[serde(default = "default_true")]
    pub diff_wrap_lines: bool,
    /// 差异高亮粒度（"word" / "line" / "none"）。
    #[serde(default = "default_diff_highlight")]
    pub diff_highlight: String,
    /// 差异视图默认折叠未变更上下文行。
    #[serde(default = "default_true")]
    pub diff_collapse_unchanged: bool,
    /// 受保护的分支名（逗号分隔，推送时若涉及此类分支将触发二次防呆警告）。
    #[serde(default = "default_protected_branches")]
    pub protected_branches: String,
    /// 全局 Git 提交作者名（为空时遵循系统或仓库 .gitconfig）。
    #[serde(default)]
    pub author_name: String,
    /// 全局 Git 提交邮箱（为空时遵循系统或仓库 .gitconfig）。
    #[serde(default)]
    pub author_email: String,
}

impl Default for GitSettings {
    fn default() -> Self {
        Self {
            executable_path: default_executable_path(),
            auto_update_on_focus: true,
            auto_fetch_interval: default_auto_fetch(),
            push_dialog: true,
            clear_message_on_commit: true,
            warn_uncommitted: true,
            default_push_tags: default_follow_tags(),
            default_diff_mode: default_diff_mode(),
            ignore_whitespace: false,
            diff_ignore_whitespace_level: None,
            diff_show_line_numbers: true,
            diff_wrap_lines: true,
            diff_highlight: default_diff_highlight(),
            diff_collapse_unchanged: true,
            protected_branches: default_protected_branches(),
            author_name: String::new(),
            author_email: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_settings_default_and_roundtrip() {
        let defaults = GitSettings::default();
        assert_eq!(defaults.executable_path, "git");
        assert!(defaults.auto_update_on_focus);
        assert_eq!(defaults.auto_fetch_interval, 15);
        assert!(defaults.push_dialog);
        assert_eq!(defaults.default_push_tags, "follow");
        assert_eq!(defaults.protected_branches, "main, master");

        let json = serde_json::to_string(&defaults).unwrap();
        let loaded: GitSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(defaults, loaded);

        // 缺省旧 JSON 反序列化平滑降级
        let legacy: GitSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.executable_path, "git");
        assert!(legacy.warn_uncommitted);
    }
}
