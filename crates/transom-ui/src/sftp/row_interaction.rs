//! 本地与远端文件行的共用交互：外观、悬停预览、选择、双击与拖拽。
//!
//! 设计原理（Why）：本地栏与远端栏的行外观完全相同，此前各自复制了约
//! 一百行“构造行 + 悬停 + 左键选择 + 右键主项 + 拖拽”的接线，差异只有
//! 数据源、选中集合与导航入口。收敛到一个以 [`SftpSide`] 分发的构造函数后，
//! 两侧只负责准备行数据，交互语义与视觉不再各自漂移。
//!
//! 路径用 [`SftpRowPath`] 区分本地与远端：本地路径可能是非 UTF-8，展示与
//! 拖拽载荷允许有损转换，但选择、连选与导航必须回到原始 `PathBuf`，不能
//! 从展示字符串反推。
//!
//! 代价与局限：两侧的右键上下文菜单仍留在各自模块——菜单项与业务动作
//! 差异较大，强行合并会把分发逻辑变成新的分支泥团。行内行为新增时，
//! 需要在本模块与两侧数据准备点同时评估。

use super::file_row::{FileRowProps, file_row};
use super::preview::{PreviewKey, SftpHoverTarget};
use super::side_remote::{SftpDragItem, SftpDragPreview};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::*;
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use transom_core::{ConnectionId, TabId};

/// 文件行所属的一侧。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SftpSide {
    Local,
    Remote,
}

/// 行的原始路径：本地保留 `PathBuf` 以免丢失非 UTF-8 文件名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SftpRowPath {
    Local(PathBuf),
    Remote(String),
}

impl SftpRowPath {
    /// 展示与拖拽载荷使用的字符串；本地路径可能有损。
    pub(crate) fn display(&self) -> Cow<'_, str> {
        match self {
            Self::Local(path) => path.to_string_lossy(),
            Self::Remote(path) => Cow::Borrowed(path),
        }
    }

    fn local(&self) -> Option<&Path> {
        match self {
            Self::Local(path) => Some(path),
            Self::Remote(_) => None,
        }
    }

    fn remote(&self) -> Option<&str> {
        match self {
            Self::Remote(path) => Some(path),
            Self::Local(_) => None,
        }
    }
}

/// 构造一条文件行所需的全部数据与上下文。
///
/// `visible_paths` 是本次渲染的可见条目路径快照，供 Shift 连选计算区间；
/// `source_connection_id` 只在远端行拖拽时携带，本地拖拽为 `None`。
pub(crate) struct SftpRowContext {
    pub(crate) tab_id: TabId,
    pub(crate) side: SftpSide,
    pub(crate) index: usize,
    pub(crate) path: SftpRowPath,
    pub(crate) name: String,
    pub(crate) is_dir: bool,
    pub(crate) is_parent: bool,
    pub(crate) size: u64,
    pub(crate) permissions: Option<u32>,
    pub(crate) selected: bool,
    pub(crate) visible_paths: Rc<Vec<SftpRowPath>>,
    pub(crate) source_connection_id: Option<ConnectionId>,
}

impl Clone for SftpRowContext {
    fn clone(&self) -> Self {
        Self {
            tab_id: self.tab_id,
            side: self.side,
            index: self.index,
            path: self.path.clone(),
            name: self.name.clone(),
            is_dir: self.is_dir,
            is_parent: self.is_parent,
            size: self.size,
            permissions: self.permissions,
            selected: self.selected,
            visible_paths: self.visible_paths.clone(),
            source_connection_id: self.source_connection_id,
        }
    }
}

