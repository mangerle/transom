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

/// Git 版本控制偏好设置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSettings {
    /// Git 可执行文件路径（支持自定义完整路径或系统 PATH 查找）。
    #[serde(default = "default_executable_path")]
    pub executable_path: String,
    /// 窗口聚焦时自动刷新仓库状态（`git status`）。
    #[serde(default = "default_true")]
    pub auto_update_on_focus: bool,
    /// 执行推送前始终弹出推送提交核对对话框（Push Commits Dialog）。
    #[serde(default = "default_true")]
    pub push_dialog: bool,
    /// 提交成功后自动清理提交消息输入框草稿。
    #[serde(default = "default_true")]
    pub clear_message_on_commit: bool,
    /// 默认差异对比视图形态（"unified" 单栏合并 或 "split" 双栏并排）。
    #[serde(default = "default_diff_mode")]
    pub default_diff_mode: String,
    /// 差异对比中默认忽略空白字符改动。
    #[serde(default)]
    pub ignore_whitespace: bool,
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
            push_dialog: true,
            clear_message_on_commit: true,
            default_diff_mode: default_diff_mode(),
            ignore_whitespace: false,
            protected_branches: default_protected_branches(),
            author_name: String::new(),
            author_email: String::new(),
        }
    }
}
