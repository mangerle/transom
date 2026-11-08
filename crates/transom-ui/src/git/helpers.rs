//! Git 面板的零散辅助能力：状态定位、剪贴板与占位动作。
//!
//! 设计原理（Why）：这些方法被侧栏、标签页与两个模态框共同需要，放在任意
//! 一个视图文件里都会造成反向依赖，单独成模块保持「视图 → 辅助」的单向引用。

use crate::app::AppShell;
use gpui_kit::*;
use std::cmp::Ordering;
use transom_core::{DropSide, GitFolder, GitFolderId, GitProject, GitProjectId, TabId};

// 排序与归组的公共约定（项目与分组共用，各自实现只差在"跟随父级"这一步）：
// - 数组顺序就是显示顺序，也是落盘顺序，所以排序只需回答"落到哪一格"，
//   不维护额外的序号字段——多存一份顺序就多一份会走歪的真相；
// - 必须先摘掉源条目再定位锚点：源在锚点之前时锚点会整体前移一格，
//   按摘除前的下标插入就会偏一位（会话目录踩过同一个坑）；
// - 落在末行之后用 `min` 饱和，避免锚点是最后一行时下标越界 panic；
// - 项目只与同组项目比先后：插到锚点旁边即等价于插到该组内的对应位置，
//   因此扁平数组 + 按组过滤显示不需要额外的组内序号。

/// 把一次拖放落点折算成项目数组里的新位置，并让被移动项跟随锚点所在分组。
///
/// 返回 `false` 表示源或锚点已不在列表里（刷新期间被移除），调用方不落盘。
pub(crate) fn reorder_git_projects(
    projects: &mut Vec<GitProject>,
    id: GitProjectId,
    anchor: GitProjectId,
    side: DropSide,
) -> bool {
    if id == anchor {
        return false;
    }
    let anchor_index = projects.iter().position(|item| item.id == anchor);
    let source_index = projects.iter().position(|item| item.id == id);
    let (Some(anchor_index), Some(source_index)) = (anchor_index, source_index) else {
        return false;
    };
    // 目标分组跟随锚点：跨组拖一次就完成"移动 + 排序"，不用用户先归组再排第二次。
    let target_folder = projects[anchor_index].folder_id;
    let mut project = projects.remove(source_index);
    project.folder_id = target_folder;
    let anchor_index = match source_index.cmp(&anchor_index) {
        Ordering::Less => anchor_index - 1,
        Ordering::Greater | Ordering::Equal => anchor_index,
    };
    let insert_at = match side {
        DropSide::Before => anchor_index,
        DropSide::After => (anchor_index + 1).min(projects.len()),
    };
    projects.insert(insert_at, project);
    true
}

/// 分组之间换序：与项目同一套下标算术，只是没有父级可跟随。
pub(crate) fn reorder_git_folders(
    folders: &mut Vec<GitFolder>,
    id: GitFolderId,
    anchor: GitFolderId,
    side: DropSide,
) -> bool {
    if id == anchor {
        return false;
    }
    let anchor_index = folders.iter().position(|item| item.id == anchor);
    let source_index = folders.iter().position(|item| item.id == id);
    let (Some(anchor_index), Some(source_index)) = (anchor_index, source_index) else {
        return false;
    };
    let folder = folders.remove(source_index);
    let anchor_index = match source_index.cmp(&anchor_index) {
        Ordering::Less => anchor_index - 1,
        Ordering::Greater | Ordering::Equal => anchor_index,
    };
    let insert_at = match side {
        DropSide::Before => anchor_index,
        DropSide::After => (anchor_index + 1).min(folders.len()),
    };
    folders.insert(insert_at, folder);
    true
}

/// 把项目放进某个分组末尾；`None` 表示移出到根层。
///
/// 设计原理（Why）：落在分组行的中段就是"归这个组"，用户没表达组内顺序，
/// 所以按数组末尾追加——显示时组内按数组原序输出，追加即排在组内最后，
/// 也不会打乱用户已经排好的其他成员。
pub(crate) fn move_git_project(
    projects: &mut Vec<GitProject>,
    id: GitProjectId,
    folder: Option<GitFolderId>,
) -> bool {
    let Some(index) = projects.iter().position(|item| item.id == id) else {
        return false;
    };
    if projects[index].folder_id == folder {
        return false;
    }
    let mut project = projects.remove(index);
    project.folder_id = folder;
    projects.push(project);
    true
}

