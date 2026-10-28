//! SFTP 工具栏的"已打开编辑器"浮层：列出当前标签打开的编辑面板，
//! 点击置顶、聚焦并还原折叠，替代在层叠悬浮面板里凭记忆找回文件。
//!
//! 面板状态在 `editor_panel.rs`，窗体动作在 `editor_actions.rs`。
//!
//! 设计原理（Why）：
//! - 面板按 `TabId` 挂在窗格上，列表也只展示当前标签的面板，与"同一文件
//!   不会跨标签开两个面板"的身份规则一致。
//! - 行数据在渲染时快照：Popover 的内容闭包运行在浮层自身上下文里，拿不到
//!   `AppShell` 状态，点击动作通过捕获的 `Entity<AppShell>` 回写。
//! - 悬浮面板以层叠顺序表达"当前在看哪个"，列表只是跳转入口，不复制面板
//!   自身的关闭/折叠按钮，避免两套并行的管理入口。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Selectable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::EditorState;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::TabId;

/// 渲染"已打开编辑器"浮层：触发按钮 + 当前面板的文件列表。
pub(crate) fn render_editor_list_popover(
    this: &AppShell,
    tab_id: TabId,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let open = this.editors.list_open;
    // 渲染时快照面板数据：浮层内容闭包运行在 Popover 上下文，无法回读 AppShell。
    let panels: Vec<_> = this
        .editors
        .panels
        .iter()
        .filter(|item| item.tab_id == tab_id)
        .map(|item| (item.editor.clone(), item.file_name.clone(), item.dirty))
        .collect();
    let app = cx.entity().clone();
    let count = panels.len();
    let card_palette = p.clone();
    let trigger = Button::new("sftp-open-editors")
        .ghost()
        .xsmall()
        .icon(IconName::Files)
        .selected(open)
        .tooltip(if count > 0 {
            format!("已打开的编辑器（{count}）")
        } else {
            "已打开的编辑器".to_string()
        });

    let app_open = app.clone();
    Popover::new(ElementId::Name(
        format!("editor-list-popover-{tab_id}").into(),
    ))
    .anchor(Anchor::TopLeft)
    .open(open)
    .on_open_change(move |open, _window, cx| {
        let open = *open;
        app_open.update(cx, |this, cx| {
            if this.editors.list_open != open {
                this.editors.list_open = open;
                cx.notify();
            }
        });
    })
    .trigger(trigger)
    .content(move |_, _, _| render_editor_list_card(&app, &panels, &card_palette))
    .into_any_element()
}

/// 浮层卡片：逐行列出文件，点击置顶并聚焦；行上区分脏/净图标。
fn render_editor_list_card(
    app: &Entity<AppShell>,
    panels: &[(Entity<EditorState>, String, bool)],
    p: &Palette,
) -> AnyElement {
    if panels.is_empty() {
        return v_flex()
            .w(px(240.0))
            .items_center()
            .py_4()
            .text_xs()
            .text_color(p.muted_foreground)
            .child("当前标签没有打开的编辑文件")
            .into_any_element();
    }
    let rows: Vec<_> = panels
        .iter()
        .map(|(editor, file_name, dirty)| {
            let row_app = app.clone();
            let row_editor = editor.clone();
            h_flex()
                .id(ElementId::Name(
                    format!("editor-list-row-{}", row_editor.entity_id()).into(),
                ))
                .w_full()
                .min_w_0()
                .h(px(30.0))
                .gap_2()
                .px_2()
                .items_center()
                .rounded(p.radius.md)
                .cursor_pointer()
                .hover(|style| style.bg(p.foreground.opacity(0.10)))
                .on_click(move |_, window, cx| {
                    row_app.update(cx, |this, cx| {
                        // 折叠的面板先展开，否则置顶后仍只剩标题条，看起来像没反应。
                        if let Some(item) = this.editor_panel_mut(&row_editor) {
                            item.collapsed = false;
                        }
                        this.raise_editor_panel(&row_editor, cx);
                        row_editor.update(cx, |state, cx| state.focus(window, cx));
                        this.editors.list_open = false;
                        cx.notify();
                    });
                })
                .child(
                    Icon::new(if *dirty {
                        IconName::FilePen
                    } else {
                        IconName::FileText
                    })
                    .xsmall()
                    .text_color(if *dirty {
                        p.sidebar_accent_foreground
                    } else {
                        p.muted_foreground
                    }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .child(file_name.clone()),
                )
        })
        .collect();
    v_flex()
        .w(px(240.0))
        .max_h(px(280.0))
        .overflow_y_scrollbar()
        .gap_1()
        .p_1()
        .children(rows)
        .into_any_element()
}
