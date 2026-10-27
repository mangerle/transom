//! 会话表单中的串口设备枚举辅助。

use crate::app::AppShell;
use crate::palette::localized_error;
use gpui_kit::*;

impl AppShell {
    /// 重新枚举系统串口并写回当前会话表单。
    ///
    /// 设备枚举可能因权限或平台后端失败，此时保留手动输入能力，仅展示错误
    /// 提示；若设备路径为空则自动选中第一个可用端口，减少新建会话的操作步骤。
    pub(crate) fn refresh_serial_ports(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.services.serial_ports.list();
        let Some(form) = self.modals.form.as_mut() else {
            return;
        };
        match result {
            Ok(ports) => {
                form.serial_ports_error = None;
                form.serial_ports = ports;
                let current = form.serial_port.read(cx).value().trim().to_string();
                if current.is_empty()
                    && let Some(first) = form.serial_ports.first()
                {
                    let port_name = first.port_name.clone();
                    form.serial_port.update(cx, |state, cx| {
                        state.set_value(&port_name, window, cx);
                    });
                }
            }
            Err(error) => form.serial_ports_error = Some(localized_error(&error)),
        }
        cx.notify();
    }
}
