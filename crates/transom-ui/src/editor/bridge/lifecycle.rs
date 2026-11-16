//! 编辑器关闭拦截、订阅裁剪与“跳转到行”弹窗桥接。

use crate::app::{AppShell, AppState};
use crate::editor::{
    EditorCloseRequest, EditorGotoLineModalState, PendingEditorClose, apply_editor_goto,
};
use crate::palette::Palette;
use crate::palette::sftp_text_input;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::RopeExt;
use gpui_kit::component::input::{EditorState, Input, InputEvent};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::TabId;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::{AppIcon, UiIcon};

/// 丢弃标签已不存在的对比面板。
pub(crate) fn prune_tab_diff_overlay(app_state: &mut AppState, cx: &App) {
    let Some(view) = app_state.features.diff_viewer.as_ref() else {
        return;
    };
    let tab_id = view.read(cx).state.tab_id;
    let alive = app_state.workspace.tabs.iter().any(|tab| tab.id == tab_id);
    if !alive {
        app_state.features.diff_viewer = None;
        app_state.runtime.tasks.subscriptions.diff_viewer = None;
    }
}

/// 按存活标签集合裁剪编辑面板、对比面板与容器/Git 工作区订阅。
pub(crate) fn prune_tab_overlays(app_state: &mut AppState, cx: &mut Context<AppShell>) {
    let alive: Vec<TabId> = app_state.workspace.tabs.iter().map(|tab| tab.id).collect();
    app_state
        .features
        .editors
        .update(cx, |panels, _| panels.prune_dead_tabs(&alive));
    app_state
        .runtime
        .tasks
        .subscriptions
        .prune_container_workspaces(&alive);
    app_state
        .runtime
        .tasks
        .subscriptions
        .prune_git_workspaces(&alive);
    prune_tab_diff_overlay(app_state, cx);
}

/// 关闭路径的脏面板闸门；实体决定是否拦截并展开确认横幅。
pub(crate) fn block_close_for_dirty_editors(
    app_state: &mut AppState,
    tab_ids: &[TabId],
    intent: PendingEditorClose,
    cx: &mut Context<AppShell>,
) -> bool {
    app_state
        .features
        .editors
        .update(cx, |panels, cx| panels.block_close(tab_ids, intent, cx))
}

/// 脏面板全部处理完毕后续行挂起的关闭。
///
/// 实体回调期间外壳已被租借，不能在原处执行关闭动作；这里只把关闭意图
/// 排进窗口请求队列，由外壳在渲染期统一消费。
pub(crate) fn resume_pending_editor_close(app_state: &mut AppState, cx: &mut Context<AppShell>) {
    let Some(intent) = app_state.features.editors.read(cx).pending_close() else {
        return;
    };
    let dirty_tabs = app_state.features.editors.read(cx).dirty_tabs();
    if let Some(request) = try_finish_pending_close(app_state, intent, &dirty_tabs, cx) {
        app_state.ui.window_requests.push_editor_close(request);
    }
}

/// 延迟到下一帧空转点再续行关闭，避免在实体回调中重入读写。
pub(crate) fn defer_editor_close_resume(
    _app_state: &mut AppState,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    cx.defer_in(window, |shell, _window, cx| {
        resume_pending_editor_close(&mut shell.state, cx)
    });
}

/// 按挂起意图与当前脏标签快照决定是否续行；由保存成功等外壳事件复用。
///
/// 返回外壳需要执行的关闭请求：脏标签未处理完或目标标签已消失时返回
/// `None`。关闭请求交回外壳能保证关闭动作与实体回调分离，避免在实体
/// 更新期间回读外壳。
pub(crate) fn try_finish_pending_close(
    app_state: &mut AppState,
    intent: PendingEditorClose,
    dirty_tabs: &[TabId],
    cx: &mut Context<AppShell>,
) -> Option<EditorCloseRequest> {
    let tab_ids = tabs_for_pending_close(app_state, &intent);
    if tab_ids.is_empty() {
        clear_editor_pending_close(app_state, cx);
        cx.notify();
        return None;
    }
    if tab_ids.iter().any(|tab_id| dirty_tabs.contains(tab_id)) {
        return None;
    }
    clear_editor_pending_close(app_state, cx);
    Some(match intent {
        PendingEditorClose::Group(group_id) => EditorCloseRequest::Group(group_id),
        PendingEditorClose::Pane(tab_id) => EditorCloseRequest::Pane(tab_id),
        PendingEditorClose::Session(session_id) => EditorCloseRequest::Session(session_id),
        PendingEditorClose::Tiling(groups) => EditorCloseRequest::Tiling(groups),
    })
}

