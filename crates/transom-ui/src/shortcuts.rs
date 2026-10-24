//! 快捷键录制弹窗与运行时改键应用。

use crate::app::{AppShell, ShortcutCapture};
use crate::keymap;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
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

    pub(crate) fn render_shortcut_capture(
        &mut self,
        cx: &mut Context<Self>,
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

        let mut panel = v_flex()
            .id("shortcut-capture-panel")
            .w(px(440.0))
            .gap_3()
            .p_6()
            .rounded(p.radius.xl)
            .bg(p.popover)
            .text_color(p.popover_foreground)
            .border_1()
            .border_color(p.border)
            .shadow_2xl()
            .track_focus(&focus)
            .key_context(keymap::OVERLAY_CONTEXT)
            .on_key_down(cx.listener(Self::on_shortcut_capture_key))
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::BOLD)
                    .child(rust_i18n::t!("settings.shortcut_capture_title").to_string()),
            )
            .child(div().text_sm().text_color(p.muted_foreground).child(action))
            .child(
                div()
                    .text_sm()
                    .child(rust_i18n::t!("settings.shortcut_capture_hint").to_string()),
            );
        if let Some(error) = error {
            panel = panel.child(div().text_sm().text_color(p.danger).child(error));
        }
        panel = panel.child(
            h_flex().justify_end().child(
                Button::new("cancel-shortcut-capture")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.shortcut_recording.capture = None;
                        cx.notify();
                    })),
            ),
        );
        Self::overlay(panel, p)
    }
}
