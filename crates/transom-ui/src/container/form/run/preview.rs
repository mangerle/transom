//! 运行容器表单右侧预览区：镜像元数据、运行预设模板与实时 Docker CLI 命令行预览。
//!
//! 设计原理（Why）：
//! - 直观呈现根据当前表单动态拼装的 `docker run` 命令，消除图形界面的配置黑盒感；
//! - 提供后台服务与交互排查双预设切换，并支持一键将完整命令复制到系统剪贴板；
//! - 预览与预设动作都归 [`ContainerRunFormView`]，镜像摘要由弹窗宿主在只读
//!   外壳里快照注入，实体不反查工作区。

use super::ContainerRunFormView;
use crate::container::types::{RunContainerFormState, RunPreset};
use crate::format::format_size;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::ContainerImageSummary;

impl ContainerRunFormView {
    /// 渲染右侧预览与摘要区；镜像摘要由外壳按当前镜像输入匹配后注入。
    pub(super) fn render_run_preview_panel(
        &self,
        form: &RunContainerFormState,
        image_meta: Option<&ContainerImageSummary>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let preview_cmd = form.generate_preview_command(cx);
        let cmd_to_copy = preview_cmd.clone();
        let copy_toast = form.copy_toast;
        let current_preset = form.preset;
        let image_raw = form.image.read(cx).value().trim().to_string();

        v_flex()
            .w(px(270.0))
            .min_w(px(240.0))
            .gap_3()
            .pl_3()
            .border_l_1()
            .border_color(p.border.opacity(0.18))
            // 1. 快速预设模式切换
            .child(
                v_flex()
                    .gap_1p5()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.foreground)
                            .child(rust_i18n::t!("container.run.preset_title").to_string()),
                    )
                    .child(
                        h_flex()
                            .gap_1p5()
                            .child(
                                Button::new("preset-daemon-btn")
                                    .small()
                                    .ghost()
                                    .selected(current_preset == RunPreset::Daemon)
                                    .icon(IconName::Server)
                                    .label(rust_i18n::t!("container.run.preset_daemon").to_string())
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.set_preset(RunPreset::Daemon, cx);
                                    })),
                            )
                            .child(
                                Button::new("preset-interactive-btn")
                                    .small()
                                    .ghost()
                                    .selected(current_preset == RunPreset::Interactive)
                                    .icon(IconName::Terminal)
                                    .label(
                                        rust_i18n::t!("container.run.preset_interactive")
                                            .to_string(),
                                    )
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.set_preset(RunPreset::Interactive, cx);
                                    })),
                            ),
                    ),
            )
            // 2. 镜像元数据简要看板
            .child(
                v_flex()
                    .gap_1p5()
                    .p_2p5()
                    .rounded(p.radius.md)
                    .bg(p.muted.opacity(0.06))
                    .border_1()
                    .border_color(p.stroke.faint)
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .child(
                                Icon::new(IconName::Layers)
                                    .with_size(px(13.0))
                                    .text_color(p.primary),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.foreground)
                                    .child(rust_i18n::t!("container.run.meta_title").to_string()),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                h_flex()
                                    .justify_between()
                                    .text_xs()
                                    .child(
                                        div().text_color(p.muted_foreground).child(
                                            rust_i18n::t!("container.run.image").to_string(),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(p.foreground)
                                            .child(if image_raw.is_empty() {
                                                "—".to_string()
                                            } else {
                                                image_raw
                                            }),
                                    ),
                            )
                            .when_some(image_meta, |this, meta| {
                                this.child(
                                    h_flex()
                                        .justify_between()
                                        .text_xs()
                                        .child(div().text_color(p.muted_foreground).child(
                                            rust_i18n::t!("container.run.meta_size").to_string(),
                                        ))
                                        .child(
                                            div()
                                                .font_family(p.mono_font_family.clone())
                                                .text_color(p.muted_foreground)
                                                .child(format_size(meta.size)),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .justify_between()
                                        .text_xs()
                                        .child(div().text_color(p.muted_foreground).child(
                                            rust_i18n::t!("container.run.meta_id").to_string(),
                                        ))
                                        .child(
                                            div()
                                                .font_family(p.mono_font_family.clone())
                                                .text_color(p.muted_foreground)
                                                .child(transom_core::short_id(&meta.id)),
                                        ),
                                )
                            }),
                    ),
            )
            // 3. 实时 Docker CLI 命令行预览黑盒
            .child(
                v_flex()
                    .flex_1()
                    .min_h(px(130.0))
                    .gap_1p5()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p.foreground)
                                    .child(
                                        rust_i18n::t!("container.run.preview_title").to_string(),
                                    ),
                            )
                            .child(
                                Button::new("copy-run-cmd-btn")
                                    .ghost()
                                    .xsmall()
                                    .icon(if copy_toast {
                                        IconName::Check
                                    } else {
                                        IconName::Copy
                                    })
                                    .label(if copy_toast {
                                        rust_i18n::t!("container.run.copied").to_string()
                                    } else {
                                        rust_i18n::t!("container.run.copy_command").to_string()
                                    })
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.copy_command(&cmd_to_copy, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .p_2p5()
                            .rounded(p.radius.md)
                            .bg(p.background.opacity(0.85))
                            .border_1()
                            .border_color(p.border.opacity(0.25))
                            .font_family(p.mono_font_family.clone())
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .overflow_x_scrollbar()
                            .child(preview_cmd),
                    ),
            )
            .into_any_element()
    }
}
