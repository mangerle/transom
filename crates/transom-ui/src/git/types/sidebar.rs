//! Git 侧栏状态与拖放类型。

use std::collections::HashMap;
use transom_core::GitProject;

/// Git 子系统的全局状态（跨标签页共享的那一小部分）。
#[derive(Default)]
pub(crate) struct GitHub {
    /// 左侧槽位是否正在显示 Git 项目列表。
    pub sidebar_open: bool,
    /// 已纳入管理的项目列表，顺序即界面显示顺序（用户在侧栏上下拖拽排定，落盘同序）。
    pub projects: Vec<GitProject>,
    /// 单层分组（组标题行），顺序同样是用户拖拽排定的显示顺序。
    ///
    /// 为什么只有一层：分组在这儿的职责是「把十几个仓库按人/服务器归类」，
    /// 嵌套会连带引入父子环检测与缩进对齐成本，收益却几乎为零。
    pub folders: Vec<transom_core::GitFolder>,
    /// 侧栏每行的运行期状态（分支、脏文件数），按项目 id 索引。
    ///
    /// 为什么单独存：侧栏要在没打开标签页时也显示分支与脏数，而这些值只能
    /// 从子进程取；缓存后刷新才有依据，也避免每帧重算。
    pub badges: HashMap<GitProjectKey, GitBadge>,
    /// 在途克隆任务：弹窗关闭后由右下角进度卡接管，跨标签页全局唯一。
    pub clone_task: Option<crate::git::GitCloneTask>,
    /// 在途的新建仓库初始化请求（表单发起）。
    pub pending_init: Option<(transom_core::GitRequestId, crate::git::GitProjectDraft)>,
    /// 在途的已有项目初始化请求。
    pub pending_project_init: Option<(transom_core::GitRequestId, transom_core::GitProjectId)>,
    /// 定时自动抓取远端更新的调度节拍器。
    pub auto_fetch_schedule: crate::cloud_sync::GitSyncScheduler,
}

impl GitHub {
    pub(crate) fn new(projects: Vec<GitProject>, folders: Vec<transom_core::GitFolder>) -> Self {
        Self {
            sidebar_open: false,
            projects,
            folders,
            badges: HashMap::new(),
            clone_task: None,
            pending_init: None,
            pending_project_init: None,
            auto_fetch_schedule: crate::cloud_sync::GitSyncScheduler::default(),
        }
    }
}

/// 侧栏行缓存的键：`GitProjectId` 的薄封装，避免直接依赖其可见性。
pub(crate) type GitProjectKey = transom_core::GitProjectId;

/// 侧栏一行的派生展示信息。
#[derive(Debug, Clone, Default)]
pub(crate) struct GitBadge {
    pub branch: Option<String>,
    pub dirty: usize,
    pub ahead: u32,
    pub behind: u32,
    pub loading: bool,
    pub error: Option<String>,
}

/// 侧栏拖动载荷：项目与分组共用一条通道，同时充当落点锚点。
///
/// 与会话侧栏的 `SidebarDrag` 分开定义：两者都是 UUID 新类型，混用会在编译期
/// 看不出问题，运行时却能把项目 id 当分组 id 用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitSidebarDrag {
    Project(transom_core::GitProjectId),
    Folder(transom_core::GitFolderId),
}

/// 侧栏拖放落点。
///
/// `Folder` 表示放进该分组（落在分组行中段）；`Relative` 表示插到某行的前半区
/// 或后半区。单层分组下只有两类落点有意义：项目跟项目换序（并跟随对方分组）、
/// 分组跟分组换序；其余跨类型组合按"能落进容器"解释或直接忽略。
#[derive(Debug, Clone, Copy)]
pub(crate) enum GitDropTarget {
    Folder(transom_core::GitFolderId),
    Relative {
        anchor: GitSidebarDrag,
        side: transom_core::DropSide,
    },
}
