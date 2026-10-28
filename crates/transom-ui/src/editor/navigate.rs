//! SFTP 编辑面板的"跳转到行"：面板内快捷键唤起输入模态，确认后展开折叠并
//! 移动光标到目标行行首。
//!
//! 面板状态在 `editor_panel.rs`，读取与保存分别在 `editor_open.rs` /
//! `editor_ops.rs`，渲染外壳在 `editor_view.rs`。
//!
//! 设计原理（Why）：
//! - 几百行的配置文件定位到第 N 行靠拖拽滚动不可接受；引擎已提供
//!   `set_cursor_position` 与 `unfold_at`，缺的只是行号输入出口。
//! - 模态在打开瞬间捕获目标编辑器实体，而不是确认时再找焦点面板：模态输入框
//!   会抢走焦点，届时"焦点所在面板"已经不再是用户想跳转的那个。
//! - 行号解析与越界收敛在确认时完成：非法输入就地报错重试，超界行号跳到
//!   最后一行（与常见编辑器行为一致），而不是拒绝服务。

use crate::app::{AppShell, EditorGotoLineModalState};
use crate::keymap;
use crate::palette::{Palette, sftp_text_input};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, Position, RopeExt};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

impl AppShell {
    /// 面板内"跳转到行"快捷键：打开行号输入模态，捕获当前焦点面板的编辑器。
    pub(crate) fn on_goto_editor_line(
        &mut self,
        _: &keymap::GoToEditorLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self
            .focused_editor_panel_index(window, cx)
            .and_then(|index| self.editors.panels.get(index))
            .map(|item| item.editor.clone())
        else {
            return;
        };
        let input = sftp_text_input(window, cx, "", "行号，如 120");
        input.update(cx, |state, cx| state.focus(window, cx));
        // 输入框内按回车直接确认：订阅句柄随模态存亡，关闭模态即退订。
        let enter_sub =
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.commit_editor_goto_line(window, cx);
                }
            });
        self.sftp_modals.goto_line = Some(EditorGotoLineModalState {
            editor,
            input,
            error: None,
            _enter_sub: enter_sub,
        });
        cx.notify();
    }

    /// 关闭跳转模态并把焦点还给编辑器。
    pub(crate) fn cancel_editor_goto_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.sftp_modals.goto_line.take().map(|modal| modal.editor);
        if let Some(editor) = editor {
            editor.update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    /// 确认跳转：解析行号，越界收敛到 [1, 总行数]，展开遮挡折叠后移动光标。
    pub(crate) fn commit_editor_goto_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.sftp_modals.goto_line.as_ref() else {
            return;
        };
        let editor = modal.editor.clone();
        let raw = modal.input.read(cx).value().trim().to_string();
        let Some(line) = raw.parse::<usize>().ok().filter(|line| *line >= 1) else {
            if let Some(modal) = self.sftp_modals.goto_line.as_mut() {
                modal.error = Some("请输入大于 0 的整数行号".to_string());
            }
            cx.notify();
            return;
        };
        // 收敛到有效区间：空文件（0 行）至少定位到第 1 行，超界跳到最后一行。
        let total = editor.read(cx).text().lines_len().max(1);
        let line = line.min(total);
        let position = Position::new((line - 1) as u32, 0);
        editor.update(cx, |state, cx| {
            state.unfold_at(position, cx);
            state.set_cursor_position(position, window, cx);
        });
        self.sftp_modals.goto_line = None;
        cx.notify();
    }

    /// 渲染"跳转到行"输入模态：单行输入 + 取消/跳转，错误就地显示。
    pub(crate) fn render_editor_goto_line_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.sftp_modals.goto_line.as_ref() else {
            return div().into_any_element();
        };
        let editor = modal.editor.clone();
        let input = modal.input.clone();
        let error = modal.error.clone();
        let total = editor.read(cx).text().lines_len();

        let content = v_flex()
            .gap_3()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(format!("目标行号（1 - {total}），超出范围将跳到最后一行")),
            )
            .child(Input::new(&input).w_full())
            .when_some(error, |this, err| {
                this.child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .text_xs()
                        .text_color(p.danger)
                        .child(Icon::new(IconName::CircleAlert).xsmall())
                        .child(err),
                )
            });

        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("editor-goto-cancel")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.cancel_editor_goto_line(window, cx);
                    })),
            )
            .child(
                Button::new("editor-goto-confirm")
                    .primary()
                    .icon(IconName::Check)
                    .label("跳转")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.commit_editor_goto_line(window, cx);
                    })),
            );

        let dialog = Self::dialog_frame(
            "editor-goto-line-modal",
            "跳转到行",
            None,
            content,
            Some(footer),
            px(420.0),
            p,
        );
        Self::overlay(dialog, p)
    }
}
