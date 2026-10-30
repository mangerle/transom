//! 终端键盘复制模式的入口动作。
//!
//! 设计原理（Why）：模式本身活在终端视图里（每个连接一份），应用侧只负责
//! "把当前焦点窗格的切换请求转过去"。状态栏提示不走这里，而是由视图发出的
//! `CopyModeChanged` 事件统一维护——进入与退出可能来自按键、鼠标或复制动作，
//! 只在一处响应才不会出现"某个出口没清提示"的半态。

use crate::app::AppShell;
use crate::keymap;
use gpui_kit::*;

impl AppShell {
    pub(crate) fn on_toggle_copy_mode(
        &mut self,
        _: &keymap::ToggleCopyMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = self.active_pane_terminal() else {
            return;
        };
        // 先把焦点交给终端：从侧栏/面板触发时焦点还在输入框里，模式虽然
        // 打开了，后续按键却继续落进那个输入框，像"按了没反应"。
        let focus_handle = terminal.read(cx).focus_handle().clone();
        window.focus(&focus_handle, cx);
        terminal.update(cx, |view, cx| {
            view.toggle_copy_mode(cx);
        });
        cx.notify();
    }
}
