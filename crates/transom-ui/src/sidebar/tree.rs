//! 会话树行构建、拖拽载荷与文件夹/会话行渲染。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use transom_core::{Connection, FolderId, Session, SessionCatalog, SessionFolder, SessionId};

/// 侧边栏拖动载荷：会话与文件夹共用一条拖放通道。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SidebarDrag {
    Session(SessionId),
    Folder(FolderId),
}

/// 拖动时跟随鼠标的轻量预览卡片。
pub(crate) struct SidebarDragPreview {
    label: String,
    icon: IconName,
}

impl SidebarDragPreview {
    fn new(label: String, icon: IconName) -> Self {
        Self { label, icon }
    }
}

impl Render for SidebarDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        h_flex()
            .px_3()
            .py_1p5()
            .gap_2()
            .items_center()
            .rounded_md()
            .bg(p.popover)
            .border_1()
            .border_color(p.border)
            .shadow_md()
            .text_sm()
            .text_color(p.popover_foreground)
            .child(Icon::new(self.icon).xsmall().text_color(p.muted_foreground))
            .child(self.label.clone())
    }
}

#[derive(Clone)]
pub(crate) struct FolderRow {
    pub(crate) id: FolderId,
    pub(crate) name: String,
    pub(crate) depth: usize,
    pub(crate) collapsed: bool,
    pub(crate) session_count: usize,
    pub(crate) folder_count: usize,
}

#[derive(Clone)]
pub(crate) struct SessionRow {
    pub(crate) session: Box<Session>,
    pub(crate) depth: usize,
}

#[derive(Clone)]
pub(crate) enum SidebarRow {
    Folder(FolderRow),
    Session(SessionRow),
}

impl SidebarRow {
    pub(crate) fn height(&self) -> Pixels {
        match self {
            Self::Folder(_) => px(30.0),
            Self::Session(_) => px(44.0),
        }
    }
}

