use super::{SftpActivePane, SftpInputModalState, SftpInputTarget, join_remote, parent_remote};
use crate::app::AppShell;
use crate::keymap;
use crate::palette::sftp_text_input;
use gpui_kit::*;
use std::path::Path;
use transom_core::{RemoteDeleteTarget, SftpOperation, TabId};

impl AppShell {
    /// 批量删除远端选中项；目录按递归语义删除。
    pub(crate) fn sftp_delete_selection(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp.as_mut() else {
            return;
        };
        let paths = panel.selected_remote_paths();
        if paths.is_empty() {
            panel.error = Some(rust_i18n::t!("sftp.select_file").to_string());
            cx.notify();
            return;
        }
        let targets = paths
            .iter()
            .filter_map(|path| {
                panel
                    .entries
                    .iter()
                    .find(|entry| &entry.path == path)
                    .map(|entry| RemoteDeleteTarget {
                        path: entry.path.clone(),
                        is_dir: entry.is_dir,
                    })
            })
            .collect::<Vec<_>>();
        if targets.is_empty() {
            panel.error = Some(rust_i18n::t!("sftp.select_file").to_string());
            cx.notify();
            return;
        }
        let count = targets.len();
        self.send_sftp_op(tab_id, SftpOperation::DeleteMany { targets }, cx);
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            panel.status = Some(format!("正在删除 {count} 个远端项目"));
            cx.notify();
        }
    }

    /// Delete 快捷键：直接删除当前活动栏选中的项目（与右键“删除”行为一致，无二次确认）。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：文件管理器惯例是选中后按 Delete 直接删除；双栏共用同一键盘上下文，
    ///   因此以“最近交互栏”（`active_pane`）确定删除目标；
    /// - 核心优势：与右键菜单删除共用同一操作路径，行为完全一致，
    ///   活动栏由统一选中方法维护，不会出现删错栏位的失配；
    /// - 代价与局限：无二次确认，Delete 按下即删除；如需误删保护需在设计层面
    ///   为所有删除入口统一补确认机制。
    pub(crate) fn sftp_delete_selected(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(active_pane) = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_ref())
            .map(|panel| panel.active_pane)
        else {
            return;
        };
        match active_pane {
            SftpActivePane::Remote => self.sftp_delete_selection(tab_id, cx),
            SftpActivePane::Local => self.sftp_local_delete_selection(tab_id, cx),
            SftpActivePane::SideRemote(_) => {}
        }
    }

    pub(crate) fn on_delete_sftp_selection(
        &mut self,
        _: &keymap::DeleteSftpSelection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab_id) = self.active_tab {
            self.sftp_delete_selected(tab_id, cx);
        }
    }

    pub(crate) fn sftp_open_mkdir_modal(
        &mut self,
        tab_id: TabId,
        is_remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (title, prompt, target) = if is_remote {
            (
                "新建远端文件夹".to_string(),
                "请输入新建远端文件夹的名称:".to_string(),
                SftpInputTarget::RemoteMkdir,
            )
        } else {
            (
                "新建本地文件夹".to_string(),
                "请输入新建本地文件夹的名称:".to_string(),
                SftpInputTarget::LocalMkdir,
            )
        };
        let input = sftp_text_input(window, cx, "", "文件夹名称");
        input.update(cx, |state, cx| state.focus(window, cx));
        self.sftp_modals.input = Some(SftpInputModalState {
            tab_id,
            title,
            prompt,
            input,
            target,
            error: None,
        });
        cx.notify();
    }

    /// 打开"新建远端文件并编辑"输入模态：提交后由编辑器打开空面板，
    /// 首次保存才在远端创建文件。
    pub(crate) fn sftp_open_editor_new_file_modal(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = sftp_text_input(window, cx, "", "新文件名称");
        input.update(cx, |state, cx| state.focus(window, cx));
        self.sftp_modals.input = Some(SftpInputModalState {
            tab_id,
            title: "新建远端文件".to_string(),
            prompt: "输入新文件名称（在当前远端目录下创建）:".to_string(),
            input,
            target: SftpInputTarget::RemoteEditorNewFile,
            error: None,
        });
        cx.notify();
    }

    pub(crate) fn sftp_open_rename_modal(
        &mut self,
        tab_id: TabId,
        is_remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp.as_ref() else {
            return;
        };
        let (title, prompt, initial_value, target) = if is_remote {
            let Some(selected) = panel.selected.clone() else {
                return;
            };
            let initial = selected
                .trim_end_matches('/')
                .rsplit_once('/')
                .map(|(_, name)| name.to_string())
                .unwrap_or_else(|| selected.clone());
            (
                "重命名远端文件".to_string(),
                "请输入新的名称:".to_string(),
                initial,
                SftpInputTarget::RemoteRename { old_path: selected },
            )
        } else {
            let Some(selected) = panel.local_selected.clone() else {
                return;
            };
            let initial = selected
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            (
                "重命名本地文件".to_string(),
                "请输入新的名称:".to_string(),
                initial,
                SftpInputTarget::LocalRename { old_path: selected },
            )
        };
        let input = sftp_text_input(window, cx, &initial_value, "新名称");
        input.update(cx, |state, cx| state.focus(window, cx));
        self.sftp_modals.input = Some(SftpInputModalState {
            tab_id,
            title,
            prompt,
            input,
            target,
            error: None,
        });
        cx.notify();
    }

    /// 打开「创建软链接」输入弹窗：远端基于当前选中条目，本地基于当前选中项目。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：创建软链接需要用户为新链接命名，且必须明确被指向的目标路径，
    ///   因此复用通用输入弹窗承载「名称输入 + 目标信息提示」；
    /// - 核心优势：默认名称在所选项目名后追加 `.link` 后缀，避免与既有项目重名
    ///   导致服务端直接拒绝，用户也可按需改写；
    /// - 代价与局限：链接固定创建于当前目录，暂不支持跨目录指定链接存放位置。
    pub(crate) fn sftp_open_symlink_modal(
        &mut self,
        tab_id: TabId,
        is_remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp.as_ref() else {
            return;
        };
        let (title, prompt, initial_value, target) = if is_remote {
            let Some(selected) = panel.selected.clone() else {
                return;
            };
            let Some(entry) = panel.entries.iter().find(|entry| entry.path == selected) else {
                return;
            };
            (
                "创建远端软链接".to_string(),
                format!("新链接将创建于当前远端目录，指向: {}", entry.path),
                format!("{}.link", entry.name),
                SftpInputTarget::RemoteSymlink {
                    target: entry.path.clone(),
                },
            )
        } else {
            let Some(selected) = panel.local_selected.clone() else {
                return;
            };
            let initial_name = selected
                .file_name()
                .map(|name| format!("{}.link", name.to_string_lossy()))
                .unwrap_or_else(|| "link".to_string());
            (
                "创建本地软链接".to_string(),
                format!("新链接将创建于当前本地目录，指向: {}", selected.display()),
                initial_name,
                SftpInputTarget::LocalSymlink {
                    target_path: selected,
                },
            )
        };
        let input = sftp_text_input(window, cx, &initial_value, "软链接名称");
        input.update(cx, |state, cx| state.focus(window, cx));
        self.sftp_modals.input = Some(SftpInputModalState {
            tab_id,
            title,
            prompt,
            input,
            target,
            error: None,
        });
        cx.notify();
    }

    pub(crate) fn commit_sftp_input_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.sftp_modals.input.take() else {
            return;
        };
        let tab_id = modal.tab_id;
        let name = modal.input.read(cx).value().trim().to_string();
        if name.is_empty() {
            let mut modal = modal;
            modal.error = Some("名称不能为空".to_string());
            let input = modal.input.clone();
            self.sftp_modals.input = Some(modal);
            input.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }
        if let Some(message) = name_validation_error(&modal.target, &name) {
            let mut modal = modal;
            modal.error = Some(message);
            let input = modal.input.clone();
            self.sftp_modals.input = Some(modal);
            input.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }

        // 新建文件在通用校验后直接分流：目录列表里已有同名条目时不创建，
        // 目录报错回填、文件直接打开编辑；全新路径才生成空编辑面板。
        // 整条分支在 match 之前完成，避免部分移动的模态再被改写。
        if matches!(modal.target, SftpInputTarget::RemoteEditorNewFile) {
            let existing = self
                .tabs
                .iter()
                .find(|t| t.id == tab_id)
                .and_then(|t| t.sftp.as_ref())
                .map(|panel| {
                    let path = join_remote(&panel.path, &name);
                    let is_dir = panel
                        .entries
                        .iter()
                        .find(|entry| entry.path == path)
                        .map(|entry| entry.is_dir);
                    (path, is_dir)
                });
            let Some((path, existing_is_dir)) = existing else {
                return;
            };
            match existing_is_dir {
                Some(true) => {
                    let mut modal = modal;
                    modal.error = Some(format!("同名目录已存在，无法创建文件：{name}"));
                    let input = modal.input.clone();
                    self.sftp_modals.input = Some(modal);
                    input.update(cx, |state, cx| state.focus(window, cx));
                    cx.notify();
                }
                // 列表里已有同名文件：交给既有打开流程（读取失败会在面板内报错）。
                Some(false) => self.sftp_edit_remote_file(tab_id, path, window, cx),
                // 目录列表未命中：按新文件打开空编辑面板。
                None => self.spawn_new_editor_panel(tab_id, path, name, window, cx),
            }
            return;
        }

        match modal.target {
            SftpInputTarget::RemoteMkdir => {
                let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
                    return;
                };
                let Some(panel) = tab.sftp.as_ref() else {
                    return;
                };
                let op = SftpOperation::Mkdir {
                    path: join_remote(&panel.path, &name),
                };
                self.send_sftp_op(tab_id, op, cx);
            }
            // 新建文件已在 match 之前的前置分支完整处理：同名条目分流、
            // 目录报错回填、文件直接打开或生成空面板。
            SftpInputTarget::RemoteEditorNewFile => {}
            SftpInputTarget::RemoteRename { old_path } => {
                let op = SftpOperation::Rename {
                    from: old_path.clone(),
                    to: join_remote(&parent_remote(&old_path), &name),
                };
                self.send_sftp_op(tab_id, op, cx);
            }
            SftpInputTarget::RemoteSymlink { target } => {
                let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
                    return;
                };
                let Some(panel) = tab.sftp.as_ref() else {
                    return;
                };
                let op = SftpOperation::Symlink {
                    target,
                    link_path: join_remote(&panel.path, &name),
                };
                self.send_sftp_op(tab_id, op, cx);
            }
            SftpInputTarget::LocalMkdir => {
                if let Some(panel) = self
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == tab_id)
                    .and_then(|t| t.sftp.as_mut())
                {
                    let new_dir = panel.local_path.join(&name);
                    if let Err(err) = std::fs::create_dir_all(&new_dir) {
                        panel.local_error = Some(format!("新建本地文件夹失败: {err}"));
                    } else {
                        panel.status = Some(format!("已成功创建本地文件夹: {name}"));
                        self.spawn_local_dir_refresh(tab_id, None, cx);
                    }
                    cx.notify();
                }
            }
            SftpInputTarget::LocalRename { old_path } => {
                if let Some(panel) = self
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == tab_id)
                    .and_then(|t| t.sftp.as_mut())
                {
                    if let Some(parent) = old_path.parent() {
                        let new_path = parent.join(&name);
                        if let Err(err) = std::fs::rename(&old_path, &new_path) {
                            panel.local_error = Some(format!("重命名本地文件失败: {err}"));
                        } else {
                            panel.status = Some(format!("已重命名为: {name}"));
                            self.spawn_local_dir_refresh(tab_id, Some(new_path), cx);
                        }
                    }
                    cx.notify();
                }
            }
            SftpInputTarget::LocalSymlink { target_path } => {
                self.create_local_symlink(tab_id, &target_path, &name, cx);
            }
        }
        self.focus_active_sftp_panel(window, cx);
        cx.notify();
    }

    /// 在当前本地目录创建指向指定目标的软链接（仅 Unix 平台支持）。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：本地栏与远端栏保持右键菜单能力对称，避免用户在两栏间形成心智差异；
    /// - 核心优势：直接复用标准库 `std::os::unix::fs::symlink`，无额外依赖，
    ///   且创建后立即刷新并选中新链接，反馈闭环完整；
    /// - 代价与局限：Windows 需要特权与文件/目录区分调用，当前不提供该平台支持，
    ///   以显式报错提示代替静默失败。
    fn create_local_symlink(
        &mut self,
        tab_id: TabId,
        target_path: &Path,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.sftp.as_mut())
        else {
            return;
        };
        let link_path = panel.local_path.join(name);
        #[cfg(unix)]
        let result =
            std::os::unix::fs::symlink(target_path, &link_path).map_err(|error| error.to_string());
        #[cfg(not(unix))]
        let result = Err("当前平台暂不支持创建本地软链接".to_string());
        match result {
            Ok(()) => {
                panel.status = Some(format!("已创建本地软链接: {name}"));
                self.spawn_local_dir_refresh(tab_id, Some(link_path), cx);
            }
            Err(err) => {
                panel.local_error = Some(format!("创建本地软链接失败: {err}"));
            }
        }
        cx.notify();
    }
}

