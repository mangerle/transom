//! 左侧槽位的 Git 项目列表：行表构建、顶栏入口与右键菜单。
//!
//! 设计原理（Why）：
//! - 形态照 IDEA 的 Git 工具窗口——列表一行一个仓库，顶部一个「添加」入口，
//!   行内直接显示当前分支与脏文件数：用户挑仓库时要回答的问题就是
//!   「哪个仓库有活没干完」；
//! - 分组只有一层（组下只能挂项目）：因此没有 `parent` 字段，也就不需要会话目录
//!   那套父子环检测，落点语义只剩「组与组换序」和「项目进组」两种；
//! - 行的几何、配色与拖放落点整套复用会话侧栏，具体控件在 `rows` 子模块，
//!   本文件只管「有哪些行、按什么顺序、右键有什么」。

mod entity;
mod rows;

pub(crate) use entity::{GitSidebarEvent, GitSidebarView};

use crate::divider::{DividerEdge, DividerStyled};
use crate::git::types::{GitDropTarget, GitHub, GitSidebarDrag};
use crate::palette::Palette;
use crate::theme::tokens::{size, space};
use crate::widgets::{IconButton, SidebarRowState, action_button, menu_item, sidebar_list_frame};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use transom_core::{GitFolderId, GitProject, GitProjectId};
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

/// Git 侧栏渲染快照入参。
pub(crate) struct GitSidebarParams<'a> {
    pub(crate) view: &'a Entity<GitSidebarView>,
    pub(crate) git: &'a GitHub,
    pub(crate) active_project: Option<GitProjectId>,
    pub(crate) active_branches: &'a HashMap<GitProjectId, String>,
}

/// 侧栏的一行：分组标题或项目。
#[derive(Clone)]
pub(crate) enum GitSidebarRow {
    Folder(GitFolderRow),
    Project(GitProjectRow),
}

/// 分组行的展示数据。
#[derive(Clone)]
pub(crate) struct GitFolderRow {
    pub(crate) id: GitFolderId,
    pub(crate) name: String,
    pub(crate) collapsed: bool,
    /// 组内项目数（折叠时也照实统计，否则折叠后就看不出组里还有没有东西）。
    pub(crate) project_count: usize,
}

/// 项目行的展示数据。
#[derive(Clone)]
pub(crate) struct GitProjectRow {
    pub(crate) project: GitProject,
    /// 是否挂在某个分组下：决定缩进一级。
    pub(crate) in_folder: bool,
}

/// 侧栏内容：标题行 + 分组与项目行 + 空态。
pub(crate) fn render_git_sidebar(params: GitSidebarParams<'_>, p: &Palette) -> AnyElement {
    let folders = git_folder_options(params.git);
    let selected_project = params.git.selected_project;
    let mut rows = sidebar_list_frame("git-sidebar-rows");
    for row in build_git_rows(params.git) {
        rows = match row {
            GitSidebarRow::Folder(row) => {
                rows.child(rows::render_git_folder_row(params.view, &row, p))
            }
            GitSidebarRow::Project(row) => {
                let id = row.project.id;
                let state = if params.active_project == Some(id) {
                    SidebarRowState::Current
                } else if selected_project == Some(id) {
                    SidebarRowState::Selected
                } else {
                    SidebarRowState::Normal
                };
                let badge = params.git.badges.get(&id).cloned().unwrap_or_default();
                let active_branch = params.active_branches.get(&id).map(String::as_str);
                rows.child(rows::render_git_project_row(
                    rows::GitProjectRowProps {
                        view: params.view,
                        row: &row,
                        badge: &badge,
                        active_branch,
                        row_state: state,
                        folders: &folders,
                    },
                    p,
                ))
            }
        };
    }
    if params.git.projects.is_empty() {
        rows = rows.child(render_git_empty(params.view, p));
    }

    let list = v_flex()
        .flex_1()
        .id("git-sidebar-list")
        .overflow_y_scrollbar()
        .child(rows);

    let menu_view = params.view.clone();
    v_flex()
        .size_full()
        .bg(p.background)
        .text_color(p.foreground)
        .child(render_git_sidebar_header(params.view, p))
        .child(list)
        .context_menu(move |menu, _window, _cx| render_git_sidebar_menu(menu, &menu_view))
        .into_any_element()
}

fn emit_sidebar(view: &Entity<GitSidebarView>, event: GitSidebarEvent, cx: &mut App) {
    view.update(cx, |_, cx| cx.emit(event));
}

