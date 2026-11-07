//! Git 面板：左侧项目列表 + 工作区标签页（日志/差异）+ 提交模态框。
//!
//! 设计原理（Why）：
//! - 界面完全照 IntelliJ IDEA 的 Git 工具窗口布局组织：侧栏是仓库列表，点开
//!   一个仓库就在右侧开一个标签页；标签页顶部两个入口——「提交」开模态框，
//!   「操作」进日志工作区。用户的心智模型已经在 IDE 里建立，这里不另创一套；
//! - 每个标签页持有一份 [`GitWorkspaceState`]（挂在 `TabContent::Git` 上），
//!   控制器不保存任何界面状态：关闭标签即随视图一起释放，不存在跨标签串数据；
//! - 所有 git 数据都经命令总线异步取回，UI 线程不跑子进程；界面按
//!   `request_id` 丢弃迟到的结果，避免快速切换仓库时旧数据覆盖新视图。

pub(crate) mod actions;
mod bottom_log;
mod commit_modal;
mod commit_page;
mod diff_modal;
mod diff_view;
mod form;
pub(crate) mod graph_paint;
pub(crate) mod log_view;
mod name_modal;
mod project_modal;
mod rebase_modal;
mod shelf;
mod sidebar;
pub(crate) mod types;
mod workspace;
pub(crate) mod worktree_modal;

mod clone_modal;
mod dir_browser;
mod folder_field;
mod helpers;
mod patch;
mod web_link;

pub(crate) use clone_modal::{GitCloneModal, GitCloneTask};
pub(crate) use form::{GitProjectDraft, GitProjectModal};
pub(crate) use name_modal::{GitNameModal, GitNamePurpose};
pub(crate) use rebase_modal::GitRebaseModal;
pub(crate) use types::{GitCommitTab, GitHub, GitWorkspaceState};
pub(crate) use worktree_modal::GitWorktreeModal;

use gpui_kit::assets::IconName;
use transom_core::{ChangeCode, StatusRecord};

/// 变更条目在界面里的视觉表达。
///
/// 设计原理（Why）：IDEA 用「字母 + 颜色」双通道标注状态（M 棕、A 绿、D 红…），
/// 颜色单独不足以区分，故这里同时给出图标与配色键，界面两侧共用一份口径。
pub(crate) struct ChangeVisual {
    /// 状态图标。
    pub icon: IconName,
    /// 配色分组：列表字母与图标共用一套语义色。
    pub accent: ChangeAccent,
}

/// 着色分组，避免每个状态各写一遍调色板取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChangeAccent {
    Added,
    Modified,
    Deleted,
    Conflict,
    Neutral,
}

/// 把状态码折算成界面图标与配色分组。
pub(crate) fn change_visual(code: ChangeCode) -> ChangeVisual {
    match code {
        ChangeCode::Added => ChangeVisual {
            icon: IconName::CirclePlus,
            accent: ChangeAccent::Added,
        },
        ChangeCode::Modified => ChangeVisual {
            icon: IconName::Pen,
            accent: ChangeAccent::Modified,
        },
        ChangeCode::Deleted => ChangeVisual {
            icon: IconName::CircleMinus,
            accent: ChangeAccent::Deleted,
        },
        ChangeCode::Renamed => ChangeVisual {
            icon: IconName::MoveRight,
            accent: ChangeAccent::Modified,
        },
        ChangeCode::Copied => ChangeVisual {
            icon: IconName::Copy,
            accent: ChangeAccent::Added,
        },
        ChangeCode::TypeChange | ChangeCode::Unmerged => ChangeVisual {
            icon: IconName::TriangleAlert,
            accent: ChangeAccent::Conflict,
        },
        ChangeCode::Unmodified => ChangeVisual {
            icon: IconName::Circle,
            accent: ChangeAccent::Neutral,
        },
        ChangeCode::Unknown(_) => ChangeVisual {
            icon: IconName::CircleAlert,
            accent: ChangeAccent::Neutral,
        },
    }
}

/// 未跟踪条目的图标（没有状态码可映射）。
pub(crate) fn untracked_visual() -> ChangeVisual {
    ChangeVisual {
        icon: IconName::CircleDashed,
        accent: ChangeAccent::Neutral,
    }
}

/// 取一条变更用于展示的「主状态」：优先索引侧，其次工作区侧。
///
/// 设计原理（Why）：`AM`（已新增又被改）这类双态条目在列表里只能显示一个字母，
/// IDEA 的口径是先看索引侧；未跟踪与冲突单独成类。
pub(crate) fn primary_record(record: &StatusRecord) -> ChangeVisual {
    match record {
        StatusRecord::Tracked { index, worktree } => {
            if *index == ChangeCode::Unmodified {
                change_visual(*worktree)
            } else {
                change_visual(*index)
            }
        }
        StatusRecord::Conflict => ChangeVisual {
            icon: IconName::TriangleAlert,
            accent: ChangeAccent::Conflict,
        },
        StatusRecord::Untracked => untracked_visual(),
        StatusRecord::Ignored => ChangeVisual {
            icon: IconName::EyeOff,
            accent: ChangeAccent::Neutral,
        },
    }
}

/// 把 Unix 秒折算成 IDEA 风格的相对时间（"3 小时前"）。
pub(crate) fn relative_time(unix_seconds: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default();
    let delta = now.saturating_sub(unix_seconds);
    if delta < 0 {
        return rust_i18n::t!("git.time.future").to_string();
    }
    let (value, key) = if delta < 60 {
        (delta, "git.time.seconds_ago")
    } else if delta < 3600 {
        (delta / 60, "git.time.minutes_ago")
    } else if delta < 86_400 {
        (delta / 3600, "git.time.hours_ago")
    } else {
        (delta / 86_400, "git.time.days_ago")
    };
    rust_i18n::t!(key, count = value).to_string()
}
