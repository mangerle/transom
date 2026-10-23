//! 会话树行模型、拖拽载荷与文件夹/会话行渲染。

mod rows;

pub(crate) use rows::build_rows;

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{Connection, FolderId, Session, SessionId};

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
        let (icon_name, detail_text) = match &session.connection {
            Connection::Ssh(ssh) => (IconName::Server, format!("{}@{}", ssh.username, ssh.host)),
            Connection::Local(local) => {
                let shell = local.shell.as_deref().unwrap_or("sh");
                (IconName::Terminal, format!("Local: {shell}"))
            }
            Connection::Serial(serial) => (
                IconName::Cable,
                format!("Serial: {} @ {}", serial.port, serial.baud_rate),
            ),
            _ => (IconName::Server, String::new()),
        };
        let is_active = active_session == Some(session_id);
        let selection_mode = self.selection_mode;
        let is_selected = self.selected_sessions.contains(&session_id);
        let drag_label = session.name.clone();
        let drag_icon = icon_name;
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
        } else if selection_mode && is_selected {
            row_element = row_element.bg(p.sidebar_accent.opacity(0.45));
        } else {
            row_element = row_element.hover(|style| {
                style
                    .bg(p.sidebar_accent.opacity(0.7))
                    .text_color(p.sidebar_accent_foreground)
            });
        }
        let mut element = row_element
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.selection_mode {
                    let selected = this.selected_sessions.contains(&session_id);
                    this.toggle_session_selection(session_id, !selected, cx);
                } else {
                    this.connect_session(session_id, window, cx);
                }
            }))
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
            });
        if selection_mode {
            element = element.child(
                Checkbox::new(ElementId::Name(
                    format!("session-select-{session_id}").into(),
                ))
                .small()
                .checked(is_selected)
                .on_click(cx.listener(move |this, &checked, _, cx| {
                    cx.stop_propagation();
                    this.toggle_session_selection(session_id, checked, cx);
                })),
            );
        }
        element
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
