//! 会话树侧边栏与会话列表筛选模块。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable, v_virtual_list};
use gpui_kit::*;
use transom_core::{Connection, Session, SessionId};

impl AppShell {
    pub(crate) fn filtered_sessions(&self, cx: &Context<Self>) -> Vec<Session> {
        let query = self.search.read(cx).value().trim().to_ascii_lowercase();
        let mut sessions: Vec<Session> = self
            .sessions
            .iter()
            .filter(|session| {
                if query.is_empty() {
                    return true;
                }
                let endpoint = match &session.connection {
                    Connection::Ssh(ssh) => format!("{} {}", ssh.host, ssh.username),
                    _ => String::new(),
                };
                let haystack = format!(
                    "{} {} {}",
                    session.name,
                    session.group.clone().unwrap_or_default(),
                    endpoint
                )
                .to_ascii_lowercase();
                haystack.contains(&query)
            })
            .cloned()
            .collect();
        sessions.sort_by(|a, b| {
            a.group.cmp(&b.group).then_with(|| {
                a.name
                    .to_ascii_lowercase()
                    .cmp(&b.name.to_ascii_lowercase())
            })
        });
        sessions
    }

    pub(crate) fn active_session_id(&self) -> Option<SessionId> {
        self.active_tab.and_then(|tab_id| {
            self.tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .map(|tab| tab.session_id)
        })
    }

