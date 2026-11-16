//! 侧边栏会话树行渲染。

use super::super::entity::{SessionSidebarEvent, SessionSidebarView};
use super::reorder::{
    RowActionButtonProps, SidebarDragPreview, reorder_overlay, row_action_button,
};
use super::{
    ROW_HEIGHT_REM, ROW_ICON_SIZE, SessionRow, SidebarDrag, SidebarTarget, indent_padding,
};
use crate::connection::session_host;
use crate::palette::Palette;
use crate::probe::ProbeBadge;
use crate::session_appearance::{session_color, session_icon};
use crate::widgets::{SidebarRowState, sidebar_row_band};
use gpui_kit::base::v_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::{FolderId, HostAction, Session, SessionId};
use transom_ui_icons::{AppIcon, AppIconElement, DomainIcon, UiIcon};

/// 渲染单个会话行所需的只读上下文参数。
pub(crate) struct SessionRowRenderParams<'a> {
    pub(crate) view: &'a Entity<SessionSidebarView>,
    pub(crate) row: &'a SessionRow,
    pub(crate) active_session: Option<SessionId>,
    pub(crate) selection_mode: bool,
    pub(crate) is_selected: bool,
    pub(crate) is_marked: bool,
    pub(crate) probe_badge: Option<&'a ProbeBadge>,
}

/// 会话行主体（图标、名称、详情、选中框与活动条）的渲染参数。
struct SessionBandParams {
    session_id: SessionId,
    title: String,
    detail: String,
    icon: AppIcon,
    icon_color: Option<Hsla>,
    selection_mode: bool,
    is_selected: bool,
    state: SidebarRowState,
}

/// 渲染会话行主体；行尾动作（拨测、拖拽落点）由调用方追加。
fn session_band(
    params: SessionBandParams,
    view: &Entity<SessionSidebarView>,
    p: &Palette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let SessionBandParams {
        session_id,
        title,
        detail,
        icon,
        icon_color,
        selection_mode,
        is_selected,
        state,
    } = params;
    let mut inner = sidebar_row_band(
        ElementId::Name(format!("session-band-{session_id}").into()),
        state,
        p,
    );
    if selection_mode {
        let check_view = view.clone();
        inner = inner.child(
            Checkbox::new(ElementId::Name(
                format!("session-select-{session_id}").into(),
            ))
            .small()
            .checked(is_selected)
            .on_click(move |&checked, _, cx| {
                cx.stop_propagation();
                check_view.update(cx, |this, cx| {
                    this.emit_action(
                        SessionSidebarEvent::ToggleSessionSelection {
                            session_id,
                            selected: checked,
                        },
                        cx,
                    );
                });
            }),
        );
    }
    inner
        .child(
            AppIconElement::new(icon)
                .with_rems(ROW_ICON_SIZE)
                .text_color(icon_color.unwrap_or(if state.is_highlighted() {
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
                        .child(title),
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
fn session_context_menu(
    session_id: SessionId,
    folder_id: Option<FolderId>,
    menu_host: Option<String>,
    is_ssh: bool,
    view: Entity<SessionSidebarView>,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
    move |menu, _window, _cx| {
        let emit_item =
            |menu: PopupMenu, label: String, icon: AppIcon, event: SessionSidebarEvent| {
                let view = view.clone();
                menu.item(crate::widgets::menu_item(label, icon, move |_, _, cx| {
                    view.update(cx, |this, cx| this.emit_action(event.clone(), cx));
                }))
            };

        let mut menu = emit_item(
            menu,
            rust_i18n::t!("menu.connect").to_string(),
            AppIcon::Domain(DomainIcon::TERMINAL),
            SessionSidebarEvent::ConnectSession(session_id),
        );
        if is_ssh {
            menu = emit_item(
                menu,
                rust_i18n::t!("menu.manage_containers").to_string(),
                AppIcon::Domain(DomainIcon::CONTAINER),
                SessionSidebarEvent::OpenContainerWorkspace(session_id),
            );
        }
        if folder_id.is_some() {
            menu = emit_item(
                menu,
                rust_i18n::t!("menu.move_to_root").to_string(),
                AppIcon::Ui(UiIcon::SHOW_AS_TREE),
                SessionSidebarEvent::Drop(SidebarDrag::Session(session_id), SidebarTarget::Root),
            );
        }
        menu = emit_item(
            menu,
            rust_i18n::t!("menu.duplicate").to_string(),
            AppIcon::Ui(UiIcon::ADD),
            SessionSidebarEvent::DuplicateSession(session_id),
        );
        menu = emit_item(
            menu,
            rust_i18n::t!("menu.edit").to_string(),
            AppIcon::Ui(UiIcon::SETTINGS),
            SessionSidebarEvent::EditSession(session_id),
        );
        menu = emit_item(
            menu,
            rust_i18n::t!("audit_viewer.menu_item").to_string(),
            AppIcon::Ui(UiIcon::DIFF),
            SessionSidebarEvent::OpenAuditViewer(session_id),
        );

        // 与标签栏同一口径：取不到地址（本地终端、串口）就不放这一项。
        if let Some(host) = menu_host.clone() {
            let copy_view = view.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("menu.copy_host").to_string(),
                AppIcon::Ui(UiIcon::COPY),
                move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(host.clone()));
                    copy_view.update(cx, |this, cx| {
                        this.emit_action(SessionSidebarEvent::CopyHost(host.clone()), cx);
                    });
                },
            ));
        }

        let menu = menu.item(PopupMenuItem::separator());
        emit_item(
            menu,
            rust_i18n::t!("menu.delete").to_string(),
            AppIcon::Ui(UiIcon::DELETE),
            SessionSidebarEvent::DeleteSession(session_id),
        )
    }
}

/// 会话行的展示快照：由目录会话与侧栏选中态推导。
struct SessionRowVisuals {
    is_ssh: bool,
    menu_host: Option<String>,
    icon_name: AppIcon,
    detail_text: String,
    icon_color: Option<Hsla>,
    drag_label: String,
    drag_icon: AppIcon,
    state: SidebarRowState,
}

/// 会话行尾的拨测展示部件。
struct SessionProbeParts {
    badge: Option<AnyElement>,
    button: Option<AnyElement>,
}

/// 行尾拨测状态点与按钮；无网络地址的会话两者都不出现。
fn session_probe_parts(
    view: &Entity<SessionSidebarView>,
    session: &Session,
    probe_badge: Option<&ProbeBadge>,
    highlighted: bool,
    p: &Palette,
) -> SessionProbeParts {
    let session_id = session.id;
    let badge = probe_badge.map(|badge| crate::probe::render_probe_badge(session_id, badge, p));
    let button = HostAction::probe_for(session).map(|_| {
        let probe_view = view.clone();
        row_action_button(
            RowActionButtonProps {
                element_id: format!("session-probe-{session_id}"),
                icon: AppIcon::Ui(UiIcon::EXCEPTION_ANALYZER),
                tooltip: rust_i18n::t!("probe.session_tooltip").to_string(),
                idle_color: if highlighted {
                    p.sidebar_accent_foreground
                } else {
                    p.muted_foreground
                },
                on_click: move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                    cx.stop_propagation();
                    probe_view.update(cx, |this, cx| {
                        this.emit_action(SessionSidebarEvent::ProbeSession(session_id), cx);
                    });
                },
            },
            p,
        )
    });
    SessionProbeParts {
        badge,
        button: button.map(IntoElement::into_any_element),
    }
}

