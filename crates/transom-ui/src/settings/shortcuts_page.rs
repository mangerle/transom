//! 偏好设置中的快捷键自定义页。

use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled};
use crate::keymap;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

impl AppShell {
    pub(crate) fn render_settings_shortcuts(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows: Vec<AnyElement> = keymap::specs()
            .iter()
            .map(|spec| {
                let effective = keymap::effective_keys(spec, &self.settings.shortcuts);
                let customized = self.settings.shortcuts.bindings.contains_key(spec.id);
                let id = spec.id;
                let key_elements =
                    h_flex()
                        .gap_1()
                        .children(effective.into_iter().map(|key_str| {
                            if let Ok(keystroke) = Keystroke::parse(&key_str) {
                                Kbd::new(keystroke).outline().into_any_element()
                            } else {
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded(p.radius.md)
                                    .border_1()
                                    .border_color(p.border)
                                    .bg(p.muted)
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(p.foreground)
                                    .child(key_str)
                                    .into_any_element()
                            }
                        }));
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .py_2()
                    .border_color(p.divider(p.border))
                    .divider_edge(DividerEdge::Bottom, p.divider_width)
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!(spec.label_key).to_string()),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(key_elements)
                            .child(
                                Button::new(ElementId::Name(format!("shortcut-edit-{id}").into()))
                                    .ghost()
                                    .xsmall()
                                    .label(rust_i18n::t!("settings.shortcut_edit").to_string())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_shortcut_capture(id, window, cx);
                                    })),
                            )
                            .when(customized, |row| {
                                row.child(
                                    Button::new(ElementId::Name(
                                        format!("shortcut-reset-{id}").into(),
                                    ))
                                    .ghost()
                                    .xsmall()
                                    .label(rust_i18n::t!("settings.shortcut_reset").to_string())
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.settings.shortcuts.bindings.remove(id);
                                            cx.notify();
                                        },
                                    )),
                                )
                            }),
                    )
                    .into_any_element()
            })
            .collect();

        let conflicts = keymap::conflict_pairs(&self.settings.shortcuts);
        let conflict_warning = (!conflicts.is_empty()).then(|| {
            let detail = conflicts
                .iter()
                .map(|(first, second)| {
                    format!(
                        "{} ↔ {}",
                        rust_i18n::t!(first.label_key),
                        rust_i18n::t!(second.label_key)
                    )
                })
                .collect::<Vec<_>>()
                .join("、");
            div()
                .text_xs()
                .text_color(p.danger)
                .child(rust_i18n::t!("settings.shortcut_conflicts", list = detail).to_string())
        });

        let has_custom = !self.settings.shortcuts.bindings.is_empty();
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .child(rust_i18n::t!("settings.tab_shortcuts").to_string()),
                    )
                    .when(has_custom, |row| {
                        row.child(
                            Button::new("reset-all-shortcuts")
                                .ghost()
                                .xsmall()
                                .label(rust_i18n::t!("settings.shortcut_reset_all").to_string())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.settings.shortcuts.bindings.clear();
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.shortcut_hint").to_string()),
            )
            .when_some(conflict_warning, |panel, warning| panel.child(warning))
            .child(v_flex().gap_1().children(rows))
            .into_any_element()
    }
}
