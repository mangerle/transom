//! SFTP 编辑面板的视图：外壳定位、标题栏、编辑器主体与行列状态栏。
//!
//! 面板状态与窗体动作在 `editor_panel.rs`，生命周期在 `editor_ops.rs`。
//!
//! 渲染入口之外全部是接收 `&EditorPanelState` 的自由函数：面板状态存在 `self` 里，
//! 若渲染方法按 `&mut self` 取面板再挂 `cx.listener`，同一帧内会对 `self`
//! 既读又写而被借用检查拒绝。图标一律取组件库资产，不用字符拟态。

use super::editor_panel::{EditorPanelState, EditorPhase, PANEL_KEEP_VISIBLE, clamp_axis};
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
        let (icon, label) = if item.phase == EditorPhase::Reading {
            (IconName::Loader, format!("正在读取 {}", item.remote_path))
        } else {
            (
                IconName::CircleAlert,
                "远端内容未载入，无法编辑；关闭面板后重新打开该文件".to_string(),
            )
        };
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_2()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(Icon::new(icon).xsmall())
            .child(label)
            .into_any_element()
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
    if !item.confirm_discard {
        return body.into_any_element();
    }
    let keep_editor = editor.clone();
    let discard_editor = editor.clone();
    body.child(
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
                    .child("有未保存的修改，确定关闭并丢弃吗？"),
            )
            .child(
                Button::new("editor-keep-editing")
                    .ghost()
                    .compact()
                    .label("留下继续编辑")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.keep_editor_panel(&keep_editor, window, cx);
                    })),
            )
            .child(
                Button::new("editor-discard-close")
                    .danger()
                    .compact()
                    .label("丢弃修改并关闭")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.discard_editor_panel(&discard_editor, window, cx);
                    })),
            ),
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
        EditorPhase::Failed => if item.dirty {
            "回传失败"
        } else {
            "读取失败"
        }
        .to_string(),
    };
    let save_editor = editor.clone();

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
