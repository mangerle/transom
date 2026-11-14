//! 提交页分栏装配与页签按钮。

use super::*;
use gpui_kit::component::resizable::ResizableState;

pub(super) fn render_commit_page_split(
    shell: &mut AppShell,
    tab_id: TabId,
    tabs: impl IntoElement,
    content: impl IntoElement,
    split: Entity<ResizableState>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let left = v_flex()
        .w_full()
        .h_full()
        .min_h_0()
        .overflow_hidden()
        .child(tabs)
        .child(content);

    // 右栏每次重新借一次状态：左栏构建期间要 `&mut AppShell`。
    let Some(workspace) = shell.state.git_workspace(tab_id) else {
        return left.into_any_element();
    };
    let right = {
        let state = &workspace.read(cx).state;
        v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(diff_view::render(state, cx.entity(), tab_id, p))
    };

    let left_panel = resizable_panel()
        .size(px(COMMIT_PANEL_WIDTH))
        .size_range(px(COMMIT_PANEL_RANGE.0)..px(COMMIT_PANEL_RANGE.1))
        .child(left);
    let right_panel = resizable_panel().child(right);
    div()
        .flex_1()
        .min_h_0()
        .size_full()
        .key_context(crate::keymap::GIT_WORKSPACE_CONTEXT)
        .on_action(
            cx.listener(move |this, _: &crate::keymap::DiffPrevChange, _, cx| {
                crate::git::actions::git_step_diff_change(this, tab_id, false, cx);
            }),
        )
        .on_action(
            cx.listener(move |this, _: &crate::keymap::DiffNextChange, _, cx| {
                crate::git::actions::git_step_diff_change(this, tab_id, true, cx);
            }),
        )
        .child(
            h_resizable(ElementId::Name(format!("git-commit-split-{tab_id}").into()))
                .with_state(&split)
                .child(left_panel)
                .child(right_panel),
        )
        .into_any_element()
}

/// 提交页顶部单个页签胶囊按钮。
pub(super) fn commit_tab_btn(
    id: &'static str,
    label: String,
    active: bool,
    p: &Palette,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Tab {
    Tab::new(id)
        .relative()
        .h_full()
        .px_2()
        .text_size(rems(typography::BODY))
        .cursor_pointer()
        .font_weight(if active {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        })
        .when(active, |el| el.child(underline_tab_active_line(p)))
        .child(label)
        .on_click(on_click)
}
