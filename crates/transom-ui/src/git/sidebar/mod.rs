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

mod rows;

use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled};
use crate::git::types::{GitDropTarget, GitHub, GitSidebarDrag};
use crate::palette::Palette;
use crate::widgets::menu_item;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use std::collections::HashSet;
use transom_core::{GitFolderId, GitProject, GitProjectId};
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

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

impl AppShell {
    /// 侧栏内容：标题行 + 分组与项目行 + 空态。
    pub(crate) fn render_git_sidebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let rows = build_git_rows(&self.state.git);
        // 分组列表在渲染期就取好带走：右键菜单闭包运行时再回查 `self.state.git` 的借用不成立。
        let folders = git_folder_options(&self.state.git);
        let active_project = self.active_git_project_id(cx);
        let mut list = v_flex()
            .flex_1()
            .id("git-sidebar-list")
            .overflow_y_scrollbar();
        for row in rows {
            list = match row {
                GitSidebarRow::Folder(row) => list.child(self.render_git_folder_row(&row, p, cx)),
                GitSidebarRow::Project(row) => list.child(self.render_git_project_row(
                    &row,
                    active_project == Some(row.project.id),
                    &folders,
                    p,
                    cx,
                )),
            };
        }
        if self.state.git.projects.is_empty() {
            list = list.child(self.render_git_empty(p, cx));
        }

        let app = cx.entity().clone();
        v_flex()
            .size_full()
            .bg(p.sidebar)
            .text_color(p.sidebar_foreground)
            .child(self.render_git_sidebar_header(window, p, cx))
            .child(list)
            .context_menu(move |menu, _window, _cx| render_git_sidebar_menu(menu, &app))
            .into_any_element()
    }

    fn render_git_sidebar_header(
        &mut self,
        _window: &mut Window,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .px_2()
            .py_1p5()
            .gap_1()
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
                Button::new("git-sidebar-new-folder")
                    .ghost()
                    .xsmall()
                    .icon(UiIcon::ADD)
                    .accessibility_label(rust_i18n::t!("git.sidebar.new_folder").to_string())
                    .tooltip(rust_i18n::t!("git.sidebar.new_folder").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_git_folder_modal(window, cx);
                    })),
            )
            .child(
                Button::new("git-sidebar-refresh")
                    .ghost()
                    .xsmall()
                    .icon(UiIcon::REFRESH)
                    .accessibility_label(rust_i18n::t!("git.sidebar.refresh").to_string())
                    .tooltip(rust_i18n::t!("git.sidebar.refresh").to_string())
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.refresh_all_git_projects(cx);
                    })),
            )
            .child(
                Button::new("git-sidebar-clone")
                    .ghost()
                    .xsmall()
                    .icon(UiIcon::FETCH)
                    .accessibility_label(rust_i18n::t!("git.clone.title").to_string())
                    .tooltip(rust_i18n::t!("git.clone.title").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_git_clone_modal(window, cx);
                    })),
            )
            .child(
                Button::new("git-sidebar-add")
                    .ghost()
                    .xsmall()
                    .icon(UiIcon::ADD)
                    .accessibility_label(rust_i18n::t!("git.sidebar.add").to_string())
                    .tooltip(rust_i18n::t!("git.sidebar.add").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_git_project_modal(None, window, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_git_empty(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_2()
            .p_3()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("git.sidebar.empty").to_string())
            .child(
                Button::new("git-sidebar-add-empty")
                    .primary()
                    .small()
                    .icon(UiIcon::ADD)
                    .label(rust_i18n::t!("git.sidebar.add").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_git_project_modal(None, window, cx);
                    })),
            )
            .into_any_element()
    }
}

