//! 密钥生成与管理向导的 GPUI 视图。
//!
//! 渲染期实体已被租借，因此这里只读 `state()`；按钮回调统一经 `cx.listener`
//! 回到 [`KeyWizardView`]。部署目标属于外壳的工作区状态，由调用方以
//! `&[KeyTarget]` 注入，实体不在渲染中反查外壳。

use super::entity::KeyWizardView;
use super::keygen::{KeyAlgorithm, PublicKeyInfo};
use super::state::{KeyTarget, KeyWizardState};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable, Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

impl KeyWizardView {
    /// 弹窗主体：生成区、公钥列表、部署区与页脚。
    pub(crate) fn render_body(
        &self,
        targets: &[KeyTarget],
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(state) = self.state() else {
            return div().into_any_element();
        };
        let content = v_flex()
            .id(ElementId::Name("key-wizard-content".into()))
            .gap_4()
            .max_h(px(470.0))
            .overflow_y_scroll()
            .child(render_generate_section(state, cx, p))
            .child(render_list_section(state, cx, p))
            .child(render_deploy_section(state, targets, cx, p))
            .when_some(state.message.clone(), |this, message| {
                this.child(crate::widgets::notice(
                    crate::widgets::NoticeKind::Success,
                    message,
                    p,
                ))
            })
            .when_some(state.error.clone(), |this, error| {
                this.child(crate::widgets::notice(
                    crate::widgets::NoticeKind::Error,
                    error,
                    p,
                ))
            });

        let footer = Button::new("close-key-wizard")
            .ghost()
            .label(rust_i18n::t!("app.close").to_string())
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx)));

        crate::widgets::dialog_body(
            "key-wizard-modal",
            rust_i18n::t!("keys.title").to_string(),
            Some(rust_i18n::t!("keys.hint").to_string()),
            content,
            Some(footer),
        )
    }
}

/// 生成区：算法切换与注释/路径/口令字段。
fn render_generate_section(
    state: &KeyWizardState,
    cx: &mut Context<KeyWizardView>,
    p: &Palette,
) -> AnyElement {
    let current_algorithm = state.algorithm;
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
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.select_algorithm(algorithm, window, cx);
                            }))
                    }),
                ),
        )
        .child(key_generate_fields(state, cx, p))
        .into_any_element()
}

/// 公钥清单区：每行可选，选中项显示勾选标记。
fn render_list_section(
    state: &KeyWizardState,
    cx: &mut Context<KeyWizardView>,
    p: &Palette,
) -> AnyElement {
    let selected = state.selected;
    let rows: Vec<AnyElement> = state
        .keys
        .iter()
        .enumerate()
        .map(|(index, key)| key_row(index, key, selected == Some(index), cx, p))
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

/// 部署区：已选公钥摘要、目标按钮与空态提示。
fn render_deploy_section(
    state: &KeyWizardState,
    targets: &[KeyTarget],
    cx: &mut Context<KeyWizardView>,
    p: &Palette,
) -> AnyElement {
    let selected_key = state.selected.and_then(|index| state.keys.get(index));
    let selected_summary = selected_key.map(|key| selected_key_summary(key, cx, p));

    let deploy_buttons: Vec<AnyElement> = targets
        .iter()
        .map(|target| {
            let label = rust_i18n::t!("keys.deploy_to", name = target.title.clone()).to_string();
            Button::new(ElementId::Name(
                format!("deploy-key-{}", target.tab_id).into(),
            ))
            .small()
            .icon(IconName::Server)
            .label(label)
            .on_click(cx.listener({
                let target = target.clone();
                move |this, _: &ClickEvent, _, cx| {
                    this.deploy_selected_key(&target, cx);
                }
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

/// 已选公钥摘要：文件名、指纹与复制/导出动作。
fn selected_key_summary(
    key: &PublicKeyInfo,
    cx: &mut Context<KeyWizardView>,
    p: &Palette,
) -> AnyElement {
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
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.copy_selected_key(cx)),
                        ),
                )
                .child(
                    Button::new("export-selected-public-key")
                        .small()
                        .icon(IconName::Download)
                        .label(rust_i18n::t!("keys.export").to_string())
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.export_selected_key(cx)),
                        ),
                ),
        )
        .into_any_element()
}

/// 单个公钥行：文件名、指纹/注释、选择态与当前标记。
fn key_row(
    index: usize,
    key: &PublicKeyInfo,
    is_selected: bool,
    cx: &mut Context<KeyWizardView>,
    p: &Palette,
) -> AnyElement {
    let file_name = key
        .path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| key.path.display().to_string());

    let mut row = h_flex()
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
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.select_key(index, cx);
        }))
        .child(
            Icon::new(IconName::KeyRound)
                .small()
                .text_color(if is_selected {
                    p.sidebar_accent_foreground
                } else {
                    p.muted_foreground
                }),
        )
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
                                .child(key.algorithm.clone()),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .truncate()
                        .child(key.fingerprint.clone()),
                )
                .when(!key.comment.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .truncate()
                            .child(key.comment.clone()),
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
}

/// 生成区字段：注释、路径、口令与生成按钮。
fn key_generate_fields(
    state: &KeyWizardState,
    cx: &mut Context<KeyWizardView>,
    p: &Palette,
) -> AnyElement {
    v_flex()
        .gap_3()
        .child(crate::widgets::form_field(
            rust_i18n::t!("keys.comment").to_string(),
            state.comment.clone(),
            p,
        ))
        .child(crate::widgets::form_field(
            rust_i18n::t!("keys.path").to_string(),
            state.path.clone(),
            p,
        ))
        .child(crate::widgets::form_field(
            rust_i18n::t!("keys.passphrase").to_string(),
            state.passphrase.clone(),
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
                        .label(if state.generating {
                            rust_i18n::t!("keys.generating").to_string()
                        } else {
                            rust_i18n::t!("keys.generate").to_string()
                        })
                        .disabled(state.generating)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.generate_key(window, cx);
                        })),
                ),
        )
        .into_any_element()
}
