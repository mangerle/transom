//! 终端视图渲染入口与拖放交互。
use super::*;

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.process_events(cx);

        let right_click = self.interaction.right_click_behavior();
        let menu_entity = cx.entity();
        let layer = div()
            .id(ElementId::Name(
                format!("terminal-layer-{}", cx.entity().entity_id()).into(),
            ))
            .size_full()
            .overflow_hidden()
            .track_focus(&self.focus_handle)
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_action_send_tab))
            .on_action(cx.listener(Self::on_action_send_backtab))
            .on_action(cx.listener(Self::on_action_terminal_copy))
            .on_action(cx.listener(Self::on_action_terminal_paste))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_hover(cx.listener(Self::on_terminal_hover))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(self.render_canvas(cx));
        let layer = Self::wrap_context_menu(layer, menu_entity, right_click);

        let search_bar = self.render_search_bar(cx);
        let block_actions = self.render_block_actions(cx);
        // 拖放高亮用主色调半透明蒙色：不改布局、不依赖主题里的具体色值，
        // 深浅色终端下都能看清"松手就落在这里"。
        let drop_tint = cx.theme().primary.opacity(0.12);
        let entity = cx.entity().clone();
        // 检索栏与终端层平级：放在终端层子树里会让输入框的按键沿焦点路径冒泡到
        // `on_key_down`，敲搜索词的同时也把字符送进了 Shell。
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .on_action(cx.listener(Self::on_search_enter))
            .on_action(cx.listener(Self::on_search_escape))
            // 外部文件拖放：命中区域就是网格本身，视图只投递事件，
            // 要不要接收、传到哪儿由应用层按会话类型决定。
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_tint))
            .on_drop(move |paths: &ExternalPaths, _window, cx| {
                let dropped = paths.paths().to_vec();
                if dropped.is_empty() {
                    return;
                }
                entity.update(cx, |_, cx| cx.emit(TerminalEvent::FilesDropped(dropped)));
            })
            .child(layer)
            .children(block_actions)
            .children(self.render_suggestion_bar(cx))
            .children(search_bar)
    }
}