/// 侧栏行表：分组在前（各自紧跟组内项目），未归组的项目排在最后。
///
/// 设计原理（Why）：
/// - 严格照数组原始顺序输出，绝不按名称排序：数组顺序就是用户拖拽排定的显示顺序，
///   一旦在这里排序，拖拽结果会被静默覆盖（会话侧栏 `tree/rows.rs` 同一教训）；
/// - 折叠只影响显示，不影响归属：组折叠时组内项目不出现，但仍在项目列表里，
///   刷新、标签页与提交面板都不受影响；
/// - `folder_id` 指向已不存在的分组（手改过文件或旧数据）时按根层处理：
///   宁可让仓库出现在根层，也不能让它从列表里消失。
fn build_git_rows(git: &GitHub) -> Vec<GitSidebarRow> {
    let known: HashSet<GitFolderId> = git.folders.iter().map(|folder| folder.id).collect();
    let mut rows = Vec::with_capacity(git.projects.len() + git.folders.len());
    for folder in &git.folders {
        let members: Vec<&GitProject> = git
            .projects
            .iter()
            .filter(|project| project.folder_id == Some(folder.id))
            .collect();
        rows.push(GitSidebarRow::Folder(GitFolderRow {
            id: folder.id,
            name: folder.name.clone(),
            collapsed: folder.collapsed,
            project_count: members.len(),
        }));
        if folder.collapsed {
            continue;
        }
        rows.extend(members.into_iter().cloned().map(|project| {
            GitSidebarRow::Project(GitProjectRow {
                project,
                in_folder: true,
            })
        }));
    }
    rows.extend(
        git.projects
            .iter()
            .filter(|project| !project.folder_id.is_some_and(|id| known.contains(&id)))
            .cloned()
            .map(|project| {
                GitSidebarRow::Project(GitProjectRow {
                    project,
                    in_folder: false,
                })
            }),
    );
    rows
}

/// 分组下拉/菜单用的 (id, 名称) 列表：渲染期就取好带走，菜单闭包运行时再回查
/// `self.state.git` 的借用不成立。
fn git_folder_options(git: &GitHub) -> Vec<(GitFolderId, String)> {
    git.folders
        .iter()
        .map(|folder| (folder.id, folder.name.clone()))
        .collect()
}

