//! GPUI 顶层应用外壳状态与生命周期主入口。

// 功能模块的真实父模块是 `app`，文件仍留在原目录。
//
// 设计原理（Why）：AppShell 的字段只应对应用外壳及其功能模块可见。
// 若把功能模块继续挂在 crate 根，它们与 AppShell 是兄弟模块，只能通过
// `pub(crate)` 字段访问状态，封装形同虚设。用 `#[path]` 把模块挂到 `app`
// 之下后，字段可以收成模块私有，同时不必搬动文件；crate 根再重导出旧路径，
// 现有 `crate::sftp` / `crate::settings` 引用无需批量改写。
#[path = "../activity_bar/mod.rs"]
pub(crate) mod activity_bar;
#[path = "../audit_viewer/mod.rs"]
pub(crate) mod audit_viewer;
#[path = "../backup/mod.rs"]
pub(crate) mod backup;
#[path = "../batch/mod.rs"]
pub(crate) mod batch;
#[path = "../broadcast/mod.rs"]
pub(crate) mod broadcast;
#[path = "../chrome.rs"]
pub(crate) mod chrome;
#[path = "../clipboard_prompt.rs"]
pub(crate) mod clipboard_prompt;
#[path = "../cloud_sync/mod.rs"]
pub(crate) mod cloud_sync;
#[path = "../command_palette/mod.rs"]
pub(crate) mod command_palette;
#[path = "../connection/mod.rs"]
pub(crate) mod connection;
#[path = "../dialog_host/mod.rs"]
pub(crate) mod dialog_host;
#[path = "../diff_viewer/mod.rs"]
pub(crate) mod diff_viewer;
#[path = "../editor/mod.rs"]
pub(crate) mod editor;
#[path = "../event_handlers.rs"]
pub(crate) mod event_handlers;
#[path = "../events.rs"]
pub(crate) mod events;
#[path = "../forward/mod.rs"]
pub(crate) mod forward;
#[path = "../identity/mod.rs"]
pub(crate) mod identity;
#[path = "../keys/mod.rs"]
pub(crate) mod keys;
#[path = "../modals.rs"]
pub(crate) mod modals;
#[path = "../monitor/mod.rs"]
pub(crate) mod monitor;
#[path = "../notify.rs"]
pub(crate) mod notify;
#[path = "../probe.rs"]
pub(crate) mod probe;
#[path = "../record/mod.rs"]
pub(crate) mod record;
#[path = "../app_render.rs"]
mod render;
#[path = "../session_appearance.rs"]
pub(crate) mod session_appearance;
#[path = "../session_form/mod.rs"]
pub(crate) mod session_form;
#[path = "../session_serial.rs"]
pub(crate) mod session_serial;
#[path = "../settings/mod.rs"]
pub(crate) mod settings;
#[path = "../sftp/mod.rs"]
pub(crate) mod sftp;
#[path = "../sftp_handlers.rs"]
pub(crate) mod sftp_handlers;
#[path = "../shortcuts.rs"]
pub(crate) mod shortcuts;
#[path = "../sidebar/mod.rs"]
pub(crate) mod sidebar;
#[path = "../snippets/mod.rs"]
pub(crate) mod snippets;
#[path = "../split.rs"]
pub(crate) mod split;
#[path = "../startup.rs"]
pub(crate) mod startup;
#[path = "../status_bar/mod.rs"]
pub(crate) mod status_bar;
#[path = "../tabs/mod.rs"]
pub(crate) mod tabs;
#[path = "../terminal_copy_mode.rs"]
pub(crate) mod terminal_copy_mode;
#[path = "../terminal_search.rs"]
pub(crate) mod terminal_search;
#[path = "../tiling/mod.rs"]
pub(crate) mod tiling;
#[path = "../trigger_panel/mod.rs"]
pub(crate) mod trigger_panel;
#[path = "../window_state.rs"]
pub(crate) mod window_state;
#[path = "../workspace/mod.rs"]
pub(crate) mod workspace;
#[path = "../zmodem.rs"]
pub(crate) mod zmodem;

mod bootstrap;
mod modal_hub;
mod overlays;
mod persistence;
mod secret_prefill;
mod state;
mod task_hub;
mod types;

