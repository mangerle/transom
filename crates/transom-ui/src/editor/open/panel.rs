//! 远端编辑面板的打开、新建与初始读取（外壳入口）。
//!
//! 目录项闸门与错误提示属于工作区状态，留在外壳；面板装配与读取请求由
//! `EditorPanels` 完成，外壳只负责投递读取命令。

use super::*;
use crate::sftp::format_size;

impl AppShell {
    pub(crate) fn sftp_edit_remote_file_on(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        remote_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let max_bytes = self.state.settings.sftp.edit_max_bytes;
        let gate = self
            .state
            .workspace
            .tab(tab_id)
            .and_then(|tab| {
                let panel = self.sftp_state(tab_id, cx)?;
                let entries = if tab.connection_id == Some(connection_id) {
                    &panel.remote.entries
                } else {
                    &panel
                        .side_remotes
                        .iter()
                        .find(|side| side.connection_id == Some(connection_id))?
                        .entries
                };
                let entry = entries.iter().find(|item| item.path == remote_path)?;
                if entry.is_dir {
                    return Some(EditGate::IsDir);
                }
                if entry.size > max_bytes {
                    return Some(EditGate::TooLarge {
                        name: entry.name.clone(),
                        size: entry.size,
                    });
                }
                Some(EditGate::Allowed {
                    connection_id,
                    name: entry.name.clone(),
                    permissions: entry.permissions,
                })
            })
            .unwrap_or(EditGate::Missing);

        let (connection_id, file_name, permissions) = match gate {
            EditGate::Missing => {
                self.editor_reject(
                    tab_id,
                    connection_id,
                    rust_i18n::t!("sftp.select_file").to_string(),
                    cx,
                );
                return;
            }
            EditGate::IsDir => return,
            EditGate::TooLarge { name, size } => {
                self.editor_reject(
                    tab_id,
                    connection_id,
                    rust_i18n::t!(
                        "editor.error_too_large",
                        name = name,
                        size = format_size(size),
                        max = format_size(max_bytes)
                    )
                    .to_string(),
                    cx,
                );
                return;
            }
            EditGate::Allowed {
                connection_id,
                name,
                permissions,
            } => (connection_id, name, permissions),
        };

        // 同一文件已在编辑中就不再重复取文件：抬到最前并聚焦编辑器。
        if let Some(editor) = self.state.editors.update(cx, |panels, _| {
            panels.raise_existing(tab_id, connection_id, &remote_path)
        }) {
            editor.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }

        // 这里的尺寸闸门只是第一道：目录列表的尺寸可能过期，net 层还会在读取循环内
        // 按同一上限二次设限，两道一起才挡得住"列出之后文件变大"的情况。
        let op_id = TransferId::new();
        let settings = self.editor_panel_settings();
        let request = self.state.editors.update(cx, |panels, cx| {
            panels.spawn_editor_panel(
                SpawnEditorPanelParams {
                    tab_id,
                    connection_id,
                    remote_path,
                    file_name,
                    seed: PanelSeed::Reading { op_id, permissions },
                    pending_goto: None,
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

    /// 新建远端文件并打开编辑面板：空缓冲区立即可编辑，首次保存才创建远端文件。
    ///
    /// 面板携带零尺寸指纹：若同名非空文件恰好已存在（目录列表过期时可能发生），
    /// 首次保存会被 net 层指纹比对拦成冲突，弹出横幅而不是静默覆盖。
    pub(crate) fn spawn_new_editor_panel(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        remote_path: String,
        file_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = self.editor_panel_settings();
        self.state.editors.update(cx, |panels, cx| {
            panels.spawn_editor_panel(
                SpawnEditorPanelParams {
                    tab_id,
                    connection_id,
                    remote_path,
                    file_name,
                    seed: PanelSeed::NewFile,
                    pending_goto: None,
                },
                settings,
                window,
                cx,
            )
        });
    }

    /// 面板编辑器外观设置快照：实体装配时直接使用，避免反向读取工作区。
    fn editor_panel_settings(&self) -> EditorPanelSettings {
        EditorPanelSettings {
            line_numbers: self.state.settings.sftp.editor_line_numbers,
            soft_wrap: self.state.settings.sftp.editor_soft_wrap,
        }
    }
}
