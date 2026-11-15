//! 容器重命名输入条：打开、取消与提交。

use super::*;

impl ContainerWorkspace {
    /// 打开容器改名条。
    pub(crate) fn open_rename(
        &mut self,
        id: String,
        current: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(current));
        self.state.modals.rename_container = Some((id, input));
        cx.notify();
    }

    /// 取消改名：丢弃本次打开期的输入控件。
    pub(crate) fn cancel_container_rename(&mut self, cx: &mut Context<Self>) {
        self.state.modals.rename_container = None;
        cx.notify();
    }

    /// 提交改名：名称留空或与原名相同都不下发，避免无意义的请求与 409。
    ///
    /// 校验失败时弹窗保持打开、输入保留，提示经
    /// [`ContainerWorkspaceEvent::ShowToast`] 交给外壳；成功投递命令后立即清空
    /// 改名状态，弹窗随之关闭，结果事件只负责收尾忙碌状态与刷新。
    pub(crate) fn submit_container_rename(&mut self, cx: &mut Context<Self>) {
        let Some((id, input)) = self.state.modals.rename_container.clone() else {
            return;
        };
        let name = input.read(cx).value().trim().to_string();
        let same = self
            .state
            .containers
            .iter()
            .any(|item| item.id == id && item.name == name);
        if name.is_empty() || same {
            cx.emit(ContainerWorkspaceEvent::ShowToast {
                message: rust_i18n::t!("container.rename.invalid", name = name).to_string(),
                success: false,
            });
            return;
        }
        let request_id = ContainerRequestId::new();
        self.state.busy = true;
        self.state.modals.rename_container = None;
        self.send_tracked(
            request_id,
            ContainerCommand::RenameContainer {
                tab_id: self.tab_id,
                request_id,
                container: id,
                name,
            },
        );
        cx.notify();
    }
}
