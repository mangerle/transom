//! Git 差异面板内置查找（Ctrl+F）：打开/关闭、文本匹配重算与上/下一处高亮定位。

use crate::git::GitWorkspace;
use crate::git::types::GitWorkspaceState;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{AppContext as _, Context, Focusable, Window};

impl GitWorkspace {
    /// 打开 Git 差异内置查找栏（若已开启则聚焦输入框；支持再次快捷键聚焦）。
    pub(crate) fn open_diff_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let existing_input = self.state.diff_search.input.clone();
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
                                this.update_diff_search_query(query, cx);
                            }
                            InputEvent::PressEnter { shift, .. } => {
                                this.step_diff_search(!shift, cx);
                            }
                            _ => {}
                        }
                    });
                self.state.diff_search.input = Some(input.clone());
                self.state.diff_search.subscription = Some(sub);
                input
            }
        };

        self.state.diff_search.open = true;
        input.focus_handle(cx).focus(window, cx);
        self.recompute_diff_search();
        cx.notify();
    }

    /// 切换 Git 差异内置查找栏（用于工具栏搜索按钮）。
    pub(crate) fn toggle_diff_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.diff_search.open {
            self.close_diff_search(cx);
        } else {
            self.open_diff_search(window, cx);
        }
    }

    /// 关闭 Git 差异内置查找栏并清理高亮。
    pub(crate) fn close_diff_search(&mut self, cx: &mut Context<Self>) {
        self.state.diff_search.open = false;
        self.state.diff_search.clear_matches();
        cx.notify();
    }

    /// 输入框内容变更后重算匹配位置并自动滚到第一处命中。
    fn update_diff_search_query(&mut self, query: String, cx: &mut Context<Self>) {
        self.state.diff_search.query = query;
        self.state.diff_search.current_idx = 0;
        recompute_workspace_diff_search(&mut self.state);
        if let Some(pos) = self.state.diff_search.current_position()
            && let Some(ds) = self.state.active_diff().and_then(|d| d.diff_state.clone())
        {
            ds.update(cx, |state, cx| state.scroll_to_line(pos, cx));
        }
        cx.notify();
    }

    /// 跳转到当前差异内的上一处/下一处查找命中。
    pub(crate) fn step_diff_search(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(pos) = self.state.diff_search.step(forward) else {
            return;
        };
        if let Some(ds) = self.state.active_diff().and_then(|d| d.diff_state.clone()) {
            ds.update(cx, |state, cx| state.scroll_to_line(pos, cx));
            cx.notify();
        }
    }

    /// 在激活差异切换或重新计算完成后刷新已展开的搜索匹配列表。
    pub(crate) fn recompute_diff_search(&mut self) {
        if self.state.diff_search.open {
            recompute_workspace_diff_search(&mut self.state);
        }
    }
}

pub(super) fn recompute_workspace_diff_search(state: &mut GitWorkspaceState) {
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
