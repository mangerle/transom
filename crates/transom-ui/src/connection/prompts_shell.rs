//! 连接确认弹窗的外壳职责：实体事件消费。
//!
//! 状态、输入控件、应答通道与全部弹窗动作都在
//! [`ConnectionPromptsView`](super::ConnectionPromptsView)；外壳只处理跨域副作用：
//! 把保存意图组装成待落盘凭据、给终端视图授予会话级剪贴板权限、写入本地剪贴板
//! 与状态栏。事件自带全部数据，这里禁止回读实体，避免实体 `emit` 期间形成
//! GPUI 的双重租借。

use super::{ConnectionPromptsEvent, PendingCredential};
use crate::app::AppShell;
use gpui_kit::*;
use transom_core::TabId;

impl AppShell {
    /// 消费连接确认弹窗实体上报的意图。
    pub(crate) fn on_connection_prompts_event(
        &mut self,
        event: &ConnectionPromptsEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            ConnectionPromptsEvent::CredentialToSave {
                session_id,
                connection_id,
                kind,
                value,
            } => {
                self.state.modals.connection.pending_credential = Some(PendingCredential {
                    session_id: *session_id,
                    // 归属解析留在外壳：引用凭据模板时会话可能只是模板别名。
                    owner: self.state.secret_owner_for(*session_id),
                    connection_id: *connection_id,
                    kind: *kind,
                    value: value.clone(),
                });
            }
            ConnectionPromptsEvent::ClipboardAllowed { tab_id, text } => {
                self.apply_clipboard_allowed(*tab_id, text, cx);
            }
            ConnectionPromptsEvent::Denied => {
                self.state.status = Some(rust_i18n::t!("clipboard_consent.denied").to_string());
            }
        }
        cx.notify();
    }

    /// 允许本会话写入：授予会话级权限，并把本次待写内容落盘。
    ///
    /// 授权位落在终端视图上（连接级），本次内容在这里直接写入：视图的授权方法
    /// 只改变后续请求的走向，不重复写剪贴板。终端可能已随标签关闭而消失，
    /// 此时仍写入本次内容与状态栏，不把一次已经获准的复制吞掉。
    fn apply_clipboard_allowed(&mut self, tab_id: TabId, text: &str, cx: &mut Context<Self>) {
        let terminal = self
            .state
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.session_pane_mut())
            .and_then(|pane| pane.terminal.clone());
        if let Some(terminal) = terminal {
            terminal.update(cx, |view, _| view.allow_remote_clipboard_write());
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        self.state.status = Some(rust_i18n::t!("clipboard_consent.allowed").to_string());
    }
}
