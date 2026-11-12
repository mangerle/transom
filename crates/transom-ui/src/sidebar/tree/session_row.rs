//! 侧边栏会话树行渲染。

use super::reorder::{
    RowActionButtonProps, SidebarDragPreview, reorder_overlay, row_action_button,
};
use super::{
    BAND_HEIGHT_REM, ROW_HEIGHT_REM, ROW_ICON_SIZE, SessionRow, SidebarDrag, SidebarTarget,
    indent_padding,
};
use crate::app::AppShell;
use crate::connection::session_host;
use crate::palette::Palette;
use crate::session_appearance::{session_color, session_icon};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::*;
use transom_core::{HostAction, SessionId};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

/// 会话行主体（图标、名称、详情、选中框与活动条）的渲染参数。
struct SessionBandParams {
    session_id: SessionId,
    title: String,
    detail: String,
    icon: AppIcon,
    icon_color: Option<Hsla>,
    is_active: bool,
    selection_mode: bool,
    is_selected: bool,
    is_selected_row: bool,
    fill: Hsla,
}

/// 渲染会话行主体；行尾动作（拨测、拖拽落点）由调用方追加。
///
/// 拆出来是为了让 `render_session_row` 只负责行级交互与菜单装配，
/// 视觉结构（选中底、活动条、图标着色、两行文本）集中在一处，
/// 便于与文件夹行对齐行高与内边距。
fn session_band(
    params: SessionBandParams,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let SessionBandParams {
        session_id,
        title,
        detail,
        icon,
        icon_color,
        is_active,
        selection_mode,
        is_selected,
        is_selected_row,
        fill,
    } = params;
    let mut inner = h_flex()
        .id(ElementId::Name(format!("session-band-{session_id}").into()))
        .relative()
        .w_full()
        .h(rems(BAND_HEIGHT_REM))
        .px_2()
        .gap_2()
        .items_center()
        .rounded(p.radius.md)
        .bg(fill)
        .text_color(if is_active {
            p.sidebar_accent_foreground
        } else {
            p.sidebar_foreground
        });
    if is_active {
        // 当前会话：列表激活底色 + 左侧竖条。竖条绝对定位、不占布局宽度，
        // 切换活动会话时行内图标不会发生位移。
        inner = inner.child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(2.0))
                .rounded_full()
                .bg(p.list_active_border),
        );
    } else if !is_selected_row {
        // 悬停只给未选中的行，避免悬浮色把选中态的底色顶掉。
        inner = inner.hover(|style| style.bg(p.list_hover));
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
    inner
        .child(
            AppIconElement::new(icon)
                .with_rems(ROW_ICON_SIZE)
                .text_color(icon_color.unwrap_or(if is_active {
                    p.sidebar_accent_foreground
                } else {
                    p.muted_foreground
                })),
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
                        .child(title.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .line_height(px(14.0))
                        .truncate()
                        .text_color(p.muted_foreground)
                        .child(detail),
                ),
        )
}

