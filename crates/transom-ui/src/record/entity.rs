//! 终端回放选择弹窗实体：打开状态、录制目录与扫描世代号都归它。
//!
//! # 设计原理（Why）
//! 回放弹窗此前由 `ModalHub.tools.replay` 与散落的 `impl AppShell` 方法共同维护：
//! 打开、扫描与渲染分居 `ops.rs`、`view.rs`，读者要同时读三处才能回答“谁拥有
//! 这次打开”。实体化后，一次打开期的目标标签、脚本清单与在途扫描同生共死，
//! 外壳只持有句柄并消费意图事件。
//!
//! `seq` 是每次打开或关闭时递增的世代号：后台扫描返回时若世代已变，结果直接
//! 丢弃，避免关闭后旧结果把弹窗重新填上名单。
//!
//! # 代价与局限
//! 渲染拆到 `render.rs`，打开播放器与终端回放等窗口级动作只能由事件交回外壳；
//! 所有跨域参数必须随事件带全，外壳不再回读实体。

use super::types::{ReplayModalState, scan_recordings};
use crate::app::spawn_blocking_local;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::TabId;
use transom_platform::paths;

/// 回放弹窗交给外壳处理的意图。
///
/// 事件自带全部数据：外壳处理时禁止回读实体，否则会在实体回调期间触发
/// GPUI 的双重租借崩溃。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReplayEvent {
    /// 用独立播放器打开指定录制文件。
    OpenPlayer {
        path: PathBuf,
        title: Option<String>,
    },
    /// 在当前终端中按时间序回放指定脚本。
    ReplayInTerminal { tab_id: TabId, path: PathBuf },
    /// 打开外部 `.cast` 文件并在独立播放器中打开。
    PickExternalPlayer,
    /// 打开外部 `.cast` 文件并在目标终端中回放。
    PickExternalReplay { tab_id: TabId },
}

impl EventEmitter<ReplayEvent> for ReplayModalView {}

/// 回放脚本选择弹窗实体。
pub(crate) struct ReplayModalView {
    /// 未打开时为 `None`。
    state: Option<ReplayModalState>,
    /// 录制目录在构造时固定，避免渲染与异步扫描重复读全局路径。
    dir: PathBuf,
    /// 扫描世代号：关闭或重新打开时递增，过期结果直接丢弃。
    seq: u64,
}

impl ReplayModalView {
    /// 组装常驻实体；打开状态由 [`Self::open`] 控制。
    pub(crate) fn new() -> Self {
        Self {
            state: None,
            dir: paths::recordings_dir(),
            seq: 0,
        }
    }

    /// 弹窗是否展开。
    pub(crate) fn is_open(&self) -> bool {
        self.state.is_some()
    }

    /// 当前打开期的只读状态；渲染与状态查询共用同一入口。
    pub(crate) fn state(&self) -> Option<&ReplayModalState> {
        self.state.as_ref()
    }

    /// 打开弹窗并异步扫描录制目录。
    pub(crate) fn open(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.state = Some(ReplayModalState {
            target_tab_id: tab_id,
            entries: Vec::new(),
            selected_index: 0,
        });
        self.start_scan(cx);
        cx.notify();
    }

    /// 关闭弹窗；在途扫描结果因世代号变化被丢弃。
    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.seq = self.seq.wrapping_add(1);
        self.state = None;
        cx.notify();
    }

    /// 选中指定下标的脚本；越界或未打开时忽略。
    pub(crate) fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if index >= state.entries.len() {
            return;
        }
        state.selected_index = index;
        cx.notify();
    }

    /// 确认：在独立播放器中打开选中的脚本。
    ///
    /// 空列表或未选中时不动作，弹窗保持打开；事件只携带结果数据，播放器
    /// 标签由外壳创建。
    pub(crate) fn confirm(&mut self, cx: &mut Context<Self>) {
        let Some((path, title)) = self.selected_entry() else {
            return;
        };
        self.close(cx);
        cx.emit(ReplayEvent::OpenPlayer {
            path,
            title: Some(title),
        });
    }

    /// 在当前终端中按时间序回放选中的脚本。
    pub(crate) fn replay_in_terminal(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.state.as_ref().map(|state| state.target_tab_id) else {
            return;
        };
        let Some((path, _)) = self.selected_entry() else {
            return;
        };
        self.close(cx);
        cx.emit(ReplayEvent::ReplayInTerminal { tab_id, path });
    }

    /// 请求外壳打开外部 `.cast` 文件并在独立播放器中打开。
    pub(crate) fn pick_external_player(&mut self, cx: &mut Context<Self>) {
        self.close(cx);
        cx.emit(ReplayEvent::PickExternalPlayer);
    }

    /// 请求外壳打开外部 `.cast` 文件并在目标终端中回放。
    pub(crate) fn pick_external_replay(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.state.as_ref().map(|state| state.target_tab_id) else {
            return;
        };
        self.close(cx);
        cx.emit(ReplayEvent::PickExternalReplay { tab_id });
    }

    /// 当前选中脚本的路径与标题；空列表或未选中时返回 `None`。
    fn selected_entry(&self) -> Option<(PathBuf, String)> {
        let state = self.state.as_ref()?;
        let entry = state.entries.get(state.selected_index)?;
        Some((entry.path.clone(), entry.title.clone()))
    }

    /// 重新扫描录制目录；旧世代的结果在返回时被丢弃。
    fn start_scan(&mut self, cx: &mut Context<Self>) {
        let dir = self.dir.clone();
        self.seq = self.seq.wrapping_add(1);
        let seq = self.seq;
        cx.spawn(async move |this, cx| {
            let Some(entries) = spawn_blocking_local(move || scan_recordings(&dir)).await else {
                log::error!("扫描录制目录的线程创建失败，回放列表为空");
                return;
            };
            let _ = this.update(cx, |view, cx| {
                if view.seq != seq {
                    return;
                }
                if let Some(state) = view.state.as_mut() {
                    state.entries = entries;
                    state.selected_index = 0;
                }
                cx.notify();
            });
        })
        .detach();
    }
}
