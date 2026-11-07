use super::font_select::selected_font;
use super::*;
use crate::app::{app_settings_mut, update_app_settings};

impl SettingsView {
    /// 表单 → 内存设置的收集与校验；返回 false 表示校验失败、弹窗应保持打开。
    ///
    /// 拆成按领域的收集器后，`save_settings` 只负责“收集 → 落盘 → 应用效果”
    /// 三段流程；新增一个设置项时只需改对应收集器，不必在百行函数里找位置。
    pub(super) fn collect_settings_from_form(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.collect_key_bindings(cx) {
            return false;
        }
        self.collect_interface_preferences(cx);
        self.collect_sync_preferences(window, cx);
        // 外观偏好在弹窗内已即时写入，落盘前归一化兜底值，保证文件可回读。
        app_settings_mut(cx).appearance.normalize();
        self.collect_terminal_profile(cx);
        self.sync_terminal_palette(cx);
        self.commit_proxy_form(cx);
        self.collect_git_preferences(cx);
        true
    }

    /// 收集 Git 版本控制偏好设置。
    pub(super) fn collect_git_preferences(&mut self, cx: &mut Context<Self>) {
        let exe = self
            .state
            .git_executable_path
            .read(cx)
            .value()
            .trim()
            .to_string();
        let protected = self
            .state
            .git_protected_branches
            .read(cx)
            .value()
            .trim()
            .to_string();
        let author_name = self
            .state
            .git_author_name
            .read(cx)
            .value()
            .trim()
            .to_string();
        let author_email = self
            .state
            .git_author_email
            .read(cx)
            .value()
            .trim()
            .to_string();

        update_app_settings(cx, |s| {
            if !exe.is_empty() {
                s.git.executable_path = exe.clone();
            }
            s.git.protected_branches = protected;
            s.git.author_name = author_name.clone();
            s.git.author_email = author_email.clone();
        });
        transom_git::set_git_program(&exe);
        transom_git::set_git_author(&author_name, &author_email);
    }

    /// 自定义按键映射先整体校验：一行写错就整份不落盘，并保持弹窗打开，
    /// 避免用户以为已保存、实际运行期静默跳过某条映射。
    pub(super) fn collect_key_bindings(&mut self, cx: &mut Context<Self>) -> bool {
        let raw = self
            .state
            .terminal_key_bindings
            .read(cx)
            .value()
            .to_string();
        match crate::keymap::parse_terminal_key_bindings(&raw) {
            Ok(bindings) => {
                update_app_settings(cx, |s| s.terminal_profile.key_bindings = bindings);
                true
            }
            Err(error) => {
                cx.emit(SettingsEvent::Error(error));
                cx.notify();
                false
            }
        }
    }

    /// 界面外观、文件面板与块视图的轻量偏好。
    pub(super) fn collect_interface_preferences(&mut self, cx: &mut Context<Self>) {
        // 界面字体族：留空表示跟随主题。
        app_settings_mut(cx).appearance.ui_font_family =
            selected_font(&self.state.ui_font_family, cx);
        // 文件面板显示排除：原样保存（glob 语法与传输过滤一致，非法规则在
        // 匹配时被静默跳过，不阻断保存）。
        app_settings_mut(cx).sftp.panel_hidden_patterns = self
            .state
            .panel_hidden_patterns
            .read(cx)
            .value()
            .to_string();
        if let Ok(max) = self
            .state
            .max_concurrent_transfers
            .read(cx)
            .value()
            .trim()
            .parse::<usize>()
            && (1..=16).contains(&max)
        {
            update_app_settings(cx, |s| s.sftp.max_concurrent_transfers = max);
        }
        // 阈值越界回退存量；0 是“不自动折叠”的合法表达。
        if let Ok(rows) = self
            .state
            .block_collapse_rows
            .read(cx)
            .value()
            .trim()
            .parse::<usize>()
        {
            update_app_settings(cx, |s| s.block_view.auto_collapse_rows = rows);
        }
    }

    /// 云同步：凭据先落钥匙串（非空才写），端点拓扑清洗后进设置文件。
    pub(super) fn collect_sync_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.persist_sync_credentials(window, cx) {
            let sync = self.collect_sync_settings(cx);
            update_app_settings(cx, |s| s.sync = sync);
            app_settings_mut(cx).sync.normalize();
        }
    }

    /// 终端字体、尺寸、高亮、滚动与通知阈值。
    pub(super) fn collect_terminal_profile(&mut self, cx: &mut Context<Self>) {
        app_settings_mut(cx).terminal_profile.font_family =
            selected_font(&self.state.font_family, cx);
        app_settings_mut(cx).terminal_profile.cjk_font_family =
            selected_font(&self.state.cjk_font, cx);
        // 关键词列表先解析再清洗（去空、不分大小写去重、限长限条数），
        // 让“界面上看到的”与“存进文件的”是同一份。
        app_settings_mut(cx).terminal_profile.highlight.keywords =
            transom_core::TerminalHighlight {
                enabled: app_settings(cx).terminal_profile.highlight.enabled,
                keywords: transom_core::TerminalHighlight::parse_keywords(
                    self.state.highlight_keywords.read(cx).value().as_ref(),
                ),
            }
            .sanitized()
            .keywords;
        if let Ok(font_size) = self.state.font_size.read(cx).value().trim().parse::<f32>()
            && (8.0..=36.0).contains(&font_size)
        {
            update_app_settings(cx, |s| s.terminal_profile.font_size = font_size);
        }
        if let Ok(line_height) = self
            .state
            .line_height
            .read(cx)
            .value()
            .trim()
            .parse::<f32>()
            && (1.0..=2.5).contains(&line_height)
        {
            update_app_settings(cx, |s| s.terminal_profile.line_height = line_height);
        }
        // 空串是有值意义的（不按任何字符断词），因此这里不 trim、也不拒绝空值。
        app_settings_mut(cx).terminal_profile.word_separators =
            self.state.word_separators.read(cx).value().to_string();
        if let Some(threshold) = crate::notify::parse_notify_threshold(
            self.state.notify_threshold.read(cx).value().as_ref(),
        ) {
            update_app_settings(cx, |s| s.window.notify_threshold_secs = threshold);
        }
        if let Ok(scrollback) = self
            .state
            .scrollback
            .read(cx)
            .value()
            .trim()
            .parse::<usize>()
            && (100..=100_000).contains(&scrollback)
        {
            update_app_settings(cx, |s| s.terminal_profile.scrollback = scrollback);
        }
    }
}
