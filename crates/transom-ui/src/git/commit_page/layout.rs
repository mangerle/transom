//! 提交页分栏装配与页签按钮。

use super::*;
use gpui_kit::component::resizable::ResizableState;
use transom_core::TabId;

pub(super) fn render_commit_page_split(
    workspace: Entity<GitWorkspace>,
    tab_id: TabId,
    tabs: impl IntoElement,
    content: impl IntoElement,
    split: Entity<ResizableState>,
    p: &Palette,
    cx: &App,
) -> AnyElement {
    let left = v_flex()
        .w_full()
        .h_full()
        .min_h_0()
        .overflow_hidden()
        .child(tabs)
        .child(content);

    let right = {
        let state = &workspace.read(cx).state;
        v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(diff_view::render(state, workspace.clone(), p))
    };

    let prev_ws = workspace.clone();
    let next_ws = workspace;
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
        .on_action(move |_: &crate::keymap::DiffPrevChange, _, cx| {
            prev_ws.update(cx, |ws, cx| ws.step_diff_change(false, cx));
        })
        .on_action(move |_: &crate::keymap::DiffNextChange, _, cx| {
            next_ws.update(cx, |ws, cx| ws.step_diff_change(true, cx));
        })
        .child(
            h_resizable(ElementId::Name(format!("git-commit-split-{tab_id}").into()))
                .with_state(&split)
                .child(left_panel)
                .child(right_panel),
        )
        .into_any_element()
}

/// 提交页顶部页签的装配参数。
///
/// 页签位置与总数用于辅助技术播报“第几项、共几项”，不能再从按钮外观
/// 反推；与标题、选中态一起收进参数对象，避免调用点平铺过多位置参数。
pub(super) struct CommitTabParams<'a> {
    pub(super) id: &'static str,
    pub(super) label: String,
    pub(super) active: bool,
    pub(super) position: usize,
    pub(super) size: usize,
    pub(super) palette: &'a Palette,
}

/// 提交页顶部单个页签胶囊按钮。
pub(super) fn commit_tab_btn(
    params: CommitTabParams<'_>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Tab {
    let CommitTabParams {
        id,
        label,
        active,
        position,
        size,
        palette,
    } = params;
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
        .selected(active)
        .accessibility_label(label.clone())
        .set_position(position, size)
        .when(active, |el| el.child(underline_tab_active_line(palette)))
        .child(label)
        .on_click(on_click)
}
