use super::*;
use crate::app::update_app_settings;
use crate::palette::localized_error;
use transom_core::SyncSettings;

impl SettingsView {
    /// 打开设置弹窗时回填云同步表单；秘密一律留空。
    pub(super) fn fill_sync_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        super::sync_form::fill_sync_form(&self.state, window, cx);
    }

    /// 从输入框汇总当前云同步配置（不落盘）。
    pub(super) fn collect_sync_settings(&self, cx: &App) -> SyncSettings {
        super::sync_form::collect_sync_settings(&self.state, cx)
    }

    /// 把非空的凭据输入写入钥匙串并清空输入框。
    ///
    /// 返回 `false` 表示校验或钥匙串写入失败，调用方应中止本次动作。
    pub(super) fn persist_sync_credentials(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match super::sync_form::persist_sync_credentials(
            &self.state,
            self.services.secret_store.as_ref(),
            window,
            cx,
        ) {
            Ok(()) => true,
            Err(error) => {
                cx.emit(SettingsEvent::Error(localized_error(&error)));
                false
            }
        }
    }

    /// 把当前全局代理配置回填到设置弹窗的输入框。
    ///
    /// 口令同样回填（掩码显示）：代理口令随配置持久化，用户需要能核对和修改；
    /// 日志与界面展示都不会输出明文。
    pub(super) fn fill_proxy_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (kind, host, port, username, password) = match &app_settings(cx).proxy {
            transom_core::OutboundProxy::Direct => (
                ProxyProtocol::Direct,
                String::new(),
                "1080".to_string(),
                String::new(),
                String::new(),
            ),
            transom_core::OutboundProxy::Socks5 { host, port, auth } => (
                ProxyProtocol::Socks5,
                host.clone(),
                port.to_string(),
                auth.as_ref()
                    .map_or_else(String::new, |a| a.username.clone()),
                auth.as_ref()
                    .map_or_else(String::new, |a| a.password.clone()),
            ),
            transom_core::OutboundProxy::Http { host, port, auth } => (
                ProxyProtocol::Http,
                host.clone(),
                port.to_string(),
                auth.as_ref()
                    .map_or_else(String::new, |a| a.username.clone()),
                auth.as_ref()
                    .map_or_else(String::new, |a| a.password.clone()),
            ),
        };
        self.state.proxy.kind = kind;
        let inputs = [
            (&self.state.proxy.host, host),
            (&self.state.proxy.port, port),
            (&self.state.proxy.username, username),
            (&self.state.proxy.password, password),
        ];
        for (input, value) in inputs {
            input.clone().update(cx, |state, cx| {
                state.set_value(value.clone(), window, cx);
            });
        }
    }

    /// 从设置弹窗的输入框组装全局代理配置。
    ///
    /// 非法配置保留原值并提示原因：静默退回直连会让用户以为代理已生效，
    /// 在受限网络中表现为“设置改了却还是连不上”。
    pub(super) fn commit_proxy_form(&mut self, cx: &mut Context<Self>) {
        let form = &self.state.proxy;
        let host = form.host.read(cx).value().trim().to_string();
        let port = form
            .port
            .read(cx)
            .value()
            .trim()
            .parse::<u16>()
            .unwrap_or(0);
        let username = form.username.read(cx).value().trim().to_string();
        let password = form.password.read(cx).value().to_string();
        let auth = (!username.is_empty() || !password.is_empty())
            .then_some(transom_core::ProxyAuth { username, password });
        let proxy = match form.kind {
            ProxyProtocol::Direct => transom_core::OutboundProxy::Direct,
            ProxyProtocol::Socks5 => transom_core::OutboundProxy::Socks5 { host, port, auth },
            ProxyProtocol::Http => transom_core::OutboundProxy::Http { host, port, auth },
        };
        if let Err(reason) = proxy.validate() {
            log::warn!("全局出站代理配置非法，保留原配置，原因: {reason}");
            cx.emit(SettingsEvent::Error(reason));
            return;
        }
        update_app_settings(cx, |s| s.proxy = proxy);
    }

    /// 将全局 Git 偏好设置回填到设置弹窗的输入框。
    pub(super) fn fill_git_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let git = &app_settings(cx).git;
        let inputs = [
            (&self.state.git_executable_path, git.executable_path.clone()),
            (
                &self.state.git_protected_branches,
                git.protected_branches.clone(),
            ),
            (&self.state.git_author_name, git.author_name.clone()),
            (&self.state.git_author_email, git.author_email.clone()),
        ];
        for (input, value) in inputs {
            input.clone().update(cx, |state, cx| {
                state.set_value(value, window, cx);
            });
        }
        self.state.git_test_result = None;
        self.state.git_testing = false;
    }
}
