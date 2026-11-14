//! SFTP 编辑面板的视图：外壳定位、标题栏、编辑器主体与状态栏。
//!
//! 面板状态与窗体动作在 `panel.rs` / `actions.rs`，打开与读取在 `open.rs`，
//! 回传在 `ops.rs`，关闭保护在 `close.rs`，状态栏在 `status.rs`。
//!
//! 设计原理（Why）：
//! - 面板正文是"编辑器画布"而不是"嵌在弹窗里的输入框"：编辑器的输入框外框、
//!   圆角与内边距全部关掉，正文区不留内边距，面板底色直接取主题的
//!   `editor.background`。标题栏与状态栏用 `popover` 做外框，只比画布亮/暗一档，
//!   形成"编辑器 + 一圈外框"的层次。
//! - 横幅（错误、冲突、二次确认）是画布之外的附加信息，单独包一层内边距，
//!   不让它们把编辑器挤成带边距的盒子。
//! - 渲染归 `EditorPanels` 实体：按钮回调直接改实体状态，发命令/开弹窗等跨域
//!   动作走面板事件。图标一律取组件库资产，不用字符拟态。

use super::drag_tracker::EditorDragTracker;
use super::entity::EditorPanels;
use super::open::parent_dir;
use super::panel::{
    EditorPanelState, EditorPhase, PANEL_KEEP_VISIBLE, clamp_axis,
};
use crate::theme::tokens::{control, size};
use crate::divider::{DividerEdge, DividerStyled};
use crate::keymap;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

impl Render for EditorPanels {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        let font_size = self.editor_font_size(cx);
        let layers = self.render_panels(window, &p, font_size, cx);
        // 绝对定位容器不参与主布局，也不遮挡下层命中：面板自身负责 occlude。
        // 拖拽追踪器随实体渲染挂载，窗口级监听在 paint 阶段注册。
        div()
            .absolute()
            .inset_0()
            .children(layers)
            .child(EditorDragTracker::new(cx.entity().downgrade()))
    }
}

impl EditorPanels {
    /// 渲染全部编辑面板；返回值按 Vec 顺序叠放，末尾即最上层。
    fn render_panels(
        &self,
        window: &Window,
        p: &Palette,
        font_size: f32,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        self.manager()
            .panels
            .iter()
            .map(|item| self.render_editor_panel(item, window, p, font_size, cx))
            .collect()
    }

    /// 渲染单个悬浮面板外壳：绝对定位、只对自身遮挡命中。
    fn render_editor_panel(
        &self,
        item: &EditorPanelState,
        window: &Window,
        p: &Palette,
        font_size: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editor = item.editor.clone();
        let frame = v_flex()
            .id(ElementId::Name(panel_element_key(item).into()))
            .key_context(keymap::EDITOR_PANEL_CONTEXT)
            .on_action(cx.listener(EditorPanels::on_save_editor_panel))
            .on_action(cx.listener(EditorPanels::on_goto_editor_line))
            .absolute()
            .flex_col()
            .overflow_hidden()
            .rounded(p.radius.none)
            .border_1()
            .border_color(p.border)
            .bg(p.editor_background)
            .text_color(p.foreground)
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
            let (viewport_w, viewport_h): (f32, f32) =
                (viewport.width.into(), viewport.height.into());
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

        let header = self.render_panel_header(item, &editor, p, cx);
        if item.collapsed {
            return frame.child(header).into_any_element();
        }
        frame
            .child(header)
            .child(self.render_panel_body(item, &editor, p, font_size, cx))
            .child(self.render_panel_status(item, &editor, p, cx))
            .into_any_element()
    }

