//! 容器配置反向导出为运行命令与 Compose 片段展示条。
//!
//! 设计原理（Why）：
//! - 将容器的当前配置反向导出为可重现的命令与编排片段，方便开发者备份、
//!   跨机迁移或排查参数差异；
//! - 以展开条（Drawer / Banner）形式就地展示在详情上方，避免模态遮挡主要
//!   元数据，同时支持快捷一键复制到剪贴板。

use super::*;
use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use gpui_kit::ClipboardItem;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;

impl AppShell {
    /// 打开容器配置反向导出条。
    pub(crate) fn open_container_export(
        &mut self,
        tab_id: TabId,
        name: String,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(ref details) = workspace.details else {
            return;
        };
        let run_cmd = transom_core::generate_docker_run(details);
        let compose_yaml = transom_core::generate_docker_compose(details);
        workspace.export_config = Some((name, run_cmd, compose_yaml, false));
        cx.notify();
    }

    /// 关闭导出条。
    pub(crate) fn cancel_container_export(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.export_config = None;
        }
        cx.notify();
    }

    /// 切换导出格式（Docker Run / Docker Compose）。
    pub(crate) fn toggle_container_export_mode(
        &mut self,
        tab_id: TabId,
        is_compose: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            if let Some((_, _, _, ref mut cur)) = workspace.export_config {
                *cur = is_compose;
            }
        }
        cx.notify();
    }

    /// 复制当前导出的配置代码到系统剪贴板。
    pub(crate) fn copy_container_export(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, run_cmd, compose_yaml, is_compose)) = self
            .state
            .container_workspace(tab_id)
            .and_then(|workspace| workspace.export_config.as_ref())
        else {
            return;
        };
        let text = if *is_compose {
            compose_yaml.clone()
        } else {
            run_cmd.clone()
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.show_toast(
            rust_i18n::t!("container.export.copied").to_string(),
            true,
            window,
            cx,
        );
    }

    /// 渲染容器配置导出展开条。
    pub(crate) fn render_container_export_bar(
        &mut self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<AppShell>,
    ) -> Option<AnyElement> {
        let (name, run_cmd, compose_yaml, is_compose) = self
            .state
            .container_workspace(tab_id)?
            .export_config
            .clone()?;

        let title = rust_i18n::t!("container.export.title", name = name).to_string();
        let display_content = if is_compose {
            compose_yaml
        } else {
            run_cmd
        };

        Some(
            v_flex()
                .gap_2()
                .p_2()
                .bg(p.sidebar_accent.opacity(0.18))
                .border_color(p.divider(p.border))
                .divider_edge(DividerEdge::Bottom, p.divider_width)
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .justify_between()
                        .child(
                            h_flex()
                                .gap_1p5()
                                .items_center()
                                .child(Icon::new(IconName::FileText).xsmall().text_color(p.primary))
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(p.foreground)
                                        .child(title),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .child(
                                    Button::new("export-mode-run")
                                        .xsmall()
                                        .selected(!is_compose)
                                        .label(rust_i18n::t!("container.export.docker_run").to_string())
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.toggle_container_export_mode(tab_id, false, cx);
                                        })),
                                )
                                .child(
                                    Button::new("export-mode-compose")
                                        .xsmall()
                                        .selected(is_compose)
                                        .label(rust_i18n::t!("container.export.compose").to_string())
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.toggle_container_export_mode(tab_id, true, cx);
                                        })),
                                )
                                .child(
                                    Button::new("export-copy-btn")
                                        .primary()
                                        .xsmall()
                                        .icon(IconName::Copy)
                                        .label(rust_i18n::t!("container.export.copy").to_string())
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.copy_container_export(tab_id, window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("export-close-btn")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Close)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.cancel_container_export(tab_id, cx);
                                        })),
                                ),
                        ),
                )
                .child(
                    div()
                        .id("container-export-scroll")
                        .w_full()
                        .max_h(px(220.0))
                        .overflow_y_scroll()
                        .overflow_x_scroll()
                        .bg(p.background)
                        .border_1()
                        .border_color(p.divider(p.border))
                        .rounded_md()
                        .p_2()
                        .font_family(p.mono_font_family.clone())
                        .text_size(p.mono_font_size)
                        .text_color(p.foreground)
                        .child(display_content),
                )
                .into_any_element(),
        )
    }
}
