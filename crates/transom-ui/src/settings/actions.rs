//! 设置实体的打开回填、表单收集、保存/取消与外观下发。
//!
//! 设计原理（Why）：
//! - 设置值的所有权在实体：输入框、草稿快照、保存流程与页面渲染都在
//!   [`SettingsView`] 内完成，外壳只负责打开弹窗与消费跨域事件；
//! - 保存是"收集 → 校验 → 落盘 → 通知"四段：任何一段失败都保持弹窗打开，
//!   不出现"界面已是新值、磁盘还是旧值"的静默分裂；
//! - 外观预览与取消回滚共用同一条 `theme::apply_appearance` 写路径，
//!   保证"看到的效果"与"保存后的效果"出自同一逻辑。
//!
//! 代价与局限：保存成功后的活窗格热更新需要窗口级遍历，交给外壳的
//! [`SettingsEvent::Saved`] 处理；实体自身不持有工作区。

use super::SettingsEvent;
use super::SettingsView;
use super::WindowAppearanceSnapshot;
use super::font_select::{CJK_DEFAULT_KEY, MONO_DEFAULT_KEY, UI_DEFAULT_KEY, reset_font_select};
use crate::app::{app_settings, app_settings_mut, update_app_settings};
use crate::cloud_sync::DEFAULT_SYNC_DIRECTORY;
use crate::palette::localized_error;
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::slider::{SliderEvent, SliderValue};
use gpui_kit::*;
use transom_core::{GitSyncSettings, SyncEndpoint};

