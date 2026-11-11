//! 活动栏右键上下文菜单与条目可见性控制。

use crate::app::AppShell;
use crate::app::app_settings_mut;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::ActivityItemId;
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

impl AppShell {
    /// 活动栏单个图标项的上下文菜单：固定项仅提示，非固定项可隐藏；底部附带恢复默认动作。
    pub(super) fn activity_context_menu(
        menu: PopupMenu,
        id: ActivityItemId,
        app: &Entity<AppShell>,
    ) -> PopupMenu {
        let mut menu = menu;
        if id.is_pinned() {
            menu = menu.item(
                crate::widgets::MenuItem::new(
                    rust_i18n::t!("activity.pinned").to_string(),
                    AppIcon::Ui(UiIcon::LOCKED),
                )
                .disabled(true)
                .render(|_, _, _| {}),
            );
        } else {
            let app_hide = app.clone();
            menu = menu.item(crate::widgets::menu_item(
                rust_i18n::t!("activity.hide_item").to_string(),
                AppIcon::Ui(UiIcon::HIDE),
                move |_, _, cx| {
                    app_hide.update(cx, |this, cx| this.hide_activity_item(id, cx));
                },
            ));
        }
        let app_restore = app.clone();
        menu.item(PopupMenuItem::separator())
            .item(crate::widgets::menu_item(
                rust_i18n::t!("activity.restore_default").to_string(),
                AppIcon::Ui(UiIcon::UNDO),
                move |_, _, cx| {
                    app_restore.update(cx, |this, cx| this.restore_activity_bar(cx));
                },
            ))
    }

    /// 空白区的兜底菜单：没有具体条目可操作，只保留恢复默认动作。
    pub(super) fn activity_bar_context_menu(menu: PopupMenu, app: &Entity<AppShell>) -> PopupMenu {
        let app_restore = app.clone();
        menu.item(crate::widgets::menu_item(
            rust_i18n::t!("activity.restore_default").to_string(),
            AppIcon::Ui(UiIcon::UNDO),
            move |_, _, cx| {
                app_restore.update(cx, |this, cx| this.restore_activity_bar(cx));
            },
        ))
    }

    fn hide_activity_item(&mut self, id: ActivityItemId, cx: &mut Context<Self>) {
        if app_settings_mut(cx).activity_bar.set_visible(id, false) {
            self.persist_layout_settings(cx);
        } else {
            cx.notify();
        }
    }

    pub(crate) fn restore_activity_bar(&mut self, cx: &mut Context<Self>) {
        if app_settings_mut(cx).activity_bar.restore_defaults() {
            self.persist_layout_settings(cx);
        } else {
            cx.notify();
        }
    }
}
