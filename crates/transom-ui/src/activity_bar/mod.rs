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
use crate::app::app_settings;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::sidebar::SidebarSection;
use drag::{
    ActivityDrag, ActivityDragPreview, render_activity_blank_zone, render_activity_reorder_overlay,
};
use gpui_kit::base::v_flex;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use metrics::METRICS;
use transom_core::AppSettings;
use transom_core::{ActivityItemId, Connection, TabPosition};
use transom_ui_icons::AppIconElement;

pub(crate) use self::labels::{activity_item_icon, activity_item_label};
pub(crate) use menu::restore_activity_bar;

fn activity_item_selected(
    shell: &AppShell,
    id: ActivityItemId,
    monitor_open: bool,
    cx: &App,
) -> bool {
    let section = shell
        .state
        .sidebar
        .section()
        .resolved_for_tab_position(app_settings(cx).window.tab_position);
    match id {
        ActivityItemId::Sessions => {
            shell.state.sidebar.visible() && section == SidebarSection::Sessions
        }
        ActivityItemId::Tabs => shell.state.sidebar.visible() && section == SidebarSection::Tabs,
        ActivityItemId::Forwards => shell
            .state
            .workspace
            .active_tab
            .and_then(|tab_id| shell.state.workspace.tab(tab_id))
            .is_some_and(|tab| tab.forwards_view().is_some()),
        ActivityItemId::Sftp => shell
            .state
            .workspace
            .active_tab
            .and_then(|tab_id| shell.state.workspace.tab(tab_id)?.sftp_view())
            .is_some_and(|view| view.read(cx).state.open),
        ActivityItemId::Monitor => monitor_open,
        ActivityItemId::Containers => {
            shell.state.sidebar.visible() && section == SidebarSection::Containers
        }
        ActivityItemId::Git => shell.state.sidebar.visible() && section == SidebarSection::Git,
        ActivityItemId::CloudSync => {
            shell.state.sidebar.visible() && section == SidebarSection::CloudSync
        }
        ActivityItemId::Keys => shell.state.modals.tools.key_wizard.read(cx).is_open(),
        _ => false,
    }
}

fn activity_item_disabled(shell: &AppShell, id: ActivityItemId) -> bool {
    if !matches!(id, ActivityItemId::Forwards | ActivityItemId::Sftp) {
        return false;
    }
    let Some(tab) = shell
        .state
        .workspace
        .active_tab
        .and_then(|tab_id| shell.state.workspace.tab(tab_id))
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

pub(crate) fn render_activity_bar(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
    p: &Palette,
) -> AnyElement {
    let top: Vec<ActivityItemId> = app_settings(cx)
        .activity_bar
        .top_items()
        .filter(|id| activity_item_available(shell, *id, app_settings(cx)))
        .collect();
    let bottom: Vec<ActivityItemId> = app_settings(cx)
        .activity_bar
        .bottom_items()
        .filter(|id| activity_item_available(shell, *id, app_settings(cx)))
        .collect();
    let mut top_children = Vec::with_capacity(top.len());
    for id in top {
        top_children.push(render_activity_item(shell, window, id, p, cx));
    }
    let mut bottom_children = Vec::with_capacity(bottom.len());
    for id in bottom {
        bottom_children.push(render_activity_item(shell, window, id, p, cx));
    }

    let app = cx.entity().clone();
    let menu_app = app.clone();
    let mut bar = v_flex()
        .id("activity-bar")
        .w(rems(METRICS.width))
        .flex_none()
        .h_full()
        .items_center()
        .py(px(METRICS.padding_y))
        .bg(p.sidebar);
    // 侧栏展开时，活动栏右缘就是与侧栏的分界线；侧栏收起后主面板的
    // 手柄线紧贴这条边，这里再画一条会并排成 2px，所以收起时不画。
    if shell.state.sidebar.visible() {
        bar = bar
            .border_color(p.divider(p.border))
            .divider_edge(DividerEdge::Right, p.divider_width);
    }
    bar.child(
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
    .context_menu(move |menu, _window, _cx| menu::activity_bar_context_menu(menu, &menu_app))
    .into_any_element()
}

fn render_activity_item(
    shell: &mut AppShell,
    window: &Window,
    id: ActivityItemId,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let label = activity_item_label(id);
    let icon = activity_item_icon(id);
    let tooltip = activity_item_tooltip(shell, id, &label, app_settings(cx));
    let selected = activity_item_selected(shell, id, shell.state.monitor.read(cx).is_open(), cx);
    let disabled = activity_item_disabled(shell, id);
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
        .child(
            AppIconElement::new(icon).with_size(crate::theme::tokens::rem_px(
                METRICS.icon,
                window.rem_size(),
            )),
        )
        .child(render_activity_reorder_overlay(&app, id, p));

    if selected {
        button = button
            .bg(p.surface.control_selected)
            .text_color(p.sidebar_accent_foreground)
            .cursor_pointer();
    } else if disabled {
        button = button.text_color(p.text.disabled).cursor_not_allowed();
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
            activate_activity_item(this, id, window, cx);
        }));
    }

    button
        .on_drag(ActivityDrag(id), move |_, _, _, cx| {
            cx.new(|_| ActivityDragPreview::new(icon))
        })
        .context_menu(move |menu, _window, _cx| menu::activity_context_menu(menu, id, &app))
        .into_any_element()
}

