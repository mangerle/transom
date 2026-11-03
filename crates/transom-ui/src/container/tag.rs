//! 镜像打标签输入条：打开、取消与提交。
//!
//! 设计原理（Why）：直接在镜像详情头部展开标签输入条，支持用户输入新 tag
//! （如 `my-app:v1.0.0` 或 `registry.example.com/repo:tag`），
//! 输入完成下发 TagImage 命令，并在成功后自动刷新镜像列表。

use super::*;
use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::Sizable;
use transom_core::TabId;

impl AppShell {
    /// 打开镜像打标签输入条。
    pub(crate) fn open_image_tag(
        &mut self,
        tab_id: TabId,
        image: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(
                rust_i18n::t!("container.tag.placeholder").to_string(),
            )
        });
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.tag_image = Some((image, input));
        }
        cx.notify();
    }

    /// 取消镜像打标签。
    pub(crate) fn cancel_image_tag(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.tag_image = None;
        }
        cx.notify();
    }

    /// 提交镜像新标签。
    pub(crate) fn submit_image_tag(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((image, input)) = self
            .state
            .container_workspace(tab_id)
            .and_then(|workspace| workspace.tag_image.clone())
        else {
            return;
        };

        let raw = input.read(cx).text().to_string();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            self.show_toast(
                rust_i18n::t!("container.tag.invalid").to_string(),
                false,
                window,
                cx,
            );
            return;
        }

        // 解析 repo 和 tag：若包含冒号则拆分为 repo 和 tag
        let (repo, tag) = match trimmed.rsplit_once(':') {
            Some((r, t)) if !r.is_empty() && !t.contains('/') => (r.to_string(), Some(t.to_string())),
            _ => (trimmed.to_string(), None),
        };

        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.busy = true;
            workspace.tag_image = None;
        }

        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::TagImage {
                tab_id,
                request_id,
                image,
                repo,
                tag,
            },
        );
        cx.notify();
    }

    /// 渲染镜像打标签输入条。
    pub(crate) fn render_image_tag_bar(
        &mut self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (_image, input) = self
            .state
            .container_workspace(tab_id)
            .and_then(|workspace| workspace.tag_image.clone())?;

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
                        .child(rust_i18n::t!("container.tag.label").to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&input).xsmall()),
                )
                .child(
                    Button::new("tag-submit")
                        .xsmall()
                        .primary()
                        .icon(IconName::Check)
                        .label(rust_i18n::t!("common.confirm").to_string())
                        .on_click(cx.listener({
                            move |this, _, window, cx| {
                                this.submit_image_tag(tab_id, window, cx);
                            }
                        })),
                )
                .child(
                    Button::new("tag-cancel")
                        .xsmall()
                        .ghost()
                        .icon(IconName::Close)
                        .label(rust_i18n::t!("common.cancel").to_string())
                        .on_click(cx.listener({
                            move |this, _, _, cx| {
                                this.cancel_image_tag(tab_id, cx);
                            }
                        })),
                )
                .into_any_element(),
        )
    }
}