/// 会话行的展示快照：图标/颜色、详情、选中态与拖拽标签。
fn session_row_visuals(params: &SessionRowRenderParams<'_>) -> SessionRowVisuals {
    let session = &params.row.session;
    let session_id = session.id;
    let menu_host = session_host(session).map(str::to_string);
    let base_icon = crate::connection::icon(&session.connection);
    let detail_text = crate::connection::display_compact(&session.connection);
    let icon_color = session_color(session.color);
    let icon_name = session_icon(session.icon).unwrap_or(base_icon);
    let is_active = params.active_session == Some(session_id);
    let row_selected = params.is_marked || (params.selection_mode && params.is_selected);
    let state = if is_active {
        SidebarRowState::Current
    } else if row_selected {
        SidebarRowState::Selected
    } else {
        SidebarRowState::Normal
    };
    let is_ssh = matches!(session.connection, transom_core::Connection::Ssh(_));
    SessionRowVisuals {
        is_ssh,
        menu_host,
        icon_name,
        detail_text,
        icon_color,
        drag_label: session.name.clone(),
        drag_icon: icon_name,
        state,
    }
}

/// 渲染单个会话树行，支持连接状态着色、双击连接、拖放排序与右键菜单。
pub(crate) fn render_session_row(params: SessionRowRenderParams<'_>, p: &Palette) -> AnyElement {
    let session = &params.row.session;
    let session_id = session.id;
    let folder_id = session.folder_id;
    let selection_mode = params.selection_mode;
    let is_selected = params.is_selected;
    let SessionRowVisuals {
        is_ssh,
        menu_host,
        icon_name,
        detail_text,
        icon_color,
        drag_label,
        drag_icon,
        state,
    } = session_row_visuals(&params);

    let click_view = params.view.clone();
    let drop_view = params.view.clone();
    let menu_view = params.view.clone();

    let element = div()
        .id(ElementId::Name(format!("session-{session_id}").into()))
        .w_full()
        .h(rems(ROW_HEIGHT_REM))
        .pl(indent_padding(params.row.depth))
        .pr(px(4.0))
        .flex()
        .items_center()
        .cursor_pointer()
        .on_click(move |event: &ClickEvent, _, cx| {
            let action = if selection_mode {
                SessionSidebarEvent::ToggleSessionSelection {
                    session_id,
                    selected: !is_selected,
                }
            } else if event.click_count() >= 2 {
                SessionSidebarEvent::ConnectSession(session_id)
            } else {
                SessionSidebarEvent::SelectTreeSession(session_id)
            };
            click_view.update(cx, |this, cx| this.emit_action(action, cx));
        })
        .on_drag(SidebarDrag::Session(session_id), move |_, _, _, cx| {
            cx.new(|_| SidebarDragPreview::new(drag_label.clone(), drag_icon))
        })
        .on_drop(move |drag: &SidebarDrag, window, cx| {
            let drag = *drag;
            let target = match folder_id {
                Some(folder_id) => SidebarTarget::Folder(folder_id),
                None => SidebarTarget::Root,
            };
            drop_view.update(cx, |this, cx| {
                this.emit_action(SessionSidebarEvent::Drop(drag, target), cx);
            });
            window.refresh();
        })
        .context_menu(session_context_menu(
            session_id, folder_id, menu_host, is_ssh, menu_view,
        ));

    let inner = session_band(
        SessionBandParams {
            session_id,
            title: session.name.clone(),
            detail: detail_text,
            icon: icon_name,
            icon_color,
            selection_mode,
            is_selected,
            state,
        },
        params.view,
        p,
    );
    let probe = session_probe_parts(
        params.view,
        session,
        params.probe_badge,
        state.is_highlighted(),
        p,
    );
    let anchor = SidebarDrag::Session(session_id);
    let inner = inner
        .children(probe.badge)
        .children(probe.button)
        .child(reorder_overlay(params.view, anchor, false, p));
    element.child(inner).into_any_element()
}