    pub(crate) fn render_sidebar(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let sessions = self.filtered_sessions(cx);
        let content = if sessions.is_empty() {
            v_flex()
                .p_4()
                .gap_2()
                .items_center()
                .justify_center()
                .child(Icon::new(IconName::Server).text_color(p.muted_foreground))
                .child(
                    div()
                        .text_sm()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("app.no_sessions").to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("app.no_sessions_hint").to_string()),
                )
                .into_any_element()
        } else {
            #[derive(Clone)]
            enum SidebarItem {
                GroupHeader(String),
                Session(Box<Session>),
            }

            let mut row_items: Vec<SidebarItem> = Vec::new();
            let mut last_group: Option<String> = None;
            for session in sessions {
                if session.group != last_group {
                    if let Some(group) = &session.group {
                        row_items.push(SidebarItem::GroupHeader(group.clone()));
                    }
                    last_group = session.group.clone();
                }
                row_items.push(SidebarItem::Session(Box::new(session)));
            }

            let item_sizes = std::rc::Rc::new(
                row_items
                    .iter()
                    .map(|item| match item {
                        SidebarItem::GroupHeader(_) => size(px(0.), px(28.)),
                        SidebarItem::Session(_) => size(px(0.), px(44.)),
                    })
                    .collect::<Vec<_>>(),
            );
            let row_items_rc = std::rc::Rc::new(row_items);
            let p = *p;

            v_virtual_list(
                cx.entity().clone(),
                "sidebar-sessions-virtual-list",
                item_sizes,
                move |this, range, _window, cx| {
                    let active_session = this.active_session_id();
                    let app_entity = cx.entity().clone();
                    range
                        .filter_map(|ix| {
                            let item = row_items_rc.get(ix)?;
                            match item {
                                SidebarItem::GroupHeader(group) => Some(
                                    div()
                                        .w_full()
                                        .px_3()
                                        .pt_3()
                                        .pb_1()
                                        .text_xs()
                                        .text_color(p.muted_foreground)
                                        .child(group.clone())
                                        .into_any_element(),
                                ),
                                SidebarItem::Session(session) => {
                                    let session_id = session.id;
                                    let (is_local, detail_text) = match &session.connection {
                                        Connection::Ssh(ssh) => {
                                            (false, format!("{}@{}", ssh.username, ssh.host))
                                        }
                                        Connection::Local(local) => {
                                            let shell = local.shell.as_deref().unwrap_or("sh");
                                            (true, format!("Local: {shell}"))
                                        }
                                        _ => (false, String::new()),
                                    };
                                    let is_active = active_session == Some(session_id);
                                    let mut row = div()
                                        .id(ElementId::Name(format!("session-{session_id}").into()))
                                        .w_full()
                                        .flex()
                                        .flex_row()
                                        .gap_2()
                                        .items_center()
                                        .px_3()
                                        .py_2()
                                        .rounded_md()
                                        .cursor_pointer();
                                    if is_active {
                                        row = row
                                            .bg(p.sidebar_accent)
                                            .text_color(p.sidebar_accent_foreground);
                                    } else {
                                        row = row.hover(|style| {
                                            style
                                                .bg(p.sidebar_accent.opacity(0.7))
                                                .text_color(p.sidebar_accent_foreground)
                                        });
                                    }
                                    let icon_name = if is_local {
                                        IconName::Terminal
                                    } else {
                                        IconName::Server
                                    };
                                    Some(
                                        row.on_click(cx.listener(move |this, _, window, cx| {
                                            this.connect_session(session_id, window, cx)
                                        }))
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
                                        .context_menu({
                                            let app_entity = app_entity.clone();
                                            move |menu, _window, _cx| {
                                                let app = app_entity.clone();
                                                menu.item(
                                                    PopupMenuItem::new(
                                                        rust_i18n::t!("menu.connect").to_string(),
                                                    )
                                                    .icon(IconName::Terminal)
                                                    .on_click({
                                                        let app = app.clone();
                                                        move |_, window, cx| {
                                                            app.update(cx, |this, cx| {
                                                                this.connect_session(
                                                                    session_id, window, cx,
                                                                );
                                                            });
                                                        }
                                                    }),
                                                )
                                                .item(
                                                    PopupMenuItem::new(
                                                        rust_i18n::t!("menu.duplicate").to_string(),
                                                    )
                                                    .icon(IconName::Plus)
                                                    .on_click({
                                                        let app = app.clone();
                                                        move |_, _, cx| {
                                                            app.update(cx, |this, cx| {
                                                                this.duplicate_session(
                                                                    session_id, cx,
                                                                );
                                                            });
                                                        }
                                                    }),
                                                )
                                                .item(
                                                    PopupMenuItem::new(
                                                        rust_i18n::t!("menu.edit").to_string(),
                                                    )
                                                    .icon(IconName::Settings)
                                                    .on_click({
                                                        let app = app.clone();
                                                        move |_, window, cx| {
                                                            app.update(cx, |this, cx| {
                                                                this.open_edit_session(
                                                                    session_id, window, cx,
                                                                );
                                                            });
                                                        }
                                                    }),
                                                )
                                                .item(PopupMenuItem::separator())
                                                .item(
                                                    PopupMenuItem::new(
                                                        rust_i18n::t!("menu.delete").to_string(),
                                                    )
                                                    .icon(IconName::Trash)
                                                    .on_click({
                                                        let app = app.clone();
                                                        move |_, _, cx| {
                                                            app.update(cx, |this, cx| {
                                                                this.delete_session_by_id(
                                                                    session_id, cx,
                                                                );
                                                            });
                                                        }
                                                    }),
                                                )
                                            }
                                        })
                                        .into_any_element(),
                                    )
                                }
                            }
                        })
                        .collect()
                },
            )
            .into_any_element()
        };

        v_flex()
            .size_full()
            .overflow_hidden()
            .bg(p.sidebar)
            .text_color(p.sidebar_foreground)
            .border_r_1()
            .border_color(p.sidebar_border)
            .child(
                h_flex()
                    .w_full()
                    .p_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(p.sidebar_border)
                    .child(div().flex_1().child(Input::new(&self.search).w_full()))
                    .child(
                        div()
                            .id("sidebar-new")
                            .size(px(26.0))
                            .flex_none()
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .text_color(p.muted_foreground)
                            .hover(|s| s.bg(p.sidebar_accent).text_color(p.sidebar_foreground))
                            .active(|s| s.bg(p.border))
                            .tooltip(move |_window, cx| {
                                Tooltip::new(rust_i18n::t!("app.new_session").to_string())
                                    .build(_window, cx)
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_new_session(window, cx);
                            }))
                            .child(Icon::new(IconName::Plus).xsmall()),
                    ),
            )
            .child(
                v_flex()
                    .id("session-list")
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .p_2()
                    .child(content),
            )
            .into_any_element()
    }
}
