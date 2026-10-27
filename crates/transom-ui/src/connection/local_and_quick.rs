//! 本地终端启动、快捷连接、桌面深链与命令历史记录。

use crate::app::{AppShell, TerminalTab};
use crate::keymap;
use gpui_kit::*;
use transom_core::{DeepLinkRequest, LocalConnection, Session, TabId, parse_ssh_config};

impl AppShell {
    pub(crate) fn on_new_local_terminal(
        &mut self,
        _: &keymap::NewLocalTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_local_terminal(window, cx);
    }

    pub(crate) fn on_quick_open(
        &mut self,
        _: &keymap::QuickOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_quick_open(window, cx);
    }

    pub(crate) fn toggle_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modals.show_quick_open = !self.modals.show_quick_open;
        if self.modals.show_quick_open {
            self.modals.quick_open_state.update(cx, |state, cx| {
                state.set_query("", window, cx);
                state.focus(window, cx);
            });
        }
        cx.notify();
    }

    pub(crate) fn open_local_terminal(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.spawn_local_terminal(None, cx);
    }

    /// 新开一个本地终端标签。
    pub(crate) fn spawn_local_terminal(
        &mut self,
        working_dir: Option<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let title = match working_dir.as_deref().and_then(|dir| {
            dir.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        }) {
            Some(name) => format!("{} · {name}", rust_i18n::t!("app.new_local")),
            None => rust_i18n::t!("app.new_local").to_string(),
        };
        let tab_id = TabId::new();
        let session = Session::new(
            title,
            transom_core::Connection::Local(LocalConnection {
                shell: self.settings.local_shell.clone(),
                working_dir,
            }),
        );
        self.create_local_group(tab_id, session.id, &session.name);
        self.workspace
            .tabs
            .push(TerminalTab::pending(tab_id, session.clone()));
        self.workspace.active_tab = Some(tab_id);
        self.present_active_group();
        self.start_connection(tab_id, session, cx);
    }

    /// Quick Connect：解析主机串并发起不落盘的临时 SSH 连接。
    pub(crate) fn connect_quick_host(&mut self, host: &str, cx: &mut Context<Self>) {
        let session = match transom_core::quick_connect_session(host) {
            Ok(session) => session,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let tab_id = TabId::new();
        self.create_group(tab_id, session.id);
        self.workspace
            .tabs
            .push(TerminalTab::pending(tab_id, session.clone()));
        self.workspace.active_tab = Some(tab_id);
        self.present_active_group();
        self.start_connection(tab_id, session, cx);
    }

    /// 桌面深链唤起。
    pub(crate) fn handle_deep_link(
        &mut self,
        request: DeepLinkRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.activate(true);
        window.activate_window();
        match request {
            DeepLinkRequest::Ssh { url } => self.connect_quick_host(&url, cx),
            DeepLinkRequest::Dir { path } => self.spawn_local_terminal(Some(path), cx),
            DeepLinkRequest::Activate => cx.notify(),
        }
    }

    /// 记录一条采集到的命令：更新内存快照并后台落盘。
    pub(crate) fn record_history_command(&mut self, command: &str, cx: &mut Context<Self>) {
        if !self.settings.history.enabled {
            return;
        }
        let recorded = {
            let mut snapshot = self.history.write();
            transom_core::record_command(&mut snapshot, command)
        };
        if !recorded {
            return;
        }
        let snapshot: Vec<String> = self.history.read().clone();
        let store = self.services.history.clone();
        cx.spawn(async move |_, cx| {
            let error = cx
                .background_executor()
                .spawn(async move { store.save(&snapshot).err() })
                .await;
            if let Some(error) = error {
                log::warn!("命令历史写盘失败，本次仍保留在内存中，原因: {error}");
            }
        })
        .detach();
    }

    /// 导入本地 ~/.ssh/config 配置。
    pub(crate) fn import_ssh_config(&mut self, cx: &mut Context<Self>) {
        let ssh_config_path =
            directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".ssh").join("config"));

        let Some(path) = ssh_config_path else {
            self.error = Some(rust_i18n::t!("app.import_none").to_string());
            cx.notify();
            return;
        };

        if !path.exists() {
            self.error = Some(rust_i18n::t!("app.import_none").to_string());
            cx.notify();
            return;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(e.to_string());
                cx.notify();
                return;
            }
        };

        let parsed = parse_ssh_config(&content);
        if parsed.is_empty() {
            self.error = Some(rust_i18n::t!("app.import_none").to_string());
            cx.notify();
            return;
        }

        let folder = self.catalog.ensure_folder_path("SSH Config");
        let mut added_count = 0;
        for mut session in parsed {
            if !self.catalog.sessions.iter().any(|s| s.name == session.name) {
                session.folder_id = folder;
                self.catalog.sessions.push(session);
                added_count += 1;
            }
        }

        if added_count > 0 {
            if self.persist_catalog() {
                self.status =
                    Some(rust_i18n::t!("app.import_success", count = added_count).to_string());
            }
        } else {
            self.status = Some(rust_i18n::t!("app.import_none").to_string());
        }
        cx.notify();
    }
}
