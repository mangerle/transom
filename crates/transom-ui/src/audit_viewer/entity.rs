//! 审计日志查看器实体：打开状态、文件列表、选中内容与后台扫描都归它。
//!
//! # 设计原理（Why）
//! 查看器此前由外壳字段 `ModalHub.tools.audit_viewer` 与散落的 `impl AppShell`
//! 方法共同维护：打开、扫描、选中、删除、复制与渲染分居 `ops.rs`、`view.rs`，
//! 读者必须同时读三处才能回答“谁拥有这次打开”。实体化后，一次打开期的全部
//! 状态与在途任务同生共死，外壳只负责计算会话信息与消费 [`AuditViewerEvent`]。
//!
//! `seq` 是每次打开/切换范围时递增的世代号：后台扫描返回时若世代已变，结果
//! 直接丢弃。这样快速切换“仅当前会话/全部会话”不会让旧结果覆盖新列表。
//!
//! # 代价与局限
//! 渲染拆到 `render.rs`，事件只携带结果，外壳不再能回读实体补数据；所有跨域
//! 信息必须在事件里带全。复制与删除的状态栏提示因此改由外壳订阅后转发。

use super::types::{AuditViewer, MAX_AUDIT_LOG_BYTES, find_session_audit_logs, read_log_content};
use crate::app::spawn_blocking_local;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::path::PathBuf;
use transom_platform::paths;

/// 审计查看器交给外壳处理的意图。
///
/// 事件自带全部数据：外壳处理时禁止回读实体，否则会在实体回调期间触发
/// GPUI 的双重租借崩溃。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuditViewerEvent {
    /// 状态栏提示（复制成功、删除成功）。
    Notice(String),
    /// 操作失败，由外壳转为应用内错误提示。
    Error(String),
    /// 空态引导：查看器已自行关闭，外壳据此打开设置。
    OpenSettings,
}

impl EventEmitter<AuditViewerEvent> for AuditViewerView {}

/// 审计日志查看器实体。
pub(crate) struct AuditViewerView {
    /// 未打开时为 `None`；每次打开都重新创建搜索输入，避免残留上一次的关键字。
    state: Option<AuditViewer>,
    /// 审计目录在构造时固定，避免渲染或异步任务里重复读全局路径。
    audit_dir: PathBuf,
    /// 扫描世代号：关闭或重新扫描时递增，过期结果直接丢弃。
    seq: u64,
}

impl AuditViewerView {
    /// 组装常驻实体；打开状态由 [`Self::open`] 控制。
    pub(crate) fn new() -> Self {
        Self {
            state: None,
            audit_dir: paths::audit_dir(),
            seq: 0,
        }
    }

    /// 查看器是否展开。
    pub(crate) fn is_open(&self) -> bool {
        self.state.is_some()
    }

    /// 当前打开期的只读状态；渲染与状态查询共用同一入口。
    pub(crate) fn viewer(&self) -> Option<&AuditViewer> {
        self.state.as_ref()
    }

    /// 当前选中项下标；未打开时返回 0，供视图按钮读取。
    pub(crate) fn selected_index(&self) -> usize {
        self.state
            .as_ref()
            .map_or(0, |viewer| viewer.selected_index)
    }