fn clear_editor_pending_close(app_state: &mut AppState, cx: &mut Context<AppShell>) {
    app_state
        .features
        .editors
        .update(cx, |panels, _| panels.clear_pending_close());
}

fn tabs_for_pending_close(app_state: &AppState, intent: &PendingEditorClose) -> Vec<TabId> {
    match intent {
        PendingEditorClose::Group(group_id) => app_state
            .workspace
            .groups
            .iter()
            .find(|group| group.id == *group_id)
            .map(|group| group.pane_ids())
            .unwrap_or_default(),
        PendingEditorClose::Pane(tab_id) => Vec::from([*tab_id]),
        PendingEditorClose::Session(session_id) => app_state
            .workspace
            .tabs
            .iter()
            .filter(|tab| tab.session_id == *session_id)
            .map(|tab| tab.id)
            .collect(),
        PendingEditorClose::Tiling(groups) => groups
            .iter()
            .filter_map(|group_id| {
                app_state
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
    app_state: &mut AppState,
    editor: Entity<EditorState>,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let input = sftp_text_input(window, cx, "", &rust_i18n::t!("editor.goto_placeholder"));
    input.update(cx, |state, cx| state.focus(window, cx));
    let enter_sub = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
        if matches!(event, InputEvent::PressEnter { .. }) {
            commit_editor_goto_line(&mut this.state, window, cx);
        }
    });
    app_state.ui.modals.tools.editor_goto_line = Some(EditorGotoLineModalState {
        editor,
        input,
        error: None,
        _enter_sub: enter_sub,
    });
    cx.notify();
}

/// 关闭跳转模态并把焦点还给编辑器。
pub(crate) fn cancel_editor_goto_line(
    app_state: &mut AppState,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let editor = app_state
        .ui
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
    app_state: &mut AppState,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(modal) = app_state.ui.modals.tools.editor_goto_line.as_ref() else {
        return;
    };
    let editor = modal.editor.clone();
    let raw = modal.input.read(cx).value().trim().to_string();
    let Some(line) = raw.parse::<usize>().ok().filter(|line| *line >= 1) else {
        if let Some(modal) = app_state.ui.modals.tools.editor_goto_line.as_mut() {
            modal.error = Some(rust_i18n::t!("editor.goto_error").to_string());
        }
        cx.notify();
        return;
    };
    apply_editor_goto(&editor, line as u32, None, window, cx);
    app_state.ui.modals.tools.editor_goto_line = None;
    cx.notify();
}

/// 渲染"跳转到行"输入模态。
pub(crate) fn render_editor_goto_line_body(
    app_state: &AppState,
    app: &Entity<AppShell>,
    cx: &App,
    p: &Palette,
) -> AnyElement {
    let Some(modal) = app_state.ui.modals.tools.editor_goto_line.as_ref() else {
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
                cancel_editor_goto_line(&mut this.state, window, cx);
            }),
        ))
        .child(crate::widgets::dialog_confirm_button(
            "editor-goto-confirm",
            rust_i18n::t!("editor.goto_action").to_string(),
            Some(AppIcon::Ui(UiIcon::CHECK)),
            crate::widgets::DialogConfirmKind::Primary,
            crate::dialog_host::on_event(app, |this, _: &ClickEvent, window, cx| {
                commit_editor_goto_line(&mut this.state, window, cx);
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
