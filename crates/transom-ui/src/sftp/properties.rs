//! SFTP 文件属性查看：远端异步 stat 请求与本地同步属性读取。

use super::{SftpPropertiesModalState, local_file_properties};
use crate::app::AppShell;
use gpui_kit::*;
use transom_core::{SftpOperation, TabId};

impl AppShell {
    /// 请求查看远端路径的详细属性（异步读取，结果经 `AppEvent::SftpMetadata` 回传）
    pub(crate) fn sftp_request_remote_properties(
        &mut self,
        tab_id: TabId,
        path: String,
        cx: &mut Context<Self>,
    ) {
        self.send_sftp_op(tab_id, SftpOperation::Stat { path }, cx);
    }

    /// 读取并直接展示本地路径属性。
    ///
    /// 设计原理（Why）：本地元数据读取为微秒级同步系统调用，远小于一次渲染帧预算，
    /// 无需绕行后台命令通道与跨线程事件，直接在 UI 线程读取可避免额外状态机开销。
    pub(crate) fn sftp_show_local_properties(
        &mut self,
        tab_id: TabId,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        match local_file_properties(&path) {
            Ok(properties) => {
                self.sftp_modals.properties = Some(SftpPropertiesModalState {
                    scope: "本地".to_string(),
                    properties,
                });
            }
            Err(err) => {
                if let Some(panel) = self
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == tab_id)
                    .and_then(|t| t.sftp.as_mut())
                {
                    panel.local_error = Some(format!("读取本地文件属性失败: {err}"));
                }
            }
        }
        cx.notify();
    }
}
