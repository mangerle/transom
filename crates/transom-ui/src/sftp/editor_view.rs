//! SFTP 编辑面板的视图：外壳定位、标题栏、编辑器主体与行列状态栏。
//!
//! 面板状态与窗体动作在 `editor_panel.rs`，打开与读取在 `editor_open.rs`，
//! 回传在 `editor_ops.rs`，关闭保护在 `editor_close.rs`。
//!
//! 渲染入口之外全部是接收 `&EditorPanelState` 的自由函数：面板状态存在 `self` 里，
//! 若渲染方法按 `&mut self` 取面板再挂 `cx.listener`，同一帧内会对 `self`
//! 既读又写而被借用检查拒绝。图标一律取组件库资产，不用字符拟态。

use super::editor_panel::{
    EditorPanelState, EditorPhase, PANEL_KEEP_VISIBLE, PanelConfirm, clamp_axis,
};
use super::format_size;
use crate::app::AppShell;
use crate::keymap;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::{Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

impl AppShell {
    /// 渲染全部编辑面板；返回值按 Vec 顺序叠放，末尾即最上层。
    pub(crate) fn render_editor_panels(
        &self,
        window: &Window,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        self.editor_panels
            .iter()
            .map(|item| render_editor_panel(item, window, p, cx))
            .collect()
    }
}

/// 渲染单个悬浮面板外壳：绝对定位、只对自身遮挡命中。
fn render_editor_panel(
    item: &EditorPanelState,
    window: &Window,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let editor = item.editor.clone();
    let frame = v_flex()
        .id(ElementId::Name(panel_element_key(item).into()))
        .key_context(keymap::EDITOR_PANEL_CONTEXT)
        .on_action(cx.listener(AppShell::on_save_editor_panel))
        .absolute()
        .flex_col()
        .overflow_hidden()
        .rounded_lg()
        .border_1()
        .border_color(p.border)
        .bg(p.popover)
        .text_color(p.popover_foreground)
        .shadow_2xl()
        // 面板自身遮挡下层命中；面板以外不加遮罩，终端与文件列表仍可操作。
        .occlude()
        .on_mouse_down(MouseButton::Left, {
            let editor = editor.clone();
            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                this.raise_editor_panel(&editor, cx);
            })
        });

    let frame = if item.maximized {
        frame.inset_0()
    } else {
        let viewport = window.viewport_size();
        let (viewport_w, viewport_h): (f32, f32) = (viewport.width.into(), viewport.height.into());
        frame
            .left(px(clamp_axis(
                item.left,
                item.width,
                viewport_w,
                PANEL_KEEP_VISIBLE,
            )))
            .top(px(clamp_axis(
                item.top,
                item.current_height(),
                viewport_h,
                PANEL_KEEP_VISIBLE,
            )))
            .w(px(item.width))
            .h(px(item.current_height()))
    };

    let header = render_panel_header(item, &editor, p, cx);
    if item.collapsed {
        return frame.child(header).into_any_element();
    }
    frame
        .child(header)
        .child(render_panel_body(item, &editor, p, cx))
        .child(render_panel_status(item, &editor, p, cx))
        .into_any_element()
}

