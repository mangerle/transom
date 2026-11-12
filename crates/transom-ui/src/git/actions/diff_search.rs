//! Git 差异面板内置查找（Ctrl+F）：打开/关闭、文本匹配重算与上/下一处高亮定位。

use crate::app::AppShell;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{AppContext as _, Context, Focusable, Window};
use transom_core::TabId;

impl AppShell {
    /// 打开 Git 差异内置查找栏（若已开启则聚焦输入框；支持再次快捷键聚焦）。
    pub(crate) fn git_open_diff_search(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let existing_input = workspace.read(cx).state.diff_search.input.clone();
        let input = match existing_input {
            Some(input) => input,
            None => {
                let placeholder = rust_i18n::t!("git.diff.search_placeholder").to_string();
                let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
                let sub =
                    cx.subscribe_in(&input, window, move |this, input, event, _window, cx| {
                        match event {
                            InputEvent::Change => {
                                let query = input.read(cx).value().to_string();
                                this.git_update_diff_search_query(tab_id, query, cx);
                            }
                            InputEvent::PressEnter { shift, .. } => {
                                this.git_step_diff_search(tab_id, !shift, cx);
                            }
                            _ => {}
                        }
                    });
                workspace.update(cx, |workspace, _| {
                    workspace.state.diff_search.input = Some(input.clone());
                    workspace.state.diff_search.subscription = Some(sub);
                });
                input
            }
        };

        workspace.update(cx, |workspace, _| {
            workspace.state.diff_search.open = true;
        });
        input.focus_handle(cx).focus(window, cx);
        self.git_recompute_diff_search(tab_id, cx);
        cx.notify();
    }

    /// 切换 Git 差异内置查找栏（用于工具栏搜索按钮）。
    pub(crate) fn git_toggle_diff_search(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        if workspace.read(cx).state.diff_search.open {
            self.git_close_diff_search(tab_id, cx);
        } else {
            self.git_open_diff_search(tab_id, window, cx);
        }
    }

    /// 关闭 Git 差异内置查找栏并清理高亮。
    pub(crate) fn git_close_diff_search(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        workspace.update(cx, |workspace, _| {
            workspace.state.diff_search.open = false;
            workspace.state.diff_search.clear_matches();
        });
        cx.notify();
    }

    /// 输入框内容变更后重算匹配位置并自动滚到第一处命中。
    fn git_update_diff_search_query(
        &mut self,
        tab_id: TabId,
        query: String,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let target = workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            state.diff_search.query = query;
            state.diff_search.current_idx = 0;
            recompute_workspace_diff_search(state);
            let pos = state.diff_search.current_position()?;
            let ds = state.active_diff().and_then(|d| d.diff_state.clone())?;
            Some((ds, pos))
        });
        if let Some((ds, pos)) = target {
            ds.update(cx, |state, cx| state.scroll_to_line(pos, cx));
        }
        cx.notify();
    }

    /// 跳转到当前差异内的上一处/下一处查找命中。
    pub(crate) fn git_step_diff_search(
        &mut self,
        tab_id: TabId,
        forward: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let target = workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            let pos = state.diff_search.step(forward)?;
            let ds = state.active_diff().and_then(|d| d.diff_state.clone())?;
            Some((ds, pos))
        });
        if let Some((ds, pos)) = target {
            ds.update(cx, |state, cx| state.scroll_to_line(pos, cx));
            cx.notify();
        }
    }

    /// 在激活差异切换或重新计算完成后刷新已展开的搜索匹配列表。
    pub(crate) fn git_recompute_diff_search(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        workspace.update(cx, |workspace, _| {
            if workspace.state.diff_search.open {
                recompute_workspace_diff_search(&mut workspace.state);
            }
        });
    }
}

pub(super) fn recompute_workspace_diff_search(state: &mut crate::git::types::GitWorkspaceState) {
    if !state.diff_search.open {
        return;
    }
    let payload = state.active_diff().and_then(|diff| {
        let content = diff.content.as_ref()?;
        (!content.binary && diff.diff_state.is_some()).then(|| {
            (
                diff.path.clone(),
                content.old_text.clone(),
                content.new_text.clone(),
            )
        })
    });
    if let Some((path, old_text, new_text)) = payload {
        state.diff_search.recompute(&path, &old_text, &new_text);
    } else {
        state.diff_search.clear_matches();
    }
}
