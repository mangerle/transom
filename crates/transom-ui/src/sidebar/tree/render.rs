//! 会话树行的渲染：文件夹行、会话行、拖拽预览与排序落点。
//!
//! 行模型、落点语义与拖放处理留在父模块 `tree`，本文件只负责把状态拼成 GPUI 元素，
//! 拆开的直接原因是合在一起后生产代码行数超过单文件阈值。

use super::{
    BAND_HEIGHT, FolderRow, REORDER_ZONE, ROW_CHEVRON_SIZE, ROW_HEIGHT, ROW_ICON_SIZE, SessionRow,
    SidebarDrag, SidebarTarget, TINT_FOLDER_HOVER, TINT_FOLDER_IDLE, TINT_HOVER, TINT_IDLE,
    TINT_SELECTED, TINT_SELECTED_HOVER, indent_padding,
};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{Connection, DropSide, SessionId};

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

impl AppShell {
    /// 行上下沿的排序落点：命中即把拖拽条目插到该行之前或之后。
    ///
    /// 用绝对定位窄条覆盖行边缘，而不是把整行切成两块，是为了让行中部继续
    /// 承担“放入本行容器”的语义；插入线直接借 `drag_over` 描边，
    /// 无需额外维护悬停状态，也不会与行自身的选中/激活底色冲突。
    fn reorder_drop_zone(
        app: &Entity<AppShell>,
        anchor: SidebarDrag,
        top: bool,
        p: &Palette,
    ) -> AnyElement {
        let target = SidebarTarget::Relative {
            anchor,
            side: if top {
                DropSide::Before
            } else {
                DropSide::After
            },
        };
        // `drag_over` 的闭包要求 `'static`，因此把颜色取成局部值再捕获。
        // 插入线用 accent_foreground：accent 本身已经是行底色，画上去看不出。
        let line_color = p.sidebar_accent_foreground;
        let zone = div()
            .absolute()
            .left_0()
            .w_full()
            .h(REORDER_ZONE)
            .border_color(line_color);
        let zone = if top {
            zone.top_0()
                .drag_over::<SidebarDrag>(move |style, _, _, _| {
                    style.border_t_2().border_color(line_color)
                })
        } else {
            zone.bottom_0()
                .drag_over::<SidebarDrag>(move |style, _, _, _| {
                    style.border_b_2().border_color(line_color)
                })
        };
        let app = app.clone();
        zone.on_drop(move |drag: &SidebarDrag, window, cx| {
            let drag = *drag;
            app.update(cx, |this, cx| {
                this.handle_sidebar_drop(drag, target, cx);
                window.refresh();
            });
        })
        .into_any_element()
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
                        this.handle_sidebar_drop(drag, SidebarTarget::Folder(folder_id), cx);
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
            .relative()
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
        let anchor = SidebarDrag::Folder(folder_id);
        band = band
            .child(Self::reorder_drop_zone(&app_entity, anchor, true, p))
            .child(Self::reorder_drop_zone(&app_entity, anchor, false, p));
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
                    // 落在会话行中部：放进该会话所在层级末尾（根层级会话则回到根）。
                    let target = match folder_id {
                        Some(folder_id) => SidebarTarget::Folder(folder_id),
                        None => SidebarTarget::Root,
                    };
                    app.update(cx, |this, cx| {
                        this.handle_sidebar_drop(drag, target, cx);
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
                                                SidebarTarget::Root,
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
            .relative()
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
        let anchor = SidebarDrag::Session(session_id);
        let inner = inner
            .child(Self::reorder_drop_zone(&app_entity, anchor, true, p))
            .child(Self::reorder_drop_zone(&app_entity, anchor, false, p));
        element.child(inner).into_any_element()
    }
}
