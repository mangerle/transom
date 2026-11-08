//! 连接建立过程的窗口级确认弹窗实体。
//!
//! 设计原理（Why）：SSH 鉴权问答、首连主机指纹与 OSC 52 剪贴板授权此前以三个
//! 独立 `Option` 字段挂在 `ModalHub.connection` 上，打开入口、动作与渲染分居
//! `connection/auth_dialog.rs`、`clipboard_prompt.rs` 与 `event_handlers.rs`。
//! 三者的生命周期相同：都只在一次连接协商期存活，关闭时要把裁决结果交还等待中
//! 的连接任务。合并为一个实体后，输入控件、应答通道与保存意图随弹窗打开同生
//! 共死，外壳只持有句柄。
//!
//! 实体不反查外壳：凭据归属、终端授权与剪贴板写入等跨域副作用通过实体事件
//! 上交，事件自带全部数据，外壳处理时禁止回读实体。
//!
//! # 代价与局限
//! 三个弹窗共享一份状态但彼此独立：本实体不强制三者互斥，互斥与否仍由打开
//! 入口判断（鉴权与指纹不会同时发生，剪贴板请求可能来自另一个窗格）。渲染放在
//! `connection/prompts_render.rs`，本文件只承载状态与动作。

use super::{AuthPromptState, HostKeyConfirmState};
use crate::clipboard_prompt::ClipboardWritePrompt;
use gpui_kit::*;

/// 连接确认弹窗实体。
pub(crate) struct ConnectionPromptsView {
    auth: Option<AuthPromptState>,
    host_key: Option<HostKeyConfirmState>,
    clipboard: Option<ClipboardWritePrompt>,
}

impl ConnectionPromptsView {
    /// 组装常驻实体；三个弹窗初始都关闭。
    pub(crate) fn new() -> Self {
        Self {
            auth: None,
            host_key: None,
            clipboard: None,
        }
    }

    /// SSH 鉴权问答是否打开。
    pub(crate) fn is_auth_open(&self) -> bool {
        self.auth.is_some()
    }

    /// 首连主机指纹确认是否打开。
    pub(crate) fn is_host_key_open(&self) -> bool {
        self.host_key.is_some()
    }

    /// 远端剪贴板写入授权是否打开。
    pub(crate) fn is_clipboard_open(&self) -> bool {
        self.clipboard.is_some()
    }

    /// 当前鉴权问答的只读状态；渲染与状态查询共用同一入口。
    pub(crate) fn auth(&self) -> Option<&AuthPromptState> {
        self.auth.as_ref()
    }

    /// 当前主机指纹确认的只读状态。
    pub(crate) fn host_key(&self) -> Option<&HostKeyConfirmState> {
        self.host_key.as_ref()
    }

    /// 当前剪贴板授权的只读状态。
    pub(crate) fn clipboard(&self) -> Option<&ClipboardWritePrompt> {
        self.clipboard.as_ref()
    }

    /// 打开鉴权问答；新问题覆盖旧问题，旧应答通道随状态析构而关闭。
    pub(crate) fn open_auth(&mut self, state: AuthPromptState, cx: &mut Context<Self>) {
        self.auth = Some(state);
        cx.notify();
    }

    /// 打开首连主机指纹确认。
    pub(crate) fn open_host_key(&mut self, state: HostKeyConfirmState, cx: &mut Context<Self>) {
        self.host_key = Some(state);
        cx.notify();
    }

    /// 打开展示中的远端剪贴板写入请求。
    pub(crate) fn open_clipboard(&mut self, prompt: ClipboardWritePrompt, cx: &mut Context<Self>) {
        self.clipboard = Some(prompt);
        cx.notify();
    }

    /// 关闭鉴权问答并丢弃应答通道。
    ///
    /// 应答通道随状态一起析构：等待方读到通道关闭后会按"用户取消"收尾，
    /// 与关闭弹窗的语义一致，不会把连接永久挂住。
    pub(crate) fn close_auth(&mut self, cx: &mut Context<Self>) {
        self.auth = None;
        cx.notify();
    }

    /// 关闭剪贴板授权且不回任何结果；拒绝文案由调用方决定。
    pub(crate) fn close_clipboard(&mut self, cx: &mut Context<Self>) {
        self.clipboard = None;
        cx.notify();
    }

    /// 切换"保存本次凭据"开关；弹窗已关闭时忽略。
    pub(crate) fn set_save(&mut self, save: bool, cx: &mut Context<Self>) {
        if let Some(state) = self.auth.as_mut() {
            state.save = save;
            cx.notify();
        }
    }

    /// 取走鉴权问答状态并重绘；提交与取消都先把状态移出实体。
    pub(crate) fn take_auth(&mut self, cx: &mut Context<Self>) -> Option<AuthPromptState> {
        let state = self.auth.take();
        if state.is_some() {
            cx.notify();
        }
        state
    }

    /// 取走主机指纹确认状态并重绘。
    pub(crate) fn take_host_key(&mut self, cx: &mut Context<Self>) -> Option<HostKeyConfirmState> {
        let state = self.host_key.take();
        if state.is_some() {
            cx.notify();
        }
        state
    }

    /// 取走剪贴板请求并重绘。
    pub(crate) fn take_clipboard(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<ClipboardWritePrompt> {
        let prompt = self.clipboard.take();
        if prompt.is_some() {
            cx.notify();
        }
        prompt
    }
}
