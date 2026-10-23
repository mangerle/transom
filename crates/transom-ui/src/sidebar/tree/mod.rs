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

/// 行槽位高度：虚拟列表按它分配空间，渲染时必须严格一致，否则行会错位或裁切。
const ROW_HEIGHT: Pixels = px(38.0);
/// 内层高亮条高度：小于行槽位，留出上下间隙，同时把背景限制在内容区，缩进区保持透明。
const BAND_HEIGHT: Pixels = px(34.0);
/// 缩进基准与每级步长（逻辑像素）。
const INDENT_BASE: f32 = 6.0;
const INDENT_STEP: f32 = 18.0;
/// 行内图标尺寸：与 34px 内容条匹配，默认的 14px 图标在这个高度下显得偏弱。
const ROW_ICON_SIZE: Pixels = px(18.0);
/// 折叠箭头尺寸：比行图标小一档，避免与文件夹图标抢视觉重心。
const ROW_CHEVRON_SIZE: Pixels = px(14.0);
/// 行背景深浅梯度：常态最轻，悬停加重，选中再加重，当前会话用实色最重。
const TINT_IDLE: f32 = 0.14;
const TINT_HOVER: f32 = 0.28;
const TINT_SELECTED: f32 = 0.42;
/// 已选行被悬停时继续加重，避免悬停反而把选中态冲淡。
const TINT_SELECTED_HOVER: f32 = 0.5;
/// 文件夹行整体比会话行更重一档，用于区分层级容器。
const TINT_FOLDER_IDLE: f32 = 0.22;
const TINT_FOLDER_HOVER: f32 = 0.36;

/// 按层级计算行左内边距，文件夹与会话共用，保证同层对齐。
fn indent_padding(depth: usize) -> Pixels {
    px(INDENT_BASE + depth as f32 * INDENT_STEP)
}

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
        // 文件夹与会话保持统一行高；内层内容条再压缩，兼顾两行信息与紧凑度。
        ROW_HEIGHT
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

        let element = div()
            .id(ElementId::Name(format!("folder-{folder_id}").into()))
            .w_full()
            .h(ROW_HEIGHT)
            .pl(indent_padding(row.depth))
            .pr(px(4.0))
            .flex()
            .items_center()
            .cursor_pointer()
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
            });
        let mut band = h_flex()
            .w_full()
            .h(BAND_HEIGHT)
            .px_2()
            .gap_1()
            .items_center()
            .rounded_md()
            .bg(p.sidebar_accent.opacity(TINT_FOLDER_IDLE))
            .border_b_1()
            .border_color(p.sidebar_border.opacity(0.55))
            .text_color(p.sidebar_foreground)
            .hover(|style| style.bg(p.sidebar_accent.opacity(TINT_FOLDER_HOVER)))
            .child(Icon::new(chevron).with_size(ROW_CHEVRON_SIZE))
            .child(
                Icon::new(if row.collapsed {
                    IconName::Folder
                } else {
                    IconName::FolderOpen
                })
                .with_size(ROW_ICON_SIZE),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(row.name.clone()),
            )
            .child(div().text_xs().child(row.session_count.to_string()));

        if row.folder_count > 0 {
            band = band.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground.opacity(0.7))
                    .child(format!("({})", row.folder_count)),
            );
        }
        element.child(band).into_any_element()
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
        let element = div()
            .id(ElementId::Name(format!("session-{session_id}").into()))
            .w_full()
            .h(ROW_HEIGHT)
            .pl(indent_padding(row.depth))
            .pr(px(4.0))
            .flex()
            .items_center()
            .cursor_pointer()
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
        // 高亮与悬停只作用于内层内容条，缩进区保持透明，层级靠左偏移表达。
        let is_selected_row = selection_mode && is_selected;
        let tint = if is_active {
            1.0
        } else if is_selected_row {
            TINT_SELECTED
        } else {
            TINT_IDLE
        };
        let hover_tint = if is_selected_row {
            TINT_SELECTED_HOVER
        } else {
            TINT_HOVER
        };
        let mut inner = h_flex()
            .w_full()
            .h(BAND_HEIGHT)
            .px_2()
            .gap_2()
            .items_center()
            .rounded_md()
            .bg(p.sidebar_accent.opacity(tint))
            .text_color(if is_active {
                p.sidebar_accent_foreground
            } else {
                p.sidebar_foreground
            });
        if !is_active {
            inner = inner.hover(|style| style.bg(p.sidebar_accent.opacity(hover_tint)));
        }
        if selection_mode {
            inner = inner.child(
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
        let inner = inner
            .child(
                Icon::new(icon_name)
                    .with_size(ROW_ICON_SIZE)
                    .text_color(if is_active {
                        p.sidebar_accent_foreground
                    } else {
                        p.muted_foreground
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .overflow_hidden()
                    .child(
                        div()
                            .text_sm()
                            .line_height(px(17.0))
                            .font_weight(FontWeight::MEDIUM)
                            .truncate()
                            .child(session.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .line_height(px(14.0))
                            .truncate()
                            .text_color(if is_active {
                                p.sidebar_accent_foreground
                            } else {
                                p.muted_foreground
                            })
                            .child(detail_text),
                    ),
            );
        element.child(inner).into_any_element()
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
