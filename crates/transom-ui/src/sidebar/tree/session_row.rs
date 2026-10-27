//! 侧边栏会话树行渲染。

use super::reorder::{SidebarDragPreview, reorder_overlay, row_action_button};
use super::{
    BAND_HEIGHT, ROW_HEIGHT, ROW_ICON_SIZE, SessionRow, SidebarDrag, SidebarTarget, TINT_IDLE,
    TINT_SELECTED, indent_padding,
};
use crate::app::AppShell;
use crate::palette::Palette;
use crate::session_appearance::{session_color, session_icon};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{Connection, HostAction, SessionId};

impl AppShell {
    /// 渲染单个会话树行，支持连接状态着色、双击连接、拖放排序与右键菜单。
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
            Connection::Telnet(telnet) => (
                IconName::Network,
                format!("Telnet: {}:{}", telnet.host, telnet.port),
            ),
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
        // 会话专属图标覆盖连接类型图标；环境色直接染在图标上：侧栏行里没有
        // 放竖杠的位置，图标着色既能标出环境，又不挤占名称与详情的宽度。
        let session_color = session_color(session.color);
        let icon_name = session_icon(session.icon).unwrap_or(icon_name);
        let is_active = active_session == Some(session_id);
        let selection_mode = self.selection_mode;
        let is_selected = self.selected_sessions.contains(&session_id);
        let is_marked = self.tree_selection == Some(session_id);
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
            // 单击只负责选中这一行，双击才建立连接：会话行是列表里最容易被顺手点到的
            // 元素，把连接挂在单击上会让"看一眼"和"连上去"没法区分。
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if this.selection_mode {
                    let selected = this.selected_sessions.contains(&session_id);
                    this.toggle_session_selection(session_id, !selected, cx);
                } else if event.click_count() >= 2 {
                    this.connect_session(session_id, window, cx);
                } else {
                    this.tree_selection = Some(session_id);
                    cx.notify();
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
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("audit_viewer.menu_item").to_string())
                            .icon(IconName::FileText)
                            .on_click({
                                let app = app.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.open_audit_viewer_for_session(session_id, window, cx);
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
        // 单击选中的行与多选勾选的行共用"选中"这一档深浅，避免再引入第五种底色。
        let is_selected_row = is_marked || (selection_mode && is_selected);
        let tint = if is_active {
            1.0
        } else if is_selected_row {
            TINT_SELECTED
        } else {
            TINT_IDLE
        };
        // 悬浮样式必须与 `.id()` 落在同一个元素上：GPUI 只给带 id 的元素分配
        // global_id 与悬浮状态，缺少 id 时鼠标移入不会触发重绘，高亮只能等
        // 别的事件顺带重绘才补上，看上去就是"悬停不跟手"。
        let mut inner = h_flex()
            .id(ElementId::Name(format!("session-band-{session_id}").into()))
            .relative()
            .w_full()
            .h(BAND_HEIGHT)
            .px_2()
            .gap_2()
            .items_center()
            .rounded(p.radius.md)
            .bg(p.sidebar_accent.opacity(tint))
            .text_color(if is_active {
                p.sidebar_accent_foreground
            } else {
                p.sidebar_foreground
            });
        if is_active {
            // 活动会话与悬浮态同为实心强调色，靠左侧竖条区分"当前正在使用的会话"。
            // 竖条绝对定位、不占布局宽度，切换活动会话时行内图标不会发生位移。
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
        } else {
            // 悬浮铺满实心强调色：低透明度叠加在深色侧边栏上几乎看不出差别，
            // 换成实色才有 SFTP 文件行那种"整条弹出"的清晰反馈。
            inner = inner.hover(|style| style.bg(p.sidebar_accent));
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
            .child(Icon::new(icon_name).with_size(ROW_ICON_SIZE).text_color(
                session_color.unwrap_or(if is_active {
                    p.sidebar_accent_foreground
                } else {
                    p.muted_foreground
                }),
            ))
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
        // 行尾状态点只在有在途请求或结果时存在，到期由 probe 模块自行清理。
        let probe_badge = self
            .probe_badges
            .get(&session_id)
            .map(|badge| AppShell::render_probe_badge(session_id, badge, p));
        // 本地 Shell 与串口没有可拨测的网络地址，按钮也不画：点了只会弹一句
        // "没有可拨测主机"，不如根本不给出这个动作。
        let probe_button = HostAction::probe_for(session).map(|_| {
            row_action_button(
                format!("session-probe-{session_id}"),
                IconName::Radar,
                rust_i18n::t!("probe.session_tooltip").to_string(),
                if is_active {
                    p.sidebar_accent_foreground
                } else {
                    p.muted_foreground
                },
                p,
                cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    // 拨测这一台不该同时选中这一行，更不该触发双击连接。
                    cx.stop_propagation();
                    this.probe_session(session_id, cx);
                }),
            )
        });
        let anchor = SidebarDrag::Session(session_id);
        // 会话行没有子级，整行二分为“插到之前/之后”，不存在落空的中间区。
        let inner = inner
            .children(probe_badge)
            .children(probe_button)
            .child(reorder_overlay(&app_entity, anchor, false, p));
        element.child(inner).into_any_element()
    }
}
