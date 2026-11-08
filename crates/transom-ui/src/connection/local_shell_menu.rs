//! 新建本地终端时的 Shell 选择菜单。

use crate::app::AppShell;
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_platform::shell::ShellOption;

/// 构建“默认 Shell、各已发现 Shell、Windows 管理员 Shell”的统一菜单。
pub(crate) fn build_local_terminal_menu(
    menu: PopupMenu,
    app: &Entity<AppShell>,
    shells: Vec<ShellOption>,
    window: &mut Window,
    cx: &mut App,
) -> PopupMenu {
    let default_app = app.clone();
    let mut menu = menu.item(crate::widgets::menu_item(
        rust_i18n::t!("app.local_shell_default").to_string(),
        IconName::Terminal,
        move |_, window, cx| {
            default_app.update(cx, |this, cx| this.open_local_terminal(window, cx));
        },
    ));
    menu = menu.item(PopupMenuItem::separator());
    for shell in &shells {
        let app = app.clone();
        let shell = shell.clone();
        let label = shell.label.clone();
        menu = menu.item(crate::widgets::menu_item(
            label,
            IconName::Terminal,
            move |_, _, cx| {
                app.update(cx, |this, cx| {
                    this.spawn_local_shell(shell.clone(), false, cx);
                });
            },
        ));
    }

    #[cfg(windows)]
    {
        let elevated: Vec<ShellOption> = shells
            .into_iter()
            .filter(|shell| shell.supports_elevation)
            .collect();
        let has_elevated = !elevated.is_empty();
        let submenu_app = app.clone();
        let submenu = PopupMenu::build(window, cx, move |mut menu, _window, _cx| {
            for shell in &elevated {
                let app = submenu_app.clone();
                let shell = shell.clone();
                let label = shell.label.clone();
                menu = menu.item(crate::widgets::menu_item(
                    label,
                    IconName::Terminal,
                    move |_, _, cx| {
                        app.update(cx, |this, cx| {
                            this.spawn_local_shell(shell.clone(), true, cx);
                        });
                    },
                ));
            }
            menu
        });
        menu = menu.item(
            PopupMenuItem::submenu(rust_i18n::t!("app.local_shell_admin").to_string(), submenu)
                .disabled(!has_elevated),
        );
    }

    #[cfg(not(windows))]
    {
        let _ = (window, cx);
    }

    menu
}
