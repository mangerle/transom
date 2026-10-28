//! 密钥生成与管理向导的 GPUI 视图。

use super::keygen::KeyAlgorithm;
use crate::app::AppShell;
use crate::palette::Palette;
use crate::theme::tokens::size;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable, Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

impl AppShell {
    pub(crate) fn render_key_wizard(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        if self.modals.settings.key_wizard.is_none() {
            return div().into_any_element();
        }
        let generate = self.render_key_generate_section(p, cx);
        let list = self.render_key_list_section(p, cx);
        let deploy = self.render_key_deploy_section(p, cx);
        let message = self
            .modals
            .settings
            .key_wizard
            .as_ref()
            .and_then(|state| state.message.clone());
        let error = self
            .modals
            .settings
            .key_wizard
            .as_ref()
            .and_then(|state| state.error.clone());

        let content = v_flex()
            .id(ElementId::Name("key-wizard-content".into()))
            .gap_4()
            .max_h(px(470.0))
            .overflow_y_scroll()
            .child(generate)
            .child(list)
            .child(deploy)
            .when_some(message, |this, message| {
                this.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .px_3()
                        .py_2()
                        .rounded(p.radius.md)
                        .bg(p.muted)
                        .child(Icon::new(IconName::Check).small().text_color(p.success))
                        .child(div().text_xs().text_color(p.foreground).child(message)),
                )
            })
            .when_some(error, |this, error| {
                this.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded(p.radius.md)
                        .bg(p.danger)
                        .text_xs()
                        .text_color(p.danger_foreground)
                        .child(error),
                )
            });

        let footer = Button::new("close-key-wizard")
            .ghost()
            .label(rust_i18n::t!("app.close").to_string())
            .on_click(cx.listener(|this, _, _, cx| this.close_key_wizard(cx)));

        let dialog = Self::dialog_frame(
            "key-wizard-modal",
            rust_i18n::t!("keys.title").to_string(),
            Some(rust_i18n::t!("keys.hint").to_string()),
            content,
            Some(footer),
            rems(size::DIALOG_MAX_WIDTH),
            p,
        );
        Self::overlay(dialog, p)
    }

    fn render_key_generate_section(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(state) = self.modals.settings.key_wizard.as_ref() else {
            return div().into_any_element();
        };
        let current_algorithm = state.algorithm;
        let generating = state.generating;
        let comment = state.comment.clone();
        let path = state.path.clone();
        let passphrase = state.passphrase.clone();

        v_flex()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child(rust_i18n::t!("keys.generate_title").to_string()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("keys.algorithm").to_string()),
                    )
                    .children(
                        [
                            ("key-alg-ed25519", KeyAlgorithm::Ed25519),
                            ("key-alg-rsa", KeyAlgorithm::Rsa),
                        ]
                        .into_iter()
                        .map(|(id, algorithm)| {
                            Button::new(id)
                                .small()
                                .label(algorithm.label())
                                .selected(current_algorithm == algorithm)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.select_key_algorithm(algorithm, window, cx);
                                }))
                        }),
                    ),
            )
            .child(Self::form_field(
                rust_i18n::t!("keys.comment").to_string(),
                comment,
                p,
            ))
            .child(Self::form_field(
                rust_i18n::t!("keys.path").to_string(),
                path,
                p,
            ))
            .child(Self::form_field(
                rust_i18n::t!("keys.passphrase").to_string(),
                passphrase,
                p,
            ))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("keys.passphrase_hint").to_string()),
                    )
                    .child(
                        Button::new("generate-key-pair")
                            .primary()
                            .small()
                            .icon(IconName::KeyRound)
                            .label(if generating {
                                rust_i18n::t!("keys.generating").to_string()
                            } else {
                                rust_i18n::t!("keys.generate").to_string()
                            })
                            .disabled(generating)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.generate_key(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_key_list_section(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(state) = self.modals.settings.key_wizard.as_ref() else {
            return div().into_any_element();
        };
        let keys = state.keys.clone();
        let selected = state.selected;
        let rows: Vec<AnyElement> =
            keys.into_iter()
                .enumerate()
                .map(|(index, key)| {
                    let is_selected = selected == Some(index);
                    let file_name = key
                        .path
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| key.path.display().to_string());
                    let mut row =
                        h_flex()
                            .id(ElementId::Name(format!("key-row-{index}").into()))
                            .w_full()
                            .px_3()
                            .py_2()
                            .gap_3()
                            .items_center()
                            .rounded(p.radius.md)
                            .border_1()
                            .cursor_pointer()
                            .border_color(if is_selected {
                                p.sidebar_accent_foreground
                            } else {
                                p.border
                            })
                            .bg(if is_selected {
                                p.sidebar_accent
                            } else {
                                p.muted.opacity(0.35)
                            })
                            .hover(|style| {
                                if is_selected {
                                    style
                                } else {
                                    style.border_color(p.muted_foreground)
                                }
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_key(index, cx);
                            }))
                            .child(Icon::new(IconName::KeyRound).small().text_color(
                                if is_selected {
                                    p.sidebar_accent_foreground
                                } else {
                                    p.muted_foreground
                                },
                            ))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_1()
                                    .overflow_hidden()
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .items_center()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child(file_name),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .px_1p5()
                                                    .py_0p5()
                                                    .rounded(p.radius.sm)
                                                    .bg(p.background)
                                                    .text_color(p.muted_foreground)
                                                    .child(key.algorithm),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(p.muted_foreground)
                                            .truncate()
                                            .child(key.fingerprint),
                                    )
                                    .when(!key.comment.is_empty(), |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(p.muted_foreground)
                                                .truncate()
                                                .child(key.comment),
                                        )
                                    }),
                            );
                    if is_selected {
                        row = row.child(
                            Icon::new(IconName::Check)
                                .small()
                                .text_color(p.sidebar_accent_foreground),
                        );
                    }
                    row.into_any_element()
                })
                .collect();

        v_flex()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child(rust_i18n::t!("keys.existing_title").to_string()),
            )
            .when(rows.is_empty(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("keys.empty").to_string()),
                )
            })
            .children(rows)
            .into_any_element()
    }

    fn render_key_deploy_section(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(state) = self.modals.settings.key_wizard.as_ref() else {
            return div().into_any_element();
        };
        let selected_key = state
            .selected
            .and_then(|index| state.keys.get(index))
            .cloned();
        let targets = self.key_targets();

        let selected_summary = selected_key.as_ref().map(|key| {
            let file_name = key
                .path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| key.path.display().to_string());
            h_flex()
                .justify_between()
                .items_center()
                .gap_3()
                .child(
                    v_flex()
                        .gap_1()
                        .overflow_hidden()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(file_name),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .truncate()
                                .child(key.fingerprint.clone()),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("copy-selected-public-key")
                                .small()
                                .icon(IconName::Copy)
                                .label(rust_i18n::t!("common.copy").to_string())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.copy_selected_key(cx);
                                })),
                        )
                        .child(
                            Button::new("export-selected-public-key")
                                .small()
                                .icon(IconName::Download)
                                .label(rust_i18n::t!("keys.export").to_string())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.export_selected_key(cx);
                                })),
                        ),
                )
        });

        let deploy_buttons: Vec<AnyElement> = targets
            .into_iter()
            .map(|target| {
                let label =
                    rust_i18n::t!("keys.deploy_to", name = target.title.clone()).to_string();
                Button::new(ElementId::Name(
                    format!("deploy-key-{}", target.tab_id).into(),
                ))
                .small()
                .icon(IconName::Server)
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.deploy_selected_key(target.clone(), cx);
                }))
                .into_any_element()
            })
            .collect();

        v_flex()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child(rust_i18n::t!("keys.deploy_title").to_string()),
            )
            .when_some(selected_summary, |this, summary| this.child(summary))
            .when(selected_key.is_none(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("keys.select_first").to_string()),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("keys.deploy_hint").to_string()),
            )
            .when(deploy_buttons.is_empty(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("keys.no_connections").to_string()),
                )
            })
            .children(deploy_buttons)
            .into_any_element()
    }
}