fn render_git_sidebar_header(view: &Entity<GitSidebarView>, p: &Palette) -> AnyElement {
    let folder_view = view.clone();
    let refresh_view = view.clone();
    let clone_view = view.clone();
    let add_view = view.clone();
    h_flex()
        .h(rems(size::TAB_BAR_HEIGHT))
        .px(rems(space::MD))
        .gap(rems(space::XS))
        .items_center()
        .border_color(p.divider(p.sidebar_border))
        .divider_edge(DividerEdge::Bottom, p.divider_width)
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .flex_1()
                .child(rust_i18n::t!("git.sidebar.title").to_string()),
        )
        .child(
            IconButton::new(
                "git-sidebar-new-folder",
                UiIcon::NEW_FOLDER,
                rust_i18n::t!("git.sidebar.new_folder").to_string(),
            )
            .render()
            .on_click(move |_, _, cx| {
                emit_sidebar(&folder_view, GitSidebarEvent::OpenNewFolderModal, cx);
            }),
        )
        .child(
            IconButton::new(
                "git-sidebar-refresh",
                UiIcon::REFRESH,
                rust_i18n::t!("git.sidebar.refresh").to_string(),
            )
            .render()
            .on_click(move |_, _, cx| {
                emit_sidebar(&refresh_view, GitSidebarEvent::RefreshAll, cx);
            }),
        )
        .child(
            IconButton::new(
                "git-sidebar-clone",
                UiIcon::FETCH,
                rust_i18n::t!("git.clone.title").to_string(),
            )
            .render()
            .on_click(move |_, _, cx| {
                emit_sidebar(&clone_view, GitSidebarEvent::OpenCloneModal, cx);
            }),
        )
        .child(
            IconButton::new(
                "git-sidebar-add",
                UiIcon::ADD,
                rust_i18n::t!("git.sidebar.add").to_string(),
            )
            .render()
            .on_click(move |_, _, cx| {
                emit_sidebar(&add_view, GitSidebarEvent::OpenProjectModal(None), cx);
            }),
        )
        .into_any_element()
}

fn render_git_empty(view: &Entity<GitSidebarView>, p: &Palette) -> AnyElement {
    let add_view = view.clone();
    v_flex()
        .gap_2()
        .p_3()
        .text_sm()
        .text_color(p.muted_foreground)
        .child(rust_i18n::t!("git.sidebar.empty").to_string())
        .child(
            action_button(
                "git-sidebar-add-empty",
                UiIcon::ADD,
                rust_i18n::t!("git.sidebar.add").to_string(),
            )
            .primary()
            .on_click(move |_, _, cx| {
                emit_sidebar(&add_view, GitSidebarEvent::OpenProjectModal(None), cx);
            }),
        )
        .into_any_element()
}

/// 侧栏行表：分组在前（各自紧跟组内项目），未归组的项目排在最后。
///
/// 设计原理（Why）：
/// - 分组折叠时跳过它的项目行：展开的组跟未归组的项目看起来同级（靠缩进区分），
///   收起时就像一个「可点击的文件夹行 + 右侧数量」，不需要专门的状态字段去存可见性；
/// - `folder_id` 指向已删分组的项目自动归入未归组：避免删组后仓库在侧栏"隐身"。
pub(crate) fn build_git_rows(git: &GitHub) -> Vec<GitSidebarRow> {
    let folder_ids: HashSet<GitFolderId> = git.folders.iter().map(|folder| folder.id).collect();
    let mut rows = Vec::with_capacity(git.folders.len() + git.projects.len());
    for folder in &git.folders {
        let count = git
            .projects
            .iter()
            .filter(|project| project.folder_id == Some(folder.id))
            .count();
        rows.push(GitSidebarRow::Folder(GitFolderRow {
            id: folder.id,
            name: folder.name.clone(),
            collapsed: folder.collapsed,
            project_count: count,
        }));
        if !folder.collapsed {
            for project in git
                .projects
                .iter()
                .filter(|project| project.folder_id == Some(folder.id))
            {
                rows.push(GitSidebarRow::Project(GitProjectRow {
                    project: project.clone(),
                    in_folder: true,
                }));
            }
        }
    }
    for project in &git.projects {
        if project
            .folder_id
            .is_none_or(|folder_id| !folder_ids.contains(&folder_id))
        {
            rows.push(GitSidebarRow::Project(GitProjectRow {
                project: project.clone(),
                in_folder: false,
            }));
        }
    }
    rows
}

fn git_folder_options(git: &GitHub) -> Vec<(GitFolderId, String)> {
    git.folders
        .iter()
        .map(|folder| (folder.id, folder.name.clone()))
        .collect()
}

