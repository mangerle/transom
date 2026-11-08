//! 镜像打标签状态管理：打开、取消与提交。
//!
//! 设计原理（Why）：通过居中模态弹窗支持用户输入新 tag
//! （如 `my-app:v1.0.0` 或 `registry.example.com/repo:tag`），
//! 输入完成下发 TagImage 命令，并在成功后自动刷新镜像列表。

use super::*;
use crate::app::AppShell;
use gpui_kit::component::input::InputState;
use transom_core::TabId;

impl AppShell {
    /// 打开镜像打标签弹窗。
    pub(crate) fn open_image_tag(
        &mut self,
        tab_id: TabId,
        image: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.tag.placeholder").to_string())
        });
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.tag_image = Some((image, input));
        });
        cx.notify();
    }

    /// 取消镜像打标签。
    pub(crate) fn cancel_image_tag(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.tag_image = None;
        });
        cx.notify();
    }

    /// 提交镜像打标签。
    pub(crate) fn submit_image_tag(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((image, input)) = self
            .with_container(tab_id, cx, |workspace| workspace.tag_image.clone())
            .flatten()
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
            Some((r, t)) if !r.is_empty() && !t.contains('/') => {
                (r.to_string(), Some(t.to_string()))
            }
            _ => (trimmed.to_string(), None),
        };

        let request_id = ContainerRequestId::new();
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.busy = true;
            workspace.tag_image = None;
        });

        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(
                    request_id,
                    ContainerCommand::TagImage {
                        tab_id,
                        request_id,
                        image,
                        repo,
                        tag,
                    },
                );
            });
        }
        cx.notify();
    }
}