/// 构建当前过滤条件下的扁平行序列。
///
/// 使用显式栈而不是递归遍历：文件夹深度来自用户输入，恶意或异常数据可能
/// 很深；显式栈保证不会因深树触发调用栈溢出。
pub(crate) fn build_rows(catalog: &SessionCatalog, query: &str) -> Vec<SidebarRow> {
    let query = query.trim().to_ascii_lowercase();
    let mut child_folders: HashMap<Option<FolderId>, Vec<&SessionFolder>> = HashMap::new();
    for folder in &catalog.folders {
        child_folders.entry(folder.parent).or_default().push(folder);
    }
    for folders in child_folders.values_mut() {
        folders.sort_by(|a, b| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        });
    }
    let mut child_sessions: HashMap<Option<FolderId>, Vec<&Session>> = HashMap::new();
    for session in &catalog.sessions {
        child_sessions
            .entry(session.folder_id)
            .or_default()
            .push(session);
    }
    for sessions in child_sessions.values_mut() {
        sessions.sort_by(|a, b| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        });
    }

    let (session_counts, folder_counts) = subtree_counts(catalog);
    let folder_paths: HashMap<FolderId, String> = catalog
        .folders
        .iter()
        .filter_map(|folder| catalog.folder_path(folder.id).map(|path| (folder.id, path)))
        .collect();

    let matched_sessions: HashSet<SessionId> = catalog
        .sessions
        .iter()
        .filter(|session| {
            query.is_empty()
                || session_matches(
                    session,
                    &query,
                    session
                        .folder_id
                        .and_then(|id| folder_paths.get(&id))
                        .map(String::as_str)
                        .unwrap_or_default(),
                )
        })
        .map(|session| session.id)
        .collect();

    // 名称命中的文件夹会展示整个子树；其余文件夹仅在包含命中会话时显示。
    let mut visible_folders = HashSet::new();
    for folder in &catalog.folders {
        let name_matches = !query.is_empty() && folder.name.to_ascii_lowercase().contains(&query);
        let has_session = child_sessions
            .get(&Some(folder.id))
            .is_some_and(|sessions| {
                sessions
                    .iter()
                    .any(|session| matched_sessions.contains(&session.id))
            });
        if name_matches || has_session {
            let mut current = Some(folder.id);
            while let Some(id) = current {
                if !visible_folders.insert(id) {
                    break;
                }
                current = catalog.folder(id).and_then(|folder| folder.parent);
            }
        }
    }

    enum Pending {
        Folder(FolderId, usize, bool),
        SessionList(Option<FolderId>, usize, bool),
    }

    let mut rows = Vec::with_capacity(catalog.sessions.len() + catalog.folders.len());
    let mut stack = vec![Pending::SessionList(None, 0, false)];
    if let Some(roots) = child_folders.get(&None) {
        for folder in roots.iter().rev() {
            stack.push(Pending::Folder(folder.id, 0, false));
        }
    }

    while let Some(item) = stack.pop() {
        match item {
            Pending::SessionList(parent, depth, show_all) => {
                let Some(sessions) = child_sessions.get(&parent) else {
                    continue;
                };
                for session in sessions {
                    if query.is_empty() || show_all || matched_sessions.contains(&session.id) {
                        rows.push(SidebarRow::Session(SessionRow {
                            session: Box::new((*session).clone()),
                            depth,
                        }));
                    }
                }
            }
            Pending::Folder(id, depth, ancestor_matched) => {
                let Some(folder) = catalog.folder(id) else {
                    continue;
                };
                let name_matched =
                    !query.is_empty() && folder.name.to_ascii_lowercase().contains(&query);
                let show_all = ancestor_matched || name_matched;
                if !query.is_empty() && !show_all && !visible_folders.contains(&id) {
                    continue;
                }
                rows.push(SidebarRow::Folder(FolderRow {
                    id,
                    name: folder.name.clone(),
                    depth,
                    collapsed: folder.collapsed && query.is_empty(),
                    session_count: session_counts.get(&id).copied().unwrap_or(0),
                    folder_count: folder_counts.get(&id).copied().unwrap_or(0),
                }));
                if query.is_empty() && folder.collapsed {
                    continue;
                }
                stack.push(Pending::SessionList(Some(id), depth + 1, show_all));
                if let Some(children) = child_folders.get(&Some(id)) {
                    for child in children.iter().rev() {
                        stack.push(Pending::Folder(child.id, depth + 1, show_all));
                    }
                }
            }
        }
    }
    rows
}

/// 统计每个文件夹子树中的会话与子文件夹总数。
///
/// 通过沿父链向上累加实现，避免递归；`catalog.normalize()` 已保证无环。
fn subtree_counts(
    catalog: &SessionCatalog,
) -> (HashMap<FolderId, usize>, HashMap<FolderId, usize>) {
    let mut direct_sessions: HashMap<Option<FolderId>, usize> = HashMap::new();
    for session in &catalog.sessions {
        *direct_sessions.entry(session.folder_id).or_default() += 1;
    }
    let mut session_counts: HashMap<FolderId, usize> = HashMap::new();
    let mut folder_counts: HashMap<FolderId, usize> = HashMap::new();
    for folder in &catalog.folders {
        let direct = direct_sessions.get(&Some(folder.id)).copied().unwrap_or(0);
        *session_counts.entry(folder.id).or_default() += direct;
        let mut current = folder.parent;
        while let Some(parent) = current {
            *session_counts.entry(parent).or_default() += direct;
            *folder_counts.entry(parent).or_default() += 1;
            current = catalog.folder(parent).and_then(|folder| folder.parent);
        }
    }
    (session_counts, folder_counts)
}

fn session_matches(session: &Session, query: &str, folder_path: &str) -> bool {
    let endpoint = match &session.connection {
        Connection::Ssh(ssh) => format!("{} {} {}", ssh.username, ssh.host, ssh.port),
        Connection::Local(local) => {
            format!("local {}", local.shell.as_deref().unwrap_or("sh"))
        }
        _ => String::new(),
    };
    let haystack = format!(
        "{} {} {}",
        session.name.to_ascii_lowercase(),
        folder_path.to_ascii_lowercase(),
        endpoint.to_ascii_lowercase()
    );
    haystack.contains(query)
}