/// 会话行右键菜单：连接、移动、复制、编辑、审计、复制主机与删除。
///
/// 菜单在渲染期构建、点击后才回交外壳；`app` 是外壳实体句柄，菜单闭包
/// 不持有会话行的借用，避免菜单还开着时会话被删除导致的悬垂引用。
fn session_context_menu(
    session_id: SessionId,
    folder_id: Option<transom_core::FolderId>,
    menu_host: Option<String>,
    is_ssh: bool,
    app_entity: Entity<AppShell>,
) -> impl Fn(
    gpui_kit::component::menu::PopupMenu,
    &mut Window,
    &mut Context<gpui_kit::component::menu::PopupMenu>,
) -> gpui_kit::component::menu::PopupMenu
+ 'static {
    move |menu, _window, _cx| {
        let app = app_entity.clone();
        let mut menu = menu.item(crate::widgets::menu_item(
            rust_i18n::t!("menu.connect").to_string(),
            AppIcon::Domain(DomainIcon::TERMINAL),
            {
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.connect_session(session_id, window, cx);
                        this.reveal_vertical_tabs_panel(cx);
                    });
                }
            },
        ));
        if is_ssh {
            let container_app = app.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("menu.manage_containers").to_string(),
                AppIcon::Domain(DomainIcon::CONTAINER),
                move |_, window, cx| {
                    container_app.update(cx, |this, cx| {
                        this.open_container_workspace_for_session(session_id, window, cx);
                    });
                },
            ));
        }
        if folder_id.is_some() {
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("menu.move_to_root").to_string(),
                AppIcon::Ui(UiIcon::SHOW_AS_TREE),
                {
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
                },
            ));
        }
        menu = menu
            .item(crate::widgets::menu_item(
                rust_i18n::t!("menu.duplicate").to_string(),
                AppIcon::Ui(UiIcon::ADD),
                {
                    let app = app.clone();
                    move |_, _, cx| {
                        app.update(cx, |this, cx| {
                            this.duplicate_session(session_id, cx);
                        });
                    }
                },
            ))
            .item(crate::widgets::menu_item(
                rust_i18n::t!("menu.edit").to_string(),
                AppIcon::Ui(UiIcon::SETTINGS),
                {
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.open_edit_session(session_id, window, cx);
                        });
                    }
                },
            ))
            .item(crate::widgets::menu_item(
                rust_i18n::t!("audit_viewer.menu_item").to_string(),
                AppIcon::Ui(UiIcon::DIFF),
                {
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.open_audit_viewer_for_session(session_id, window, cx);
                        });
                    }
                },
            ));

        // 与标签栏同一口径：取不到地址（本地终端、串口）就不放这一项。
        if let Some(host) = menu_host.clone() {
            let copy_app = app.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("menu.copy_host").to_string(),
                AppIcon::Ui(UiIcon::COPY),
                move |_, _, cx| {
                    let text = host.clone();
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    copy_app.update(cx, |this, cx| {
                        this.state.status =
                            Some(rust_i18n::t!("sidebar.host_copied", host = text).to_string());
                        cx.notify();
                    });
                },
            ));
        }

        menu.item(PopupMenuItem::separator())
            .item(crate::widgets::menu_item(
                rust_i18n::t!("menu.delete").to_string(),
                AppIcon::Ui(UiIcon::DELETE),
                {
                    let app = app.clone();
                    move |_, _, cx| {
                        app.update(cx, |this, cx| {
                            this.delete_session_by_id(session_id, cx);
                        });
                    }
                },
            ))
    }
}

/// 会话行的展示快照：由目录会话与侧栏选中态推导。
struct SessionRowVisuals {
    is_ssh: bool,
    menu_host: Option<String>,
    icon_name: AppIcon,
    detail_text: String,
    icon_color: Option<Hsla>,
    is_active: bool,
    selection_mode: bool,
    is_selected: bool,
    drag_label: String,
    drag_icon: AppIcon,
    is_selected_row: bool,
    fill: Hsla,
}

/// 会话行尾的拨测展示部件。
struct SessionProbeParts {
    badge: Option<AnyElement>,
    button: Option<AnyElement>,
}

