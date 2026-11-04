//! 容器就地提交新镜像（Commit）输入条：打开、取消与提交。
//!
//! 设计原理（Why）：
//! - 在排查现场故障或保存特定测试状态时，允许开发者就地将容器的可写层与
//!   运行时环境固化为一个新的 Docker 镜像；
//! - 以条状输入形式嵌入在容器详情头部下方，避免弹出全屏模态打断上下文；
//! - 自动为 Tag 预填充带当前时间戳的默认值，降低用户输入负担。

use super::*;
use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use chrono::Local;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Icon, Sizable};
use transom_core::TabId;

impl AppShell {
    /// 打开容器提交镜像输入条。
    pub(crate) fn open_container_commit(
        &mut self,
        tab_id: TabId,
        container: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let clean_name = container.trim_start_matches('/').to_string();
        let default_tag = format!("snapshot-{}", Local::now().format("%Y%m%d-%H%M%S"));

        let repo_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(clean_name)
                .placeholder(rust_i18n::t!("container.commit.repo_placeholder").to_string())
        });
        let tag_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(default_tag)
                .placeholder(rust_i18n::t!("container.commit.tag_placeholder").to_string())
        });
        let comment_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.commit.comment_placeholder").to_string())
        });

        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.commit_container =
                Some((container, repo_input, tag_input, comment_input));
        }
        cx.notify();
    }

    /// 取消容器提交。
    pub(crate) fn cancel_container_commit(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.commit_container = None;
        }
        cx.notify();
    }

    /// 确认并提交新镜像。
    pub(crate) fn submit_container_commit(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((container, repo_input, tag_input, comment_input)) = self
            .state
            .container_workspace(tab_id)
            .and_then(|workspace| workspace.commit_container.clone())
        else {
            return;
        };

        let repo = repo_input.read(cx).value().trim().to_string();
        if repo.is_empty() {
            self.show_toast(
                rust_i18n::t!("container.commit.repo_required").to_string(),
                false,
                window,
                cx,
            );
            return;
        }

        let tag_text = tag_input.read(cx).value().trim().to_string();
        let tag = if tag_text.is_empty() {
            None
        } else {
            Some(tag_text)
        };

        let comment_text = comment_input.read(cx).value().trim().to_string();
        let comment = if comment_text.is_empty() {
            None
        } else {
            Some(comment_text)
        };

        let request_id = ContainerRequestId::new();
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.busy = true;
            workspace.commit_container = None;
        }

        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::CommitContainer {
                tab_id,
                request_id,
                container,
                repo,
                tag,
                comment,
                author: None,
            },
        );
        cx.notify();
    }

    /// 渲染容器提交镜像输入条。
    pub(crate) fn render_container_commit_bar(
        &mut self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<AppShell>,
    ) -> Option<AnyElement> {
        let (_, repo_input, tag_input, comment_input) = self
            .state
            .container_workspace(tab_id)?
            .commit_container
            .clone()?;

        Some(
            h_flex()
                .gap_1p5()
                .px_2()
                .py_1()
                .items_center()
                .bg(p.sidebar_accent.opacity(0.12))
                .border_color(p.divider(p.border))
                .divider_edge(DividerEdge::Bottom, p.divider_width)
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .flex_none()
                        .child(Icon::new(IconName::Layers).xsmall().text_color(p.primary))
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("container.commit.label").to_string()),
                        ),
                )
                .child(Input::new(&repo_input).w(px(140.0)).min_w_0())
                .child(Input::new(&tag_input).w(px(160.0)).min_w_0())
                .child(Input::new(&comment_input).flex_1().min_w_0())
                .child(
                    Button::new("container-commit-submit")
                        .primary()
                        .xsmall()
                        .icon(IconName::Check)
                        .label(rust_i18n::t!("common.confirm").to_string())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.submit_container_commit(tab_id, window, cx);
                        })),
                )
                .child(
                    Button::new("container-commit-cancel")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .label(rust_i18n::t!("common.cancel").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.cancel_container_commit(tab_id, cx);
                        })),
                )
                .into_any_element(),
        )
    }
}
