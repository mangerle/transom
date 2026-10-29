//! 本地面板阻塞文件 IO 的后台化封装。
//!
//! 设计原理（Why）：
//! - 实现初衷：本地大目录 `read_dir` 与递归删除在慢速磁盘、FUSE/网络挂载上
//!   可能挂起秒级甚至分钟级；这些调用出现在 UI 事件回调里会直接冻结整个
//!   界面（GPUI 的渲染与事件分发共用主线程）；
//! - 核心优势：阻塞段交给专用线程执行（缩略图解码等短任务仍走 gpui 后台
//!   执行器，两者按预期耗时区分），完成后回到 `AppShell` 实体应用结果；
//!   过期防护通过比对回填时的目录路径实现，导航离开后旧结果不会覆盖新目录；
//! - 代价与局限：刷新从"同步可见"变为"异步落地"，期间面板短暂显示旧内容
//!   （导航场景先清空条目给出即时反馈）；同一目录的并发刷新以后完成者胜出。

use super::{SftpConfirmModalState, SftpPendingDelete, list_local_dir};
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
            .tab(tab_id)
            .and_then(|tab| tab.sftp())
            .map(|panel| panel.local.path.clone())
        else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let scan_dir = dir.clone();
            let Some(result) = spawn_blocking_local(move || list_local_dir(&scan_dir)).await else {
                log::error!("本地目录读取线程启动失败，跳过本次刷新");
                return;
            };
            let _ = this.update(cx, |this, cx| {
                let Some(panel) = this
                    .workspace
                    .tab_mut(tab_id)
                    .and_then(|tab| tab.sftp_mut())
                else {
                    return;
                };
                if panel.local.path != dir {
                    // 面板已导航到其他目录，丢弃过期结果
                    return;
                }
                match result {
                    Ok(entries) => {
                        let valid_paths: BTreeSet<PathBuf> =
                            entries.iter().map(|entry| entry.path.clone()).collect();
                        panel.local.entries = entries;
                        if let Some(path) = select_after {
                            panel.select_local(path);
                        } else {
                            panel.sync_local_selection(&valid_paths);
                        }
                        panel.clear_local_error();
                    }
                    Err(error) => {
                        panel.set_local_error(format!("读取本地目录失败: {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 批量删除本地选中项：先弹确认，确认后再执行。
    pub(crate) fn sftp_local_delete_selection(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.dismiss_sftp_hover_preview(tab_id, cx);
        let Some(panel) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.sftp_mut())
        else {
            return;
        };
        let paths = panel.selected_local_paths();
        if paths.is_empty() {
            panel.set_local_error("请先选择一个本地项目".to_string());
            cx.notify();
            return;
        }
        let count = paths.len();
        self.sftp_modals.confirm = Some(SftpConfirmModalState {
            action: SftpPendingDelete::Local { tab_id, paths },
            title: "删除本地项目".to_string(),
            description: format!(
                "将永久删除 {count} 个本地项目（目录按递归删除），不经回收站，此操作不可撤销。"
            ),
        });
        cx.notify();
    }

    /// 执行已确认的本地删除：目录递归删除，符号链接只删除链接本身。
    ///
    /// 删除与随后的目录刷新都是阻塞 IO，整体放后台执行；期间状态栏先提示
    /// "正在删除"，完成后回填成功/失败统计并刷新列表。
    pub(crate) fn perform_sftp_local_delete(
        &mut self,
        tab_id: TabId,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_sftp_hover_preview(tab_id, cx);
        let Some(panel) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.sftp_mut())
        else {
            return;
        };
        panel.clear_local_error();
        panel.set_status(format!("正在删除 {} 个本地项目...", paths.len()));
        let dir = panel.local.path.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let Some(outcome) = spawn_blocking_local(move || delete_local_paths(&paths)).await
            else {
                log::error!("本地删除线程启动失败，本次删除未执行");
                return;
            };
            let scan_dir = dir.clone();
            let entries = spawn_blocking_local(move || list_local_dir(&scan_dir)).await;
            let _ = this.update(cx, |this, cx| {
                let Some(panel) = this
                    .workspace
                    .tab_mut(tab_id)
                    .and_then(|tab| tab.sftp_mut())
                else {
                    return;
                };
                if outcome.failures.is_empty() {
                    panel.set_status(format!("已删除 {} 个本地项目", outcome.deleted));
                } else {
                    let detail = outcome
                        .failures
                        .iter()
                        .take(3)
                        .map(|(path, error)| format!("{}（{error}）", path.display()))
                        .collect::<Vec<_>>()
                        .join("；");
                    log::warn!(
                        "本地删除部分失败，成功 {} 个，失败 {} 个，示例: {detail}",
                        outcome.deleted,
                        outcome.failures.len()
                    );
                    panel.set_local_error(format!(
                        "已删除 {} 个本地项目，{} 个失败：{detail}",
                        outcome.deleted,
                        outcome.failures.len()
                    ));
                }
                // 删除期间用户可能已导航到其他目录：只在目录未变时回填刷新
                if panel.local.path == dir
                    && let Some(Ok(entries)) = entries
                {
                    let valid_paths: BTreeSet<PathBuf> =
                        entries.iter().map(|entry| entry.path.clone()).collect();
                    panel.local.entries = entries;
                    panel.sync_local_selection(&valid_paths);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// 本地批量删除的结果：成功数与逐项失败原因。
struct LocalDeleteOutcome {
    deleted: usize,
    failures: Vec<(PathBuf, String)>,
}

/// 串行删除本地路径：目录递归删除，其他（含符号链接）按文件删除。
/// 单项失败不中断，保留每项路径与原因供界面逐条提示。
/// 阻塞函数，仅允许经 [`spawn_blocking_local`] 在专用线程上调用。
fn delete_local_paths(paths: &[PathBuf]) -> LocalDeleteOutcome {
    let mut deleted = 0usize;
    let mut failures = Vec::new();
    for path in paths {
        let result = match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path),
            _ => std::fs::remove_file(path),
        };
        match result {
            Ok(()) => deleted = deleted.saturating_add(1),
            Err(error) => failures.push((path.clone(), error.to_string())),
        }
    }
    LocalDeleteOutcome { deleted, failures }
}

/// 在专用线程执行阻塞的本地文件操作。
///
/// 设计原理（Why）：递归删除与超大目录扫描可能运行数十秒，而 GPUI 的后台
/// 执行器同时承担缩略图解码等短任务，占住其工作线程会拖慢其它后台工作。
/// 专用线程只跑这一件事，发送结果后立即退出；异步侧收到结果即 join，
/// 线程生命周期全程可追踪，不产生脱缰线程。
pub(crate) fn spawn_blocking_local<T, F>(job: F) -> impl std::future::Future<Output = Option<T>>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = flume::bounded(1);
    let spawned = std::thread::Builder::new()
        .name("transom-sftp-local".into())
        .spawn(move || {
            let _ = tx.send(job());
        });
    async move {
        let handle = match spawned {
            Ok(handle) => handle,
            Err(error) => {
                log::error!("创建本地文件操作线程失败，本次操作已取消: {error}");
                return None;
            }
        };
        let result = rx.recv_async().await.ok();
        let _ = handle.join();
        result
    }
}