fn render_git_sidebar_menu(menu: PopupMenu, view: &Entity<GitSidebarView>) -> PopupMenu {
    let view_refresh = view.clone();
    let view_folder = view.clone();
    menu.item(menu_item(
        rust_i18n::t!("git.sidebar.refresh_all").to_string(),
        UiIcon::REFRESH,
        move |_, _, cx| {
            emit_sidebar(&view_refresh, GitSidebarEvent::RefreshAll, cx);
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.sidebar.new_folder").to_string(),
        UiIcon::ADD,
        move |_, _, cx| {
            emit_sidebar(&view_folder, GitSidebarEvent::OpenNewFolderModal, cx);
        },
    ))
}

pub(super) fn render_git_project_menu(
    menu: PopupMenu,
    view: Entity<GitSidebarView>,
    project: GitProject,
    folders: Vec<(GitFolderId, String)>,
) -> PopupMenu {
    let id = project.id;
    let view_open = view.clone();
    let view_terminal = view.clone();
    let view_refresh = view.clone();
    let mut menu = menu
        .item(menu_item(
            rust_i18n::t!("git.menu.open").to_string(),
            UiIcon::FOLDER,
            move |_, _, cx| {
                emit_sidebar(
                    &view_open,
                    GitSidebarEvent::OpenProjectTab(project.clone()),
                    cx,
                );
            },
        ))
        .item(menu_item(
            rust_i18n::t!("git.menu.open_in_terminal").to_string(),
            DomainIcon::TERMINAL,
            move |_, _, cx| {
                emit_sidebar(&view_terminal, GitSidebarEvent::OpenInTerminal(id), cx);
            },
        ))
        .item(menu_item(
            rust_i18n::t!("git.menu.refresh").to_string(),
            UiIcon::REFRESH,
            move |_, _, cx| {
                emit_sidebar(&view_refresh, GitSidebarEvent::RefreshProject(id), cx);
            },
        ))
        .item(PopupMenuItem::separator());
    for (folder_id, name) in folders {
        let view_move = view.clone();
        menu = menu.item(menu_item(
            format!("{} {}", rust_i18n::t!("git.menu.move_to"), name),
            UiIcon::FOLDER,
            move |_, _, cx| {
                emit_sidebar(
                    &view_move,
                    GitSidebarEvent::Drop(
                        GitSidebarDrag::Project(id),
                        GitDropTarget::Folder(folder_id),
                    ),
                    cx,
                );
            },
        ));
    }
    let view_root = view.clone();
    menu = menu.item(menu_item(
        rust_i18n::t!("git.menu.move_to_root").to_string(),
        UiIcon::TREE,
        move |_, _, cx| {
            emit_sidebar(&view_root, GitSidebarEvent::MoveProjectToRoot(id), cx);
        },
    ));
    menu.item(menu_item(
        rust_i18n::t!("git.menu.remove").to_string(),
        UiIcon::DELETE,
        move |_, _, cx| {
            emit_sidebar(&view, GitSidebarEvent::RemoveProject(id), cx);
        },
    ))
}

pub(super) fn render_git_folder_menu(
    menu: PopupMenu,
    view: Entity<GitSidebarView>,
    id: GitFolderId,
) -> PopupMenu {
    let view_add = view.clone();
    let view_rename = view.clone();
    menu.item(menu_item(
        rust_i18n::t!("git.folder.new_project").to_string(),
        UiIcon::ADD,
        move |_, _, cx| {
            emit_sidebar(
                &view_add,
                GitSidebarEvent::OpenProjectModal(Some(id)),
                cx,
            );
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.folder.rename").to_string(),
        UiIcon::FOLDER,
        move |_, _, cx| {
            emit_sidebar(
                &view_rename,
                GitSidebarEvent::OpenRenameFolderModal(id),
                cx,
            );
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.folder.delete").to_string(),
        UiIcon::DELETE,
        move |_, _, cx| {
            emit_sidebar(&view, GitSidebarEvent::DeleteFolder(id), cx);
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::{GitSidebarRow, build_git_rows};
    use crate::git::GitHub;
    use std::path::PathBuf;
    use transom_core::{GitFolder, GitProject};

    fn project(name: &str) -> GitProject {
        GitProject::new(PathBuf::from(format!("/srv/{name}")))
    }

    fn labels(rows: &[GitSidebarRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                GitSidebarRow::Folder(row) => format!("组:{}", row.name),
                GitSidebarRow::Project(row) => format!(
                    "项目:{}{}",
                    row.project.display_name(),
                    if row.in_folder { "@组内" } else { "" }
                ),
            })
            .collect()
    }

    #[test]
    fn folders_appear_first_with_collapsed_hiding_children() {
        let mut work = GitFolder::new("工作");
        work.collapsed = true;
        let personal = GitFolder::new("个人");
        let mut p1 = project("alpha");
        p1.folder_id = Some(work.id);
        let mut p2 = project("beta");
        p2.folder_id = Some(personal.id);
        let p3 = project("orphan");

        let hub = GitHub::new(vec![p1, p2, p3], vec![work, personal]);
        let rows = build_git_rows(&hub);
        assert_eq!(
            labels(&rows),
            vec!["组:工作", "组:个人", "项目:beta@组内", "项目:orphan"]
        );
    }

    #[test]
    fn project_with_dangling_folder_falls_back_to_root() {
        let missing = transom_core::GitFolderId::new();
        let mut p = project("lost");
        p.folder_id = Some(missing);
        let hub = GitHub::new(vec![p], vec![]);
        let rows = build_git_rows(&hub);
        assert_eq!(labels(&rows), vec!["项目:lost"]);
    }
}