impl AppShell {
    /// 渲染单个会话树行，支持连接状态着色、双击连接、拖放排序与右键菜单。
    /// 行尾拨测状态点与按钮；无网络地址的会话两者都不出现。
    fn session_probe_parts(
        &self,
        session: &transom_core::Session,
        is_active: bool,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> SessionProbeParts {
        let session_id = session.id;
        // 行尾状态点只在有在途请求或结果时存在，到期由 probe 模块自行清理。
        let probe_badge = self
            .state
            .probe_badges
            .get(&session_id)
            .map(|badge| AppShell::render_probe_badge(session_id, badge, p));
        // 本地 Shell 与串口没有可拨测的网络地址，按钮也不画：点了只会弹一句
        // "没有可拨测主机"，不如根本不给出这个动作。
        let probe_button = HostAction::probe_for(session).map(|_| {
            row_action_button(
                RowActionButtonProps {
                    element_id: format!("session-probe-{session_id}"),
                    icon: AppIcon::Ui(UiIcon::EXCEPTION_ANALYZER),
                    tooltip: rust_i18n::t!("probe.session_tooltip").to_string(),
                    idle_color: if is_active {
                        p.sidebar_accent_foreground
                    } else {
                        p.muted_foreground
                    },
                    on_click: cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        // 拨测这一台不该同时选中这一行，更不该触发双击连接。
                        cx.stop_propagation();
                        this.probe_session(session_id, cx);
                    }),
                },
                p,
            )
        });
        SessionProbeParts {
            badge: probe_badge,
            button: probe_button.map(IntoElement::into_any_element),
        }
    }

    /// 会话行的展示快照：图标/颜色、详情、选中态与拖拽标签。
    fn session_row_visuals(
        &self,
        row: &SessionRow,
        active_session: Option<SessionId>,
        p: &Palette,
    ) -> SessionRowVisuals {
        let session = &row.session;
        let session_id = session.id;
        // 右键菜单在渲染期就把地址算好带走：菜单闭包运行时会话行早已渲染完，
        // 届时再回查 row 的借用不成立。
        let menu_host = session_host(session).map(str::to_string);
        let icon_name = crate::connection::icon(&session.connection);
        let detail_text = crate::connection::display_compact(&session.connection);
        // 会话专属图标覆盖连接类型图标；环境色直接染在图标上。
        let icon_color = session_color(session.color);
        let icon_name = session_icon(session.icon).unwrap_or(icon_name);
        let is_active = active_session == Some(session_id);
        let selection_mode = self.state.sidebar.selection_mode;
        let is_selected = self.state.sidebar.selected_sessions.contains(&session_id);
        let is_marked = self.state.sidebar.tree_selection == Some(session_id);
        // 行底取主题的列表令牌：常态是列表背景，选中与当前用列表激活色。
        let is_selected_row = is_marked || (selection_mode && is_selected);
        let fill = if is_active || is_selected_row {
            p.list_active
        } else {
            p.list_background
        };
        let is_ssh = matches!(session.connection, transom_core::Connection::Ssh(_));
        SessionRowVisuals {
            is_ssh,
            menu_host,
            icon_name,
            detail_text,
            icon_color,
            is_active,
            selection_mode,
            is_selected,
            drag_label: session.name.clone(),
            drag_icon: icon_name,
            is_selected_row,
            fill,
        }
    }

    /// 会话行单击：选择模式下切换多选，双击连接，普通单击只标记。
    fn session_row_click(
        &mut self,
        session_id: SessionId,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.sidebar.selection_mode {
            let selected = self.state.sidebar.selected_sessions.contains(&session_id);
            self.toggle_session_selection(session_id, !selected, cx);
        } else if event.click_count() >= 2 {
            self.connect_session(session_id, window, cx);
            // 连接入口在会话树，连接完成后用户要看的是终端列表：
            // 左侧标签模式自动把槽位切到标签面板。
            self.reveal_vertical_tabs_panel(cx);
        } else {
            self.state.sidebar.tree_selection = Some(session_id);
            cx.notify();
        }
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
        let SessionRowVisuals {
            is_ssh,
            menu_host,
            icon_name,
            detail_text,
            icon_color: session_color,
            is_active,
            selection_mode,
            is_selected,
            drag_label,
            drag_icon,
            is_selected_row,
            fill,
        } = self.session_row_visuals(row, active_session, p);
        let element = div()
            .id(ElementId::Name(format!("session-{session_id}").into()))
            .w_full()
            .h(rems(ROW_HEIGHT_REM))
            .pl(indent_padding(row.depth))
            .pr(px(4.0))
            .flex()
            .items_center()
            .cursor_pointer()
            // 单击只负责选中这一行，双击才建立连接：会话行是列表里最容易被顺手点到的
            // 元素，把连接挂在单击上会让"看一眼"和"连上去"没法区分。
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.session_row_click(session_id, event, window, cx);
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
            .context_menu(session_context_menu(
                session_id,
                folder_id,
                menu_host,
                is_ssh,
                app_entity.clone(),
            ));
        // 悬浮样式必须与 `.id()` 落在同一个元素上：GPUI 只给带 id 的元素分配
        // global_id 与悬浮状态，缺少 id 时鼠标移入不会触发重绘，高亮只能等
        // 别的事件顺带重绘才补上，看上去就是"悬停不跟手"。
        let inner = session_band(
            SessionBandParams {
                session_id,
                title: session.name.clone(),
                detail: detail_text,
                icon: icon_name,
                icon_color: session_color,
                is_active,
                selection_mode,
                is_selected,
                is_selected_row,
                fill,
            },
            p,
            cx,
        );
        let probe = self.session_probe_parts(session, is_active, p, cx);
        let anchor = SidebarDrag::Session(session_id);
        // 会话行没有子级，整行二分为“插到之前/之后”，不存在落空的中间区。
        let inner = inner
            .children(probe.badge)
            .children(probe.button)
            .child(reorder_overlay(&app_entity, anchor, false, p));
        element.child(inner).into_any_element()
    }
}
