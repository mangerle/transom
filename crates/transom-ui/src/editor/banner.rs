//! SFTP 编辑面板的横幅区：错误提示、冲突三选一横幅与破坏性操作二次确认横幅。
//!
//! 面板状态在 `editor_panel.rs`，窗体动作在 `editor_actions.rs`，
//! 回传在 `editor_ops.rs`，渲染外壳在 `editor_view.rs`，状态栏在 `editor_status.rs`。

use super::panel::PanelConfirm;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::EditorState;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

/// 错误横幅：读取/回传失败的现场文本；权限类错误附一条 Sudo 提权出口。
///
/// 文案来自 net 层的中文诊断，原样展示不做二次包装，避免把具体的远端路径、
/// 字节数等现场信息吞掉。
pub(crate) fn render_error_banner(
    error: String,
    editor: &Entity<EditorState>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let sudo_editor = editor.clone();
    let is_permission = error.contains("Permission denied") || error.contains("权限");
    h_flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded(p.radius.md)
        .bg(p.muted)
        .child(
            Icon::new(IconName::CircleAlert)
                .xsmall()
                .text_color(p.danger),
        )
        .child(div().flex_1().text_xs().text_color(p.danger).child(error))
        .when(is_permission, |this| {
            this.child(
                Button::new("editor-sudo-banner-btn")
                    .warning()
                    .compact()
                    .icon(IconName::Shield)
                    .label(rust_i18n::t!("editor.sudo_save_action").to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.prompt_sudo_save(&sudo_editor, window, cx);
                    })),
            )
        })
        .into_any_element()
}

/// 冲突横幅：远端已被修改时提供重新载入、强制覆盖、继续编辑三个出口。
///
/// 重新载入会丢弃本地改动（按钮文案明示），强制覆盖跳过指纹比对，两者都是
/// 不可逆动作，因此各自放在明确的按钮上而不是合并成"确定"。
pub(crate) fn render_conflict_banner(
    editor: &Entity<EditorState>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let reload_editor = editor.clone();
    let force_editor = editor.clone();
    let dismiss_editor = editor.clone();
    h_flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded(p.radius.md)
        .bg(p.muted)
        .child(
            div()
                .flex_1()
                .text_xs()
                .text_color(p.danger)
                .child(rust_i18n::t!("editor.conflict_hint").to_string()),
        )
        .child(
            Button::new("editor-conflict-reload")
                .ghost()
                .compact()
                .icon(IconName::RefreshCw)
                .label(rust_i18n::t!("editor.conflict_reload").to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.redrive_editor_read(&reload_editor, cx);
                })),
        )
        .child(
            Button::new("editor-conflict-overwrite")
                .danger()
                .compact()
                .label(rust_i18n::t!("editor.conflict_overwrite").to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.overwrite_editor_panel_after_conflict(&force_editor, window, cx);
                })),
        )
        .child(
            Button::new("editor-conflict-dismiss")
                .ghost()
                .compact()
                .label(rust_i18n::t!("editor.conflict_continue").to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dismiss_editor_conflict(&dismiss_editor, window, cx);
                })),
        )
        .into_any_element()
}

/// 二次确认横幅：关闭丢弃与恢复远端共用容器，按操作种类渲染文案与出口。
pub(crate) fn render_confirm_banner(
    kind: PanelConfirm,
    editor: &Entity<EditorState>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let keep_editor = editor.clone();
    let save_close_editor = editor.clone();
    let confirm_editor = editor.clone();
    let (message, cancel_label, confirm_label, confirm_id) = match kind {
        PanelConfirm::DiscardClose => (
            rust_i18n::t!("editor.confirm_discard_close").to_string(),
            rust_i18n::t!("editor.confirm_keep_editing").to_string(),
            rust_i18n::t!("editor.confirm_discard").to_string(),
            "editor-discard-close",
        ),
        PanelConfirm::Revert => (
            rust_i18n::t!("editor.confirm_revert").to_string(),
            rust_i18n::t!("common.cancel").to_string(),
            rust_i18n::t!("editor.action_revert").to_string(),
            "editor-revert-confirm",
        ),
    };
    h_flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded(p.radius.md)
        .bg(p.muted)
        .child(div().flex_1().text_xs().text_color(p.danger).child(message))
        .child(
            Button::new("editor-keep-editing")
                .ghost()
                .compact()
                .label(cancel_label)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.keep_editor_panel(&keep_editor, window, cx);
                })),
        )
        // 关闭场景补"保存并关闭"：最常见的第三选择，省去"保存后再点关闭"两步。
        // 恢复场景没有对应动作，不渲染。
        .when(kind == PanelConfirm::DiscardClose, |this| {
            this.child(
                Button::new("editor-save-and-close")
                    .primary()
                    .compact()
                    .icon(IconName::Check)
                    .label(rust_i18n::t!("editor.save_and_close").to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.save_and_close_editor_panel(&save_close_editor, window, cx);
                    })),
            )
        })
        .child(
            Button::new(confirm_id)
                .danger()
                .compact()
                .label(confirm_label)
                .on_click(cx.listener(move |this, _, window, cx| match kind {
                    PanelConfirm::DiscardClose => {
                        this.discard_editor_panel(&confirm_editor, window, cx)
                    }
                    PanelConfirm::Revert => {
                        this.confirm_revert_editor_panel(&confirm_editor, window, cx)
                    }
                })),
        )
        .into_any_element()
}
