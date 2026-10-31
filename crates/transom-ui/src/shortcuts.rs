//! 快捷键录制弹窗与运行时改键应用。

use crate::app::AppShell;
use crate::keymap;
use crate::keymap::ShortcutCapture;
use crate::palette::Palette;
use gpui_kit::base::v_flex;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::{
    DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
};
use gpui_kit::*;

impl AppShell {
    /// 打开按键录制弹窗并抢占键盘焦点。
    pub(crate) fn open_shortcut_capture(
        &mut self,
        id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.shortcut_recording.capture = Some(ShortcutCapture {
            id,
            error: None,
            focus,
        });
        cx.notify();
    }

    /// 将当前设置中的快捷键应用到运行中的键位表。
    ///
    /// GPUI 键位表只支持追加，因此先为上一次生效的键位追加 `NoAction`
    /// 抑制绑定，再追加本次完整键位；这样旧键位立即失效，且无需重建
    /// 输入框、命令面板等组件自带的默认绑定。
    pub(crate) fn apply_shortcut_settings(&mut self, cx: &mut Context<Self>) {
        let previous = std::mem::take(&mut self.shortcut_recording.applied_keys);
        let suppression = keymap::suppression_bindings(&previous);
        let bindings = keymap::bindings(&self.settings.shortcuts);
        let reserved = keymap::reserved_keystrokes(&self.settings.shortcuts);
        self.shortcut_recording.applied_keys =
            keymap::effective_key_strings(&self.settings.shortcuts);
        cx.bind_keys(suppression);
        cx.bind_keys(bindings);
        *self.shortcut_recording.filter.write() = reserved;
        cx.notify();
    }

    fn on_shortcut_capture_key(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(capture) = self.shortcut_recording.capture.as_ref() else {
            return;
        };
        let id = capture.id;
        let keystroke = &event.keystroke;
        if keystroke.key.eq_ignore_ascii_case("escape") {
            self.shortcut_recording.capture = None;
            cx.notify();
            return;
        }

        match keymap::format_captured(keystroke) {
            Ok(candidate) => {
                if let Some(conflict) =
                    keymap::conflict_with(id, &candidate, &self.settings.shortcuts)
                {
                    let name = rust_i18n::t!(conflict.label_key).to_string();
                    if let Some(capture) = self.shortcut_recording.capture.as_mut() {
                        capture.error = Some(
                            rust_i18n::t!("settings.shortcut_conflict", name = name).to_string(),
                        );
                    }
                } else {
                    self.settings
                        .shortcuts
                        .bindings
                        .insert(id.to_string(), vec![candidate]);
                    self.shortcut_recording.capture = None;
                    self.status = Some(rust_i18n::t!("settings.shortcut_recorded").to_string());
                }
            }
            Err(issue) => {
                let message = match issue {
                    keymap::CaptureIssue::ModifierOnly => {
                        rust_i18n::t!("settings.shortcut_modifier_only").to_string()
                    }
                    keymap::CaptureIssue::NeedsModifier => {
                        rust_i18n::t!("settings.shortcut_needs_modifier").to_string()
                    }
                };
                if let Some(capture) = self.shortcut_recording.capture.as_mut() {
                    capture.error = Some(message);
                }
            }
        }
        cx.notify();
    }

    /// 改键录入弹窗的弹窗体。
    ///
    /// 保留元素自己的 `on_key_down`：录键期间任意按键都要被它截下来当候选，
    /// Escape 清捕获是这条链的一部分；容器再挂一层取消只是同一条动作的兜底。
    pub(crate) fn render_shortcut_capture_body(
        &self,
        app: &Entity<AppShell>,
        _window: &mut Window,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(capture) = self.shortcut_recording.capture.as_ref() else {
            return div().into_any_element();
        };
        let focus = capture.focus.clone();
        let action = keymap::spec_by_id(capture.id)
            .map(|spec| rust_i18n::t!(spec.label_key).to_string())
            .unwrap_or_else(|| capture.id.to_string());
        let error = capture.error.clone();

        let header = DialogHeader::new()
            .child(
                DialogTitle::new()
                    .child(rust_i18n::t!("settings.shortcut_capture_title").to_string()),
            )
            .child(DialogDescription::new().child(action));

        let mut content = DialogContent::new().child(
            div()
                .text_sm()
                .child(rust_i18n::t!("settings.shortcut_capture_hint").to_string()),
        );
        if let Some(error) = error {
            content = content.child(div().text_sm().text_color(p.danger).child(error));
        }

        let footer = DialogFooter::new().child(
            Button::new("cancel-shortcut-capture")
                .ghost()
                .label(rust_i18n::t!("common.cancel").to_string())
                .on_click(crate::dialog_host::on_event(
                    app,
                    |this, _: &ClickEvent, _, cx| {
                        this.shortcut_recording.capture = None;
                        cx.notify();
                    },
                )),
        );

        let panel = v_flex()
            .id("shortcut-capture-panel")
            .w_full()
            .gap_3()
            .track_focus(&focus)
            .key_context(keymap::OVERLAY_CONTEXT)
            .on_key_down(crate::dialog_host::on_event(
                app,
                Self::on_shortcut_capture_key,
            ))
            .child(header)
            .child(content)
            .child(footer);
        panel.into_any_element()
    }
}
