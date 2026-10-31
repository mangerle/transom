//! 远端编辑面板的打开、新建与初始读取。

use super::*;

impl AppShell {
    pub(crate) fn sftp_edit_remote_file_on(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        remote_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let max_bytes = self.settings.sftp.edit_max_bytes;
        let gate = self
            .workspace
            .tab(tab_id)
            .and_then(|tab| {
                let panel = tab.sftp()?;
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
        if let Some(index) = self
            .editors
            .panels
            .iter()
            .position(|item| item.same_file(tab_id, connection_id, &remote_path))
        {
            let item = self.editors.panels.remove(index);
            let editor = item.editor.clone();
            self.editors.panels.push(item);
            editor.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }

        // 这里的尺寸闸门只是第一道：目录列表的尺寸可能过期，net 层还会在读取循环内
        // 按同一上限二次设限，两道一起才挡得住"列出之后文件变大"的情况。
        let op_id = TransferId::new();
        self.spawn_editor_panel(
            SpawnEditorPanelParams {
                tab_id,
                connection_id,
                remote_path: remote_path.clone(),
                file_name,
                seed: PanelSeed::Reading { op_id, permissions },
            },
            window,
            cx,
        );
        self.request_remote_text_read(
            RemoteTextReadParams {
                tab_id,
                connection_id,
                remote_path,
                op_id,
            },
            cx,
        );
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
        self.spawn_editor_panel(
            SpawnEditorPanelParams {
                tab_id,
                connection_id,
                remote_path,
                file_name,
                seed: PanelSeed::NewFile,
            },
            window,
            cx,
        );
    }

    /// 建立编辑面板：空缓冲区、订阅就位、按已开面板数量级联落位。
    ///
    /// 读取态与新建态共用一套装配，只差初始状态字段（见 `PanelSeed`）；
    /// 新建面板可编辑、无在途任务，读取面板等待 `SftpTextRead` 装载内容。
    pub(crate) fn spawn_editor_panel(
        &mut self,
        params: SpawnEditorPanelParams,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SpawnEditorPanelParams {
            tab_id,
            connection_id,
            remote_path,
            file_name,
            seed,
        } = params;
        let language = editor_language_token(&remote_path);
        // 软换行与行号跟随设置页选择：日志类内容常关行号开换行，
        // 配置文件则相反，属于高频个性化项。
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language.clone())
                .line_number(self.settings.sftp.editor_line_numbers)
                .folding(true)
                .searchable(true)
                .soft_wrap(self.settings.sftp.editor_soft_wrap)
        });
        // 无写权限的文件（如 444 的密钥、锁定的配置）直接按只读打开：
        // 用户能看到内容但编辑器拒绝输入，避免改完才发现保存必然失败。
        // 新建文件没有已存在的权限位可依据，视作可写。
        let permissions = match &seed {
            PanelSeed::Reading { permissions, .. } => *permissions,
            PanelSeed::NewFile => None,
        };
        let writable = permissions_writable(permissions);
        if !writable {
            editor.update(cx, |state, cx| state.set_readonly(true, cx));
        }
        editor.update(cx, |state, cx| state.focus(window, cx));
        let caret = caret_of(&editor, cx);
        let cascade = self.editors.len();
        let viewport = window.viewport_size();
        let (left, top) = cascade_origin(cascade, viewport);
        let change_sub = cx.subscribe(&editor, |this, editor, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            if let Some(item) = this
                .editors
                .panels
                .iter_mut()
                .find(|item| item.editor == editor)
            {
                // 未成功装载远端内容的缓冲区不允许变成"可保存的改动"，
                // 否则读取失败后随手打几个字就能把远端文件覆盖掉。
                if !item.loaded {
                    return;
                }
                item.dirty = true;
                item.phase = EditorPhase::Idle;
                item.written_size = None;
                item.error = None;
            }
            cx.notify();
        });
        // 光标移动只 notify、不 emit 事件，因此观察通知本身；只在行列真的变了时
        // 才重绘，既让状态栏实时，也切断「面板重绘 → 编辑器通知 → 面板重绘」的自激。
        let caret_sub = cx.observe(&editor, |this, editor, cx| {
            let next = caret_of(&editor, cx);
            let changed = this
                .editors
                .panels
                .iter_mut()
                .find(|item| item.editor == editor)
                .is_some_and(|item| {
                    if item.caret == next {
                        false
                    } else {
                        item.caret = next;
                        true
                    }
                });
            if changed {
                cx.notify();
            }
        });
        let (loaded, phase, pending_op, remote_meta, new_file) = match seed {
            PanelSeed::Reading { op_id, .. } => {
                (false, EditorPhase::Reading, Some(op_id), None, false)
            }
            // 新建面板：可编辑即可打字；零尺寸指纹让首次保存具备同名覆盖保护。
            PanelSeed::NewFile => (
                true,
                EditorPhase::Idle,
                None,
                Some(RemoteTextMeta {
                    size: 0,
                    mtime: None,
                    bom: false,
                    crlf: false,
                    transcoded: false,
                }),
                true,
            ),
        };
        self.editors.panels.push(EditorPanelState {
            tab_id,
            connection_id,
            remote_path,
            file_name,
            language,
            editor,
            baseline: SharedString::default(),
            remote_meta,
            new_file,
            pending_goto: None,
            pending_sudo_probe: None,
            sudo_probe_attempted: false,
            caret,
            dirty: false,
            loaded,
            phase,
            written_size: None,
            pending_op,
            error: None,
            permissions,
            writable,
            close_after_write: false,
            diff: None,
            showing_diff: false,
            conflict: false,
            confirm: None,
            collapsed: false,
            maximized: false,
            left,
            top,
            width: PANEL_WIDTH,
            height: PANEL_HEIGHT,
            drag_grab: None,
            _change_sub: change_sub,
            _caret_sub: caret_sub,
        });
        cx.notify();
    }
}
