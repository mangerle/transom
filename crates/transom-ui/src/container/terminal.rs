//! 容器控制台：按用户身份打开 exec 终端并接入历史与快捷键过滤。

use super::*;

impl AppShell {
    /// 切到终端页签并按指定用户建立会话。
    ///
    /// 同一容器只保留一个终端：换了用户（root ↔ 镜像默认）就结束旧会话重建，
    /// 选的是同一个用户则只切页签，避免无谓地杀掉一次已经在用的 shell。
    pub(crate) fn open_container_terminal_as(
        &mut self,
        tab_id: TabId,
        user: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return;
        };
        let same_user = workspace
            .terminal
            .as_ref()
            .is_some_and(|terminal| terminal.user == user);
        if same_user {
            if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                workspace.detail_tab = ContainerDetailTab::Terminal;
            }
            cx.notify();
            return;
        }
        let pending = workspace
            .terminal_request
            .as_ref()
            .map(|(request_id, _)| *request_id);
        if let Some(request_id) = pending {
            self.cancel_container_request(tab_id, request_id);
        }
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            // 丢弃旧终端即可结束旧 exec（TerminalIo 的 RAII 清理），再建新会话。
            workspace.terminal = None;
            workspace.terminal_request = None;
            workspace.detail_tab = ContainerDetailTab::Terminal;
        }
        self.ensure_container_terminal(tab_id, user, cx);
        cx.notify();
    }

    /// 建立终端会话；`user` 为 `None` 表示镜像默认用户，`Some("root")` 表示提权进入。
    ///
    /// 建立 exec 是异步的：真机上连点页签会在结果回来前重复下发，一次会话变出一串
    /// shell。`terminal_request` 在发出时占位，结果（成功或失败）回来才清空。
    pub(crate) fn ensure_container_terminal(
        &mut self,
        tab_id: TabId,
        user: Option<String>,
        _cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        if workspace.terminal.is_some() || workspace.terminal_request.is_some() {
            return;
        }
        let Some(container) = workspace.selected_container_name() else {
            return;
        };
        let request_id = ContainerRequestId::new();
        workspace.terminal_request = Some((request_id, user.clone()));
        let mut options = ContainerConnection::new(container);
        options.user = user;
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::ExecTerminal {
                tab_id,
                request_id,
                options,
                size: TerminalSize::new(100, 30),
            },
        );
    }

    pub(super) fn create_container_terminal(
        &mut self,
        io: transom_core::TerminalIo,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<TerminalView>, Subscription) {
        let settings = app_settings(cx);
        let mut profile = settings.terminal_profile.clone();
        profile.background_transparency = settings.window.window_transparency;
        let interaction = settings.terminal_interaction;
        let reserved = self.state.shortcut_recording.filter.clone();
        let key_filter: KeyFilter = Arc::new(move |keystroke| {
            reserved
                .read()
                .iter()
                .any(|binding| keystroke.should_match(binding))
        });
        let terminal = cx.new(|cx| {
            TerminalView::new(
                window,
                cx,
                transom_terminal::TerminalViewParams {
                    io,
                    profile,
                    interaction,
                    key_filter: Some(key_filter),
                    audit_path: None,
                    shell_integration_enabled: false,
                    clear_on_first_resize: true,
                },
            )
        });
        let provider = self.state.history_provider.clone();
        terminal.update(cx, |view, _cx| view.attach_history(provider));
        let focus_handle = terminal.read(cx).focus_handle().clone();
        window.focus(&focus_handle, cx);
        let subscription = cx.subscribe_in(
            &terminal,
            window,
            move |this, _, event: &TerminalEvent, _window, cx| {
                if matches!(event, TerminalEvent::Exit) {
                    if let Some(workspace) = this.state.container_workspace_mut(tab_id) {
                        workspace.terminal = None;
                        workspace.notice =
                            Some(rust_i18n::t!("container.terminal.closed").to_string());
                    }
                    cx.notify();
                }
            },
        );
        (terminal, subscription)
    }
}