impl SettingsView {
    /// 订阅弹窗内部的滑块与步进器：外观即时预览属于弹窗自身的行为。
    ///
    /// 以前这两条订阅挂在外壳上，外壳因此必须知道弹窗内部控件的句柄；
    /// 订阅随实体创建/析构后，外壳只保留弹窗句柄，职责边界更清晰。
    pub(crate) fn install_subscriptions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let transparency = self.state.window_transparency.clone();
        let transparency_sub = cx.subscribe_in(
            &transparency,
            window,
            |this, _, event: &SliderEvent, window, cx| match event {
                SliderEvent::Change(SliderValue::Single(value)) => {
                    this.set_window_transparency(value.round() as u8, window, cx);
                }
                SliderEvent::Release(SliderValue::Single(value)) => {
                    this.set_window_transparency(value.round() as u8, window, cx);
                    this.push_window_appearance(cx);
                }
                _ => {}
            },
        );
        let divider_width = self.state.divider_width.clone();
        let divider_sub = cx.subscribe_in(
            &divider_width,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.apply_divider_width_from_input(cx);
                }
            },
        );
        self._subscriptions = vec![transparency_sub, divider_sub];
    }

    /// 注入监控面板当前的采样档位，供"采样频率"下拉显示选中项。
    pub(crate) fn set_monitor_interval(&mut self, mode: crate::monitor::SampleIntervalMode) {
        self.state.monitor_interval = mode;
    }

    /// 打开设置弹窗：回填全部输入框、刷新目录快照，并按需定位到指定页。
    ///
    /// 字体清单可能在本进程之外变化（系统装了新字体），因此每次打开都重建
    /// 选项再按配置值回填，避免下拉里看不到已存的族名。`page` 越界时退化为
    /// 普通打开，过期页号不会弹出空内容区。
    pub(crate) fn prepare_open(
        &mut self,
        window: &mut Window,
        page: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.state.jump_page = page.filter(|page| *page < super::page_entries().len());
        // 界面主题与密度在弹窗内即时预览，取消时凭这份快照恢复。
        self.state.appearance_snapshot = Some(app_settings(cx).appearance.clone());

        self.fill_appearance_inputs(window, cx);
        let profile = app_settings(cx).terminal_profile.clone();
        // 旧配置里可能存着“不透明”档：窗口背景现在只有透明与毛玻璃两档，
        // 直接按透明处理，界面上也不会出现两个按钮都没选中的状态。
        if app_settings(cx).window.window_background == transom_core::WindowBackgroundMode::Opaque {
            app_settings_mut(cx).window.window_background =
                transom_core::WindowBackgroundMode::Transparent;
        }
        // 滑块初值跟随当前配置：配置可能在本弹窗之外被改过。
        let transparency = f32::from(app_settings(cx).window.window_transparency);
        self.state.window_transparency.update(cx, |state, cx| {
            state.set_value(transparency, window, cx);
        });
        self.state.window_appearance_snapshot = Some(WindowAppearanceSnapshot {
            transparency: app_settings(cx).window.window_transparency,
            background: app_settings(cx).window.window_background,
        });
        self.state.word_separators.update(cx, |input, cx| {
            input.set_value(profile.word_separators.clone(), window, cx);
        });
        self.state.highlight_keywords.update(cx, |input, cx| {
            input.set_value(profile.highlight.keywords_joined(), window, cx);
        });
        let notify_threshold = app_settings(cx).window.notify_threshold_secs.to_string();
        self.state.notify_threshold.update(cx, |input, cx| {
            input.set_value(notify_threshold, window, cx);
        });
        let scrollback = app_settings(cx).terminal_profile.scrollback.to_string();
        self.state.scrollback.update(cx, |input, cx| {
            input.set_value(&scrollback, window, cx);
        });
        let key_bindings = transom_core::TerminalKeyBinding::to_multiline(&profile.key_bindings);
        self.state.terminal_key_bindings.update(cx, |state, cx| {
            state.set_value(key_bindings, window, cx);
        });
        let block_rows = app_settings(cx).block_view.auto_collapse_rows.to_string();
        self.state.block_collapse_rows.update(cx, |input, cx| {
            input.set_value(block_rows, window, cx);
        });
        self.fill_sync_form(window, cx);
        self.fill_proxy_form(window, cx);
        self.state.open = true;
        cx.notify();
    }

    /// 打开弹窗时回填外观输入：字体、字号/行高与分割线档位。
    fn fill_appearance_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let profile = app_settings(cx).terminal_profile.clone();
        let appearance = app_settings(cx).appearance.clone();
        let mono_default = rust_i18n::t!(MONO_DEFAULT_KEY).to_string();
        reset_font_select(
            &self.state.font_family,
            &mono_default,
            profile.font_family.as_deref(),
            window,
            cx,
        );
        let ui_default = rust_i18n::t!(UI_DEFAULT_KEY).to_string();
        reset_font_select(
            &self.state.ui_font_family,
            &ui_default,
            appearance.ui_font_family.as_deref(),
            window,
            cx,
        );
        let cjk_default = rust_i18n::t!(CJK_DEFAULT_KEY).to_string();
        reset_font_select(
            &self.state.cjk_font,
            &cjk_default,
            profile.cjk_font_family.as_deref(),
            window,
            cx,
        );
        let font_size = profile.font_size.to_string();
        self.state.font_size.update(cx, |input, cx| {
            input.set_value(&font_size, window, cx);
        });
        let line_height = profile.line_height.to_string();
        self.state.line_height.update(cx, |input, cx| {
            input.set_value(&line_height, window, cx);
        });
        // 分割线粗细步进器回填当前档位（1/2/3 像素）；配置可能在弹窗之外被改过。
        let divider_width = app_settings(cx)
            .appearance
            .divider
            .width
            .pixels()
            .to_string();
        self.state.divider_width.update(cx, |input, cx| {
            input.set_value(divider_width, window, cx);
        });
    }

    /// 直接操作型设置（活动栏、状态栏、快捷命令执行模式）的即时落盘入口。
    ///
    /// 拖拽或显隐发生在设置弹窗之外，不能等待弹窗的保存按钮；这里只做保存与
    /// 错误上报，不重复解析字体等输入框状态，也不改变弹窗的取消语义。
    pub(crate) fn persist_layout_settings(&mut self, cx: &mut Context<Self>) {
        let settings_snapshot = app_settings(cx).clone();
        self.persist_settings_reported(&settings_snapshot, cx);
        cx.notify();
    }

    /// 保存设置：收集表单 → 落盘 → 通知外壳热应用 → 关闭弹窗。
    pub(crate) fn save_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.collect_settings_from_form(window, cx) {
            return;
        }
        // 窗口外观已随本次保存落盘，取消时的回滚快照作废。
        self.state.window_appearance_snapshot = None;
        let settings_snapshot = app_settings(cx).clone();
        if !self.persist_settings_reported(&settings_snapshot, cx) {
            // 保存失败时保持弹窗打开，让用户看到错误并重试，而不是误以为已保存。
            cx.notify();
            return;
        }
        log::info!(
            "偏好设置已保存，语言: {}，日志级别: {}",
            app_settings(cx).language,
            app_settings(cx).logs.level.as_str()
        );
        cx.emit(SettingsEvent::Saved(Box::new(settings_snapshot)));
        cx.emit(SettingsEvent::Status(
            rust_i18n::t!("status.ready").to_string(),
        ));
        self.state.open = false;
        self.state.jump_page = None;
        cx.notify();
    }

    /// 取消设置：外观部分恢复到打开弹窗前的偏好并回滚预览效果。
    ///
    /// 为什么只有外观回滚：它引入了“未保存也生效”的即时预览，取消语义必须
    /// 成对闭环，否则点取消会留下改了一半的界面；其余设置项维持既有的内存态
    /// 修改保留、等待下次保存覆盖的行为。
    pub(crate) fn cancel_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(snapshot) = self.state.appearance_snapshot.take() {
            update_app_settings(cx, |s| s.appearance = snapshot);
            let appearance = app_settings(cx).appearance.clone();
            crate::theme::apply_appearance(&appearance, cx);
        }
        // 滑块是即时预览：取消要把窗口外观改回打开弹窗时的值。
        if let Some(snapshot) = self.state.window_appearance_snapshot.take() {
            update_app_settings(cx, |s| s.window.window_transparency = snapshot.transparency);
            update_app_settings(cx, |s| s.window.window_background = snapshot.background);
            crate::window_appearance::refresh_window(window, cx);
        }
        cx.emit(SettingsEvent::Cancelled);
        self.state.open = false;
        self.state.jump_page = None;
        cx.notify();
    }

    /// 统一的“保存并上报”入口：失败时记录日志并把错误交给通知层。
    ///
    /// 高频开关不应各自决定“失败要不要提示”；统一走这里，避免再次出现
    /// `let _ = save(...)` 这类静默降级。
    pub(crate) fn persist_settings_reported(
        &mut self,
        settings: &transom_core::AppSettings,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.services.persistence.save_settings(settings) {
            Ok(()) => true,
            Err(error) => {
                log::error!("保存设置失败，界面仍是新值但下次启动会回退，原因: {error}");
                cx.emit(SettingsEvent::Error(localized_error(&error)));
                false
            }
        }
    }

    /// 更新界面外观偏好并即时应用：弹窗内改一档立刻全局预览。
    ///
    /// 落盘仍由 `save_settings` 单点负责；取消走 `cancel_settings` 凭快照回滚。
    /// 预览与回滚共用 [`crate::theme::apply_appearance`] 这一条写路径，保证
    /// “看到的效果”与“保存后的效果”出自同一逻辑。
    pub(crate) fn update_appearance(
        &mut self,
        change: impl FnOnce(&mut transom_core::AppearanceSettings),
        cx: &mut Context<Self>,
    ) {
        super::preview_appearance(change, cx);
        self.sync_terminal_palette(cx);
        cx.notify();
    }

    /// 终端配色跟随界面主题：`theme_name` 为哨兵值时，把当前生效 UI 主题的
    /// 伴生终端调色板写入终端 profile 并通知外壳热应用到活窗格。显式选了预设的
    /// 用户不受影响。设置页外观变化、保存落盘、系统明暗切换三个触发点共用。
    pub(crate) fn sync_terminal_palette(&mut self, cx: &mut Context<Self>) {
        if super::sync_terminal_palette(cx) {
            cx.emit(SettingsEvent::ProfilePreview);
        }
    }

    /// 切换云同步端点类型，并把对应拓扑回填到输入框。
    ///
    /// 只改内存态，保存时统一落盘；切换端点会清空另一种方式的残留拓扑，
    /// 避免“界面显示 WebDAV、文件里还留着 Git 远端”的双配状态。
    pub(crate) fn switch_sync_endpoint(
        &mut self,
        kind: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if kind == super::sync_page::sync_endpoint_kind(app_settings(cx)) {
            return;
        }
        match kind {
            "off" => {
                update_app_settings(cx, |s| s.sync.endpoint = None);
                update_app_settings(cx, |s| s.sync.git = None);
                self.state.sync_form.git_remote.update(cx, |input, cx| {
                    input.set_value(String::new(), window, cx);
                });
            }
            "webdav" => {
                update_app_settings(cx, |s| s.sync.git = None);
                self.state.sync_form.git_remote.update(cx, |input, cx| {
                    input.set_value(String::new(), window, cx);
                });
                if !matches!(
                    app_settings(cx).sync.endpoint,
                    Some(SyncEndpoint::Webdav { .. })
                ) {
                    app_settings_mut(cx).sync.endpoint = Some(SyncEndpoint::Webdav {
                        url: String::new(),
                        username: String::new(),
                        directory: DEFAULT_SYNC_DIRECTORY.to_string(),
                    });
                }
            }
            "s3" => {
                update_app_settings(cx, |s| s.sync.git = None);
                self.state.sync_form.git_remote.update(cx, |input, cx| {
                    input.set_value(String::new(), window, cx);
                });
                if !matches!(
                    app_settings(cx).sync.endpoint,
                    Some(SyncEndpoint::S3 { .. })
                ) {
                    app_settings_mut(cx).sync.endpoint = Some(SyncEndpoint::S3 {
                        endpoint: String::new(),
                        region: String::new(),
                        bucket: String::new(),
                        access_key: String::new(),
                        directory: DEFAULT_SYNC_DIRECTORY.to_string(),
                    });
                }
            }
            "git" => {
                update_app_settings(cx, |s| s.sync.endpoint = None);
                if app_settings(cx).sync.git.is_none() {
                    app_settings_mut(cx).sync.git = Some(GitSyncSettings {
                        remote: String::new(),
                        branch: "transom-sync".to_string(),
                        interval_minutes: 0,
                    });
                }
            }
            _ => return,
        }
        self.fill_sync_form(window, cx);
        cx.notify();
    }

    /// 切换界面语言：写全局、换 locale 并立即落盘。
    ///
    /// 语言是少数不需要“保存”按钮的设置：切换后界面文案立刻变化，若等保存
    /// 才落盘，用户关掉弹窗就会看到半中半英的界面。
    pub(crate) fn set_language(&mut self, language: String, cx: &mut Context<Self>) {
        if app_settings(cx).language == language {
            return;
        }
        update_app_settings(cx, |s| s.language = language);
        gpui_kit::component::set_locale(&crate::i18n::resolve_language(&app_settings(cx).language));
        let settings_snapshot = app_settings(cx).clone();
        self.persist_settings_reported(&settings_snapshot, cx);
        cx.notify();
    }

    /// 恢复活动栏默认顺序与显隐，并立即落盘。
    pub(crate) fn restore_activity_bar(&mut self, cx: &mut Context<Self>) {
        if app_settings_mut(cx).activity_bar.restore_defaults() {
            self.persist_layout_settings(cx);
        } else {
            cx.notify();
        }
    }

    /// 恢复状态栏默认顺序与显隐，并立即落盘。
    pub(crate) fn restore_status_bar(&mut self, cx: &mut Context<Self>) {
        if app_settings_mut(cx).status_bar.restore_defaults() {
            self.persist_layout_settings(cx);
        } else {
            cx.notify();
        }
    }
}
