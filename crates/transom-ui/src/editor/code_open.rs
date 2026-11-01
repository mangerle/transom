//! 终端代码链接到远端编辑器的落地：打开未列出的远端文件并定位行列。
//!
//! 设计原理（Why）：终端只投递"路径 + 行 + 列"，真正的打开动作必须复用编辑器的
//! 读取通道与面板装配；单独成文件是为了让 `open.rs` 只保留列表行与新建文件的入口，
//! 两条触达路径各自可读。

use super::open::{EditorPanelSettings, PanelSeed, SpawnEditorPanelParams};
use crate::app::AppShell;
use crate::app::app_settings;
use gpui_kit::*;
use transom_core::{ConnectionId, TabId, TransferId};

/// 终端代码链接转换出的远端定位请求（行列均为 1 基）。
pub(crate) struct RemoteCodeLocation {
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) remote_path: String,
    pub(crate) line: u32,
    pub(crate) column: Option<u32>,
}

impl AppShell {
    /// 从终端 OSC 8 代码链接直接打开远端文件并定位到行列。
    ///
    /// 与双击列表行的 [`Self::sftp_edit_remote_file`] 不同：这里不要求目标出现在
    /// 当前目录列表里（编译器与 grep 链接常指向子目录或尚未列出的文件），因此
    /// 跳过目录项闸门；读取上限仍由 net 层在拉取循环内硬截断。
    pub(crate) fn open_remote_code_location(
        &mut self,
        target: RemoteCodeLocation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let RemoteCodeLocation {
            tab_id,
            connection_id,
            remote_path,
            line,
            column,
        } = target;
        if let Some(editor) = self.state.editors.update(cx, |panels, _| {
            panels.raise_existing(tab_id, connection_id, &remote_path)
        }) {
            super::navigate::apply_editor_goto(&editor, line, column, window, cx);
            cx.notify();
            return;
        }
        let file_name = remote_path
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .map_or_else(
                || rust_i18n::t!("editor.remote_file_fallback").to_string(),
                |name| name.to_string(),
            );
        let op_id = TransferId::new();
        let settings = EditorPanelSettings {
            line_numbers: app_settings(cx).sftp.editor_line_numbers,
            soft_wrap: app_settings(cx).sftp.editor_soft_wrap,
        };
        let request = self.state.editors.update(cx, |panels, cx| {
            panels.spawn_editor_panel(
                SpawnEditorPanelParams {
                    tab_id,
                    connection_id,
                    remote_path,
                    file_name,
                    seed: PanelSeed::Reading {
                        op_id,
                        permissions: None,
                    },
                    pending_goto: Some((line, column)),
                },
                settings,
                window,
                cx,
            )
        });
        if let Some(request) = request {
            self.request_remote_text_read(request, cx);
        }
    }
}
