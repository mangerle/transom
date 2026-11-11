//! 日志过滤条相关菜单：日期范围与受跟踪路径选择。

use crate::app::AppShell;
use crate::widgets::menu_item;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::TabId;
use transom_ui_icons::AppIcon;

/// 日期范围菜单。
pub(crate) fn render_date_menu(menu: PopupMenu, app: Entity<AppShell>, tab_id: TabId) -> PopupMenu {
    let presets: [(&str, Option<&str>); 4] = [
        ("git.filter.date_all", None),
        ("git.filter.date_today", Some("today")),
        ("git.filter.date_week", Some("1 week ago")),
        ("git.filter.date_month", Some("1 month ago")),
    ];
    let mut menu = menu;
    for (key, value) in presets {
        let item_app = app.clone();
        let owned = value.map(str::to_string);
        menu = menu.item(PopupMenuItem::new(rust_i18n::t!(key).to_string()).on_click(
            move |_, _, cx| {
                let since = owned.clone();
                item_app.update(cx, |this, cx| {
                    this.git_set_log_dates(tab_id, since, cx);
                });
            },
        ));
    }
    menu
}

/// 路径过滤菜单：列出跟踪文件（上限 300，超出时提示用检索框）。
pub(crate) fn render_path_menu(
    menu: PopupMenu,
    app: Entity<AppShell>,
    tab_id: TabId,
    paths: Vec<String>,
) -> PopupMenu {
    let all_app = app.clone();
    let menu = menu.item(
        PopupMenuItem::new(rust_i18n::t!("git.filter.path_all").to_string()).on_click(
            move |_, _, cx| {
                all_app.update(cx, |this, cx| this.git_set_log_path(tab_id, None, cx));
            },
        ),
    );
    if paths.is_empty() {
        return menu.item(
            PopupMenuItem::new(rust_i18n::t!("git.filter.loading_paths").to_string())
                .disabled(true),
        );
    }
    let mut menu = menu;
    for path in paths.into_iter().take(300) {
        let item_app = app.clone();
        menu = menu.item(menu_item(
            path.clone(),
            AppIcon::for_file(&path),
            move |_, _, cx| {
                let path = path.clone();
                item_app.update(cx, |this, cx| {
                    this.git_set_log_path(tab_id, Some(path), cx);
                });
            },
        ));
    }
    menu
}
