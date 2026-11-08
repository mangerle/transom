//! 容器控制台：按用户身份打开 exec 终端并接入历史与快捷键过滤。

use super::*;

impl AppShell {
    /// 切到终端页签并按指定用户与 Shell 建立会话。
    ///
    /// 同一容器只保留一个终端：换了用户（root ↔ 镜像默认）或 Shell 就结束旧会话重建，
    /// 选的是同一个用户与 Shell 则只切页签，避免无谓地杀掉一次已经在用的 shell。
    pub(crate) fn open_container_terminal_as(
        &mut self,
        tab_id: TabId,
        user: Option<String>,
        shell: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return;
        };
        let workspace = &workspace.read(cx).state;
        let same_target = workspace
            .terminal
            .as_ref()
            .is_some_and(|terminal| terminal.user == user && terminal.shell == shell);
        if same_target {
            self.with_container_mut(tab_id, cx, |workspace| {
                workspace.detail_tab = ContainerDetailTab::Terminal;
            });
            cx.notify();
            return;
        }
        let pending = workspace
            .terminal_request
            .as_ref()
            .map(|(request_id, _, _)| *request_id);
        if let Some(request_id) = pending
            && let Some(workspace) = self.state.container_workspace(tab_id)
        {
            workspace.update(cx, |workspace, _| {
                workspace.cancel_container_request(request_id)
            });
        }
        self.with_container_mut(tab_id, cx, |workspace| {
            // 丢弃旧终端即可结束旧 exec（TerminalIo 的 RAII 清理），再建新会话。
            workspace.terminal = None;
            workspace.terminal_request = None;
            workspace.detail_tab = ContainerDetailTab::Terminal;
        });
        self.ensure_container_terminal(tab_id, user, shell, cx);
        cx.notify();
    }

    /// 建立终端会话；`user` 为 `None` 表示镜像默认用户，`shell` 为 `None` 表示默认 shell。
    pub(crate) fn ensure_container_terminal(
        &mut self,
        tab_id: TabId,
        user: Option<String>,
        shell: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(container) = self
            .with_container(tab_id, cx, |workspace| {
                if workspace.terminal.is_some() || workspace.terminal_request.is_some() {
                    return None;
                }
                workspace.selected_container_name()
            })
            .flatten()
        else {
            return;
        };
        let request_id = ContainerRequestId::new();
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.terminal_request = Some((request_id, user.clone(), shell.clone()));
        });
        let mut options = ContainerConnection::new(container);
        options.user = user;
        if let Some(s) = shell {
            options.command = Some(vec![s]);
        }
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(
                    request_id,
                    ContainerCommand::ExecTerminal {
                        tab_id,
                        request_id,
                        options,
                        size: TerminalSize::new(100, 30),
                    },
                );
            });
        }
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
                    clear_on_first_resize: false,
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
                    this.with_container_mut(tab_id, cx, |workspace| {
                        workspace.terminal = None;
                        workspace.notice =
                            Some(rust_i18n::t!("container.terminal.closed").to_string());
                    });
                    cx.notify();
                }
            },
        );
        (terminal, subscription)
    }
}
