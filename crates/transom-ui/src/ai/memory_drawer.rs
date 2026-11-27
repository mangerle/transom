//! AI 面板三层记忆查看与编辑抽屉。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_ai::MemorySnapshot;

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 渲染三层记忆查看与编辑抽屉。
pub(super) fn render_memory_drawer(
    has_host: bool,
    host_name: &str,
    global_input: &Entity<InputState>,
    host_input: &Entity<InputState>,
    memory: &MemorySnapshot,
    p: &Palette,
    on_save_memory: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let _ = memory;
    let mut box_el = v_flex()
        .flex_1()
        .w_full()
        .p(rems(space::SM))
        .gap(rems(space::SM))
        .overflow_y_scrollbar()
        .child(
            div()
                .text_size(rems(typography::BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.foreground)
                .child(rust_i18n::t!("ai.global_memory_title").to_string()),
        )
        .child(Input::new(global_input));
    if has_host {
        box_el = box_el
            .child(
                div()
                    .text_size(rems(typography::BODY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.foreground)
                    .child(rust_i18n::t!("ai.host_memory_title", host = host_name).to_string()),
            )
            .child(Input::new(host_input));
    }
    box_el = box_el.child(
        h_flex().justify_end().child(
            Button::new("ai-save-memory")
                .primary()
                .label(rust_i18n::t!("ai.save_memory").to_string())
                .on_click(move |_, w, cx| on_save_memory(w, cx)),
        ),
    );
    box_el.into_any_element()
}

impl super::view::AiAssistantView {
    pub(super) fn render_memory_body(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let has_host = self.state.active_session.is_some();
        let general = rust_i18n::t!("ai.general_label").to_string();
        let host_name = self
            .state
            .active_session
            .as_ref()
            .map(|s| s.host_name.as_str())
            .unwrap_or(&general);
        let view = cx.entity();
        render_memory_drawer(
            has_host,
            host_name,
            &self.global_mem_input,
            &self.host_mem_input,
            &self.state.memory,
            p,
            move |w, cx| view.update(cx, |this, cx| this.save_memory_from_drawer(w, cx)),
        )
    }
}