/// 校验输入弹窗中的名称是否可以安全地参与路径拼接，非法时返回中文提示文本。
///
/// 设计原理（Why）：
/// - 实现初衷：新建/重命名/软链接的名称会与当前目录直接拼接成完整路径，
///   若允许 `/` 或 `..` 输入，操作会逃逸出当前目录（路径穿越），
///   与文件管理器中用户可见的范围不一致，软链接场景影响面尤大；
/// - 核心优势：在唯一提交口（`commit_sftp_input_modal`）统一拦截，
///   新增目标类型不会遗漏校验，也无需在各调用点重复判断；
/// - 代价与局限：本地「新建文件夹」保留 `a/b` 相对多级创建能力，
///   但同样拒绝绝对路径与 `..` 逃逸片段；其余目标严格要求单层名称，
///   重命名等场景不再支持输入含 `/` 的路径。
fn name_validation_error(target: &SftpInputTarget, name: &str) -> Option<String> {
    if matches!(target, SftpInputTarget::LocalMkdir) {
        if name.starts_with('/') {
            return Some("名称不能是绝对路径（仅支持当前目录内的相对多级路径）".to_string());
        }
        return name
            .split('/')
            .any(|segment| segment == "..")
            .then(|| "名称不能包含 .. 路径片段（不允许逃逸当前目录）".to_string());
    }
    if name.contains('/') {
        return Some("名称不能包含路径分隔符 /".to_string());
    }
    if name == "." || name == ".." {
        return Some("名称不能为 . 或 ..".to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    // 显式导入被测函数：避免 glob 引入 gpui 同名 `test` 属性宏导致展开递归
    use super::{SftpInputTarget, name_validation_error};

    #[test]
    fn test_name_validation_rejects_traversal() {
        let remote_symlink = SftpInputTarget::RemoteSymlink {
            target: "/srv/app".to_string(),
        };
        assert!(name_validation_error(&remote_symlink, "../evil").is_some());
        assert!(name_validation_error(&remote_symlink, "sub/link").is_some());
        assert!(name_validation_error(&remote_symlink, "..").is_some());
        assert!(name_validation_error(&remote_symlink, "app.link").is_none());

        assert!(name_validation_error(&SftpInputTarget::RemoteMkdir, "a/b").is_some());
        assert!(name_validation_error(&SftpInputTarget::RemoteMkdir, "目录").is_none());

        let local_rename = SftpInputTarget::LocalRename {
            old_path: std::path::PathBuf::from("/tmp/old"),
        };
        assert!(name_validation_error(&local_rename, "..").is_some());
        assert!(name_validation_error(&local_rename, "new-name").is_none());
    }

    #[test]
    fn test_name_validation_local_mkdir_keeps_multi_level() {
        assert!(name_validation_error(&SftpInputTarget::LocalMkdir, "a/b").is_none());
        assert!(name_validation_error(&SftpInputTarget::LocalMkdir, "a/../b").is_some());
        assert!(name_validation_error(&SftpInputTarget::LocalMkdir, "..").is_some());
        assert!(name_validation_error(&SftpInputTarget::LocalMkdir, "/tmp/escape").is_some());
    }
}
