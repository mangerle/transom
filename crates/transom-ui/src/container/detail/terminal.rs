//! 容器控制台页签。

use super::*;

pub(crate) fn render_terminal(
    tab_id: TabId,
    terminal: Option<Entity<transom_terminal::TerminalView>>,
    current_user: Option<Option<String>>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    if let Some(terminal) = terminal {
        return div().size_full().child(terminal).into_any_element();
    }
    let app = cx.entity().clone();
    let root_app = app.clone();
    let default_app = app.clone();
    let user_tooltip = match &current_user {
        Some(Some(user)) => {
            rust_i18n::t!("container.terminal.current_user", user = user.as_str()).to_string()
        }
        Some(None) => rust_i18n::t!("container.terminal.current_default_user").to_string(),
        None => rust_i18n::t!("container.terminal.as_user").to_string(),
    };
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
                            this.open_container_terminal_as(tab_id, None, cx);
                        })),
                )
                .child(
                    Button::new("container-terminal-open-menu")
                        .small()
                        .icon(IconName::ChevronDown)
                        .tooltip(user_tooltip)
                        .dropdown_menu(move |menu, _, _| {
                            let root_app = root_app.clone();
                            let default_app = default_app.clone();
                            let menu_user = current_user.clone();
                            menu.item(
                                PopupMenuItem::new(
                                    rust_i18n::t!("container.terminal.as_root").to_string(),
                                )
                                .icon(IconName::Terminal)
                                .checked(matches!(&menu_user, Some(Some(user)) if user == "root"))
                                .on_click(move |_, _, cx| {
                                    let app = root_app.clone();
                                    app.update(cx, |this, cx| {
                                        this.open_container_terminal_as(
                                            tab_id,
                                            Some("root".to_string()),
                                            cx,
                                        );
                                    });
                                }),
                            )
                            .item(
                                PopupMenuItem::new(
                                    rust_i18n::t!("container.terminal.as_default").to_string(),
                                )
                                .icon(IconName::Terminal)
                                .checked(matches!(&menu_user, Some(None)))
                                .on_click(move |_, _, cx| {
                                    let app = default_app.clone();
                                    app.update(cx, |this, cx| {
                                        this.open_container_terminal_as(tab_id, None, cx);
                                    });
                                }),
                            )
                        }),
                ),
        )
        .into_any_element()
}