/// 标题栏：文件图标 + 名称 + 远端路径 + 折叠/最大化/关闭。
fn render_panel_header(
    item: &EditorPanelState,
    editor: &Entity<EditorState>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let fold_editor = editor.clone();
    let zoom_editor = editor.clone();
    let close_editor = editor.clone();
    let drag_editor = editor.clone();

    h_flex()
        .cursor_move()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .bg(p.muted)
        .border_b_1()
        .border_color(p.border)
        .on_mouse_down(MouseButton::Left, {
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.begin_editor_panel_drag(&drag_editor, event.position, cx);
            })
        })
        .child(
            Icon::new(if item.dirty {
                IconName::FilePen
            } else {
                IconName::FileText
            })
            .xsmall()
            .text_color(p.muted_foreground),
        )
        .child(
            div()
                .flex_none()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child(item.file_name.clone()),
        )
        .child(
            div()
                .text_xs()
                .truncate()
                .text_color(p.muted_foreground)
                .child(item.remote_path.clone()),
        )
        .child(div().flex_1())
        .child(
            Button::new(if item.collapsed {
                "editor-panel-expand"
            } else {
                "editor-panel-collapse"
            })
            .ghost()
            .compact()
            .icon(if item.collapsed {
                IconName::ChevronUp
            } else {
                IconName::ChevronDown
            })
            .accessibility_label(if item.collapsed {
                "展开面板"
            } else {
                "折叠面板"
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_editor_panel_collapsed(&fold_editor, cx);
            })),
        )
        .child(
            Button::new(if item.maximized {
                "editor-panel-restore"
            } else {
                "editor-panel-maximize"
            })
            .ghost()
            .compact()
            .icon(if item.maximized {
                IconName::Minimize2
            } else {
                IconName::Maximize2
            })
            .accessibility_label(if item.maximized {
                "还原面板"
            } else {
                "最大化面板"
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_editor_panel_maximized(&zoom_editor, cx);
            })),
        )
        .child(
            Button::new("editor-panel-close")
                .ghost()
                .compact()
                .icon(IconName::Close)
                .accessibility_label("关闭面板")
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.request_close_editor_panel(&close_editor, window, cx);
                })),
        )
        .into_any_element()
}

fn render_panel_body(
    item: &EditorPanelState,
    editor: &Entity<EditorState>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    // 未成功装载远端内容时不铺空编辑器：空缓冲区会被误认为"远端文件是空的"，
    // 用户可能就地打字，随后要么被回来的内容整体覆盖，要么反手把远端覆盖成这几个字。
    let content: AnyElement = if item.phase == EditorPhase::Reading || !item.loaded {
        if item.phase == EditorPhase::Reading {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .text_sm()
                .text_color(p.muted_foreground)
                .child(Icon::new(IconName::Loader).xsmall())
                .child(format!("正在读取 {}", item.remote_path))
                .into_any_element()
        } else {
            // 读取失败：占位之外给一条重试出口，瞬时网络抖动不必关面板重开。
            let retry_editor = editor.clone();
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .text_sm()
                .text_color(p.muted_foreground)
                .child(Icon::new(IconName::CircleAlert).xsmall())
                .child("远端内容未载入，无法编辑")
                .child(
                    Button::new("editor-retry-read")
                        .compact()
                        .icon(IconName::RefreshCw)
                        .label("重试读取")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.redrive_editor_read(&retry_editor, cx);
                        })),
                )
                .into_any_element()
        }
    } else {
        Editor::new(editor).w_full().flex_1().into_any_element()
    };
    let body = v_flex().flex_1().gap_1().p_2().child(content);
    let body = body.when_some(item.error.clone(), |this, err| {
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
    let body = body.when(item.conflict, |this| {
        this.child(render_conflict_banner(editor, p, cx))
    });
    let Some(kind) = item.confirm else {
        return body.into_any_element();
    };
    body.child(render_confirm_banner(kind, editor, p, cx))
        .into_any_element()
}

/// 冲突横幅：远端已被修改时提供重新载入、强制覆盖、继续编辑三个出口。
///
/// 重新载入会丢弃本地改动（按钮文案明示），强制覆盖跳过指纹比对，两者都是
/// 不可逆动作，因此各自放在明确的按钮上而不是合并成"确定"。
fn render_conflict_banner(
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
        .rounded_md()
        .bg(p.muted)
        .child(
            div()
                .flex_1()
                .text_xs()
                .text_color(p.danger)
                .child("远端文件在打开后已被修改，直接回传会覆盖这些改动"),
        )
        .child(
            Button::new("editor-conflict-reload")
                .ghost()
                .compact()
                .icon(IconName::RefreshCw)
                .label("放弃本地并重新载入")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.redrive_editor_read(&reload_editor, cx);
                })),
        )
        .child(
            Button::new("editor-conflict-overwrite")
                .danger()
                .compact()
                .label("强制覆盖远端")
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.overwrite_editor_panel_after_conflict(&force_editor, window, cx);
                })),
        )
        .child(
            Button::new("editor-conflict-dismiss")
                .ghost()
                .compact()
                .label("继续编辑")
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dismiss_editor_conflict(&dismiss_editor, window, cx);
                })),
        )
        .into_any_element()
}

