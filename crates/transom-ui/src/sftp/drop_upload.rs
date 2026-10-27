//! 拖文件到终端网格 → 自动上传到该会话的远端目录。
//!
//! 设计原理（Why）：
//! - 实现初衷："先在文件面板里找到目录、再点上传"是两步操作，而人拖文件时
//!   心里想的是"把这个丢到我现在这台机器上"。把命中区域做成终端网格本身，
//!   操作意图与落点就重合了。
//! - 核心优势：复用既有上传队列（`enqueue_uploads`），因此拖放上传与面板按钮
//!   走完全相同的续传/校验/并发调度路径，进度也就天然显示在队列里；
//! - 代价与局限：目标目录取自 SFTP 面板当前路径，面板没开时退化为从终端标题
//!   推断（`extract_candidate_path`），再不行落到远端 Home。没有 shell 集成
//!   上报 cwd 时无法精确知道用户 `cd` 到了哪里——这与面板初始路径是同一套
//!   既有限制，不另造第二套推断。

use crate::app::{AppShell, TabStatus};
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{Connection, Session, TabId};

/// 该标签能否接住拖进来的文件；不能时返回界面文案键。
///
/// 独立成纯函数是为了把"只有已连接的 SSH 会话可传"这条规则单测锁死：
/// 本地终端与串口没有远端可言，Telnet 更是没有文件通道。
fn drop_rejection(session: &Session, status: &TabStatus) -> Option<&'static str> {
    match &session.connection {
        Connection::Ssh(_) => {
            if matches!(status, TabStatus::Connected) {
                None
            } else {
                Some("sftp.drop_not_connected")
            }
        }
        _ => Some("sftp.drop_needs_ssh"),
    }
}

impl AppShell {
    /// 终端网格上松手：为该标签打开（或复用）SFTP 面板并把文件排进上传队列。
    pub(crate) fn handle_dropped_files(
        &mut self,
        tab_id: TabId,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        let Some(index) = self.workspace.tabs.iter().position(|tab| tab.id == tab_id) else {
            return;
        };
        if let Some(key) = drop_rejection(
            &self.workspace.tabs[index].session,
            &self.workspace.tabs[index].status,
        ) {
            let message = rust_i18n::t!(key).to_string();
            log::warn!(
                "拒绝拖拽上传，窗格: {tab_id}，文件数: {}，原因: {message}",
                paths.len()
            );
            self.status = Some(message);
            cx.notify();
            return;
        }

        // 面板已存在时沿用它当前的远端目录（用户看到哪儿就传到哪儿）；不存在则
        // 建一份隐藏状态：队列、续传与校验都照旧，只是进度改由终端右下角浮层呈现，
        // 不再为了传一个文件而弹开一整列文件管理器。
        if self.workspace.tabs[index].sftp().is_none() {
            self.build_sftp_panel(tab_id, index, false, window, cx);
        }
        if let Err(message) = self.enqueue_uploads(tab_id, paths, cx) {
            self.error = Some(message);
            cx.notify();
            return;
        }
        log::info!(
            "拖拽上传已入队，窗格: {tab_id}，文件数: {}，目标会话: {}",
            paths.len(),
            crate::connection::describe_target(&self.workspace.tabs[index].session)
        );
        self.show_drop_overlay(tab_id, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::drop_rejection;
    use crate::app::TabStatus;
    use transom_core::{Connection, LocalConnection, Session, SshConnection, TelnetConnection};

    fn status() -> TabStatus {
        TabStatus::Connected
    }

    #[test]
    fn only_connected_ssh_accepts_drops() {
        let ssh = Session::new("web", Connection::Ssh(SshConnection::new("h", "u")));
        assert_eq!(drop_rejection(&ssh, &status()), None);
        assert_eq!(
            drop_rejection(&ssh, &TabStatus::Connecting),
            Some("sftp.drop_not_connected")
        );

        let telnet = Session::new("bbs", Connection::Telnet(TelnetConnection::new("h")));
        assert_eq!(
            drop_rejection(&telnet, &status()),
            Some("sftp.drop_needs_ssh")
        );
        let local = Session::new("local", Connection::Local(LocalConnection::default()));
        assert_eq!(
            drop_rejection(&local, &status()),
            Some("sftp.drop_needs_ssh")
        );
    }
}
