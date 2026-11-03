//! 容器资源动态更新输入条：打开、取消与提交。
//!
//! 设计原理（Why）：直接在详情头部展开资源更新条，与容器重命名保持一致，
//! 避免弹出一层笨重的全量模态；支持独立或组合调整内存配额与 CPU 核数限制。

use super::*;
use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::Sizable;
use transom_core::{ContainerUpdateSpec, TabId};

impl AppShell {
    /// 打开容器资源更新输入条。
    pub(crate) fn open_container_update(
        &mut self,
        tab_id: TabId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mem_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(
                rust_i18n::t!("container.update_resources.memory_placeholder").to_string(),
            )
        });
        let cpu_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(
                rust_i18n::t!("container.update_resources.cpu_placeholder").to_string(),
            )
        });
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.update_container = Some((name, mem_input, cpu_input));
        }
        cx.notify();
    }

    /// 取消资源更新。
    pub(crate) fn cancel_container_update(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.update_container = None;
        }
        cx.notify();
    }

    /// 提交资源更新。
    pub(crate) fn submit_container_update(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some((name, mem_input, cpu_input)) = self
            .state
            .container_workspace(tab_id)
            .and_then(|ws| ws.update_container.clone())
        else {
            return;
        };

        let mem_text = mem_input.read(cx).text().to_string();
        let mem_text = mem_text.trim();
        let cpu_text = cpu_input.read(cx).text().to_string();
        let cpu_text = cpu_text.trim();

        let memory_bytes = if mem_text.is_empty() {
            None
        } else if let Ok(mb) = mem_text.parse::<u64>() {
            Some(mb.saturating_mul(1024 * 1024))
        } else {
            None
        };

        let nano_cpus = if cpu_text.is_empty() {
            None
        } else if let Ok(cores) = cpu_text.parse::<f64>() {
            if cores > 0.0 {
                Some((cores * 1_000_000_000.0) as u64)
            } else {
                None
            }
        } else {
            None
        };

        let spec = ContainerUpdateSpec {
            memory_bytes,
            nano_cpus,
            restart_policy: None,
        };

        self.cancel_container_update(tab_id, cx);
        self.update_container_resources(tab_id, name, spec, cx);
    }

    /// 渲染容器资源更新条。
    pub(crate) fn render_container_update_bar(
        &mut self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<AppShell>,
    ) -> Option<AnyElement> {
        let (_, mem_input, cpu_input) = self
            .state
            .container_workspace(tab_id)?
            .update_container
            .clone()?;

        Some(
            h_flex()
                .gap_1()
                .px_2()
                .py_1()
                .items_center()
                .border_color(p.divider(p.border))
                .divider_edge(DividerEdge::Bottom, p.divider_width)
                .child(
                    div()
                        .text_xs()
                        .flex_none()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.update_resources.label").to_string()),
                )
                .child(Input::new(&mem_input).w(px(140.0)).min_w_0())
                .child(Input::new(&cpu_input).w(px(140.0)).min_w_0())
                .child(
                    Button::new("container-update-submit")
                        .primary()
                        .xsmall()
                        .icon(IconName::Check)
                        .label(rust_i18n::t!("common.confirm").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.submit_container_update(tab_id, cx);
                        })),
                )
                .child(
                    Button::new("container-update-cancel")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .label(rust_i18n::t!("common.cancel").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.cancel_container_update(tab_id, cx);
                        })),
                )
                .into_any_element(),
        )
    }
}
