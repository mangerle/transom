//! 网络连接容器输入条：展开、取消与提交。
//!
//! 设计原理（Why）：直接在网络详情头部展开内联输入条，输入容器名称或 ID 即可快速
//! 将容器接入当前网络，无需复杂多步弹窗；操作成功后自动刷新网络与容器列表。

use super::*;
use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::Sizable;
use transom_core::{ContainerCommand, ContainerRequestId, TabId};

impl AppShell {
    /// 打开网络连接容器输入条。
    pub(crate) fn open_network_connect(
        &mut self,
        tab_id: TabId,
        network: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(
                rust_i18n::t!("container.network_connect.placeholder").to_string(),
            )
        });
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.connect_network = Some((network, input));
        }
        cx.notify();
    }

    /// 取消网络连接容器。
    pub(crate) fn cancel_network_connect(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.connect_network = None;
        }
        cx.notify();
    }

    /// 提交将容器连入当前网络。
    pub(crate) fn submit_network_connect(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((network, input)) = self
            .state
            .container_workspace(tab_id)
            .and_then(|workspace| workspace.connect_network.clone())
        else {
            return;
        };

        let raw = input.read(cx).text().to_string();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            self.show_toast(
                rust_i18n::t!("container.network_connect.invalid").to_string(),
                false,
                window,
                cx,
            );
            return;
        }

        let container = trimmed.to_string();
        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.busy = true;
            workspace.connect_network = None;
        }

        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::ConnectNetwork {
                tab_id,
                request_id,
                network,
                container,
            },
        );
        cx.notify();
    }

    /// 渲染网络连接容器输入条。
    pub(crate) fn render_network_connect_bar(
        &mut self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (_network, input) = self
            .state
            .container_workspace(tab_id)
            .and_then(|workspace| workspace.connect_network.clone())?;

        Some(
            h_flex()
                .gap_2()
                .items_center()
                .px_3()
                .py_1p5()
                .border_color(p.divider(p.border))
                .divider_edge(DividerEdge::Bottom, p.divider_width)
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.network_connect.label").to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&input).xsmall()),
                )
                .child(
                    Button::new("network-connect-submit")
                        .xsmall()
                        .primary()
                        .icon(IconName::Check)
                        .label(rust_i18n::t!("common.confirm").to_string())
                        .on_click(cx.listener({
                            move |this, _, window, cx| {
                                this.submit_network_connect(tab_id, window, cx);
                            }
                        })),
                )
                .child(
                    Button::new("network-connect-cancel")
                        .xsmall()
                        .ghost()
                        .icon(IconName::Close)
                        .label(rust_i18n::t!("common.cancel").to_string())
                        .on_click(cx.listener({
                            move |this, _, _, cx| {
                                this.cancel_network_connect(tab_id, cx);
                            }
                        })),
                )
                .into_any_element(),
        )
    }
}