    /// 标题栏：文件名 + 父目录面包屑 + 折叠/最大化/关闭。
    ///
    /// 高度与折叠态一致，动作按钮用 xsmall 幽灵图标；文件名不再加粗，改走编辑器
    /// 标签的字号与字重，父目录降一档灰，视觉重心落在文件本身。
    fn render_panel_header(
        &self,
        item: &EditorPanelState,
        editor: &Entity<EditorState>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fold_editor = editor.clone();
        let zoom_editor = editor.clone();
        let close_editor = editor.clone();
        let drag_editor = editor.clone();
        let directory = parent_dir(&item.remote_path).unwrap_or("");

        h_flex()
            .h(rems(size::TAB_BAR_HEIGHT))
            // 标题栏可拖拽，但保持系统默认箭头：组件库的 cursor_move 实际映射为
            // 闭合手形，悬停时会让人误以为在抓取文件内容而不是移动面板。
            .cursor_default()
            .items_center()
            .gap_2()
            .px_2()
            .bg(p.popover)
            .border_color(p.divider(p.border))
            .divider_edge(DividerEdge::Bottom, p.divider_width)
            .on_mouse_down(MouseButton::Left, {
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.begin_editor_panel_drag(&drag_editor, event.position, cx);
                })
            })
            .child(
                AppIconElement::new(AppIcon::for_file(&item.remote_path))
                    .with_rems(rems(control::ICON)),
            )
            .child(
                div()
                    .flex_none()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(item.file_name.clone()),
            )
            .when(!directory.is_empty(), |this| {
                this.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(directory.to_string()),
                )
            })
            .when(item.dirty, |this| {
                this.child(
                    AppIconElement::new(AppIcon::Ui(UiIcon::SHOW))
                        .with_size(px(6.0))
                        .text_color(p.primary),
                )
            })
            .child(div().flex_1())
            .child(self.render_editor_panel_actions(
                item,
                fold_editor,
                zoom_editor,
                close_editor,
                cx,
            ))
            .into_any_element()
    }

    /// 面板头动作：折叠/展开、最大化/还原与关闭。
    fn render_editor_panel_actions(
        &self,
        item: &EditorPanelState,
        fold_editor: Entity<EditorState>,
        zoom_editor: Entity<EditorState>,
        close_editor: Entity<EditorState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fold_icon = if item.collapsed {
            AppIcon::Ui(UiIcon::CHEVRON_UP)
        } else {
            AppIcon::Ui(UiIcon::CHEVRON_DOWN)
        };
        let fold_label = if item.collapsed {
            rust_i18n::t!("editor.expand_panel").to_string()
        } else {
            rust_i18n::t!("editor.collapse_panel").to_string()
        };
        let zoom_icon = if item.maximized {
            AppIcon::Ui(UiIcon::RESTORE)
        } else {
            AppIcon::Ui(UiIcon::MAXIMIZE)
        };
        let zoom_label = if item.maximized {
            rust_i18n::t!("editor.restore_panel").to_string()
        } else {
            rust_i18n::t!("editor.maximize_panel").to_string()
        };
        h_flex()
            .gap_1()
            .items_center()
            .child(
                crate::widgets::IconButton::new(
                    if item.collapsed {
                        "editor-panel-expand"
                    } else {
                        "editor-panel-collapse"
                    },
                    fold_icon,
                    fold_label.clone(),
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
                .accessibility_label(fold_label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_editor_panel_collapsed(&fold_editor, cx);
                })),
            )
            .child(
                crate::widgets::IconButton::new(
                    if item.maximized {
                        "editor-panel-restore"
                    } else {
                        "editor-panel-maximize"
                    },
                    zoom_icon,
                    zoom_label.clone(),
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
                .accessibility_label(zoom_label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_editor_panel_maximized(&zoom_editor, cx);
                })),
            )
            .child(
                crate::widgets::IconButton::new(
                    "editor-panel-close",
                    AppIcon::Ui(UiIcon::CLOSE),
                    rust_i18n::t!("editor.close_panel").to_string(),
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.request_close_editor_panel(&close_editor, window, cx);
                })),
            )
            .into_any_element()
    }

    fn render_panel_body(
        &self,
        item: &EditorPanelState,
        editor: &Entity<EditorState>,
        p: &Palette,
        font_size: f32,
        cx: &mut Context<Self>,
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
                    .child(Spinner::new().xsmall().color(p.muted_foreground))
                    .child(
                        rust_i18n::t!("editor.reading_path", path = item.remote_path.as_str())
                            .to_string(),
                    )
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
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::ERROR)).xsmall())
                    .child(rust_i18n::t!("editor.not_loaded").to_string())
                    .child(
                        Button::new("editor-retry-read")
                            .compact()
                            .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small())
                            .label(rust_i18n::t!("editor.retry_read").to_string())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.redrive_editor_read(&retry_editor, cx);
                            })),
                    )
                    .into_any_element()
            }
        } else if item.showing_diff {
            // 保存前差异预览：替换编辑器主体，确认回传/返回编辑在预览工具行上。
            self.render_diff_view(item, p, cx)
        } else {
            // 编辑器铺满正文：去掉输入框自带的背景、圆角与边框，层次只由面板外框提供。
            Editor::new(editor)
                .appearance(false)
                .bordered(false)
                // 设计令牌例外：编辑器字号由用户设置动态控制，不能映射到固定字号档位。
                .text_size(px(font_size))
                .w_full()
                .flex_1()
                .min_h_0()
                .into_any_element()
        };
        let body = v_flex().flex_1().min_h_0().child(content);

        let mut banners: Vec<AnyElement> = Vec::with_capacity(3);
        if let Some(error) = item.error.clone() {
            banners.push(self.render_error_banner(error, editor, p, cx));
        }
        if item.conflict {
            banners.push(self.render_conflict_banner(editor, p, cx));
        }
        if let Some(kind) = item.confirm {
            banners.push(self.render_confirm_banner(kind, editor, p, cx));
        }
        if banners.is_empty() {
            return body.into_any_element();
        }
        body.child(v_flex().px_2().pb_2().gap_1().children(banners))
            .into_any_element()
    }
}

/// 面板元素标识：标签 + 远端路径唯一确定一个编辑会话（同一文件不会开两个面板）。
///
/// 外层 `.id()` 会为后代元素划定 id 作用域，面板内部的按钮因此可以复用固定字面量 id。
fn panel_element_key(item: &EditorPanelState) -> String {
    format!("editor-panel-{}-{}", item.tab_id, item.remote_path)
}