pub(crate) use modal_hub::ModalHub;
pub(crate) use persistence::Persistence;
pub(crate) use secret_prefill::prefill_secret_fields;
pub(crate) use state::AppState;
pub(crate) use task_hub::TaskHub;

use self::broadcast::BroadcastBarState;
use self::cloud_sync::GitSyncScheduler;
use self::editor::EditorManager;
use self::keys::KnownHostsModalState;
use self::settings::SettingsModalState;
use self::sftp::SftpModals;
use self::sidebar::SidebarState;
use self::snippets::SnippetPanelState;
use self::workspace::WorkspaceState;
use crate::UiServices;
use crate::keymap;
use crate::keymap::ShortcutRecordingState;
use crate::palette::localized_error;
use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::component::slider::{SliderEvent, SliderValue};
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use transom_core::{
    AppCommand, AppSettings, PanelSide, SessionCatalog, StartupTabs, TabPosition, UiThemeMode,
};

pub(crate) use types::*;

/// 命令发送失败的界面文案：队列满提示稍后重试，控制器退出提示重启应用。
pub(crate) fn command_send_error_text(error: flume::TrySendError<AppCommand>) -> String {
    match error {
        flume::TrySendError::Full(_) => rust_i18n::t!("error.command_queue_full").to_string(),
        flume::TrySendError::Disconnected(_) => {
            rust_i18n::t!("error.controller_stopped").to_string()
        }
    }
}

pub(crate) struct AppShell {
    /// 全部应用状态集中存放；外壳只负责渲染、事件路由与生命周期。
    state: AppState,
}

impl std::ops::Deref for AppShell {
    type Target = AppState;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl std::ops::DerefMut for AppShell {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl AppShell {
    /// 设置工具面板（SFTP/转发）停靠方向。
    pub(crate) fn set_sftp_panel_side(&mut self, side: PanelSide, cx: &mut Context<Self>) {
        if self.settings.window.sftp_panel_side == side {
            return;
        }
        self.settings.window.sftp_panel_side = side;
        let state = &mut self.state;
        std::mem::swap(
            &mut state.workspace_panels,
            &mut state.workspace_panels_left,
        );
        self.persist_layout_settings(cx);
    }

    /// 按启动档位恢复工作台。
    fn restore_startup_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.settings.window.startup_tabs {
            StartupTabs::RestoreLast => {
                let saved_groups = self.settings.window.previous_tab_groups.clone();
                let saved_tilings = self.settings.window.tab_tilings.clone();
                if !saved_groups.is_empty() {
                    let mut restored = Vec::with_capacity(saved_groups.len());
                    for group in &saved_groups {
                        restored.push(self.restore_tab_group(group, window, cx));
                    }
                    let mut focus = None;
                    for saved in &saved_tilings {
                        if let Some(group) = self.restore_tab_tiling(saved, &restored) {
                            focus.get_or_insert(group);
                        }
                    }
                    if let Some(group) = focus
                        && let Some(index) = self.workspace.group_index(group)
                    {
                        let pane = self.workspace.groups[index].active_pane;
                        self.activate_tab(pane, window, cx);
                    }
                } else if !self.settings.window.previous_tab_sessions.is_empty() {
                    let session_keys = self.settings.window.previous_tab_sessions.clone();
                    for key in session_keys {
                        if key == "local" {
                            self.open_local_terminal(window, cx);
                        } else if let Some(session) = self
                            .catalog
                            .sessions
                            .iter()
                            .find(|s| s.id.to_string() == key)
                            .cloned()
                        {
                            self.connect_session(session.id, window, cx);
                        }
                    }
                }
            }
            StartupTabs::EmptyWorkspace => {}
            StartupTabs::NewLocalTerminal => self.open_local_terminal(window, cx),
        }
    }

    /// 每分钟节拍：达到配置间隔时触发一次后台 Git 同步。
    fn tick_git_sync(&mut self, cx: &mut Context<Self>) {
        let interval = self
            .settings
            .sync
            .git
            .as_ref()
            .map_or(0, |git| git.interval_minutes);
        if self.git_sync_schedule.tick(interval) {
            self.start_git_sync_scheduled(cx);
        }
    }
}
