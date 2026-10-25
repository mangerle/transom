//! 已知主机指纹管理与快捷启动面板模态框渲染模块。

use crate::app::AppShell;
use crate::palette::{Palette, localized_error};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::command::{Command, CommandItem};
use gpui_kit::component::input::Input;
use gpui_kit::*;
use transom_core::{Connection, SessionId};

impl AppShell {
    pub(crate) fn refresh_known_hosts(&mut self) {
        self.known_hosts_modal.records = self.services.known_hosts.list().unwrap_or_default();
    }

    pub(crate) fn remove_known_host(&mut self, host: &str, port: u16, cx: &mut Context<Self>) {
        if let Err(error) = self.services.known_hosts.remove(host, port) {
            self.error = Some(localized_error(&error));
        } else {
            self.status = Some(rust_i18n::t!("known_hosts.deleted").to_string());
        }
        self.refresh_known_hosts();
        cx.notify();
    }

    pub(crate) fn render_known_hosts_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let records = self.known_hosts_modal.records.clone();
        let search_query = self
            .known_hosts_modal
            .search
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let mut rows: Vec<AnyElement> = Vec::new();
        let filtered: Vec<_> = records
            .into_iter()
            .filter(|record| {
                if search_query.is_empty() {
                    return true;
                }
                record.host.to_lowercase().contains(&search_query)
                    || record.port.to_string().contains(&search_query)
                    || record.algorithm.to_lowercase().contains(&search_query)
                    || record.fingerprint.to_lowercase().contains(&search_query)
            })
            .collect();