    /// 关闭查看器；在途扫描结果因世代号变化被丢弃。
    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.seq = self.seq.wrapping_add(1);
        self.state = None;
        cx.notify();
    }

    /// 打开查看器并异步扫描审计目录。
    ///
    /// `show_all` 为 `true` 时忽略会话与主机前缀；`window` 是创建搜索输入
    /// 所必需的，`InputState` 的焦点句柄在构造时注册。
    pub(crate) fn open(
        &mut self,
        target_title: String,
        session_name: Option<String>,
        host: Option<String>,
        show_all: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("audit_viewer.search_placeholder").to_string())
        });
        self.state = Some(AuditViewer {
            target_title,
            session_name,
            host,
            entries: Vec::new(),
            selected_index: 0,
            content: String::new(),
            is_truncated: false,
            search_input,
            show_all,
        });
        self.start_scan(cx);
        cx.notify();
    }

    /// 切换查看的审计日志文件并异步读取内容。
    pub(crate) fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(viewer) = self.state.as_mut() else {
            return;
        };
        if index >= viewer.entries.len() {
            return;
        }
        viewer.selected_index = index;
        let path = viewer.entries[index].path.clone();
        cx.spawn(async move |this, cx| {
            let Some((content, is_truncated)) =
                spawn_blocking_local(move || read_log_content(&path, MAX_AUDIT_LOG_BYTES)).await
            else {
                log::error!("读取审计日志的线程创建失败");
                return;
            };
            let _ = this.update(cx, |view, cx| {
                if let Some(viewer) = view.state.as_mut()
                    && viewer.selected_index == index
                {
                    viewer.content = content;
                    viewer.is_truncated = is_truncated;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// 切换“仅当前会话”与“全部会话”范围。
    pub(crate) fn toggle_scope(&mut self, cx: &mut Context<Self>) {
        let Some(viewer) = self.state.as_mut() else {
            return;
        };
        viewer.show_all = !viewer.show_all;
        viewer.entries.clear();
        viewer.selected_index = 0;
        viewer.content.clear();
        viewer.is_truncated = false;
        self.start_scan(cx);
        cx.notify();
    }

    /// 复制当前审计日志内容到剪贴板。
    pub(crate) fn copy_content(&mut self, cx: &mut Context<Self>) {
        let Some(content) = self.state.as_ref().map(|viewer| viewer.content.clone()) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(content));
        cx.emit(AuditViewerEvent::Notice(
            rust_i18n::t!("audit_viewer.copied").to_string(),
        ));
        cx.notify();
    }

    /// 删除指定的审计日志文件，并选中相邻文件继续展示。
    pub(crate) fn delete(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(viewer) = self.state.as_mut() else {
            return;
        };
        if index >= viewer.entries.len() {
            return;
        }

        let entry = viewer.entries.remove(index);
        let log_path = entry.path.clone();
        if viewer.entries.is_empty() {
            viewer.selected_index = 0;
            viewer.content.clear();
            viewer.is_truncated = false;
        } else {
            if viewer.selected_index >= viewer.entries.len() {
                viewer.selected_index = viewer.entries.len() - 1;
            }
            viewer.content.clear();
            viewer.is_truncated = false;
        }
        let next_path = viewer
            .entries
            .get(viewer.selected_index)
            .map(|entry| entry.path.clone());
        cx.spawn(async move |this, cx| {
            let Some(removed) = spawn_blocking_local(move || std::fs::remove_file(&log_path)).await
            else {
                log::error!("删除审计日志的线程创建失败");
                return;
            };
            let read = if let Some(path) = next_path {
                let read_path = path.clone();
                let Some((content, is_truncated)) =
                    spawn_blocking_local(move || read_log_content(&read_path, MAX_AUDIT_LOG_BYTES))
                        .await
                else {
                    log::error!("删除审计日志后读取下一个文件的线程创建失败");
                    return;
                };
                Some((path, content, is_truncated))
            } else {
                None
            };
            let _ = this.update(cx, |view, cx| {
                if let Some(viewer) = view.state.as_mut()
                    && let Some((path, content, is_truncated)) = read
                    && viewer
                        .entries
                        .get(viewer.selected_index)
                        .is_some_and(|entry| entry.path == path)
                {
                    viewer.content = content;
                    viewer.is_truncated = is_truncated;
                }
                match removed {
                    Ok(()) => cx.emit(AuditViewerEvent::Notice(
                        rust_i18n::t!("audit_viewer.deleted").to_string(),
                    )),
                    Err(error) => {
                        log::error!("删除审计日志文件失败，原因: {error}");
                        cx.emit(AuditViewerEvent::Error(
                            rust_i18n::t!("audit_viewer.delete_failed").to_string(),
                        ));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// 重新扫描审计目录；旧世代的结果在返回时被丢弃。
    fn start_scan(&mut self, cx: &mut Context<Self>) {
        let Some(viewer) = self.state.as_ref() else {
            return;
        };
        let session_name = viewer.session_name.clone();
        let host = viewer.host.clone();
        let show_all = viewer.show_all;
        let audit_dir = self.audit_dir.clone();
        self.seq = self.seq.wrapping_add(1);
        let seq = self.seq;
        cx.spawn(async move |this, cx| {
            let Some((entries, content, is_truncated)) = spawn_blocking_local(move || {
                let entries = find_session_audit_logs(
                    &audit_dir,
                    session_name.as_deref(),
                    host.as_deref(),
                    show_all,
                );
                let (content, is_truncated) = entries.first().map_or_else(
                    || (String::new(), false),
                    |first| read_log_content(&first.path, MAX_AUDIT_LOG_BYTES),
                );
                (entries, content, is_truncated)
            })
            .await
            else {
                log::error!("扫描审计日志的线程创建失败，查看器保持空列表");
                return;
            };
            let _ = this.update(cx, |view, cx| {
                if view.seq != seq {
                    return;
                }
                if let Some(viewer) = view.state.as_mut() {
                    viewer.entries = entries;
                    viewer.selected_index = 0;
                    viewer.content = content;
                    viewer.is_truncated = is_truncated;
                }
                cx.notify();
            });
        })
        .detach();
    }
}
