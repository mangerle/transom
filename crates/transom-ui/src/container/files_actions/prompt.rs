//! 文件操作输入弹窗：重命名/复制/权限/属主。

use super::super::*;

use super::super::detail;

impl AppShell {
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
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.files.prompt = Some(prompt);
            workspace.files.prompt_input = Some(input);
        });
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
                let workspace = &workspace.read(cx).state;
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
        let Some(container) = self
            .with_container(tab_id, cx, |workspace| workspace.selected_container_name())
            .flatten()
        else {
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
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.files.prompt = None;
            workspace.files.prompt_input = None;
        });
        if let Some(workspace) = self.state.container_workspace(tab_id) {
            workspace.update(cx, |workspace, _| {
                workspace.send_tracked(request_id, command);
            });
        }
        cx.notify();
    }

    /// 取消容器内文件操作提示。
    pub(crate) fn cancel_container_file_rename(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.files.prompt = None;
            workspace.files.prompt_input = None;
        });
        cx.notify();
    }

    /// 取消新建目录。
    pub(crate) fn cancel_container_file_new_dir(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.files.new_dir_open = false;
            workspace.files.new_dir_input = None;
        });
        cx.notify();
    }
}
