//! 容器就地提交新镜像（Commit）输入条：打开、取消与提交。
//!
//! 设计原理（Why）：
//! - 在排查现场故障或保存特定测试状态时，允许开发者就地将容器的可写层与
//!   运行时环境固化为一个新的 Docker 镜像；
//! - 以条状输入形式嵌入在容器详情头部下方，避免弹出全屏模态打断上下文；
//! - 自动为 Tag 预填充带当前时间戳的默认值，降低用户输入负担。

use super::*;
use crate::app::AppShell;
use chrono::Local;
use gpui_kit::component::input::InputState;
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

        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.commit_container = Some(ContainerCommitState {
                container,
                repo: repo_input,
                tag: tag_input,
                comment: comment_input,
            });
        });
        cx.notify();
    }

    /// 取消容器提交。
    pub(crate) fn cancel_container_commit(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.commit_container = None;
        });
        cx.notify();
    }

    /// 确认并提交新镜像。
    pub(crate) fn submit_container_commit(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self
            .state
            .container_workspace(tab_id)
            .and_then(|workspace| workspace.read(cx).state.commit_container.clone())
        else {
            return;
        };
        let container = state.container;
        let repo_input = state.repo;
        let tag_input = state.tag;
        let comment_input = state.comment;

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
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.busy = true;
            workspace.commit_container = None;
        });

        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(
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
            });
        }
        cx.notify();
    }
}
