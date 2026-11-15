//! 编辑器关闭拦截、订阅裁剪与“跳转到行”弹窗桥接。

use crate::app::AppShell;
use crate::editor::close::PendingEditorClose;
use crate::editor::navigate::{EditorGotoLineModalState, apply_editor_goto};
use crate::palette::Palette;
use crate::palette::sftp_text_input;
use gpui_kit::component::input::RopeExt;
use transom_ui_icons::AppIconElement;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::{EditorState, Input, InputEvent};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::TabId;
use transom_ui_icons::{AppIcon, UiIcon};

/// 丢弃标签已不存在的对比面板。
pub(crate) fn prune_tab_diff_overlay(shell: &mut AppShell, cx: &App) {
    let Some(view) = shell.state.diff_viewer.as_ref() else {
        return;
    };
    let tab_id = view.read(cx).state.tab_id;
    let alive = shell
        .state
        .workspace
        .tabs
        .iter()
        .any(|tab| tab.id == tab_id);
    if !alive {
        shell.state.diff_viewer = None;
        shell.state.tasks.subscriptions.diff_viewer = None;
    }
}

/// 按存活标签集合裁剪编辑面板、对比面板与容器/Git 工作区订阅。
pub(crate) fn prune_tab_overlays(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    let alive: Vec<TabId> = shell
        .state
        .workspace
        .tabs
        .iter()
        .map(|tab| tab.id)
        .collect();
    shell
        .state
        .editors
        .update(cx, |panels, _| panels.prune_dead_tabs(&alive));
    shell
        .state
        .tasks
        .subscriptions
        .prune_container_workspaces(&alive);
    shell.state.tasks.subscriptions.prune_git_workspaces(&alive);
    prune_tab_diff_overlay(shell, cx);
}

/// 关闭路径的脏面板闸门；实体决定是否拦截并展开确认横幅。
pub(crate) fn block_close_for_dirty_editors(
    shell: &mut AppShell,
    tab_ids: &[TabId],
    intent: PendingEditorClose,
    cx: &mut Context<AppShell>,
) -> bool {
    shell
        .state
        .editors
        .update(cx, |panels, cx| panels.block_close(tab_ids, intent, cx))
}

/// 脏面板全部处理完毕后续行挂起的关闭。
pub(crate) fn resume_pending_editor_close(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(intent) = shell.state.editors.read(cx).pending_close() else {
        return;
    };
    let dirty_tabs = shell.state.editors.read(cx).dirty_tabs();
    try_finish_pending_close(shell, intent, &dirty_tabs, window, cx);
}

/// 延迟到下一帧空转点再续行关闭，避免在实体回调中重入读写。
pub(crate) fn defer_editor_close_resume(
    _shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    cx.defer_in(window, |shell, window, cx| {
        resume_pending_editor_close(shell, window, cx)
    });
}

/// 按挂起意图与当前脏标签快照决定是否续行；由保存成功等外壳事件复用。
pub(crate) fn try_finish_pending_close(
    shell: &mut AppShell,
    intent: PendingEditorClose,
    dirty_tabs: &[TabId],
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let tab_ids = tabs_for_pending_close(shell, &intent);
    if tab_ids.is_empty() {
        clear_editor_pending_close(shell, cx);
        cx.notify();
        return;
    }
    if tab_ids.iter().any(|tab_id| dirty_tabs.contains(tab_id)) {
        return;
    }
    clear_editor_pending_close(shell, cx);
    match intent {
        PendingEditorClose::Group(group_id) => shell.close_group(group_id, window, cx),
        PendingEditorClose::Pane(tab_id) => shell.close_pane(tab_id, window, cx),
        PendingEditorClose::Session(session_id) => shell.delete_session_by_id(session_id, cx),
        PendingEditorClose::Tiling(groups) => {
            crate::tiling::close_tiling_groups(shell, groups, window, cx);
        }
    }
}

fn clear_editor_pending_close(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    shell
        .state
        .editors
        .update(cx, |panels, _| panels.clear_pending_close());
}