/// 二次确认横幅：关闭丢弃与恢复远端共用容器，按操作种类渲染文案与出口。
fn render_confirm_banner(
    kind: PanelConfirm,
    editor: &Entity<EditorState>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let keep_editor = editor.clone();
    let confirm_editor = editor.clone();
    let (message, cancel_label, confirm_label, confirm_id) = match kind {
        PanelConfirm::DiscardClose => (
            "有未保存的修改，确定关闭并丢弃吗？",
            "留下继续编辑",
            "丢弃修改并关闭",
            "editor-discard-close",
        ),
        PanelConfirm::Revert => (
            "确定放弃全部未保存修改，恢复到最近回传的远端内容吗？该操作无法撤销。",
            "取消",
            "放弃修改",
            "editor-revert-confirm",
        ),
    };
    h_flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_md()
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

/// 状态栏：行列、语言、回传阶段与保存按钮。
fn render_panel_status(
    item: &EditorPanelState,
    editor: &Entity<EditorState>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    // 行列取自面板缓存：缓存随编辑器通知更新，此处再读一次反而拿到陈旧帧的值。
    let caret = if item.phase == EditorPhase::Reading {
        "行列待载入".to_string()
    } else {
        format!("行 {} · 列 {}", item.caret.0, item.caret.1)
    };
    // 读取与回传都属于"有活 in flight"，转圈图标与按钮禁用共用同一判定。
    let busy = matches!(item.phase, EditorPhase::Reading | EditorPhase::Writing);
    let phase_label = match item.phase {
        EditorPhase::Reading => "读取远端中…".to_string(),
        EditorPhase::Idle if item.dirty => "未保存".to_string(),
        EditorPhase::Idle => "无改动".to_string(),
        EditorPhase::Writing => "回传中…".to_string(),
        EditorPhase::Written => match item.written_size {
            // 带字节数：这是写完回读比对过的数字，用户据此判断"确实落盘了"。
            Some(bytes) => format!("已回传远端（{}）", format_size(bytes)),
            None => "已回传远端".to_string(),
        },
        EditorPhase::Failed => if item.conflict {
            "远端已修改"
        } else if item.dirty {
            "回传失败"
        } else {
            "读取失败"
        }
        .to_string(),
    };
    let save_editor = editor.clone();
    let revert_editor = editor.clone();

    h_flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_1()
        .border_t_1()
        .border_color(p.border)
        .text_xs()
        .text_color(p.muted_foreground)
        .child(div().flex_none().child(caret))
        .child(div().flex_none().child(item.language.clone()))
        .child(
            h_flex()
                .items_center()
                .gap_1()
                .flex_none()
                .when(busy, |this| {
                    this.child(Icon::new(IconName::Loader).xsmall())
                })
                .child(div().child(phase_label)),
        )
        .child(div().flex_1())
        // 冲突期间恢复到的是过期基线（远端已是新内容），横幅上的"重新载入"才是
        // 正确出口，因此此处不提供"放弃修改"以免误导。
        .when(item.dirty && !busy && !item.conflict, |this| {
            this.child(
                Button::new("editor-revert")
                    .ghost()
                    .compact()
                    .icon(IconName::RotateCcw)
                    .label("放弃修改")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ask_revert_editor_panel(&revert_editor, cx);
                    })),
            )
        })
        .child(
            Button::new("editor-save-upload")
                .primary()
                .compact()
                .icon(IconName::Check)
                .label("保存并回传")
                .disabled(!item.dirty || busy)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.save_editor_panel(&save_editor, window, cx);
                })),
        )
        .into_any_element()
}

/// 面板元素标识：标签 + 远端路径唯一确定一个编辑会话（同一文件不会开两个面板）。
///
/// 外层 `.id()` 会为后代元素划定 id 作用域，面板内部的按钮因此可以复用固定字面量 id。
fn panel_element_key(item: &EditorPanelState) -> String {
    format!("editor-panel-{}-{}", item.tab_id, item.remote_path)
}
