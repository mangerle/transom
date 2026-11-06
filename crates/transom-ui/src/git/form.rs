//! Git 表单的状态类型：宿主、分组字段、添加项目弹窗与项目草稿。
//!
//! 设计原理（Why）：这些类型的生命周期是"一次表单"——开窗建出来、确认或取消就
//! 整体丢掉；而 `GitHub`/`GitBadge`/`GitWorkspaceState` 的生命周期是面板与标签页。
//! 两种生命周期混在一个文件里，改表单时容易碰坏面板状态，反过来也一样。

use crate::app::AppShell;
use gpui_kit::component::input::InputState;
use gpui_kit::{AppContext, Context, Entity, Window};
use transom_core::{GitProject, GitRequestId};

/// Git 表单状态的宿主：同一个控件既给「打开本机仓库」弹窗用，也给克隆弹窗用。
///
/// 设计原理（Why）：与其把动作写成只认弹窗（克隆那边就得再抄一份列举、缓存、
/// 草稿行、分组候选的逻辑），不如把「谁持有这份状态」显式化——动作按宿主取到
/// 那棵树、那个分组字段，其余逻辑共用一份。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitHost {
    /// 「打开本机仓库」弹窗。
    Modal,
    /// 克隆弹窗。
    Clone,
}

/// 分组字段：可输入新组名，也可从既有分组里选（与会话表单的文件夹字段同一套控件）。
#[derive(Clone)]
pub(crate) struct GitFolderField {
    /// 组名输入框；空表示不归组。
    pub name: Entity<InputState>,
    /// 候选弹层是否展开。
    pub open: bool,
}

impl GitFolderField {
    /// 建一个空的分组字段。
    pub(crate) fn new(window: &mut Window, cx: &mut Context<AppShell>, placeholder: &str) -> Self {
        Self {
            name: cx.new(|cx| InputState::new(window, cx).placeholder(placeholder)),
            open: false,
        }
    }
}

/// 「添加 Git 项目」模态框状态。
pub(crate) struct GitProjectModal {
    /// 目录选择面板：路径框、树与一句错误都在里面（与克隆标签页共用同一套控件）。
    pub picker: super::dir_browser::DirPicker,
    /// 分组字段：可输入新组名，也可从既有分组里选（确认时才建组）。
    pub folder: GitFolderField,
    /// 项目专属图标与环境色（与会话同一套枚举与控件）。
    pub icon: transom_core::SessionIcon,
    pub color: transom_core::SessionColor,
    /// 探测结果：`None` 表示尚未探测或不是仓库。
    pub detected: Option<transom_core::RepoInfo>,
    pub probing: bool,
    /// 目标不是仓库时，是否先 `git init` 再添加。
    ///
    /// 为什么默认不勾：`git init` 会在用户选的目录里造出 `.git`，那是对磁盘的改动，
    /// 不该由一次点击默认发生；勾选框只在「不是仓库」时出现，就是让他自己决定。
    pub init_if_missing: bool,
    pub request_id: GitRequestId,
}

/// 添加项目的入参草稿：路径之外都是可选的外观与归组。
///
/// 设计原理（Why）：
/// - 用结构体而不是五个平铺参数（AGENTS 规范），调用点也不会把 label 与
///   folder_id 的顺序传错；
/// - `git init` 是异步的：初始化成功后要把这份草稿原样落到列表里，参数散在
///   调用栈上就会丢外观，所以草稿必须整体挂起。
#[derive(Debug, Clone)]
pub(crate) struct GitProjectDraft {
    pub path: std::path::PathBuf,
    pub folder_id: Option<transom_core::GitFolderId>,
    pub icon: transom_core::SessionIcon,
    pub color: transom_core::SessionColor,
}

impl GitProjectDraft {
    /// 只给路径的最小草稿（工作树等内部流程用，不带外观选择）。
    pub(crate) fn new(path: std::path::PathBuf) -> Self {
        Self {
            path,
            folder_id: None,
            icon: transom_core::SessionIcon::default(),
            color: transom_core::SessionColor::default(),
        }
    }

    /// 落成一条待登记的项目记录。
    ///
    /// 显示名不再由表单提供：行、标签页与提交面板都按目录名展示，同一个仓库出现
    /// 两份名字（目录名与用户别名）反而让人对不上号。
    pub(crate) fn into_project(self) -> GitProject {
        let mut project = GitProject::new(self.path);
        project.folder_id = self.folder_id;
        project.icon = self.icon;
        project.color = self.color;
        project
    }
}