fn activate_activity_item(
    shell: &mut AppShell,
    id: ActivityItemId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    match id {
        ActivityItemId::Sessions => {
            // 左侧槽位一次只展示一个视图：点击当前视图收起，点击其他视图直接切换。
            shell.state.sidebar.toggle_section(SidebarSection::Sessions);
            cx.notify();
        }
        ActivityItemId::Tabs => {
            if app_settings(cx).window.tab_position != TabPosition::Left {
                return;
            }
            shell.state.sidebar.toggle_section(SidebarSection::Tabs);
            cx.notify();
        }
        ActivityItemId::QuickOpen => crate::connection::toggle_quick_open(shell, window, cx),
        ActivityItemId::Forwards => crate::forward::toggle_forward_panel(shell, window, cx),
        ActivityItemId::Sftp => crate::sftp::toggle_sftp(shell, window, cx),
        ActivityItemId::Monitor => crate::monitor::toggle_monitor(shell, window, cx),
        ActivityItemId::Containers => {
            shell
                .state
                .sidebar
                .toggle_section(SidebarSection::Containers);
            cx.notify();
        }
        ActivityItemId::Language => shell.toggle_language(cx),
        ActivityItemId::ImportSshConfig => crate::connection::import_ssh_config(shell, cx),
        ActivityItemId::KnownHosts => {
            shell
                .state
                .modals
                .connection
                .known_hosts
                .update(cx, |view, cx| view.open(cx));
        }
        ActivityItemId::Keys => crate::keys::open_key_wizard(shell, window, cx),
        ActivityItemId::Settings => shell.open_settings_modal(None, window, cx),
        ActivityItemId::Git => crate::git::actions::toggle_git_sidebar(shell, cx),
        ActivityItemId::CloudSync => crate::cloud_sync::toggle_cloud_sync_sidebar(shell, cx),
        ActivityItemId::Unknown => {}
    }
}

fn activity_item_tooltip(
    shell: &AppShell,
    id: ActivityItemId,
    label: &str,
    settings: &AppSettings,
) -> String {
    if id == ActivityItemId::Language {
        if crate::i18n::resolve_language(&settings.language) == "zh-CN" {
            rust_i18n::t!("activity.switch_to_english").to_string()
        } else {
            rust_i18n::t!("activity.switch_to_chinese").to_string()
        }
    } else if id == ActivityItemId::Sftp
        && shell
            .state
            .workspace
            .active_tab
            .and_then(|tab_id| shell.state.workspace.tab(tab_id))
            .is_some_and(|tab| matches!(tab.session.connection, Connection::Local(_)))
    {
        rust_i18n::t!("sftp.local_files_title").to_string()
    } else {
        label.to_string()
    }
}
/// 条目在当前上下文是否可展示。
///
/// 纵向标签条目只在"标签位置 = 左侧"时有意义；顶部模式下直接不渲染，
/// 避免保留一个点击无反应的图标。设置页仍保留该条目，方便用户调整顺序。
fn activity_item_available(_shell: &AppShell, id: ActivityItemId, settings: &AppSettings) -> bool {
    id != ActivityItemId::Tabs || settings.window.tab_position == TabPosition::Left
}

mod labels {
    use transom_core::ActivityItemId;
    use transom_ui_icons::AppIcon;
    use transom_ui_icons::CustomIcon;
    use transom_ui_icons::DomainIcon;
    use transom_ui_icons::UiIcon;

    pub(crate) fn activity_item_icon(id: ActivityItemId) -> AppIcon {
        match id {
            ActivityItemId::Sessions => UiIcon::WEB_SERVER.into(),
            ActivityItemId::Tabs => DomainIcon::TERMINAL.into(),
            ActivityItemId::QuickOpen => UiIcon::SEARCH.into(),
            ActivityItemId::Forwards => DomainIcon::FORWARD.into(),
            ActivityItemId::Sftp => UiIcon::PROJECT.into(),
            ActivityItemId::Monitor => DomainIcon::MONITOR.into(),
            ActivityItemId::Containers => CustomIcon::DOCKER.into(),
            ActivityItemId::Language => UiIcon::LANGUAGE.into(),
            ActivityItemId::ImportSshConfig => UiIcon::DOWNLOAD.into(),
            ActivityItemId::KnownHosts => CustomIcon::FINGERPRINT.into(),
            ActivityItemId::Keys => UiIcon::KEY.into(),
            ActivityItemId::Settings => UiIcon::SETTINGS.into(),
            ActivityItemId::Git => UiIcon::VCS.into(),
            ActivityItemId::CloudSync => CustomIcon::CLOUD.into(),
            ActivityItemId::Unknown => UiIcon::ERROR.into(),
        }
    }

    pub(crate) fn activity_item_label(id: ActivityItemId) -> String {
        let key = match id {
            ActivityItemId::Sessions => "app.sessions",
            ActivityItemId::Tabs => "activity.tabs",
            ActivityItemId::QuickOpen => "app.quick_open",
            ActivityItemId::Forwards => "forward.title",
            ActivityItemId::Sftp => "sftp.title",
            ActivityItemId::Monitor => "monitor.title",
            ActivityItemId::Containers => "container.title",
            ActivityItemId::Git => "activity.git",
            ActivityItemId::CloudSync => "activity.cloud_sync",
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
