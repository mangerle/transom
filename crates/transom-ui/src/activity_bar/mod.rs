//! 左侧活动栏：按配置渲染条目，并处理拖拽换序、隐藏与恢复默认。
//!
//! 设计原理（Why）：
//! - 活动栏只保存一份布局（顺序 + 隐藏集合 + 上下分区）。渲染时再按当前标签
//!   计算 selected/disabled，因此自定义顺序不会影响任何动作的可用性；隐藏也
//!   只隐藏图标，不关闭已经打开的面板，快捷键仍可继续操作对应功能；
//! - 拖拽落点分两类：图标上半/下半决定插到该图标之前/之后，图标之间的空白则
//!   承载跨区移动；
//! - 模块拆分：度量提取至 `metrics`，拖拽与预览提取至 `drag`，右键菜单提取至 `menu`，
//!   主模块聚焦视图装配与条目点击分发。

mod drag;
mod menu;
mod metrics;

use crate::app::AppShell;
use crate::palette::Palette;
use drag::{
    ActivityDrag, ActivityDragPreview, render_activity_blank_zone, render_activity_reorder_overlay,
};
use gpui_kit::base::v_flex;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use metrics::METRICS;
use transom_core::{ActivityItemId, Connection};

pub(crate) use self::labels::{activity_item_icon, activity_item_label};

impl AppShell {
    pub(crate) fn render_activity_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let top: Vec<ActivityItemId> = self.settings.activity_bar.top_items().collect();
        let bottom: Vec<ActivityItemId> = self.settings.activity_bar.bottom_items().collect();
        let mut top_children = Vec::with_capacity(top.len());
        for id in top {
            top_children.push(self.render_activity_item(window, id, p, cx));
        }
        let mut bottom_children = Vec::with_capacity(bottom.len());
        for id in bottom {
            bottom_children.push(self.render_activity_item(window, id, p, cx));
        }