fn tabs_for_pending_close(shell: &AppShell, intent: &PendingEditorClose) -> Vec<TabId> {
    match intent {
        PendingEditorClose::Group(group_id) => shell
            .state
            .workspace
            .groups
            .iter()
            .find(|group| group.id == *group_id)
            .map(|group| group.pane_ids())
            .unwrap_or_default(),
        PendingEditorClose::Pane(tab_id) => Vec::from([*tab_id]),
        PendingEditorClose::Session(session_id) => shell
            .state
            .workspace
            .tabs
            .iter()
            .filter(|tab| tab.session_id == *session_id)
            .map(|tab| tab.id)
            .collect(),
        PendingEditorClose::Tiling(groups) => groups
            .iter()
            .filter_map(|group_id| {
                shell
                    .state
                    .workspace
                    .groups
                    .iter()
                    .find(|group| group.id == *group_id)
            })
            .flat_map(|group| group.pane_ids())
            .collect(),
    }
}

/// 打开"跳转到行"模态。
pub(crate) fn open_editor_goto_line(
    shell: &mut AppShell,
    editor: Entity<EditorState>,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let input = sftp_text_input(window, cx, "", &rust_i18n::t!("editor.goto_placeholder"));
    input.update(cx, |state, cx| state.focus(window, cx));
    let enter_sub = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
        if matches!(event, InputEvent::PressEnter { .. }) {
            commit_editor_goto_line(this, window, cx);
        }
    });
    shell.state.modals.tools.editor_goto_line = Some(EditorGotoLineModalState {
        editor,
        input,
        error: None,
        _enter_sub: enter_sub,
    });
    cx.notify();
}

/// 关闭跳转模态并把焦点还给编辑器。
pub(crate) fn cancel_editor_goto_line(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let editor = shell
        .state
        .modals
        .tools
        .editor_goto_line
        .take()
        .map(|modal| modal.editor);
    if let Some(editor) = editor {
        editor.update(cx, |state, cx| state.focus(window, cx));
    }
    cx.notify();
}

/// 确认跳转：解析行号，越界收敛到 [1, 总行数]，展开遮挡折叠后移动光标。
pub(crate) fn commit_editor_goto_line(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(modal) = shell.state.modals.tools.editor_goto_line.as_ref() else {
        return;
    };
    let editor = modal.editor.clone();
    let raw = modal.input.read(cx).value().trim().to_string();
    let Some(line) = raw.parse::<usize>().ok().filter(|line| *line >= 1) else {
        if let Some(modal) = shell.state.modals.tools.editor_goto_line.as_mut() {
            modal.error = Some(rust_i18n::t!("editor.goto_error").to_string());
        }
        cx.notify();
        return;
    };
    apply_editor_goto(&editor, line as u32, None, window, cx);
    shell.state.modals.tools.editor_goto_line = None;
    cx.notify();
}

/// 渲染"跳转到行"输入模态。
pub(crate) fn render_editor_goto_line_body(
    shell: &AppShell,
    app: &Entity<AppShell>,
    cx: &App,
    p: &Palette,
) -> AnyElement {
    let Some(modal) = shell.state.modals.tools.editor_goto_line.as_ref() else {
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
                .child(rust_i18n::t!("editor.goto_hint", total = total).to_string()),
        )
        .child(Input::new(&input).w_full())
        .when_some(error, |this, err| {
            this.child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(p.danger)
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::ERROR)).xsmall())
                    .child(err),
            )
        });

    let footer = h_flex()
        .gap_2()
        .child(crate::widgets::dialog_cancel_button(
            "editor-goto-cancel",
            rust_i18n::t!("common.cancel").to_string(),
            crate::dialog_host::on_event(app, |this, _: &ClickEvent, window, cx| {
                cancel_editor_goto_line(this, window, cx);
            }),
        ))
        .child(crate::widgets::dialog_confirm_button(
            "editor-goto-confirm",
            rust_i18n::t!("editor.goto_action").to_string(),
            Some(AppIcon::Ui(UiIcon::CHECK)),
            crate::widgets::DialogConfirmKind::Primary,
            crate::dialog_host::on_event(app, |this, _: &ClickEvent, window, cx| {
                commit_editor_goto_line(this, window, cx);
            }),
        ));

    crate::widgets::dialog_body(
        "editor-goto-line-modal",
        rust_i18n::t!("editor.goto_title").to_string(),
        None,
        content,
        Some(footer),
    )
}
