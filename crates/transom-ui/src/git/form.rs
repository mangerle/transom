//! Git 表单的状态类型：宿主、分组字段、添加项目弹窗与项目草稿。
//!
//! 设计原理（Why）：这些类型的生命周期是"一次表单"——开窗建出来、确认或取消就
//! 整体丢掉；而 `GitHub`/`GitBadge`/`GitWorkspaceState` 的生命周期是面板与标签页。
//! 两种生命周期混在一个文件里，改表单时容易碰坏面板状态，反过来也一样。

use gpui_kit::component::input::InputState;
use gpui_kit::{AppContext, Context, Entity, Window};
use transom_core::GitProject;

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
    pub(crate) fn new<V: 'static>(
        window: &mut Window,
        cx: &mut Context<V>,
        placeholder: &str,
    ) -> Self {
        Self {
            name: cx.new(|cx| InputState::new(window, cx).placeholder(placeholder)),
            open: false,
        }
    }

    /// 选中一个分组（空串表示不归组）：写回输入框并收起候选。
    pub(crate) fn pick<V: 'static>(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<V>,
    ) {
        self.open = false;
        self.name.update(cx, |state, cx| {
            state.set_value(name, window, cx);
        });
        cx.notify();
    }
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