        let app = cx.entity().clone();
        let menu_app = app.clone();
        v_flex()
            .id("activity-bar")
            .w(rems(METRICS.width))
            .flex_none()
            .h_full()
            .items_center()
            .py(px(METRICS.padding_y))
            .bg(p.sidebar)
            .border_r_1()
            .border_color(p.border)
            .child(
                v_flex()
                    .gap(px(METRICS.gap))
                    .items_center()
                    .children(top_children),
            )
            .child(render_activity_blank_zone(&app, true, p))
            .child(render_activity_blank_zone(&app, false, p))
            .child(
                v_flex()
                    .gap(px(METRICS.gap))
                    .items_center()
                    .children(bottom_children),
            )
            .context_menu(move |menu, _window, _cx| {
                Self::activity_bar_context_menu(menu, &menu_app)
            })
            .into_any_element()
    }

    fn render_activity_item(
        &mut self,
        window: &Window,
        id: ActivityItemId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = activity_item_label(id);
        let icon = activity_item_icon(id);
        let tooltip = self.activity_item_tooltip(id, &label);
        let selected = self.activity_item_selected(id);
        let disabled = self.activity_item_disabled(id);
        let app = cx.entity().clone();

        let mut button = div()
            .id(ElementId::Name(format!("activity-{}", id.key()).into()))
            .relative()
            .size(rems(METRICS.button))
            .rounded(p.radius.md)
            .flex()
            .items_center()
            .justify_center()
            .tooltip(move |_window, cx| Tooltip::new(tooltip.clone()).build(_window, cx))
            .child(Icon::new(icon).with_size(crate::theme::tokens::rem_px(
                METRICS.icon,
                window.rem_size(),
            )))
            .child(render_activity_reorder_overlay(&app, id, p));

        if selected {
            button = button
                .bg(p.sidebar_accent)
                .text_color(p.sidebar_accent_foreground)
                .cursor_pointer();
        } else if disabled {
            button = button
                .text_color(p.muted_foreground.opacity(0.4))
                .cursor_not_allowed();
        } else {
            button = button
                .cursor_pointer()
                .text_color(p.muted_foreground)
                .hover(|style| {
                    style
                        .bg(p.sidebar_accent.opacity(0.8))
                        .text_color(p.sidebar_foreground)
                })
                .active(|style| style.bg(p.sidebar_accent));
        }

        if !disabled {
            button = button.on_click(cx.listener(move |this, _, window, cx| {
                this.activate_activity_item(id, window, cx);
            }));
        }

        button
            .on_drag(ActivityDrag(id), move |_, _, _, cx| {
                cx.new(|_| ActivityDragPreview::new(icon))
            })
            .context_menu(move |menu, _window, _cx| Self::activity_context_menu(menu, id, &app))
            .into_any_element()
    }

    fn activity_item_tooltip(&self, id: ActivityItemId, label: &str) -> String {
        if id == ActivityItemId::Language {
            if crate::i18n::resolve_language(&self.settings.language) == "zh-CN" {
                rust_i18n::t!("activity.switch_to_english").to_string()
            } else {
                rust_i18n::t!("activity.switch_to_chinese").to_string()
            }
        } else if id == ActivityItemId::Sftp
            && self
                .workspace
                .active_tab
                .and_then(|tab_id| self.workspace.tab(tab_id))
                .is_some_and(|tab| matches!(tab.session.connection, Connection::Local(_)))
        {
            rust_i18n::t!("sftp.local_files_title").to_string()
        } else {
            label.to_string()
        }
    }

    fn activity_item_selected(&self, id: ActivityItemId) -> bool {
        match id {
            ActivityItemId::Sessions => self.sidebar.visible,
            ActivityItemId::Forwards => self
                .workspace
                .active_tab
                .and_then(|tab_id| self.workspace.tab(tab_id))
                .is_some_and(|tab| tab.forwards_panel().is_some()),
            ActivityItemId::Sftp => self
                .workspace
                .active_tab
                .and_then(|tab_id| self.workspace.tab(tab_id))
                .is_some_and(|tab| tab.sftp().is_some_and(|panel| panel.open)),
            ActivityItemId::Monitor => self.monitor.open,
            ActivityItemId::Keys => self.modals.settings.key_wizard.is_some(),
            _ => false,
        }
    }

    fn activity_item_disabled(&self, id: ActivityItemId) -> bool {
        if !matches!(id, ActivityItemId::Forwards | ActivityItemId::Sftp) {
            return false;
        }
        let Some(tab) = self
            .workspace
            .active_tab
            .and_then(|tab_id| self.workspace.tab(tab_id))
        else {
            return true;
        };
        match id {
            ActivityItemId::Forwards => !matches!(tab.session.connection, Connection::Ssh(_)),
            ActivityItemId::Sftp => !matches!(
                tab.session.connection,
                Connection::Ssh(_) | Connection::Local(_)
            ),
            _ => false,
        }
    }

    fn activate_activity_item(
        &mut self,
        id: ActivityItemId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match id {
            ActivityItemId::Sessions => {
                self.sidebar.toggle_visible();
                cx.notify();
            }
            ActivityItemId::QuickOpen => self.toggle_quick_open(window, cx),
            ActivityItemId::Forwards => self.toggle_forward_panel(window, cx),
            ActivityItemId::Sftp => self.toggle_sftp(window, cx),
            ActivityItemId::Monitor => self.toggle_monitor(window, cx),
            ActivityItemId::Language => self.toggle_language(cx),
            ActivityItemId::ImportSshConfig => self.import_ssh_config(cx),
            ActivityItemId::KnownHosts => {
                self.modals.known_hosts.open = true;
                self.refresh_known_hosts();
                cx.notify();
            }
            ActivityItemId::Keys => self.open_key_wizard(window, cx),
            ActivityItemId::Settings => self.open_settings_modal(window, cx),
            ActivityItemId::Unknown => {}
        }
    }
}

mod labels {
    use gpui_kit::assets::IconName;
    use transom_core::ActivityItemId;

    pub(crate) fn activity_item_icon(id: ActivityItemId) -> IconName {
        match id {
            ActivityItemId::Sessions => IconName::Server,
            ActivityItemId::QuickOpen => IconName::Search,
            ActivityItemId::Forwards => IconName::ArrowLeftRight,
            ActivityItemId::Sftp => IconName::Folder,
            ActivityItemId::Monitor => IconName::Activity,
            ActivityItemId::Language => IconName::Globe,
            ActivityItemId::ImportSshConfig => IconName::Download,
            ActivityItemId::KnownHosts => IconName::ShieldCheck,
            ActivityItemId::Keys => IconName::KeyRound,
            ActivityItemId::Settings => IconName::Settings,
            ActivityItemId::Unknown => IconName::CircleAlert,
        }
    }

    pub(crate) fn activity_item_label(id: ActivityItemId) -> String {
        let key = match id {
            ActivityItemId::Sessions => "app.sessions",
            ActivityItemId::QuickOpen => "app.quick_open",
            ActivityItemId::Forwards => "forward.title",
            ActivityItemId::Sftp => "sftp.title",
            ActivityItemId::Monitor => "monitor.title",
            ActivityItemId::Language => "activity.language",
            ActivityItemId::ImportSshConfig => "app.import_ssh_config",
            ActivityItemId::KnownHosts => "known_hosts.title",
            ActivityItemId::Keys => "keys.title",
            ActivityItemId::Settings => "settings.title",
            ActivityItemId::Unknown => "activity.unknown",
        };
        rust_i18n::t!(key).to_string()
    }
}
