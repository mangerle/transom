//! 会话表单中的串口设备枚举辅助。

use crate::app::AppShell;
use gpui_kit::*;

impl AppShell {
    /// 重新枚举系统串口并写回当前会话表单。
    ///
    /// 设备枚举可能因权限或平台后端失败，此时保留手动输入能力，仅展示错误
    /// 提示；若设备路径为空则自动选中第一个可用端口，减少新建会话的操作步骤。
    pub(crate) fn refresh_serial_ports(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.state.services.serial_ports.list();
        let view = self.state.modals.session.form.clone();
        view.update(cx, |view, cx| view.apply_serial_ports(result, window, cx));
        cx.notify();
    }
}
