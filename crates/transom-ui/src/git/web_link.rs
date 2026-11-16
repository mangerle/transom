//! 「在浏览器中查看」：取回提交的网页地址并交给系统浏览器。
//!
//! 设计原理（Why）：地址折算在引擎里（`transom-git::web`），这里只负责
//! 「发命令—等事件—打开」。不在点击回调里直接起 `git`：UI 线程一旦同步等子进程，
//! 整个窗口就跟着卡住。

use crate::git::GitWorkspace;
use gpui_kit::*;
use transom_core::{GitCommand, GitRequestId};

impl GitWorkspace {
    /// 请求某条提交的网页地址；结果经 `CommitUrlLoaded` 事件回来。
    pub(crate) fn browse_commit(&mut self, oid: &str, cx: &mut Context<Self>) {
        let request_id = GitRequestId::new();
        self.state.pending.push(request_id);
        self.send(GitCommand::ResolveCommitUrl {
            project: self.state.project.clone(),
            request_id,
            oid: oid.to_string(),
        });
        cx.notify();
    }
}

/// 收到地址后打开浏览器；取不到时返回底栏提示文案，不猜地址也不静默失败。
pub(crate) fn resolve_commit_url_feedback(url: Option<String>, oid: &str) -> Option<String> {
    match url {
        Some(url) => {
            // 地址由引擎折成固定 https 形态，凭据与非法字符已在折算层被拒绝。
            log::info!("打开提交网页，提交: {oid}");
            crate::audit::open_web_page(&url);
            None
        }
        None => {
            log::warn!("无法确定提交的网页地址，提交: {oid}");
            Some(rust_i18n::t!("git.browse.no_link").to_string())
        }
    }
}
