//! 左侧活动栏：按配置渲染条目，并处理拖拽换序、隐藏与恢复默认。
//!
//! 活动栏只保存一份布局（顺序 + 隐藏集合 + 上下分区）。渲染时再按当前标签
//! 计算 selected/disabled，因此自定义顺序不会影响任何动作的可用性；隐藏也
//! 只隐藏图标，不关闭已经打开的面板，快捷键仍可继续操作对应功能。
//!
//! 拖拽落点分两类：图标上半/下半决定插到该图标之前/之后，图标之间的空白则
//! 承载跨区移动。空白带的上半区把条目追加到顶组末尾，下半区把条目插到底组
//! 开头，这样"拖到空白处"有确定结果，且不会让条目在两组之间来回跳。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::theme::tokens::size;
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{ActivityItemId, Connection};

/// 活动栏的布局度量统一从这里取值。
///
/// 为什么不做成设置项：按钮尺寸与间距同时决定条宽、命中区和插入线位置，
/// 单独暴露两个像素输入框会制造一组需要互相校验的状态。宽度、按钮与图标
/// 以 rem 表达（值源在 `theme::tokens::size`），随密度档位整体缩放，这正是
/// "放大点击目标走整体密度预设"的落地形式；间距（gap/padding）是像素语义
/// 的微度量，保留 px 不随密度变化。
///
/// 图标尺寸与会话树行图标（`sidebar::tree` 的 `ROW_ICON_SIZE`，18px）保持
/// 一致，让左侧两条竖栏的视觉分量对齐；该处常量是模块私有，这里只能以注释
/// 锚定数值，调整时两处必须同步。
#[derive(Clone, Copy)]
struct ActivityBarMetrics {
    width: f32,
    button: f32,
    icon: f32,
    gap: f32,
    padding_y: f32,
}

const METRICS: ActivityBarMetrics = ActivityBarMetrics {
    width: size::ACTIVITY_BAR_WIDTH,
    button: size::ACTIVITY_BUTTON_SIZE,
    icon: size::ACTIVITY_ICON_SIZE,
    gap: 4.0,
    padding_y: 4.0,
};

/// 活动栏拖拽载荷：只携带被拖动的条目标识，落点由当前布局解析。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActivityDrag(ActivityItemId);

/// 跟随鼠标的条目预览：只显示图标。
///
/// 活动栏本身只有图标，预览再带名称会在窄条旁形成一块更宽的卡片，反而遮挡
/// 落点；保持同尺寸图标贴手移动，拖拽时只看位置即可。
struct ActivityDragPreview {
    icon: IconName,
}

impl ActivityDragPreview {
    fn new(icon: IconName) -> Self {
        Self { icon }
    }
}