impl AppShell {
    /// 构造一条带完整交互的文件行；父目录项只保留选择与右键，不参与拖拽。
    pub(crate) fn sftp_file_row(
        &self,
        row: SftpRowContext,
        palette: &Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = match row.side {
            SftpSide::Local => format!("local-file-{}", row.name),
            SftpSide::Remote => format!("remote-file-{}", row.path.display()),
        };
        let base = file_row(FileRowProps {
            id,
            name: &row.name,
            is_dir: row.is_dir,
            is_parent: row.is_parent,
            size: row.size,
            permissions: row.permissions,
            selected: row.selected,
            palette,
        });
        let hover_target = SftpHoverTarget {
            tab_id: row.tab_id,
            key: PreviewKey {
                is_remote: row.side == SftpSide::Remote,
                path: row.path.display().into_owned(),
            },
            name: row.name.clone(),
            is_dir: row.is_dir,
            size: row.size,
        };
        let hover_row = row.clone();
        let left_row = row.clone();
        let right_row = row.clone();
        let base = base
            .on_hover(cx.listener(move |this, hovered, window, cx| {
                this.sftp_hover_preview(hover_target.clone(), *hovered, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.sftp_row_left_click(&left_row, event, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    this.sftp_row_right_click(&right_row, window, cx);
                }),
            );
        if row.is_parent {
            return base;
        }
        let payload = SftpDragItem {
            source_tab_id: row.tab_id,
            source_connection_id: row.source_connection_id,
            paths: vec![row.path.display().into_owned()],
            file_name: row.name.clone(),
            is_dir: row.is_dir,
        };
        let preview_name = hover_row.name.clone();
        base.on_drag(payload, move |_, _, _, cx| {
            cx.new(|_| SftpDragPreview::new(preview_name.clone(), 1))
        })
    }

    fn sftp_row_left_click(
        &mut self,
        row: &SftpRowContext,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_sftp_hover_preview(row.tab_id, cx);
        if let Some(panel) = self.workspace.tab(row.tab_id).and_then(|tab| tab.sftp()) {
            window.focus(&panel.focus_handle, cx);
        }
        if event.click_count >= 2 {
            match row.side {
                SftpSide::Local => {
                    if row.is_parent {
                        self.sftp_local_go_up(row.tab_id, window, cx);
                    } else if row.is_dir
                        && let Some(path) = row.path.local()
                    {
                        self.sftp_local_navigate(row.tab_id, path.to_path_buf(), window, cx);
                    }
                }
                SftpSide::Remote => {
                    if row.is_parent {
                        self.sftp_go_up(row.tab_id, window, cx);
                    } else if row.is_dir {
                        if let Some(path) = row.path.remote() {
                            self.sftp_navigate(row.tab_id, path.to_string(), window, cx);
                        }
                    } else if let Some(path) = row.path.remote() {
                        self.sftp_edit_remote_file(row.tab_id, path.to_string(), window, cx);
                    }
                }
            }
            return;
        }
        if row.is_parent {
            return;
        }
        let Some(panel) = self
            .workspace
            .tab_mut(row.tab_id)
            .and_then(|tab| tab.sftp_mut())
        else {
            return;
        };
        if event.modifiers.shift {
            let anchor = match row.side {
                SftpSide::Local => panel.local.anchor.clone().map(SftpRowPath::Local),
                SftpSide::Remote => panel.remote.anchor.clone().map(SftpRowPath::Remote),
            };
            let anchor_index =
                anchor.and_then(|anchor| row.visible_paths.iter().position(|path| path == &anchor));
            if let Some(anchor_index) = anchor_index {
                let (start, end) = if anchor_index <= row.index {
                    (anchor_index, row.index)
                } else {
                    (row.index, anchor_index)
                };
                match row.side {
                    SftpSide::Local => {
                        let paths: Vec<PathBuf> = row.visible_paths[start..=end]
                            .iter()
                            .filter_map(|path| path.local().map(Path::to_path_buf))
                            .collect();
                        if let Some(primary) = row.path.local() {
                            panel.select_local_range(paths, primary.to_path_buf());
                        }
                    }
                    SftpSide::Remote => {
                        let paths: Vec<String> = row.visible_paths[start..=end]
                            .iter()
                            .filter_map(|path| path.remote().map(str::to_string))
                            .collect();
                        if let Some(primary) = row.path.remote() {
                            panel.select_remote_range(paths, primary.to_string());
                        }
                    }
                }
            } else {
                match row.side {
                    SftpSide::Local => {
                        if let Some(path) = row.path.local() {
                            panel.select_local(path.to_path_buf());
                        }
                    }
                    SftpSide::Remote => {
                        if let Some(path) = row.path.remote() {
                            panel.select_remote(path.to_string());
                        }
                    }
                }
            }
        } else if event.modifiers.control || event.modifiers.platform {
            match row.side {
                SftpSide::Local => {
                    if let Some(path) = row.path.local() {
                        panel.toggle_local_selection(path.to_path_buf());
                    }
                }
                SftpSide::Remote => {
                    if let Some(path) = row.path.remote() {
                        panel.toggle_remote_selection(path.to_string());
                    }
                }
            }
        } else {
            match row.side {
                SftpSide::Local => {
                    if let Some(path) = row.path.local() {
                        panel.select_local(path.to_path_buf());
                    }
                }
                SftpSide::Remote => {
                    if let Some(path) = row.path.remote() {
                        panel.select_remote(path.to_string());
                    }
                }
            }
        }
        let (selected_count, noun) = match row.side {
            SftpSide::Local => (panel.selected_local_paths().len(), "本地"),
            SftpSide::Remote => (panel.selected_remote_paths().len(), "远端"),
        };
        if selected_count > 1 {
            panel.queue.status = Some(format!("已选择 {selected_count} 个{noun}项目"));
        }
        let name = row.name.clone();
        panel
            .remote
            .name_input
            .update(cx, |state, cx| state.set_value(name, window, cx));
        cx.notify();
    }

    fn sftp_row_right_click(
        &mut self,
        row: &SftpRowContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_sftp_hover_preview(row.tab_id, cx);
        if let Some(panel) = self
            .workspace
            .tab_mut(row.tab_id)
            .and_then(|tab| tab.sftp_mut())
        {
            match row.side {
                SftpSide::Local => {
                    if let Some(path) = row.path.local() {
                        let path = path.to_path_buf();
                        if panel.local.selection.contains(&path) {
                            panel.set_local_primary(path);
                        } else {
                            panel.select_local(path);
                        }
                    }
                }
                SftpSide::Remote => {
                    if let Some(path) = row.path.remote() {
                        let path = path.to_string();
                        if panel.remote.selection.contains(&path) {
                            panel.set_remote_primary(path);
                        } else {
                            panel.select_remote(path);
                        }
                    }
                }
            }
            window.focus(&panel.focus_handle, cx);
        }
        cx.notify();
    }
}