/// 删除分组并把组内项目退回根层（不删仓库，只删这一行组标题）。
///
/// 为什么退回根层而不是删掉项目：分组是视图层的收纳手段，删组等于丢仓库
/// 对用户是数据丢失，必须保守处理。
pub(crate) fn delete_git_folder(
    projects: &mut [GitProject],
    folders: &mut Vec<GitFolder>,
    id: GitFolderId,
) {
    folders.retain(|folder| folder.id != id);
    for project in projects.iter_mut() {
        if project.folder_id == Some(id) {
            project.folder_id = None;
        }
    }
}

/// 在分组列表里按名字找分组（大小写敏感的精确匹配）。
///
/// 为什么精确匹配：分组名是用户自己打的标签，模糊匹配会把「公司」和「公司备份」
/// 混成一个，建组时静默合并比新建一个更像丢数据。
pub(crate) fn find_folder_by_name(
    folders: &[GitFolder],
    name: &str,
) -> Option<transom_core::GitFolderId> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    folders
        .iter()
        .find(|folder| folder.name.trim() == name)
        .map(|folder| folder.id)
}

impl AppShell {
    /// 当前活动标签页对应的 Git 项目 id：侧栏据此给行上激活底色与左竖条。
    pub(crate) fn active_git_project_id(&self, cx: &App) -> Option<GitProjectId> {
        let tab_id = self.state.workspace.active_tab?;
        self.state
            .git_workspace(tab_id)
            .map(|workspace| workspace.read(cx).state.project.id)
    }

    /// 取"当前上下文项目"：`Some(tab_id)` 时按标签定位，否则取第一个 Git 标签。
    ///
    /// 右键菜单只带得动项目 id 之外的信息，这里退回"最近一个 Git 标签"，
    /// 与 IDEA 在工具窗口里对当前仓库执行动作的语义一致。
    pub(crate) fn git_project_of_tab(&self, tab_id: Option<TabId>, cx: &App) -> Option<GitProject> {
        let workspace = match tab_id {
            Some(tab_id) => self.state.git_workspace(tab_id)?,
            None => self
                .state
                .workspace
                .tabs
                .iter()
                .rev()
                .find_map(|tab| tab.git())?,
        };
        Some(workspace.read(cx).state.project.clone())
    }

    /// 切换一个文件的勾选态（提交面板的待提交集）。
    pub(crate) fn toggle_git_checked(
        &mut self,
        project: &GitProject,
        path: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace_of_project(project.id, cx) else {
            return;
        };
        workspace.update(cx, |workspace, _| {
            let checked = &mut workspace.state.checked;
            if !checked.remove(path) {
                checked.insert(path.to_string());
            }
        });
        cx.notify();
    }

    /// 把文本写入系统剪贴板。
    pub(crate) fn write_clipboard(&mut self, text: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        self.state.status = Some(rust_i18n::t!("git.menu.copied").to_string());
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        delete_git_folder, find_folder_by_name, move_git_project, reorder_git_folders,
        reorder_git_projects,
    };
    use std::path::PathBuf;
    use transom_core::{DropSide, GitFolder, GitFolderId, GitProject, GitProjectId};

    fn project(name: &str) -> GitProject {
        GitProject::new(PathBuf::from(format!("/srv/{name}")))
    }

    fn ids(projects: &[GitProject]) -> Vec<GitProjectId> {
        projects.iter().map(|item| item.id).collect()
    }

    #[test]
    fn drop_before_anchor_inserts_ahead_of_it() {
        let (a, b, c) = (project("a"), project("b"), project("c"));
        let mut projects = vec![a.clone(), b.clone(), c.clone()];

        assert!(reorder_git_projects(
            &mut projects,
            c.id,
            a.id,
            DropSide::Before
        ));
        assert_eq!(ids(&projects), vec![c.id, a.id, b.id]);
    }

    /// 源条目在锚点之前：摘除后锚点下标前移一格，插入点必须跟着修正，否则会偏一位。
    #[test]
    fn drop_after_anchor_behind_the_source_keeps_it_directly_after() {
        let (a, b, c) = (project("a"), project("b"), project("c"));
        let mut projects = vec![a.clone(), b.clone(), c.clone()];

        assert!(reorder_git_projects(
            &mut projects,
            a.id,
            c.id,
            DropSide::After
        ));
        assert_eq!(ids(&projects), vec![b.id, c.id, a.id]);
    }

