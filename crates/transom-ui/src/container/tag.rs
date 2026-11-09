//! 镜像打标签状态管理：打开、取消与提交。
//!
//! 设计原理（Why）：通过居中模态弹窗支持用户输入新 tag
//! （如 `my-app:v1.0.0` 或 `registry.example.com/repo:tag`），
//! 输入完成下发 TagImage 命令，并在成功后自动刷新镜像列表。
//!
//! 动作归属（Why）：打开、取消与提交只读写本标签页的状态并投递命令，
//! 不依赖外壳的目录或窗口级弹窗，因此全部落在 [`ContainerWorkspace`] 上；
//! 校验失败的一次性提示经 [`ContainerWorkspaceEvent::ShowToast`] 交给外壳。

use super::*;
use gpui_kit::component::input::InputState;

impl ContainerWorkspace {
    /// 打开镜像打标签弹窗。
    pub(crate) fn open_image_tag(
        &mut self,
        image: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("container.tag.placeholder").to_string())
        });
        self.state.tag_image = Some((image, input));
        cx.notify();
    }

    /// 取消镜像打标签。
    pub(crate) fn cancel_image_tag(&mut self, cx: &mut Context<Self>) {
        self.state.tag_image = None;
        cx.notify();
    }

    /// 提交镜像打标签。
    ///
    /// 校验失败时弹窗保持打开、输入保留，提示经
    /// [`ContainerWorkspaceEvent::ShowToast`] 交给外壳；成功投递命令后立即清空
    /// 打标签状态，弹窗随之关闭。
    pub(crate) fn submit_image_tag(&mut self, cx: &mut Context<Self>) {
        let Some((image, input)) = self.state.tag_image.clone() else {
            return;
        };

        let raw = input.read(cx).text().to_string();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            cx.emit(ContainerWorkspaceEvent::ShowToast {
                message: rust_i18n::t!("container.tag.invalid").to_string(),
                success: false,
            });
            return;
        }

        // 解析 repo 和 tag：若包含冒号则拆分为 repo 和 tag。
        let (repo, tag) = match trimmed.rsplit_once(':') {
            Some((r, t)) if !r.is_empty() && !t.contains('/') => {
                (r.to_string(), Some(t.to_string()))
            }
            _ => (trimmed.to_string(), None),
        };

        let request_id = ContainerRequestId::new();
        self.state.busy = true;
        self.state.tag_image = None;
        self.send_tracked(
            request_id,
            ContainerCommand::TagImage {
                tab_id: self.tab_id,
                request_id,
                image,
                repo,
                tag,
            },
        );
        cx.notify();
    }
}
