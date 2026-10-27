//! 偏好设置中心的模态弹窗状态回填、全局代理表单组装与设置持久化动作。

use super::read_font_family;
use crate::app::{AppShell, WindowAppearanceSnapshot};
use crate::palette::localized_error;
use gpui_kit::*;
use transom_terminal::TerminalView;

impl AppShell {
    /// 直接操作型设置（活动栏、状态栏、快捷命令执行模式）的即时落盘入口。
    ///
    /// 拖拽或显隐发生在设置弹窗之外，不能等待弹窗的保存按钮；这里只做保存与
    /// 错误上报，不重复解析字体等输入框状态，也不改变弹窗的取消语义。
    pub(crate) fn persist_layout_settings(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.services.settings_store.save(&self.settings) {
            log::error!("保存布局设置失败，界面仍是新值但下次启动会回退，原因: {error}");
            self.error = Some(localized_error(&error));
        }
        cx.notify();
    }

    pub(crate) fn open_settings_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let profile = self.settings.terminal_profile.clone();
        // 界面主题与密度在弹窗内即时预览，取消时凭这份快照恢复。
        self.modals.settings.appearance_snapshot = Some(self.settings.appearance.clone());
        self.modals.settings.font_family.update(cx, |input, cx| {
            input.set_value(profile.font_family.unwrap_or_default(), window, cx);
        });
        self.modals.settings.ui_font_family.update(cx, |input, cx| {
            input.set_value(
                self.settings
                    .appearance
                    .ui_font_family
                    .clone()
                    .unwrap_or_default(),
                window,
                cx,
            );
        });
        self.modals.settings.cjk_font.update(cx, |input, cx| {
            input.set_value(profile.cjk_font_family.unwrap_or_default(), window, cx);
        });
        let font_size = self.settings.terminal_profile.font_size.to_string();
        self.modals.settings.font_size.update(cx, |input, cx| {
            input.set_value(&font_size, window, cx);
        });
        let line_height = self.settings.terminal_profile.line_height.to_string();
        self.modals.settings.line_height.update(cx, |input, cx| {
            input.set_value(&line_height, window, cx);
        });
        // 旧配置里可能存着"不透明"档：窗口背景现在只有透明与毛玻璃两档，
        // 直接按透明处理，界面上也不会出现两个按钮都没选中的状态。
        if self.settings.window.window_background == transom_core::WindowBackgroundMode::Opaque {
            self.settings.window.window_background =
                transom_core::WindowBackgroundMode::Transparent;
        }
        // 滑块初值跟随当前配置：配置可能在本弹窗之外被改过。
        let transparency = f32::from(self.settings.window.window_transparency);
        self.modals
            .settings
            .window_transparency
            .update(cx, |state, cx| state.set_value(transparency, window, cx));
        // 取消时按这份快照回滚外观预览。
        self.modals.settings.window_appearance_snapshot = Some(WindowAppearanceSnapshot {
            transparency: self.settings.window.window_transparency,
            background: self.settings.window.window_background,
        });
        self.modals
            .settings
            .word_separators
            .update(cx, |input, cx| {
                input.set_value(
                    self.settings.terminal_profile.word_separators.clone(),
                    window,
                    cx,
                );
            });
        self.modals
            .settings
            .highlight_keywords
            .update(cx, |input, cx| {
                input.set_value(profile.highlight.keywords_joined(), window, cx);
            });
        let notify_threshold = self.settings.window.notify_threshold_secs.to_string();
        self.modals
            .settings
            .notify_threshold
            .update(cx, |input, cx| {
                input.set_value(notify_threshold, window, cx);
            });
        let scrollback = self.settings.terminal_profile.scrollback.to_string();
        self.modals.settings.scrollback.update(cx, |input, cx| {
            input.set_value(&scrollback, window, cx);
        });
        let key_bindings = transom_core::TerminalKeyBinding::to_multiline(
            &self.settings.terminal_profile.key_bindings,
        );
        self.modals
            .settings
            .terminal_key_bindings
            .update(cx, |state, cx| state.set_value(key_bindings, window, cx));
        let block_rows = self.settings.block_view.auto_collapse_rows.to_string();
        self.modals
            .settings
            .block_collapse_rows
            .update(cx, |input, cx| {
                input.set_value(block_rows, window, cx);
            });
        self.fill_sync_form(window, cx);
        self.fill_proxy_form(window, cx);
        self.modals.settings.open = true;
        cx.notify();
    }

    /// 把当前全局代理配置回填到设置弹窗的输入框。
    ///
    /// 口令同样回填（掩码显示）：代理口令随配置持久化，用户需要能核对和修改；
    /// 日志与界面展示都不会输出明文。
    fn fill_proxy_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (kind, host, port, username, password) = match &self.settings.proxy {
            transom_core::OutboundProxy::Direct => (
                crate::app::ProxyProtocol::Direct,
                String::new(),
                "1080".to_string(),
                String::new(),
                String::new(),
            ),
            transom_core::OutboundProxy::Socks5 { host, port, auth } => (
                crate::app::ProxyProtocol::Socks5,
                host.clone(),
                port.to_string(),
                auth.as_ref()
                    .map_or_else(String::new, |a| a.username.clone()),
                auth.as_ref()
                    .map_or_else(String::new, |a| a.password.clone()),
            ),
            transom_core::OutboundProxy::Http { host, port, auth } => (
                crate::app::ProxyProtocol::Http,
                host.clone(),
                port.to_string(),
                auth.as_ref()
                    .map_or_else(String::new, |a| a.username.clone()),
                auth.as_ref()
                    .map_or_else(String::new, |a| a.password.clone()),
            ),
        };
        self.modals.settings.proxy.kind = kind;
        let inputs = [
            (&self.modals.settings.proxy.host, host),
            (&self.modals.settings.proxy.port, port),
            (&self.modals.settings.proxy.username, username),
            (&self.modals.settings.proxy.password, password),
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
    /// 在受限网络中表现为"设置改了却还是连不上"。
    fn commit_proxy_form(&mut self, cx: &mut Context<Self>) {
        use crate::app::ProxyProtocol;
        let form = &self.modals.settings.proxy;
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
            self.error = Some(reason);
            return;
        }
        self.settings.proxy = proxy;
    }

    /// 取消设置：外观部分恢复到打开弹窗前的偏好并回滚预览效果。
    ///
    /// 为什么只有外观回滚：它引入了"未保存也生效"的即时预览，取消语义必须
    /// 成对闭环，否则点取消会留下改了一半的界面；其余设置项维持既有的内存态
    /// 修改保留、等待下次保存覆盖的行为。
    pub(crate) fn cancel_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(snapshot) = self.modals.settings.appearance_snapshot.take() {
            self.settings.appearance = snapshot;
            crate::theme::apply_appearance(&self.settings.appearance, cx);
        }
        // 滑块是即时预览：取消要把窗口外观改回打开弹窗时的值。
        if let Some(snapshot) = self.modals.settings.window_appearance_snapshot.take() {
            self.settings.window.window_transparency = snapshot.transparency;
            self.settings.window.window_background = snapshot.background;
            self.refresh_window_appearance(window, cx);
            self.apply_appearance_to_live_panes(cx);
        }
        self.modals.settings.open = false;
        cx.notify();
    }

    pub(crate) fn save_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 自定义按键映射先整体校验：一行写错就整份不落盘，并保持弹窗打开，
        // 避免用户以为已保存、实际运行期静默跳过某条映射。
        let raw_bindings = self
            .modals
            .settings
            .terminal_key_bindings
            .read(cx)
            .value()
            .to_string();
        let key_bindings = match crate::keymap::parse_terminal_key_bindings(&raw_bindings) {
            Ok(bindings) => bindings,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.settings.terminal_profile.key_bindings = key_bindings;
        // 界面字体族：留空表示跟随主题。
        self.settings.appearance.ui_font_family =
            read_font_family(&self.modals.settings.ui_font_family, cx);
        // 文件面板显示排除：原样保存（glob 语法与传输过滤一致，非法规则在
        // 匹配时被静默跳过，不阻断保存）。
        self.settings.sftp.panel_hidden_patterns = self
            .modals
            .settings
            .panel_hidden_patterns
            .read(cx)
            .value()
            .to_string();
        if let Ok(max) = self
            .modals
            .settings
            .max_concurrent_transfers
            .read(cx)
            .value()
            .trim()
            .parse::<usize>()
            && (1..=16).contains(&max)
        {
            self.settings.sftp.max_concurrent_transfers = max;
        }
        // 云同步：凭据先落钥匙串（非空才写），端点拓扑清洗后进设置文件。
        if self.persist_sync_credentials(window, cx) {
            self.settings.sync = self.collect_sync_settings(cx);
            self.settings.sync.normalize();
        }
        // 外观偏好在弹窗内已即时写入，落盘前归一化兜底值，保证文件可回读。
        self.settings.appearance.normalize();
        self.settings.terminal_profile.font_family =
            read_font_family(&self.modals.settings.font_family, cx);
        self.settings.terminal_profile.cjk_font_family =
            read_font_family(&self.modals.settings.cjk_font, cx);
        // 关键词列表先解析再清洗（去空、不分大小写去重、限长限条数），
        // 让"界面上看到的"与"存进文件的"是同一份。
        self.settings.terminal_profile.highlight.keywords = transom_core::TerminalHighlight {
            enabled: self.settings.terminal_profile.highlight.enabled,
            keywords: transom_core::TerminalHighlight::parse_keywords(
                self.modals
                    .settings
                    .highlight_keywords
                    .read(cx)
                    .value()
                    .as_ref(),
            ),
        }
        .sanitized()
        .keywords;
        if let Ok(font_size) = self
            .modals
            .settings
            .font_size
            .read(cx)
            .value()
            .trim()
            .parse::<f32>()
            && (8.0..=36.0).contains(&font_size)
        {
            self.settings.terminal_profile.font_size = font_size;
        }
        if let Ok(line_height) = self
            .modals
            .settings
            .line_height
            .read(cx)
            .value()
            .trim()
            .parse::<f32>()
            && (1.0..=2.5).contains(&line_height)
        {
            self.settings.terminal_profile.line_height = line_height;
        }
        // 空串是有值意义的（不按任何字符断词），因此这里不 trim、也不拒绝空值。
        self.settings.terminal_profile.word_separators = self
            .modals
            .settings
            .word_separators
            .read(cx)
            .value()
            .to_string();
        if let Some(threshold) = crate::notify::parse_notify_threshold(
            self.modals
                .settings
                .notify_threshold
                .read(cx)
                .value()
                .as_ref(),
        ) {
            self.settings.window.notify_threshold_secs = threshold;
        }
        if let Ok(scrollback) = self
            .modals
            .settings
            .scrollback
            .read(cx)
            .value()
            .trim()
            .parse::<usize>()
            && (100..=100_000).contains(&scrollback)
        {
            self.settings.terminal_profile.scrollback = scrollback;
        }
        self.sync_terminal_palette(cx);
        self.commit_proxy_form(cx);
        // 阈值越界回退存量；0 是"不自动折叠"的合法表达。
        if let Ok(rows) = self
            .modals
            .settings
            .block_collapse_rows
            .read(cx)
            .value()
            .trim()
            .parse::<usize>()
        {
            self.settings.block_view.auto_collapse_rows = rows;
        }
        // 窗口外观已随本次保存落盘，取消时的回滚快照作废。
        self.modals.settings.window_appearance_snapshot = None;
        if let Err(error) = self.services.settings_store.save(&self.settings) {
            log::error!("保存偏好设置失败，界面仍是新值但下次启动会回退，原因: {error}");
            self.error = Some(localized_error(&error));
        } else {
            log::info!(
                "偏好设置已保存，语言: {}，日志级别: {}",
                self.settings.language,
                self.settings.logs.level.as_str()
            );
            self.apply_appearance_to_live_panes(cx);
            self.apply_shortcut_settings(cx);
            self.apply_block_view_settings(cx);
            // 实时下发最新触发器规则给所有运行中的终端窗格
            let triggers = self.settings.triggers.clone();
            let terminals: Vec<Entity<TerminalView>> = self
                .workspace
                .tabs
                .iter()
                .filter_map(|tab| tab.terminal())
                .collect();
            for terminal in terminals {
                terminal.update(cx, |view, _| view.reload_triggers(&triggers));
            }
            self.status = Some(rust_i18n::t!("status.ready").to_string());
        }
        self.modals.settings.open = false;
        cx.notify();
    }
}
