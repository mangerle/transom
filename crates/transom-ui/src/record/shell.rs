//! 回放弹窗的外壳职责：消费实体意图并执行窗口级动作。
//!
//! 实体只上报“要做什么”和所需数据，这里负责打开播放器标签、在目标终端执行
//! 回放与弹出外部文件选择；处理期间禁止回读实体，避免实体 `emit` 期间形成
//! 双重租借。

use super::ReplayEvent;
use crate::app::AppShell;
use crate::app::spawn_blocking_local;
use gpui_kit::*;
use std::path::Path;
use std::time::Duration;
use transom_core::TabId;
use transom_terminal::{extract_replay_inputs, parse_cast_file};

impl AppShell {
    /// 消费回放弹窗实体上报的意图。
    ///
    /// 事件自带全部数据；失败日志已在动作发生点记录，这里不重复输出。
    pub(crate) fn on_replay_event(
        &mut self,
        event: &ReplayEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ReplayEvent::OpenPlayer { path, title } => {
                self.open_cast_player(path.clone(), title.clone(), window, cx);
            }
            ReplayEvent::ReplayInTerminal { tab_id, path } => {
                self.execute_replay_file(*tab_id, path, cx);
            }
            ReplayEvent::PickExternalPlayer => {
                self.pick_and_play_external_cast(window, cx);
            }
            ReplayEvent::PickExternalReplay { tab_id } => {
                self.pick_and_replay_external_cast(*tab_id, window, cx);
            }
        }
        cx.notify();
    }

    /// 打开外部 `.cast` 文件并在独立播放器中打开。
    fn pick_and_play_external_cast(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(rust_i18n::t!("recorder.pick_replay_file").to_string())
            .add_filter("asciinema cast (*.cast)", &["cast"])
            .pick_file()
        else {
            return;
        };

        self.open_cast_player(path, None, window, cx);
    }

    /// 打开外部 `.cast` 文件并立即向当前终端重放。
    fn pick_and_replay_external_cast(
        &mut self,
        tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(rust_i18n::t!("recorder.pick_replay_file").to_string())
            .add_filter("asciinema cast (*.cast)", &["cast"])
            .pick_file()
        else {
            return;
        };

        self.execute_replay_file(tab_id, &path, cx);
    }

    /// 执行指定文件的重放。
    fn execute_replay_file(&mut self, tab_id: TabId, path: &Path, cx: &mut Context<Self>) {
        let path = path.to_path_buf();
        cx.spawn(async move |this, cx| {
            let parse_path = path.clone();
            let Some(parsed) = spawn_blocking_local(move || parse_cast_file(&parse_path)).await
            else {
                log::error!("解析回放脚本的线程创建失败");
                let _ = this.update(cx, |this, cx| {
                    this.state
                        .set_error(rust_i18n::t!("recorder.parse_failed").to_string());
                    cx.notify();
                });
                return;
            };
            let _ = this.update(cx, |this, cx| match parsed {
                Ok((header, events)) => this.execute_replay_events(tab_id, header, events, cx),
                Err(error) => {
                    log::error!("解析回放脚本失败: {error}");
                    this.state
                        .set_error(rust_i18n::t!("recorder.parse_failed").to_string());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 解析完成后的输入流调度；必须在 UI 线程执行。
    fn execute_replay_events(
        &mut self,
        tab_id: TabId,
        header: transom_terminal::CastHeader,
        events: Vec<transom_terminal::CastEvent>,
        cx: &mut Context<Self>,
    ) {
        let steps = extract_replay_inputs(&events, 1.5);
        if steps.is_empty() {
            self.state.status = Some(rust_i18n::t!("recorder.no_inputs_in_cast").to_string());
            cx.notify();
            return;
        }

        let Some(tab) = self.state.workspace.tab(tab_id) else {
            return;
        };
        let Some(terminal) = tab.terminal() else {
            return;
        };

        let step_count = steps.len();
        self.state.status = Some(
            rust_i18n::t!(
                "recorder.replaying_start",
                title = header
                    .title
                    .unwrap_or_else(|| rust_i18n::t!("recorder.script_fallback").to_string()),
                count = step_count
            )
            .to_string(),
        );

        // 异步调度输入流回放
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            for step in steps {
                if step.delay_secs > 0.05 {
                    cx.background_executor()
                        .timer(Duration::from_secs_f64(step.delay_secs))
                        .await;
                }
                terminal.update(cx, |term, cx| {
                    term.send_text(&step.text, cx);
                });
            }
            let _ = this.update(cx, |app, cx| {
                app.state.status = Some(rust_i18n::t!("recorder.replay_completed").to_string());
                cx.notify();
            });
        })
        .detach();

        cx.notify();
    }
}
