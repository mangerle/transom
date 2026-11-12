//! 私有 Git 仓库同步配置。

use serde::{Deserialize, Serialize};

/// 私有 Git 仓库同步配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSyncSettings {
    pub remote: String,
    #[serde(default = "default_git_branch")]
    pub branch: String,
    #[serde(default)]
    pub interval_minutes: u64,
}

impl Default for GitSyncSettings {
    fn default() -> Self {
        Self {
            remote: String::new(),
            branch: default_git_branch(),
            interval_minutes: 0,
        }
    }
}

fn default_git_branch() -> String {
    "transom-sync".to_string()
}

impl GitSyncSettings {
    /// 归一化并校验：远端为空或分支名非法时返回 `None`。
    pub fn sanitized(self) -> Option<Self> {
        let remote = self.remote.trim().to_string();
        let branch = self.branch.trim().to_string();
        let branch = if branch.is_empty() {
            default_git_branch()
        } else {
            branch
        };
        let valid_branch = !branch.starts_with('-')
            && branch
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.'));
        (!remote.is_empty() && valid_branch).then_some(Self {
            remote,
            branch,
            interval_minutes: self.interval_minutes.min(7 * 24 * 60),
        })
    }
}
