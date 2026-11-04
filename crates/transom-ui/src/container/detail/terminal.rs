//! 容器控制台页签。

use super::*;

/// 填充终端启动身份与 Shell 的选择菜单。
pub(crate) fn populate_terminal_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    tab_id: TabId,
    app: Entity<AppShell>,
) -> gpui_kit::component::menu::PopupMenu {
    let app1 = app.clone();
    let app2 = app.clone();
    let app3 = app.clone();
    let app4 = app.clone();
    let app5 = app;
    menu.item(
        PopupMenuItem::new(rust_i18n::t!("container.terminal.as_root_bash").to_string())
            .icon(IconName::Terminal)
            .on_click(move |_, _, cx| {
                let app = app1.clone();
                app.update(cx, |this, cx| {
                    this.open_container_terminal_as(
                        tab_id,
                        Some("root".to_string()),
                        Some("/bin/bash".to_string()),
                        cx,
                    );
                });
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("container.terminal.as_root_sh").to_string())
            .icon(IconName::Terminal)
            .on_click(move |_, _, cx| {
                let app = app2.clone();
                app.update(cx, |this, cx| {
                    this.open_container_terminal_as(
                        tab_id,
                        Some("root".to_string()),
                        Some("/bin/sh".to_string()),
                        cx,
                    );
                });
            }),
    )
    .item(PopupMenuItem::separator())
    .item(
        PopupMenuItem::new(rust_i18n::t!("container.terminal.as_default_bash").to_string())
            .icon(IconName::Terminal)
            .on_click(move |_, _, cx| {
                let app = app3.clone();
                app.update(cx, |this, cx| {
                    this.open_container_terminal_as(
                        tab_id,
                        None,
                        Some("/bin/bash".to_string()),
                        cx,
                    );
                });
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("container.terminal.as_default_sh").to_string())
            .icon(IconName::Terminal)
            .on_click(move |_, _, cx| {
                let app = app4.clone();
                app.update(cx, |this, cx| {
                    this.open_container_terminal_as(
                        tab_id,
                        None,
                        Some("/bin/sh".to_string()),
                        cx,
                    );
                });
            }),
    )
    .item(
        PopupMenuItem::new(rust_i18n::t!("container.terminal.as_default").to_string())
            .icon(IconName::Terminal)
            .on_click(move |_, _, cx| {
                let app = app5.clone();
                app.update(cx, |this, cx| {
                    this.open_container_terminal_as(tab_id, None, None, cx);
                });
            }),
    )
}

pub(crate) fn render_terminal(
    tab_id: TabId,
    terminal: Option<Entity<transom_terminal::TerminalView>>,
    current_user: Option<Option<String>>,
    current_shell: Option<Option<String>>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let app = cx.entity().clone();

    if let Some(terminal) = terminal {
        let default_user = rust_i18n::t!("container.terminal.default_user_name").to_string();
        let user_label = match &current_user {
            Some(Some(u)) => u.as_str(),
            _ => default_user.as_str(),
        };
        let default_shell = rust_i18n::t!("container.terminal.default_shell_name").to_string();
        let shell_label = match &current_shell {
            Some(Some(s)) => s.as_str(),
            _ => default_shell.as_str(),
        };
        let session_info = rust_i18n::t!(
            "container.terminal.session_info",
            user = user_label,
            shell = shell_label
        )
        .to_string();

        let menu_app = app.clone();
        let toolbar = h_flex()
            .gap_2()
            .px_2()
            .py_1()
            .items_center()
            .border_color(p.divider(p.border))
            .divider_edge(DividerEdge::Bottom, p.divider_width)
            .child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(Icon::new(IconName::Terminal).xsmall())
                    .child(session_info),
            )
            .child(div().flex_1())
            .child(
                Button::new("container-terminal-switch")
                    .ghost()
                    .xsmall()
                    .icon(IconName::RotateCw)
                    .label(rust_i18n::t!("container.terminal.switch_user").to_string())
                    .dropdown_menu(move |menu, _, _| {
                        populate_terminal_menu(menu, tab_id, menu_app.clone())
                    }),
            );

        return v_flex()
            .size_full()
            .overflow_hidden()
            .child(toolbar)
            .child(div().flex_1().min_h_0().child(terminal))
            .into_any_element();
    }

    let user_tooltip = match &current_user {
        Some(Some(user)) => {
            rust_i18n::t!("container.terminal.current_user", user = user.as_str()).to_string()
        }
        Some(None) => rust_i18n::t!("container.terminal.current_default_user").to_string(),
        None => rust_i18n::t!("container.terminal.as_user").to_string(),
    };

    let menu_app = app.clone();
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            div()
                .text_sm()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.terminal.hint").to_string()),
        )
        .child(
            h_flex()
                .gap_0()
                .items_center()
                .child(
                    Button::new("container-terminal-open")
                        .primary()
                        .small()
                        .icon(IconName::Terminal)
                        .label(rust_i18n::t!("container.terminal.open").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_container_terminal_as(tab_id, None, None, cx);
                        })),
                )
                .child(
                    Button::new("container-terminal-open-menu")
                        .small()
                        .icon(IconName::ChevronDown)
                        .tooltip(user_tooltip)
                        .dropdown_menu(move |menu, _, _| {
                            populate_terminal_menu(menu, tab_id, menu_app.clone())
                        }),
                ),
        )
        .into_any_element()
}
