//! 会话表单顶部协议切换栏与左侧分组导航栏。

use super::*;
use crate::session_form::types::SessionFormSection;

impl SessionFormView {
    /// 顶部全宽分段式协议切换栏：SSH、本地终端、Telnet、串口、Mosh。
    pub(super) fn render_session_connection_type_bar(
        &self,
        form: &SessionForm,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let protocols = [
            ("conn-ssh", ConnectionChoice::Ssh, "SSH".to_string()),
            (
                "conn-local",
                ConnectionChoice::Local,
                rust_i18n::t!("form.conn_local").to_string(),
            ),
            (
                "conn-telnet",
                ConnectionChoice::Telnet,
                "Telnet".to_string(),
            ),
            (
                "conn-serial",
                ConnectionChoice::Serial,
                rust_i18n::t!("form.conn_serial").to_string(),
            ),
            ("conn-mosh", ConnectionChoice::Mosh, "Mosh".to_string()),
            ("conn-ftp", ConnectionChoice::Ftp, "FTP".to_string()),
            ("conn-rdp", ConnectionChoice::Rdp, "RDP".to_string()),
        ];

        h_flex()
            .w_full()
            .p_1()
            .gap_1()
            .rounded(p.radius.md)
            .bg(p.muted)
            .children(protocols.into_iter().map(|(id, choice, label)| {
                let active = form.connection == choice;
                div()
                    .id(ElementId::Name(id.into()))
                    .flex_1()
                    .py_1()
                    .px_2()
                    .rounded(p.radius.sm)
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(if active {
                        p.background
                    } else {
                        gpui_kit::transparent_black()
                    })
                    .text_sm()
                    .font_weight(if active {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if active {
                        p.foreground
                    } else {
                        p.muted_foreground
                    })
                    .hover(|style| {
                        if active {
                            style
                        } else {
                            style.text_color(p.foreground)
                        }
                    })
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.set_connection(choice, window, cx);
                    }))
            }))
            .into_any_element()
    }
}

/// 按连接协议真实能力返回左侧边栏应展示的配置分组列表。
pub(in crate::app::session_form) fn sections_for_connection(
    choice: ConnectionChoice,
) -> &'static [SessionFormSection] {
    match choice {
        ConnectionChoice::Ssh => &SessionFormSection::ALL,
        ConnectionChoice::Mosh | ConnectionChoice::Telnet => &[
            SessionFormSection::Basic,
            SessionFormSection::Network,
            SessionFormSection::Terminal,
            SessionFormSection::Startup,
        ],
        ConnectionChoice::Local | ConnectionChoice::Serial => &[
            SessionFormSection::Basic,
            SessionFormSection::Terminal,
            SessionFormSection::Startup,
        ],
        ConnectionChoice::Ftp | ConnectionChoice::Rdp => {
            &[SessionFormSection::Basic, SessionFormSection::Network]
        }
    }
}

impl SessionFormView {
    /// 左侧纵向配置分组导航栏（按当前选中协议过滤不适用分组）。
    pub(super) fn render_session_sidebar(
        &self,
        form: &SessionForm,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sections = sections_for_connection(form.connection);
        v_flex()
            .w(rems(9.5))
            .flex_none()
            .gap_1()
            .children(sections.iter().copied().map(|section| {
                let active = form.section == section;
                let id = format!("session-nav-{}", section.slug());
                div()
                    .id(ElementId::Name(id.into()))
                    .w_full()
                    .px_3()
                    .py_2()
                    .rounded(p.radius.md)
                    .cursor_pointer()
                    .bg(if active {
                        p.list_active
                    } else {
                        gpui_kit::transparent_black()
                    })
                    .hover(|style| {
                        if active {
                            style
                        } else {
                            style.bg(p.list_hover)
                        }
                    })
                    .text_sm()
                    .font_weight(if active {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if active {
                        p.foreground
                    } else {
                        p.muted_foreground
                    })
                    .child(rust_i18n::t!(section.title_key()).to_string())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.set_section(section, cx);
                    }))
            }))
            .into_any_element()
    }
}
