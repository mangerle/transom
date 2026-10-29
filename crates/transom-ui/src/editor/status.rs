//! SFTP 编辑面板的编辑器式状态栏与次要操作菜单。
//!
//! 设计原理（Why）：
//! - 状态栏的重心是"文件处于什么状态"：左侧光标位置，右侧语言、编码、行尾、
//!   权限与回传阶段。对比差异、与本地对比、放弃修改这类低频动作收进溢出菜单，
//!   避免排成一条对话框式按钮带。
//! - 唯一的常驻动作是保存（或只读时的 Sudo 提权保存），用图标按钮 + tooltip
//!   承载；Ctrl+S 已在键位表里，不依赖视觉上的大色块按钮。
//! - 编码与行尾直接取自读回时的 `RemoteTextMeta`：GBK 转码只读预览、CRLF
//!   回传保留形态，都是用户需要一眼确认的事实，放在状态栏比藏在提示里更可靠。

use super::diff::diff_entry_allowed;
use super::panel::{EditorPanelState, EditorPhase};
use crate::app::AppShell;
use crate::palette::Palette;
use crate::sftp::format_size;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{EditorState, RopeExt};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::format_octal_permissions;

/// 状态栏高度：单行小字，与编辑器画布之间只留一条分隔线。
const STATUS_BAR_HEIGHT: f32 = 26.0;

/// 渲染编辑器式状态栏：左侧位置，右侧文件属性、回传阶段与动作。
pub(crate) fn render_panel_status(
    item: &EditorPanelState,
    editor: &Entity<EditorState>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let total_lines = editor.read(cx).text().lines_len();
    let busy = matches!(item.phase, EditorPhase::Reading | EditorPhase::Writing);
    let caret = if item.phase == EditorPhase::Reading {
        rust_i18n::t!("editor.status_caret_pending").to_string()
    } else {
        rust_i18n::t!(
            "editor.status_caret",
            line = item.caret.0,
            column = item.caret.1,
            total = total_lines
        )
        .to_string()
    };
    let encoding = item
        .remote_meta
        .map(|meta| if meta.transcoded { "GBK" } else { "UTF-8" });
    let eol = item
        .remote_meta
        .map(|meta| if meta.crlf { "CRLF" } else { "LF" });
    let phase = render_phase(item, p);

    h_flex()
        .h(px(STATUS_BAR_HEIGHT))
        .items_center()
        .gap_2()
        .px_2()
        .bg(p.popover)
        .border_t_1()
        .border_color(p.border)
        .text_xs()
        .text_color(p.muted_foreground)
        .child(div().flex_none().child(caret))
        .child(div().flex_1())
        .when_some(encoding, |this, label| {
            this.child(div().flex_none().child(label))
        })
        .when_some(eol, |this, label| {
            this.child(div().flex_none().child(label))
        })
        .child(div().flex_none().child(item.language.clone()))
        // 权限八进制与只读标记来自打开时刻的目录项快照，可能过期，仅作提示。
        .when_some(item.permissions, |this, permissions| {
            this.child(
                div()
                    .flex_none()
                    .child(format_octal_permissions(permissions)),
            )
        })
        .when(!item.writable, |this| {
            let transcoded = item.remote_meta.is_some_and(|meta| meta.transcoded);
            let label = if transcoded {
                rust_i18n::t!("editor.status_read_only_preview").to_string()
            } else {
                rust_i18n::t!("editor.status_read_only").to_string()
            };
            this.child(div().flex_none().text_color(p.danger).child(label))
        })
        .child(phase)
        .when_some(render_panel_actions(item, editor, cx), |this, actions| {
            this.child(actions)
        })
        .child(render_primary_action(item, editor, busy, cx))
        .into_any_element()
}