impl Render for ActivityDragPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        div()
            .size(rems(METRICS.button))
            .flex()
            .items_center()
            .justify_center()
            .rounded(p.radius.md)
            .bg(p.popover)
            .border_1()
            .border_color(p.border)
            .shadow_md()
            .child(
                Icon::new(self.icon)
                    .with_size(crate::theme::tokens::rem_px(
                        METRICS.icon,
                        window.rem_size(),
                    ))
                    .text_color(p.popover_foreground),
            )
    }
}

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
            // 两个空白半区合起来就是两组之间的弹性空间：上半区落到顶组末尾，
            // 下半区落到底组开头。它们不渲染固定占位，只承担落点与插入线。
            .child(Self::render_activity_blank_zone(&app, true, p))
            .child(Self::render_activity_blank_zone(&app, false, p))
            .child(
                v_flex()
                    .gap(px(METRICS.gap))
                    .items_center()
                    .children(bottom_children),
            )
            // 整条活动栏兜底右键菜单：图标上的菜单先命中并阻止冒泡，空白处
            // 落到这里；否则用户只能对着图标右键，空白区看起来像坏掉了。
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
            .child(Self::render_activity_reorder_overlay(&app, id, p));

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

        // 禁用项仍可拖动：disabled 只表示当前动作不可执行，与排序无关。
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
            if self.settings.language == "zh-CN" {
                "切换为英文 (Switch to English)".to_string()
            } else {
                "切换为中文 (Switch to Chinese)".to_string()
            }
        } else if id == ActivityItemId::Sftp
            && self
                .workspace
                .active_tab
                .and_then(|tab_id| self.workspace.tabs.iter().find(|tab| tab.id == tab_id))
                .is_some_and(|tab| matches!(tab.session.connection, Connection::Local(_)))
        {
            rust_i18n::t!("sftp.local_files_title").to_string()
        } else {
            label.to_string()
        }
    }

    fn activity_item_selected(&self, id: ActivityItemId) -> bool {
        match id {
            ActivityItemId::Sessions => self.sidebar_visible,
            ActivityItemId::Forwards => self
                .workspace
                .active_tab
                .and_then(|tab_id| self.workspace.tabs.iter().find(|tab| tab.id == tab_id))
                .is_some_and(|tab| tab.forwards_panel.is_some()),
            ActivityItemId::Sftp => self
                .workspace
                .active_tab
                .and_then(|tab_id| self.workspace.tabs.iter().find(|tab| tab.id == tab_id))
                .is_some_and(|tab| tab.sftp.as_ref().is_some_and(|panel| panel.open)),
            ActivityItemId::Monitor => self.monitor.open,
            ActivityItemId::Keys => self.settings_modal.key_wizard.is_some(),
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
            .and_then(|tab_id| self.workspace.tabs.iter().find(|tab| tab.id == tab_id))
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
                self.sidebar_visible = !self.sidebar_visible;
                cx.notify();
            }
            ActivityItemId::QuickOpen => self.toggle_quick_open(window, cx),
            ActivityItemId::Forwards => self.toggle_forward_panel(window, cx),
            ActivityItemId::Sftp => self.toggle_sftp(window, cx),
            ActivityItemId::Monitor => self.toggle_monitor(window, cx),
            ActivityItemId::Language => self.toggle_language(cx),
            ActivityItemId::ImportSshConfig => self.import_ssh_config(cx),
            ActivityItemId::KnownHosts => {
                self.known_hosts_modal.open = true;
                self.refresh_known_hosts();
                cx.notify();
            }
            ActivityItemId::Keys => self.open_key_wizard(window, cx),
            ActivityItemId::Settings => self.open_settings_modal(window, cx),
            ActivityItemId::Unknown => {}
        }
    }

    fn activity_context_menu(
        menu: PopupMenu,
        id: ActivityItemId,
        app: &Entity<AppShell>,
    ) -> PopupMenu {
        let mut menu = menu;
        if id.is_pinned() {
            menu = menu.item(
                PopupMenuItem::new(rust_i18n::t!("activity.pinned").to_string())
                    .icon(IconName::Lock)
                    .disabled(true),
            );
        } else {
            let app_hide = app.clone();
            menu = menu.item(
                PopupMenuItem::new(rust_i18n::t!("activity.hide_item").to_string())
                    .icon(IconName::EyeOff)
                    .on_click(move |_, _, cx| {
                        app_hide.update(cx, |this, cx| this.hide_activity_item(id, cx));
                    }),
            );
        }
        let app_restore = app.clone();
        menu.item(PopupMenuItem::separator()).item(
            PopupMenuItem::new(rust_i18n::t!("activity.restore_default").to_string())
                .icon(IconName::RotateCcw)
                .on_click(move |_, _, cx| {
                    app_restore.update(cx, |this, cx| this.restore_activity_bar(cx));
                }),
        )
    }

    /// 空白区的兜底菜单。这里没有具体条目可操作，只保留整条活动栏共有的动作。
    fn activity_bar_context_menu(menu: PopupMenu, app: &Entity<AppShell>) -> PopupMenu {
        let app_restore = app.clone();
        menu.item(
            PopupMenuItem::new(rust_i18n::t!("activity.restore_default").to_string())
                .icon(IconName::RotateCcw)
                .on_click(move |_, _, cx| {
                    app_restore.update(cx, |this, cx| this.restore_activity_bar(cx));
                }),
        )
    }

    fn render_activity_reorder_overlay(
        app: &Entity<AppShell>,
        anchor: ActivityItemId,
        p: &Palette,
    ) -> AnyElement {
        v_flex()
            .absolute()
            .inset_0()
            .child(Self::render_activity_reorder_half(app, anchor, true, p))
            .child(Self::render_activity_reorder_half(app, anchor, false, p))
            .into_any_element()
    }

    /// 图标内部的上下半区：上半插到锚点之前，下半插到锚点之后。
    fn render_activity_reorder_half(
        app: &Entity<AppShell>,
        anchor: ActivityItemId,
        before: bool,
        p: &Palette,
    ) -> AnyElement {
        let line_color = p.sidebar_accent_foreground;
        let tint = p.sidebar_accent.opacity(0.18);
        let zone = div().w_full().flex_1().border_color(line_color);
        let zone = if before {
            zone.drag_over::<ActivityDrag>(move |style, _, _, _| {
                style.border_t_2().border_color(line_color).bg(tint)
            })
        } else {
            zone.drag_over::<ActivityDrag>(move |style, _, _, _| {
                style.border_b_2().border_color(line_color).bg(tint)
            })
        };
        let app = app.clone();
        zone.on_drop(move |drag: &ActivityDrag, window, cx| {
            let dragged = drag.0;
            app.update(cx, |this, cx| {
                if this
                    .settings
                    .activity_bar
                    .move_relative_to(dragged, anchor, !before)
                {
                    this.persist_layout_settings(cx);
                } else {
                    cx.notify();
                }
                window.refresh();
            });
        })
        .into_any_element()
    }

    /// 两组之间的空白半区。`top_end` 为真时落到顶组末尾，否则落到底组开头。
    ///
    /// 插入线画在靠近目标组的一侧：顶组末尾的线贴着上半区上沿，底组开头的线
    /// 贴着下半区下沿。落点与最终位置一致，不会出现"线在中间、条目跳到另一组"
    /// 的错觉。
    fn render_activity_blank_zone(
        app: &Entity<AppShell>,
        top_end: bool,
        p: &Palette,
    ) -> AnyElement {
        let line_color = p.sidebar_accent_foreground;
        let tint = p.sidebar_accent.opacity(0.12);
        let zone = div()
            .w(px(METRICS.button))
            .flex_1()
            .border_color(line_color);
        let zone = if top_end {
            zone.drag_over::<ActivityDrag>(move |style, _, _, _| {
                style.border_t_2().border_color(line_color).bg(tint)
            })
        } else {
            zone.drag_over::<ActivityDrag>(move |style, _, _, _| {
                style.border_b_2().border_color(line_color).bg(tint)
            })
        };
        let app = app.clone();
        zone.on_drop(move |drag: &ActivityDrag, window, cx| {
            let dragged = drag.0;
            app.update(cx, |this, cx| {
                let changed = if top_end {
                    this.settings.activity_bar.move_to_top_end(dragged)
                } else {
                    this.settings.activity_bar.move_to_bottom_start(dragged)
                };
                if changed {
                    this.persist_layout_settings(cx);
                } else {
                    cx.notify();
                }
                window.refresh();
            });
        })
        .into_any_element()
    }

    fn hide_activity_item(&mut self, id: ActivityItemId, cx: &mut Context<Self>) {
        if self.settings.activity_bar.set_visible(id, false) {
            self.persist_layout_settings(cx);
        } else {
            cx.notify();
        }
    }

    pub(crate) fn restore_activity_bar(&mut self, cx: &mut Context<Self>) {
        if self.settings.activity_bar.restore_defaults() {
            self.persist_layout_settings(cx);
        } else {
            cx.notify();
        }
    }
}

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