    /// 末行的后半区是最容易越界的一格：饱和到列表末尾而不是 panic。
    #[test]
    fn drop_after_last_row_appends() {
        let (a, b) = (project("a"), project("b"));
        let mut projects = vec![a.clone(), b.clone()];

        assert!(reorder_git_projects(
            &mut projects,
            a.id,
            b.id,
            DropSide::After
        ));
        assert_eq!(ids(&projects), vec![b.id, a.id]);
    }

    #[test]
    fn self_drop_and_unknown_rows_are_no_ops() {
        let (a, b, missing) = (project("a"), project("b"), GitProjectId::new());
        let mut projects = vec![a.clone(), b.clone()];

        assert!(!reorder_git_projects(
            &mut projects,
            a.id,
            a.id,
            DropSide::Before
        ));
        assert!(!reorder_git_projects(
            &mut projects,
            a.id,
            missing,
            DropSide::After
        ));
        assert!(!reorder_git_projects(
            &mut projects,
            missing,
            a.id,
            DropSide::Before
        ));
        assert_eq!(ids(&projects), vec![a.id, b.id]);
    }

    /// 跨组拖一次就完成"移动 + 排序"：被移动项的分组必须跟随锚点项，
    /// 否则用户得先右键归组、再拖第二次排顺序。
    #[test]
    fn project_reorder_follows_anchor_folder() {
        let group = GitFolder::new("公司");
        let mut inside = project("inside");
        inside.folder_id = Some(group.id);
        let outside = project("outside");
        let mut projects = vec![outside.clone(), inside.clone()];

        assert!(reorder_git_projects(
            &mut projects,
            outside.id,
            inside.id,
            DropSide::Before
        ));
        assert_eq!(ids(&projects), vec![outside.id, inside.id]);
        assert_eq!(
            projects[0].folder_id,
            Some(group.id),
            "落到组内条目旁边就等于进这个组"
        );
    }

    #[test]
    fn drop_on_folder_body_appends_to_that_group() {
        let group = GitFolder::new("公司");
        let (a, b) = (project("a"), project("b"));
        let mut projects = vec![a.clone(), b.clone()];

        assert!(move_git_project(&mut projects, b.id, Some(group.id)));
        assert_eq!(projects[0].folder_id, None);
        assert_eq!(projects[1].folder_id, Some(group.id));
        // 已在目标组里再落一次不重排，否则组内顺序会被无意义的拖放打乱。
        assert!(!move_git_project(&mut projects, b.id, Some(group.id)));
        assert!(move_git_project(&mut projects, b.id, None));
        assert_eq!(projects.last().map(|item| item.id), Some(b.id));
    }

    /// 精确匹配：空名不算命中，前后空格忽略，相近名字不模糊合并。
    #[test]
    fn folder_lookup_is_exact_match() {
        let company = GitFolder::new("公司");
        let backup = GitFolder::new("公司备份");
        let folders = vec![company.clone(), backup.clone()];

        assert_eq!(find_folder_by_name(&folders, "公司"), Some(company.id));
        assert_eq!(find_folder_by_name(&folders, "  公司  "), Some(company.id));
        assert_eq!(find_folder_by_name(&folders, ""), None);
        assert_eq!(find_folder_by_name(&folders, "公司备份"), Some(backup.id));
        assert_eq!(find_folder_by_name(&folders, "公司x"), None);
    }

    #[test]
    fn folder_reorder_moves_the_group_row_itself() {
        let (first, second, third) = (
            GitFolder::new("一"),
            GitFolder::new("二"),
            GitFolder::new("三"),
        );
        let mut folders = vec![first.clone(), second.clone(), third.clone()];

        assert!(reorder_git_folders(
            &mut folders,
            third.id,
            first.id,
            DropSide::After
        ));
        assert_eq!(
            folders.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![first.id, third.id, second.id]
        );
        assert!(!reorder_git_folders(
            &mut folders,
            first.id,
            GitFolderId::new(),
            DropSide::Before
        ));
    }

    /// 删组只删这一行组标题，组内项目退回根层：删组等于丢仓库对用户是数据丢失。
    #[test]
    fn deleting_folder_returns_its_projects_to_root() {
        let group = GitFolder::new("公司");
        let mut inside = project("inside");
        inside.folder_id = Some(group.id);
        let outside = project("outside");
        let mut projects = vec![inside.clone(), outside];
        let mut folders = vec![group.clone()];

        delete_git_folder(&mut projects, &mut folders, group.id);

        assert!(folders.is_empty());
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].folder_id, None);
    }
}