/// 侧栏空白处右键：整体刷新 + 新建分组。
fn render_git_sidebar_menu(menu: PopupMenu, app: &Entity<AppShell>) -> PopupMenu {
    let app_refresh = app.clone();
    let app_folder = app.clone();
    menu.item(menu_item(
        rust_i18n::t!("git.sidebar.refresh_all").to_string(),
        UiIcon::REFRESH,
        move |_, _, cx| {
            app_refresh.update(cx, |this, cx| this.refresh_all_git_projects(cx));
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.sidebar.new_folder").to_string(),
        UiIcon::ADD,
        move |_, window, cx| {
            app_folder.update(cx, |this, cx| this.open_git_folder_modal(window, cx));
        },
    ))
}

/// 单行项目的右键菜单：打开 / 在本地终端打开 / 刷新 / 移到分组 / 移除。
fn render_git_project_menu(
    menu: PopupMenu,
    app: Entity<AppShell>,
    id: GitProjectId,
    folders: Vec<(GitFolderId, String)>,
) -> PopupMenu {
    let app_refresh = app.clone();
    let app_open = app.clone();
    let app_terminal = app.clone();
    let mut menu = menu
        .item(menu_item(
            rust_i18n::t!("git.menu.open").to_string(),
            UiIcon::FOLDER,
            move |_, window, cx| {
                let found = app_open.update(cx, |this, _cx| {
                    this.state
                        .git
                        .projects
                        .iter()
                        .find(|item| item.id == id)
                        .cloned()
                });
                let Some(project) = found else {
                    return;
                };
                app_open.update(cx, |this, cx| this.open_git_tab(project, window, cx));
            },
        ))
        .item(menu_item(
            rust_i18n::t!("git.menu.open_in_terminal").to_string(),
            DomainIcon::TERMINAL,
            move |_, _, cx| {
                app_terminal.update(cx, |this, cx| {
                    // 目录不能只在渲染期捕获：菜单弹出后项目可能被移除，点击时按 id 现取。
                    let path = this
                        .state
                        .git
                        .projects
                        .iter()
                        .find(|item| item.id == id)
                        .map(|item| item.path.clone());
                    if let Some(path) = path {
                        this.spawn_local_terminal(Some(path), cx);
                    }
                });
            },
        ))
        .item(menu_item(
            rust_i18n::t!("git.menu.refresh").to_string(),
            UiIcon::REFRESH,
            move |_, _, cx| {
                app_refresh.update(cx, |this, cx| {
                    if let Some(project) = this
                        .state
                        .git
                        .projects
                        .iter()
                        .find(|item| item.id == id)
                        .cloned()
                    {
                        this.git_refresh(&project, cx);
                    }
                });
            },
        ))
        .item(PopupMenuItem::separator());
    // 「移到分组」逐个组列出，再加一项退回根层：单层分组下没有层级可藏，
    // 平铺出来最省一次点击。
    for (folder_id, name) in folders {
        let app_move = app.clone();
        menu = menu.item(menu_item(
            format!("{} {}", rust_i18n::t!("git.menu.move_to"), name),
            UiIcon::FOLDER,
            move |_, _, cx| {
                app_move.update(cx, |this, cx| {
                    this.apply_git_drop(
                        GitSidebarDrag::Project(id),
                        GitDropTarget::Folder(folder_id),
                        cx,
                    );
                });
            },
        ));
    }
    let app_root = app.clone();
    menu = menu.item(menu_item(
        rust_i18n::t!("git.menu.move_to_root").to_string(),
        UiIcon::TREE,
        move |_, _, cx| {
            app_root.update(cx, |this, cx| this.git_move_project_to_root(id, cx));
        },
    ));
    let app_remove = app;
    menu.item(menu_item(
        rust_i18n::t!("git.menu.remove").to_string(),
        UiIcon::DELETE,
        move |_, _, cx| {
            app_remove.update(cx, |this, cx| this.remove_git_project(id, cx));
        },
    ))
}

/// 分组行的右键菜单：重命名 / 删除。
fn render_git_folder_menu(menu: PopupMenu, app: Entity<AppShell>, id: GitFolderId) -> PopupMenu {
    let app_add = app.clone();
    let app_rename = app.clone();
    let app_delete = app;
    menu.item(menu_item(
        rust_i18n::t!("git.folder.new_project").to_string(),
        UiIcon::ADD,
        move |_, window, cx| {
            app_add.update(cx, |this, cx| {
                this.open_git_project_modal(Some(id), window, cx);
            });
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.folder.rename").to_string(),
        UiIcon::FOLDER,
        move |_, window, cx| {
            app_rename.update(cx, |this, cx| {
                this.open_git_rename_folder_modal(id, window, cx);
            });
        },
    ))
    .item(menu_item(
        rust_i18n::t!("git.folder.delete").to_string(),
        UiIcon::DELETE,
        move |_, _, cx| {
            app_delete.update(cx, |this, cx| this.delete_git_folder(id, cx));
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

    /// 行表读起来是「组:名字 / 项目:名字」，折叠与归属都体现在行的顺序上。
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
    fn folders_group_their_projects_and_ungrouped_come_last() {
        let group = GitFolder::new("公司");
        let mut in_group = project("beta");
        in_group.folder_id = Some(group.id);
        // 数组顺序就是显示顺序：仓库名 beta 排在 alpha 之前也不会被重排。
        let git = GitHub {
            projects: vec![in_group, project("alpha")],
            folders: vec![group],
            ..GitHub::default()
        };

        assert_eq!(
            labels(&build_git_rows(&git)),
            vec!["组:公司", "项目:beta@组内", "项目:alpha"]
        );
    }

    #[test]
    fn collapsed_folder_hides_members_but_still_counts_them() {
        let mut group = GitFolder::new("公司");
        group.collapsed = true;
        let mut member = project("beta");
        member.folder_id = Some(group.id);
        let git = GitHub {
            projects: vec![member],
            folders: vec![group],
            ..GitHub::default()
        };

        let rows = build_git_rows(&git);
        assert_eq!(labels(&rows), vec!["组:公司"]);
        match &rows[0] {
            GitSidebarRow::Folder(row) => assert_eq!(
                row.project_count, 1,
                "折叠时计数照实统计，否则看不出组里还有没有东西"
            ),
            _ => panic!("首行应为分组"),
        }
    }

    /// 分组被删掉（或文件被手改过）时，项目退回根层而不是从列表里消失。
    #[test]
    fn unknown_folder_falls_back_to_root() {
        let mut orphan = project("orphan");
        orphan.folder_id = Some(transom_core::GitFolderId::new());
        let git = GitHub {
            projects: vec![orphan],
            folders: Vec::new(),
            ..GitHub::default()
        };

        assert_eq!(labels(&build_git_rows(&git)), vec!["项目:orphan"]);
    }
}
