//! SFTP 编辑面板差异预览的内置文本查找（Ctrl+F）。

use gpui_kit::component::diff::DiffState;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{AppContext as _, Context, Entity, Focusable, Window};

use super::entity::EditorPanels;

impl EditorPanels {
    /// 打开或切换编辑器差异预览内置查找栏。
    pub(crate) fn toggle_editor_diff_search(
        &mut self,
        editor: &Entity<gpui_kit::component::input::EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let already_open = self
            .map_panel(editor, |item| item.diff_search.open)
            .unwrap_or(false);
        if already_open {
            self.close_editor_diff_search(editor, cx);
        } else {
            self.open_editor_diff_search(editor, window, cx);
        }
    }

    /// 打开或聚焦编辑器差异预览查找框。
    pub(crate) fn open_editor_diff_search(
        &mut self,
        editor: &Entity<gpui_kit::component::input::EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = self
            .map_panel(editor, |item| item.diff_search.input.clone())
            .flatten();
        let input = match existing {
            Some(input) => input,
            None => {
                let placeholder = rust_i18n::t!("git.diff.search_placeholder").to_string();
                let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
                let target_editor = editor.clone();
                let sub = cx.subscribe_in(&input, window, move |this, input, event, _window, cx| {
                    match event {
                        InputEvent::Change => {
                            let query = input.read(cx).value().to_string();
                            this.update_editor_diff_search_query(&target_editor, query, cx);
                        }
                        InputEvent::PressEnter { shift, .. } => {
                            this.step_editor_diff_search(&target_editor, !shift, cx);
                        }
                        _ => {}
                    }
                });
                self.update_panel(editor, |item| {
                    item.diff_search.input = Some(input.clone());
                    item.diff_search.subscription = Some(sub);
                });
                input
            }
        };

        self.update_panel(editor, |item| {
            item.diff_search.open = true;
        });
        input.focus_handle(cx).focus(window, cx);
        self.recompute_editor_diff_search(editor, cx);
        cx.notify();
    }

    /// 关闭编辑器差异预览查找框并清理高亮。
    pub(crate) fn close_editor_diff_search(
        &mut self,
        editor: &Entity<gpui_kit::component::input::EditorState>,
        cx: &mut Context<Self>,
    ) {
        self.update_panel(editor, |item| {
            item.diff_search.open = false;
            item.diff_search.clear_matches();
        });
        cx.notify();
    }

    fn update_editor_diff_search_query(
        &mut self,
        editor: &Entity<gpui_kit::component::input::EditorState>,
        query: String,
        cx: &mut Context<Self>,
    ) {
        let current = editor.read(cx).value().to_string();
        let target: Option<(Entity<DiffState>, _)> = self.map_panel(editor, |item| {
            item.diff_search.query = query;
            item.diff_search.current_idx = 0;
            if item.diff.is_some() {
                item.diff_search
                    .recompute(&item.remote_path, &item.baseline, &current);
            } else {
                item.diff_search.clear_matches();
            }
            let pos = item.diff_search.current_position()?;
            let ds = item.diff.clone()?;
            Some((ds, pos))
        })
        .flatten();
        if let Some((ds, pos)) = target {
            ds.update(cx, |state, cx| state.scroll_to_line(pos, cx));
        }
        cx.notify();
    }

    /// 在编辑器差异预览中跳转到上一处/下一处查找命中。
    pub(crate) fn step_editor_diff_search(
        &mut self,
        editor: &Entity<gpui_kit::component::input::EditorState>,
        forward: bool,
        cx: &mut Context<Self>,
    ) {
        let target: Option<(Entity<DiffState>, _)> = self.map_panel(editor, |item| {
            let pos = item.diff_search.step(forward)?;
            let ds = item.diff.clone()?;
            Some((ds, pos))
        })
        .flatten();
        if let Some((ds, pos)) = target {
            ds.update(cx, |state, cx| state.scroll_to_line(pos, cx));
            cx.notify();
        }
    }

    /// 差异重算完成后刷新编辑器面板已打开的查找匹配列表。
    pub(crate) fn recompute_editor_diff_search(
        &mut self,
        editor: &Entity<gpui_kit::component::input::EditorState>,
        cx: &mut Context<Self>,
    ) {
        let current = editor.read(cx).value().to_string();
        self.update_panel(editor, |item| {
            if item.diff_search.open && item.diff.is_some() {
                item.diff_search
                    .recompute(&item.remote_path, &item.baseline, &current);
            } else {
                item.diff_search.clear_matches();
            }
        });
    }
}
