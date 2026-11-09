//! 容器就地提交新镜像（Commit）输入条：打开、取消与提交。
//!
//! 设计原理（Why）：
//! - 在排查现场故障或保存特定测试状态时，允许开发者就地将容器的可写层与
//!   运行时环境固化为一个新的 Docker 镜像；
//! - 以居中模态弹窗形式呈现，避免打断详情页的上下文；
//! - 自动为 Tag 预填充带当前时间戳的默认值，降低用户输入负担。
//!
//! 动作归属（Why）：打开、取消与提交只读写本标签页状态并投递命令，全部落在
//! [`ContainerWorkspace`] 上；校验失败提示经事件交外壳弹出。

use super::*;
use chrono::Local;
use gpui_kit::component::input::InputState;

impl ContainerWorkspace {
    /// 打开容器提交镜像弹窗。
    pub(crate) fn open_container_commit(
        &mut self,
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

        self.state.commit_container = Some(ContainerCommitState {
            container,
            repo: repo_input,
            tag: tag_input,
            comment: comment_input,
        });
        cx.notify();
    }

    /// 取消容器提交。
    pub(crate) fn cancel_container_commit(&mut self, cx: &mut Context<Self>) {
        self.state.commit_container = None;
        cx.notify();
    }

    /// 确认并提交新镜像。
    ///
    /// 仓库名为空时保持弹窗打开并提示；成功后关闭弹窗并投递命令，
    /// 结果事件只负责收尾忙碌状态与刷新。
    pub(crate) fn submit_container_commit(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.commit_container.clone() else {
            return;
        };
        let container = state.container;

        let repo = state.repo.read(cx).value().trim().to_string();
        if repo.is_empty() {
            cx.emit(ContainerWorkspaceEvent::ShowToast {
                message: rust_i18n::t!("container.commit.repo_required").to_string(),
                success: false,
            });
            return;
        }

        let tag_text = state.tag.read(cx).value().trim().to_string();
        let tag = (!tag_text.is_empty()).then_some(tag_text);

        let comment_text = state.comment.read(cx).value().trim().to_string();
        let comment = (!comment_text.is_empty()).then_some(comment_text);

        let request_id = ContainerRequestId::new();
        self.state.busy = true;
        self.state.commit_container = None;
        self.send_tracked(
            request_id,
            ContainerCommand::CommitContainer {
                tab_id: self.tab_id,
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
}
