//! 本地面板阻塞文件 IO 的后台化封装。
//!
//! 设计原理（Why）：
//! - 实现初衷：本地大目录 `read_dir` 与递归删除在慢速磁盘、FUSE/网络挂载上
//!   可能挂起秒级甚至分钟级；这些调用出现在 UI 事件回调里会直接冻结整个
//!   界面（GPUI 的渲染与事件分发共用主线程）；
//! - 核心优势：阻塞段统一交给 gpui 后台执行器线程池，完成后回到 `AppShell`
//!   实体应用结果，与悬浮预览缩略图加载（`preview_actions`）是同一套模式；
//!   过期防护通过比对回填时的目录路径实现，导航离开后旧结果不会覆盖新目录；
//! - 代价与局限：刷新从"同步可见"变为"异步落地"，期间面板短暂显示旧内容
//!   （导航场景先清空条目给出即时反馈）；同一目录的并发刷新以后完成者胜出。

use super::list_local_dir;
use crate::app::AppShell;
use gpui_kit::*;
use std::collections::BTreeSet;
use std::path::PathBuf;
use transom_core::TabId;

impl AppShell {
    /// 后台读取面板当前本地目录并回填条目。
    ///
    /// `select_after` 提供路径时，回填成功后单选该条目（用于重命名、新建软链接
    /// 后立刻定位新条目）；否则按新条目集合校正既有选中，剔除已消失的路径。
    pub(crate) fn spawn_local_dir_refresh(
        &mut self,
        tab_id: TabId,
        select_after: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let Some(dir) = self
            .workspace
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_ref())
            .map(|panel| panel.local_path.clone())
        else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let scan_dir = dir.clone();
            let result = cx
                .background_executor()
                .spawn(async move { list_local_dir(&scan_dir) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(panel) = this
                    .workspace
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id == tab_id)
                    .and_then(|tab| tab.sftp.as_mut())
                else {
                    return;
                };
                if panel.local_path != dir {
                    // 面板已导航到其他目录，丢弃过期结果
                    return;
                }
                match result {
                    Ok(entries) => {
                        let valid_paths: BTreeSet<PathBuf> =
                            entries.iter().map(|entry| entry.path.clone()).collect();
                        panel.local_entries = entries;
                        if let Some(path) = select_after {
                            panel.select_local(path);
                        } else {
                            panel.sync_local_selection(&valid_paths);
                        }
                        panel.local_error = None;
                    }
                    Err(error) => {
                        panel.local_error = Some(format!("读取本地目录失败: {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 批量删除本地选中项；目录递归删除，符号链接只删除链接本身。
    ///
    /// 删除与随后的目录刷新都是阻塞 IO，整体放后台执行；期间状态栏先提示
    /// "正在删除"，完成后回填成功/失败统计并刷新列表。
    pub(crate) fn sftp_local_delete_selection(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.dismiss_sftp_hover_preview(tab_id, cx);
        let Some(panel) = self
            .workspace
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        else {
            return;
        };
        let paths = panel.selected_local_paths();
        if paths.is_empty() {
            panel.local_error = Some("请先选择一个本地项目".to_string());
            cx.notify();
            return;
        }
        panel.local_error = None;
        panel.status = Some(format!("正在删除 {} 个本地项目...", paths.len()));
        let dir = panel.local_path.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (deleted, failed) = cx
                .background_executor()
                .spawn(async move { delete_local_paths(&paths) })
                .await;
            let scan_dir = dir.clone();
            let entries = cx
                .background_executor()
                .spawn(async move { list_local_dir(&scan_dir) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(panel) = this
                    .workspace
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id == tab_id)
                    .and_then(|tab| tab.sftp.as_mut())
                else {
                    return;
                };
                if failed > 0 {
                    panel.local_error =
                        Some(format!("已删除 {deleted} 个本地项目，{failed} 个删除失败"));
                } else {
                    panel.status = Some(format!("已删除 {deleted} 个本地项目"));
                }
                // 删除期间用户可能已导航到其他目录：只在目录未变时回填刷新
                if panel.local_path == dir
                    && let Ok(entries) = entries
                {
                    let valid_paths: BTreeSet<PathBuf> =
                        entries.iter().map(|entry| entry.path.clone()).collect();
                    panel.local_entries = entries;
                    panel.sync_local_selection(&valid_paths);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// 串行删除本地路径：目录递归删除，其他（含符号链接）按文件删除。
/// 返回 (成功数, 失败数)；单项失败不中断，保证批量操作的统计完整。
/// 阻塞函数，仅允许在后台执行器线程池上调用。
fn delete_local_paths(paths: &[PathBuf]) -> (usize, usize) {
    let mut deleted = 0usize;
    let mut failed = 0usize;
    for path in paths {
        let result = match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path),
            _ => std::fs::remove_file(path),
        };
        match result {
            Ok(()) => deleted = deleted.saturating_add(1),
            Err(_) => failed = failed.saturating_add(1),
        }
    }
    (deleted, failed)
}