impl AppShell {
    /// 折叠或展开文件夹，并持久化收纳状态。
    pub(crate) fn toggle_folder_collapsed(&mut self, id: FolderId, cx: &mut Context<Self>) {
        let collapsed = self.catalog.folder(id).map(|folder| !folder.collapsed);
        if let Some(collapsed) = collapsed {
            let _ = self.catalog.set_folder_collapsed(id, collapsed);
            self.persist_catalog();
        }
        cx.notify();
    }

    /// 处理会话/文件夹拖放。
    ///
    /// 目标文件夹被折叠时自动展开，保证拖入的条目立即可见；
    /// 非法移动（拖入自身子树）由领域层拒绝并显示中文原因。
    pub(crate) fn handle_sidebar_drop(
        &mut self,
        drag: SidebarDrag,
        target: Option<FolderId>,
        cx: &mut Context<Self>,
    ) {
        let result = match drag {
            SidebarDrag::Session(session_id) => self.catalog.move_session(session_id, target),
            SidebarDrag::Folder(folder_id) => {
                if target == Some(folder_id) {
                    return;
                }
                self.catalog.move_folder(folder_id, target)
            }
        };
        match result {
            Ok(()) => {
                if let Some(target_id) = target
                    && let Some(folder) =
                        self.catalog.folders.iter_mut().find(|f| f.id == target_id)
                {
                    folder.collapsed = false;
                }
                self.persist_catalog();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    pub(crate) fn render_folder_row(
        &mut self,
        row: &FolderRow,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let folder_id = row.id;
        let app_entity = cx.entity().clone();
        let label = row.name.clone();
        let chevron = if row.collapsed {
            IconName::ChevronRight
        } else {
            IconName::ChevronDown
        };

        let mut element = h_flex()
            .id(ElementId::Name(format!("folder-{folder_id}").into()))
            .w_full()
            .h(px(30.0))
            .pl(px(8.0 + row.depth as f32 * 14.0))
            .pr_2()
            .gap_1()
            .items_center()
            .rounded_md()
            .cursor_pointer()
            .text_color(p.muted_foreground)
            .hover(|style| style.bg(p.sidebar_accent.opacity(0.7)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_folder_collapsed(folder_id, cx);
            }))
            .on_drag(SidebarDrag::Folder(folder_id), move |_, _, _, cx| {
                cx.new(|_| SidebarDragPreview::new(label.clone(), IconName::Folder))
            })
            .on_drop({
                let app = app_entity.clone();
                move |drag: &SidebarDrag, window, cx| {
                    let drag = *drag;
                    app.update(cx, |this, cx| {
                        this.handle_sidebar_drop(drag, Some(folder_id), cx);
                        window.refresh();
                    });
                }
            })
            .context_menu({
                let app_entity = app_entity.clone();
                move |menu, _window, _cx| {
                    let app = app_entity.clone();
                    menu.item(
                        PopupMenuItem::new(rust_i18n::t!("menu.new_session_here").to_string())
                            .icon(IconName::Plus)
                            .on_click({
                                let app = app.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.open_new_session_in_folder(folder_id, window, cx);
                                    });
                                }
                            }),
                    )
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("menu.new_folder").to_string())
                            .icon(IconName::FolderPlus)
                            .on_click({
                                let app = app.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.open_new_folder(Some(folder_id), window, cx);
                                    });
                                }
                            }),
                    )
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("menu.rename").to_string())
                            .icon(IconName::FolderPen)
                            .on_click({
                                let app = app.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.open_rename_folder(folder_id, window, cx);
                                    });
                                }
                            }),
                    )
                    .item(PopupMenuItem::separator())
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("menu.delete_folder").to_string())
                            .icon(IconName::Trash)
                            .on_click({
                                let app = app.clone();
                                move |_, _, cx| {
                                    app.update(cx, |this, cx| {
                                        this.delete_folder(folder_id, cx);
                                    });
                                }
                            }),
                    )
                }
            })
            .child(Icon::new(chevron).xsmall())
            .child(
                Icon::new(if row.collapsed {
                    IconName::Folder
                } else {
                    IconName::FolderOpen
                })
                .small(),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .truncate()
                    .child(row.name.clone()),
            )
            .child(div().text_xs().child(row.session_count.to_string()));

        if row.folder_count > 0 {
            element = element.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground.opacity(0.7))
                    .child(format!("({})", row.folder_count)),
            );
        }
        element.into_any_element()
    }

    pub(crate) fn render_session_row(
        &mut self,
        row: &SessionRow,
        active_session: Option<SessionId>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session = &row.session;
        let session_id = session.id;
        let folder_id = session.folder_id;
        let app_entity = cx.entity().clone();
        let (is_local, detail_text) = match &session.connection {
            Connection::Ssh(ssh) => (false, format!("{}@{}", ssh.username, ssh.host)),
            Connection::Local(local) => {
                let shell = local.shell.as_deref().unwrap_or("sh");
                (true, format!("Local: {shell}"))
            }
            _ => (false, String::new()),
        };
        let is_active = active_session == Some(session_id);
        let icon_name = if is_local {
            IconName::Terminal
        } else {
            IconName::Server
        };
        let drag_label = session.name.clone();
        let drag_icon = if is_local {
            IconName::Terminal
        } else {
            IconName::Server
        };
        let mut row_element = div()
            .id(ElementId::Name(format!("session-{session_id}").into()))
            .w_full()
            .flex()
            .flex_row()
            .gap_2()
            .items_center()
            .pl(px(8.0 + row.depth as f32 * 14.0))
            .pr_2()
            .py_2()
            .rounded_md()
            .cursor_pointer();
        if is_active {
            row_element = row_element
                .bg(p.sidebar_accent)
                .text_color(p.sidebar_accent_foreground);
        } else {
            row_element = row_element.hover(|style| {
                style
                    .bg(p.sidebar_accent.opacity(0.7))
                    .text_color(p.sidebar_accent_foreground)
            });
        }
        row_element
            .on_click(
                cx.listener(move |this, _, window, cx| {
                    this.connect_session(session_id, window, cx)
                }),
            )
            .on_drag(SidebarDrag::Session(session_id), move |_, _, _, cx| {
                cx.new(|_| SidebarDragPreview::new(drag_label.clone(), drag_icon))
            })
            .on_drop({
                let app = app_entity.clone();
                move |drag: &SidebarDrag, window, cx| {
                    let drag = *drag;
                    app.update(cx, |this, cx| {
                        this.handle_sidebar_drop(drag, folder_id, cx);
                        window.refresh();
                    });
                }
            })
            .context_menu({
                let app_entity = app_entity.clone();
                move |menu, _window, _cx| {
                    let app = app_entity.clone();
                    let mut menu = menu.item(
                        PopupMenuItem::new(rust_i18n::t!("menu.connect").to_string())
                            .icon(IconName::Terminal)
                            .on_click({
                                let app = app.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.connect_session(session_id, window, cx);
                                    });
                                }
                            }),
                    );
                    if folder_id.is_some() {
                        menu = menu.item(
                            PopupMenuItem::new(rust_i18n::t!("menu.move_to_root").to_string())
                                .icon(IconName::FolderTree)
                                .on_click({
                                    let app = app.clone();
                                    move |_, _, cx| {
                                        app.update(cx, |this, cx| {
                                            this.handle_sidebar_drop(
                                                SidebarDrag::Session(session_id),
                                                None,
                                                cx,
                                            );
                                        });
                                    }
                                }),
                        );
                    }
                    menu.item(
                        PopupMenuItem::new(rust_i18n::t!("menu.duplicate").to_string())
                            .icon(IconName::Plus)
                            .on_click({
                                let app = app.clone();
                                move |_, _, cx| {
                                    app.update(cx, |this, cx| {
                                        this.duplicate_session(session_id, cx);
                                    });
                                }
                            }),
                    )
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("menu.edit").to_string())
                            .icon(IconName::Settings)
                            .on_click({
                                let app = app.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.open_edit_session(session_id, window, cx);
                                    });
                                }
                            }),
                    )
                    .item(PopupMenuItem::separator())
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("menu.delete").to_string())
                            .icon(IconName::Trash)
                            .on_click({
                                let app = app.clone();
                                move |_, _, cx| {
                                    app.update(cx, |this, cx| {
                                        this.delete_session_by_id(session_id, cx);
                                    });
                                }
                            }),
                    )
                }
            })
            .child(Icon::new(icon_name).text_color(if is_active {
                p.sidebar_accent_foreground
            } else {
                p.muted_foreground
            }))
            .child(
                v_flex()
                    .flex_1()
                    .overflow_hidden()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(session.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if is_active {
                                p.sidebar_accent_foreground
                            } else {
                                p.muted_foreground
                            })
                            .child(detail_text),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{SidebarRow, build_rows};
    use transom_core::{Connection, FolderId, LocalConnection, Session, SessionCatalog};

    fn local_session(name: &str, folder: Option<FolderId>) -> Session {
        let mut session = Session::new(name, Connection::Local(LocalConnection::default()));
        session.folder_id = folder;
        session
    }

    fn row_names(catalog: &SessionCatalog, query: &str) -> Vec<String> {
        build_rows(catalog, query)
            .into_iter()
            .map(|row| match row {
                SidebarRow::Folder(row) => format!("folder:{}", row.name),
                SidebarRow::Session(row) => format!("session:{}", row.session.name),
            })
            .collect()
    }

    #[test]
    fn nested_folders_render_depth_first_with_counts() {
        let mut catalog = SessionCatalog::default();
        let prod = catalog.ensure_folder_path("生产").unwrap();
        let db = catalog.ensure_folder_path("生产/数据库").unwrap();
        catalog.sessions.push(local_session("db-01", Some(db)));
        catalog.sessions.push(local_session("root", None));

        let rows = build_rows(&catalog, "");
        assert_eq!(
            row_names(&catalog, ""),
            vec![
                "folder:生产",
                "folder:数据库",
                "session:db-01",
                "session:root"
            ]
        );
        match &rows[0] {
            SidebarRow::Folder(row) => {
                assert_eq!(row.session_count, 1);
                assert_eq!(row.folder_count, 1);
            }
            _ => panic!("首行应为文件夹"),
        }
        match &rows[1] {
            SidebarRow::Folder(row) => {
                assert_eq!(row.depth, 1);
                assert_eq!(row.session_count, 1);
            }
            _ => panic!("第二行应为子文件夹"),
        }
        assert_eq!(prod, catalog.folders[0].id);
    }

    #[test]
    fn collapsed_folder_hides_descendants() {
        let mut catalog = SessionCatalog::default();
        let prod = catalog.ensure_folder_path("生产").unwrap();
        let db = catalog.ensure_folder_path("生产/数据库").unwrap();
        catalog.sessions.push(local_session("db-01", Some(db)));
        catalog.set_folder_collapsed(prod, true).unwrap();

        assert_eq!(row_names(&catalog, ""), vec!["folder:生产"]);
    }

    #[test]
    fn filter_keeps_ancestors_and_ignores_collapsed_state() {
        let mut catalog = SessionCatalog::default();
        let prod = catalog.ensure_folder_path("生产").unwrap();
        let db = catalog.ensure_folder_path("生产/数据库").unwrap();
        catalog.sessions.push(local_session("db-01", Some(db)));
        catalog.sessions.push(local_session("web-01", Some(prod)));
        catalog.set_folder_collapsed(prod, true).unwrap();

        assert_eq!(
            row_names(&catalog, "db-01"),
            vec!["folder:生产", "folder:数据库", "session:db-01"]
        );
    }

    #[test]
    fn folder_name_match_shows_entire_subtree() {
        let mut catalog = SessionCatalog::default();
        let prod = catalog.ensure_folder_path("生产").unwrap();
        let db = catalog.ensure_folder_path("生产/数据库").unwrap();
        catalog.sessions.push(local_session("db-01", Some(db)));
        catalog.sessions.push(local_session("web-01", Some(prod)));

        assert_eq!(
            row_names(&catalog, "生产"),
            vec![
                "folder:生产",
                "folder:数据库",
                "session:db-01",
                "session:web-01"
            ]
        );
    }
}
