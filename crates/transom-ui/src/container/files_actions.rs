//! 容器文件页的浏览与传输动作。

use super::*;
use gpui_kit::component::input::TextareaState;

impl AppShell {
    pub(crate) fn ensure_container_files(&mut self, tab_id: TabId) {
        let path = self
            .state
            .container_workspace_mut(tab_id)
            .map(|workspace| workspace.files.path.clone())
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| "/".to_string());
        self.load_container_dir(tab_id, path);
    }

    pub(crate) fn load_container_dir(&mut self, tab_id: TabId, path: String) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let is_volume = workspace.section == ContainerSection::Volumes;
        let target = if is_volume {
            workspace.selected_volume_name()
        } else {
            workspace.selected_container_name()
        };
        let Some(target) = target else {
            return;
        };
        workspace.files.loading = true;
        workspace.files.preview = None;
        let request_id = ContainerRequestId::new();
        if let Some(previous) = workspace.files.request.replace(request_id) {
            self.cancel_container_request(tab_id, previous);
        }
        let command = if is_volume {
            log::info!("打开数据卷目录，卷: {target}，路径: {path}，请求: {request_id}");
            ContainerCommand::ListVolumeDir {
                tab_id,
                request_id,
                volume: target,
                path,
            }
        } else {
            log::info!("打开容器目录，容器: {target}，路径: {path}，请求: {request_id}");
            ContainerCommand::ListDir {
                tab_id,
                request_id,
                container: target,
                path,
            }
        };
        self.send_tracked(tab_id, request_id, command);
    }

    /// 预览容器或数据卷内文本文件。
    pub(crate) fn preview_container_file(&mut self, tab_id: TabId, path: String) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let is_volume = workspace.section == ContainerSection::Volumes;
        let target = if is_volume {
            workspace.selected_volume_name()
        } else {
            workspace.selected_container_name()
        };
        let Some(target) = target else {
            return;
        };
        workspace.files.preview_editor = None;
        workspace.files.preview_saving = false;
        workspace.files.preview_binary = false;
        let request_id = ContainerRequestId::new();
        let command = if is_volume {
            ContainerCommand::ReadVolumeFile {
                tab_id,
                request_id,
                volume: target,
                path,
                max_bytes: 1024 * 1024,
            }
        } else {
            ContainerCommand::ReadFile {
                tab_id,
                request_id,
                container: target,
                path,
                max_bytes: 1024 * 1024,
            }
        };
        self.send_tracked(tab_id, request_id, command);
    }

    /// 下载容器或数据卷内文件。
    pub(crate) fn download_container_file(&mut self, tab_id: TabId, path: String) {
        let Some(local_path) = rfd::FileDialog::new()
            .set_file_name(path.rsplit('/').next().unwrap_or("download"))
            .save_file()
        else {
            return;
        };
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let is_volume = workspace.section == ContainerSection::Volumes;
        let target = if is_volume {
            workspace.selected_volume_name()
        } else {
            workspace.selected_container_name()
        };
        let Some(target) = target else {
            return;
        };
        let request_id = ContainerRequestId::new();
        workspace.notice = Some(rust_i18n::t!("container.transfer.start").to_string());
        let command = if is_volume {
            ContainerCommand::DownloadVolumeFile {
                tab_id,
                request_id,
                volume: target,
                path,
                local_path,
            }
        } else {
            ContainerCommand::Download {
                tab_id,
                request_id,
                container: target,
                path,
                local_path,
            }
        };
        self.send_tracked(tab_id, request_id, command);
    }

    /// 把容器内目录导出为本地 tar 归档。
    pub(crate) fn download_container_directory(&mut self, tab_id: TabId, path: String) {
        let is_volume = self
            .state
            .container_workspace(tab_id)
            .is_some_and(|workspace| workspace.section == ContainerSection::Volumes);
        if is_volume {
            if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
                workspace.notice =
                    Some(rust_i18n::t!("container.files.directory_volume_unsupported").to_string());
            }
            return;
        }
        let name = path.rsplit('/').next().unwrap_or("directory");
        let Some(local_path) = rfd::FileDialog::new()
            .set_file_name(format!("{name}.tar"))
            .save_file()
        else {
            return;
        };
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(target) = workspace.selected_container_name() else {
            return;
        };
        let request_id = ContainerRequestId::new();
        workspace.notice = Some(rust_i18n::t!("container.transfer.start").to_string());
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::DownloadDirectory {
                tab_id,
                request_id,
                container: target,
                path,
                local_path,
            },
        );
    }

    /// 打开文件操作提示：重命名、复制、改权限与改属主共用同一套输入弹窗。
    fn open_container_file_prompt(
        &mut self,
        tab_id: TabId,
        prompt: ContainerFilePrompt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placeholder = rust_i18n::t!(prompt.placeholder_key()).to_string();
        let default_value = prompt.default_value();
        let input = cx.new(|cx| {
            let input = InputState::new(window, cx).placeholder(placeholder);
            if default_value.is_empty() {
                input
            } else {
                input.default_value(default_value)
            }
        });
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.files.prompt = Some(prompt);
            workspace.files.prompt_input = Some(input);
        }
        cx.notify();
    }

    /// 打开文件重命名条。
    pub(crate) fn begin_container_file_rename(
        &mut self,
        tab_id: TabId,
        path: String,
        _name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_container_file_prompt(
            tab_id,
            ContainerFilePrompt::Rename { from: path },
            window,
            cx,
        );
    }

    /// 打开「复制到」输入条。
    pub(crate) fn begin_container_file_copy(
        &mut self,
        tab_id: TabId,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_container_file_prompt(
            tab_id,
            ContainerFilePrompt::Copy { from: path },
            window,
            cx,
        );
    }

    /// 打开「修改权限」输入条。
    pub(crate) fn begin_container_file_chmod(
        &mut self,
        tab_id: TabId,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_container_file_prompt(tab_id, ContainerFilePrompt::Chmod { path }, window, cx);
    }

    /// 打开「修改属主」输入条。
    pub(crate) fn begin_container_file_chown(
        &mut self,
        tab_id: TabId,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_container_file_prompt(tab_id, ContainerFilePrompt::Chown { path }, window, cx);
    }

    /// 请求删除容器内路径；目录按递归删除走二次确认。
    pub(crate) fn request_remove_container_path(
        &mut self,
        tab_id: TabId,
        path: String,
        recursive: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.confirm = Some(ContainerConfirm::FilePath { path, recursive });
        }
        cx.notify();
    }

    /// 上传指定本地路径的文件到容器当前目录。
    pub(crate) fn upload_container_path(&mut self, tab_id: TabId, local_path: std::path::PathBuf) {
        let Some(file_name) = local_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
        else {
            return;
        };
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(container) = workspace.selected_container_name() else {
            return;
        };
        let directory = if workspace.files.path.is_empty() {
            "/".to_string()
        } else {
            workspace.files.path.clone()
        };
        let target = if directory == "/" {
            format!("/{file_name}")
        } else {
            format!("{directory}/{file_name}")
        };
        let request_id = ContainerRequestId::new();
        workspace.notice = Some(rust_i18n::t!("container.transfer.start").to_string());
        let is_dir = local_path.is_dir();
        let command = if is_dir {
            ContainerCommand::UploadDirectory {
                tab_id,
                request_id,
                container,
                path: target,
                local_path,
            }
        } else {
            ContainerCommand::Upload {
                tab_id,
                request_id,
                container,
                path: target,
                local_path,
            }
        };
        self.send_tracked(tab_id, request_id, command);
    }

    /// 上传本地文件到当前目录。
    pub(crate) fn upload_container_file(&mut self, tab_id: TabId) {
        let Some(local_path) = rfd::FileDialog::new().pick_file() else {
            return;
        };
        self.upload_container_path(tab_id, local_path);
    }

    /// 上传本地目录到当前目录，目录层级原样保留。
    pub(crate) fn upload_container_directory(&mut self, tab_id: TabId) {
        let Some(local_path) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        self.upload_container_path(tab_id, local_path);
    }

    /// 重命名容器内路径。
    /// 展开或收起「新建目录」条；展开时才创建输入框，避免常驻实体。
    pub(crate) fn toggle_container_mkdir(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        workspace.files.new_dir_open = !workspace.files.new_dir_open;
        if workspace.files.new_dir_open && workspace.files.new_dir_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(rust_i18n::t!("container.files.new_dir_placeholder").to_string())
            });
            workspace.files.new_dir_input = Some(input);
        }
        cx.notify();
    }

    /// 在容器当前目录里新建子目录：上传与整理文件都需要先有目录。
    pub(crate) fn submit_container_mkdir(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(container) = workspace.selected_container_name() else {
            return;
        };
        let Some(input) = workspace.files.new_dir_input.clone() else {
            return;
        };
        let name = input.read(cx).value().trim().to_string();
        if name.is_empty() || name.contains('/') || name.contains("..") {
            self.show_toast(
                rust_i18n::t!("container.files.new_dir_invalid").to_string(),
                false,
                window,
                cx,
            );
            return;
        }
        let parent = if workspace.files.path.is_empty() {
            "/".to_string()
        } else {
            workspace.files.path.clone()
        };
        let path = if parent.ends_with('/') {
            format!("{parent}{name}")
        } else {
            format!("{parent}/{name}")
        };
        let request_id = ContainerRequestId::new();
        workspace.files.new_dir_open = false;
        workspace.busy = true;
        workspace
            .files
            .new_dir_input
            .as_ref()
            .iter()
            .for_each(|input| input.update(cx, |input, cx| input.set_value("", window, cx)));
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::CreateDir {
                tab_id,
                request_id,
                container,
                path,
            },
        );
        cx.notify();
    }

    /// 进入文件就地编辑模式。
    pub(crate) fn begin_edit_container_file(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        if workspace.files.preview_binary {
            return;
        }
        let Some((_, content, _)) = workspace.files.preview.as_ref() else {
            return;
        };
        let initial_text = content.clone();
        let editor = cx.new(|cx| TextareaState::new(window, cx).default_value(initial_text));
        workspace.files.preview_editor = Some(editor);
        cx.notify();
    }

    /// 取消文件就地编辑模式。
    pub(crate) fn cancel_edit_container_file(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        workspace.files.preview_editor = None;
        workspace.files.preview_saving = false;
        cx.notify();
    }

    /// 关闭当前文件预览与编辑。
    pub(crate) fn close_preview_container_file(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        workspace.files.preview = None;
        workspace.files.preview_editor = None;
        workspace.files.preview_saving = false;
        workspace.files.preview_binary = false;
        cx.notify();
    }

    /// 保存就地编辑的文件内容。
    pub(crate) fn save_edit_container_file(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        if workspace.files.preview_binary {
            return;
        }
        let Some((path, _, _)) = workspace.files.preview.as_ref().cloned() else {
            return;
        };
        let Some(editor) = workspace.files.preview_editor.as_ref() else {
            return;
        };
        let content = editor.read(cx).value().to_string();
        let is_volume = workspace.section == ContainerSection::Volumes;
        let target = if is_volume {
            workspace.selected_volume_name()
        } else {
            workspace.selected_container_name()
        };
        let Some(target) = target else {
            return;
        };

        workspace.files.preview_saving = true;
        let request_id = ContainerRequestId::new();
        let command = if is_volume {
            ContainerCommand::SaveVolumeFile {
                tab_id,
                request_id,
                volume: target,
                path,
                content,
            }
        } else {
            ContainerCommand::SaveFile {
                tab_id,
                request_id,
                container: target,
                path,
                content,
            }
        };
        self.send_tracked(tab_id, request_id, command);
        cx.notify();
    }

    /// 提交文件操作提示：按提示类型生成命令并清空输入状态。
    pub(crate) fn submit_container_file_prompt(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((prompt, input)) = self
            .state
            .container_workspace(tab_id)
            .and_then(|workspace| {
                let prompt = workspace.files.prompt.clone()?;
                let input = workspace.files.prompt_input.clone()?;
                Some((prompt, input))
            })
        else {
            return;
        };
        let value = input.read(cx).value().trim().to_string();
        let (action, invalid_key, same_key) = match &prompt {
            ContainerFilePrompt::Rename { .. } => {
                if value.is_empty() || value.contains('/') {
                    ("".to_string(), Some("container.files.rename_invalid"), None)
                } else {
                    (value.clone(), None, None)
                }
            }
            ContainerFilePrompt::Copy { from } => {
                if value.is_empty()
                    || !value.starts_with('/')
                    || value.split('/').any(|segment| segment == "..")
                {
                    ("".to_string(), Some("container.files.copy_invalid"), None)
                } else if &value == from {
                    ("".to_string(), None, Some("container.files.copy_same"))
                } else {
                    (value.clone(), None, None)
                }
            }
            ContainerFilePrompt::Chmod { .. } => {
                if !(3..=4).contains(&value.len())
                    || !value.bytes().all(|byte| (b'0'..=b'7').contains(&byte))
                {
                    ("".to_string(), Some("container.files.chmod_invalid"), None)
                } else {
                    (value.clone(), None, None)
                }
            }
            ContainerFilePrompt::Chown { .. } => {
                let valid = !value.is_empty()
                    && value.split(':').count() <= 2
                    && value.split(':').all(|part| {
                        !part.is_empty()
                            && part.bytes().all(|byte| {
                                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
                            })
                    });
                if valid {
                    (value.clone(), None, None)
                } else {
                    ("".to_string(), Some("container.files.chown_invalid"), None)
                }
            }
        };
        if let Some(key) = invalid_key.or(same_key) {
            self.show_toast(rust_i18n::t!(key).to_string(), false, window, cx);
            return;
        }
        let Some(workspace) = self.state.container_workspace_mut(tab_id) else {
            return;
        };
        let Some(container) = workspace.selected_container_name() else {
            return;
        };
        let request_id = ContainerRequestId::new();
        let command = match prompt {
            ContainerFilePrompt::Rename { from } => {
                let parent = detail::parent_path(&from);
                let to = if parent == "/" {
                    format!("/{action}")
                } else {
                    format!("{parent}/{action}")
                };
                ContainerCommand::Rename {
                    tab_id,
                    request_id,
                    container,
                    from,
                    to,
                }
            }
            ContainerFilePrompt::Copy { from } => ContainerCommand::CopyPath {
                tab_id,
                request_id,
                container,
                from,
                to: action,
            },
            ContainerFilePrompt::Chmod { path } => ContainerCommand::ChmodPath {
                tab_id,
                request_id,
                container,
                path,
                mode: action,
            },
            ContainerFilePrompt::Chown { path } => ContainerCommand::ChownPath {
                tab_id,
                request_id,
                container,
                path,
                owner: action,
            },
        };
        workspace.files.prompt = None;
        workspace.files.prompt_input = None;
        self.send_tracked(tab_id, request_id, command);
        cx.notify();
    }

    /// 取消容器内文件操作提示。
    pub(crate) fn cancel_container_file_rename(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.files.prompt = None;
            workspace.files.prompt_input = None;
        }
        cx.notify();
    }

    /// 取消新建目录。
    pub(crate) fn cancel_container_file_new_dir(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.files.new_dir_open = false;
            workspace.files.new_dir_input = None;
        }
        cx.notify();
    }
}