        if filtered.is_empty() {
            rows.push(
                div()
                    .p_4()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("known_hosts.empty").to_string())
                    .into_any_element(),
            );
        } else {
            for (index, record) in filtered.into_iter().enumerate() {
                let host = record.host.clone();
                let port = record.port;
                let row = h_flex()
                    .id(ElementId::Name(format!("known-host-{index}").into()))
                    .px_3()
                    .py_2()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(p.border)
                    .hover(|style| style.bg(p.muted))
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(format!("{}:{}", record.host, record.port)),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .px_1()
                                            .rounded(p.radius.sm)
                                            .bg(p.muted)
                                            .text_color(p.muted_foreground)
                                            .child(record.algorithm),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .child(record.fingerprint),
                            ),
                    )
                    .child(
                        Button::new(ElementId::Name(format!("del-kh-{index}").into()))
                            .danger()
                            .xsmall()
                            .icon(IconName::Trash)
                            .label(rust_i18n::t!("common.delete").to_string())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_known_host(&host, port, cx);
                            })),
                    )
                    .into_any_element();
                rows.push(row);
            }
        }

        let content = v_flex()
            .gap_3()
            .child(Input::new(&self.known_hosts_modal.search).w_full())
            .child(
                v_flex()
                    .id(ElementId::Name("known-hosts-list".into()))
                    .max_h(px(380.0))
                    .overflow_y_scroll()
                    .children(rows),
            );

        let footer = Button::new("close-kh-btn")
            .ghost()
            .label(rust_i18n::t!("app.close").to_string())
            .on_click(cx.listener(|this, _, _, cx| {
                this.known_hosts_modal.open = false;
                cx.notify();
            }));

        let dialog = Self::dialog_frame(
            "known-hosts-modal",
            rust_i18n::t!("known_hosts.title").to_string(),
            None,
            content,
            Some(footer),
            px(580.0),
            p,
        );

        Self::overlay(dialog, p)
    }

    pub(crate) fn render_quick_open_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let app_entity = cx.entity().clone();
        // 渲染期读取当前查询词：CommandState 在查询变化时会触发重渲染，
        // 虚拟直连项因此始终与输入框内容同步，无需额外的 on_query 状态回写。
        let query = self.quick_open_state.read(cx).query(cx).trim().to_string();

        #[derive(Clone)]
        enum QuickOpenTarget {
            Local,
            Session(SessionId),
            /// 免保存直连：查询词本身是主机地址时的虚拟条目（Quick Connect）。
            QuickConnect,
        }

        let mut targets: Vec<QuickOpenTarget> = Vec::new();
        let mut items: Vec<CommandItem> = Vec::new();

        // 本地终端会话
        targets.push(QuickOpenTarget::Local);
        items.push(
            CommandItem::new()
                .icon(IconName::Terminal)
                .label(rust_i18n::t!("app.new_local").to_string())
                .keywords(vec![
                    SharedString::from("local"),
                    SharedString::from("terminal"),
                    SharedString::from("bash"),
                    SharedString::from("zsh"),
                    SharedString::from("sh"),
                    SharedString::from("本地终端"),
                ]),
        );

        // 已保存会话
        for session in &self.catalog.sessions {
            let session_id = session.id;
            targets.push(QuickOpenTarget::Session(session_id));

            let (detail, icon_name) = match &session.connection {
                Connection::Ssh(ssh) => (
                    format!("{}@{}:{}", ssh.username, ssh.host, ssh.port),
                    IconName::Server,
                ),
                Connection::Telnet(telnet) => (
                    format!("Telnet: {}:{}", telnet.host, telnet.port),
                    IconName::Network,
                ),
                Connection::Local(l) => {
                    let shell = l.shell.as_deref().unwrap_or("sh");
                    (format!("Local: {shell}"), IconName::Terminal)
                }
                Connection::Serial(serial) => (
                    format!("Serial: {} @ {}", serial.port, serial.baud_rate),
                    IconName::Cable,
                ),
                _ => (String::new(), IconName::Server),
            };

            let mut keywords: Vec<SharedString> = vec![session.name.clone().into(), detail.into()];
            if let Some(folder_id) = session.folder_id
                && let Some(path) = self.catalog.folder_path(folder_id)
            {
                keywords.push(path.into());
            }

            items.push(
                CommandItem::new()
                    .icon(icon_name)
                    .label(session.name.clone())
                    .keywords(keywords),
            );
        }

        // 免保存直连虚拟项：查询词形似主机地址时追加，keywords 含查询词本身
        // 保证过滤命中；放在已保存会话之后，同名主机的既有会话优先。
        if transom_core::looks_like_host(&query) {
            targets.push(QuickOpenTarget::QuickConnect);
            items.push(
                CommandItem::new()
                    .icon(IconName::Globe)
                    .label(rust_i18n::t!("app.quick_connect", host = query.clone()).to_string())
                    .keywords(vec![SharedString::from(query)]),
            );
        }

        let targets = std::rc::Rc::new(targets);
        let targets_confirm = targets.clone();

        let cmd = Command::new(&self.quick_open_state)
            .placeholder(rust_i18n::t!("app.quick_open_placeholder").to_string())
            .items(items)
            .on_confirm({
                let app = app_entity.clone();
                move |index_path, window, cx| {
                    if let Some(target) = targets_confirm.get(index_path.row) {
                        match target {
                            QuickOpenTarget::Local => {
                                app.update(cx, |this, cx| {
                                    this.show_quick_open = false;
                                    this.open_local_terminal(window, cx);
                                });
                            }
                            QuickOpenTarget::Session(session_id) => {
                                let session_id = *session_id;
                                app.update(cx, |this, cx| {
                                    this.show_quick_open = false;
                                    this.connect_session(session_id, window, cx);
                                });
                            }
                            QuickOpenTarget::QuickConnect => {
                                // 主机词以确认时刻的输入框为准，避免渲染期快照过期。
                                let host = app.read_with(cx, |this, cx| {
                                    this.quick_open_state.read(cx).query(cx).trim().to_string()
                                });
                                app.update(cx, |this, cx| {
                                    this.show_quick_open = false;
                                    this.connect_quick_host(&host, cx);
                                });
                            }
                        }
                    }
                }
            })
            .on_cancel({
                let app = app_entity.clone();
                move |_window, cx| {
                    app.update(cx, |this, cx| {
                        this.show_quick_open = false;
                        cx.notify();
                    });
                }
            });

        let panel = v_flex()
            .id(ElementId::Name("quick-open-modal".into()))
            .w(px(580.0))
            .rounded(p.radius.lg)
            .bg(p.popover)
            .text_color(p.popover_foreground)
            .border_1()
            .border_color(p.border)
            .shadow_lg()
            .overflow_hidden()
            .child(cmd);

        Self::overlay(panel, p)
    }
}
