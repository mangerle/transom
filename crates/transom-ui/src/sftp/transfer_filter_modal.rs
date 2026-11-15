//! SFTP 目录传输排除规则的编辑弹窗打开入口。

use super::entity::SftpPanelView;
use crate::app::app_settings;
use gpui_kit::component::input::TextareaState;
use gpui_kit::*;

impl SftpPanelView {
    /// 打开传输排除规则编辑弹窗：文本取自外壳设置，装配在外壳完成。
    pub(crate) fn open_filter_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = app_settings(cx).sftp.transfer_filter.clone();
        let textarea = cx.new(|cx| TextareaState::new(window, cx).default_value(text));
        textarea.update(cx, |state, cx| state.focus(window, cx));
        cx.emit(crate::sftp::SftpPanelEvent::OpenFilterModal { textarea });
        cx.notify();
    }
}