/// 回传/读取阶段：忙碌时转圈，脏内容前加一个状态点，冲突时文字转危险色。
fn render_phase(item: &EditorPanelState, p: &Palette) -> AnyElement {
    let busy = matches!(item.phase, EditorPhase::Reading | EditorPhase::Writing);
    let label = match item.phase {
        EditorPhase::Reading => rust_i18n::t!("editor.phase_reading").to_string(),
        EditorPhase::Idle if item.dirty => rust_i18n::t!("editor.phase_unsaved").to_string(),
        EditorPhase::Idle => rust_i18n::t!("editor.phase_clean").to_string(),
        EditorPhase::Writing if item.pending_sudo_probe.is_some() => {
            rust_i18n::t!("editor.phase_sudo_probe").to_string()
        }
        EditorPhase::Writing => rust_i18n::t!("editor.phase_uploading").to_string(),
        EditorPhase::Written => match item.written_size {
            // 带字节数：这是写完回读比对过的数字，用户据此判断"确实落盘了"。
            Some(bytes) => {
                rust_i18n::t!("editor.phase_uploaded_size", size = format_size(bytes)).to_string()
            }
            None => rust_i18n::t!("editor.phase_uploaded").to_string(),
        },
        EditorPhase::Failed => if item.conflict {
            rust_i18n::t!("editor.phase_remote_changed")
        } else if item.dirty {
            rust_i18n::t!("editor.phase_upload_failed")
        } else {
            rust_i18n::t!("editor.phase_read_failed")
        }
        .to_string(),
    };
    h_flex()
        .items_center()
        .gap_1()
        .flex_none()
        .when(item.conflict, |this| this.text_color(p.danger))
        .when(busy, |this| {
            this.child(Spinner::new().xsmall().color(p.muted_foreground))
        })
        .when(item.dirty && !busy, |this| {
            this.child(
                div()
                    .size(px(6.0))
                    .rounded_full()
                    .flex_none()
                    .bg(if item.conflict { p.danger } else { p.warning }),
            )
        })
        .child(div().child(label))
        .into_any_element()
}

/// 溢出菜单：对比差异、与本地对比、放弃修改。全部不可用时整块不渲染。
fn render_panel_actions(
    item: &EditorPanelState,
    editor: &Entity<EditorState>,
    cx: &mut Context<AppShell>,
) -> Option<AnyElement> {
    let busy = matches!(item.phase, EditorPhase::Reading | EditorPhase::Writing);
    let can_diff = diff_entry_allowed(item);
    // 与本地对比取远端基线（磁盘上的两端），面板新建、读取中或冲突期间不提供。
    let can_local = item.loaded && !item.new_file && !busy && !item.showing_diff && !item.conflict;
    // 冲突期间"放弃修改"恢复到的是过期基线，正确出口是横幅上的重新载入。
    let can_revert = item.dirty && !busy && !item.conflict && !item.showing_diff;
    if !(can_diff || can_local || can_revert) {
        return None;
    }
    let app = cx.entity().clone();
    let menu_editor = editor.clone();
    let trigger = Button::new("editor-panel-actions")
        .ghost()
        .xsmall()
        .icon(IconName::Ellipsis)
        .tooltip(rust_i18n::t!("editor.actions_more").to_string())
        .dropdown_menu(move |menu, _window, _cx| {
            let mut menu = menu;
            if can_diff {
                let app = app.clone();
                let editor = menu_editor.clone();
                menu = menu.item(
                    PopupMenuItem::new(rust_i18n::t!("editor.action_compare").to_string())
                        .icon(IconName::FileDiff)
                        .on_click(move |_, _, cx| {
                            app.update(cx, |this, cx| this.show_editor_diff(&editor, cx));
                        }),
                );
            }
            if can_local {
                let app = app.clone();
                let editor = menu_editor.clone();
                menu = menu.item(
                    PopupMenuItem::new(rust_i18n::t!("editor.action_compare_local").to_string())
                        .icon(IconName::FileDiff)
                        .on_click(move |_, _, cx| {
                            app.update(cx, |this, cx| {
                                this.open_editor_local_diff(&editor, cx);
                            });
                        }),
                );
            }
            if can_revert {
                let app = app.clone();
                let editor = menu_editor.clone();
                menu = menu.item(
                    PopupMenuItem::new(rust_i18n::t!("editor.action_revert").to_string())
                        .icon(IconName::RotateCcw)
                        .on_click(move |_, _, cx| {
                            app.update(cx, |this, cx| this.ask_revert_editor_panel(&editor, cx));
                        }),
                );
            }
            menu
        });
    Some(trigger.into_any_element())
}

/// 常驻动作：可写时是保存，只读时是 Sudo 提权保存。
fn render_primary_action(
    item: &EditorPanelState,
    editor: &Entity<EditorState>,
    busy: bool,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let action_editor = editor.clone();
    if item.writable {
        return Button::new("editor-save-upload")
            .ghost()
            .xsmall()
            .icon(IconName::Check)
            .tooltip(rust_i18n::t!("editor.save_upload_tooltip").to_string())
            .disabled(!item.dirty || busy)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.save_editor_panel(&action_editor, window, cx);
            }))
            .into_any_element();
    }
    Button::new("editor-sudo-save")
        .warning()
        .xsmall()
        .icon(IconName::Shield)
        .tooltip(rust_i18n::t!("editor.sudo_save").to_string())
        .disabled(!item.dirty || busy)
        .on_click(cx.listener(move |this, _, window, cx| {
            this.prompt_sudo_save(&action_editor, window, cx);
        }))
        .into_any_element()
}
