//! 终端录制启停、回放触发与独立播放器打开逻辑。

use super::types::{ReplayModalState, scan_recordings};
use crate::app::{AppShell, TerminalTab};
use crate::cast_player::{CastPlayerParams, CastPlayerView};
use gpui_kit::*;
use std::path::{Path, PathBuf};
use std::time::Duration;
use transom_core::{Connection, Session, TabId};
use transom_platform::paths;
use transom_terminal::{extract_replay_inputs, parse_cast_file};

impl AppShell {
    /// 触发当前活动会话的录制开关（录制中则停止，未录制则启动）。
    pub(crate) fn toggle_recording_for_active_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(active_id) = self.workspace.active_tab else {
            return;
        };
        self.toggle_recording(active_id, window, cx);
    }

    /// 切换指定标签的录制状态。
    pub(crate) fn toggle_recording(
        &mut self,
        tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workspace.tab_mut(tab_id) else {
            return;
        };
        let Some(terminal) = tab.terminal() else {
            return;
        };

        let is_recording = terminal.read(cx).is_recording();
        if is_recording {
            // 停止录制
            let saved_path = terminal.update(cx, |term, _| term.stop_recording());
            if let Some(path) = saved_path {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("session.cast");
                self.status = Some(rust_i18n::t!("recorder.saved_notice", name = name).to_string());
            }
        } else {
            // 开始录制
            let dir = paths::recordings_dir();
            if let Err(e) = std::fs::create_dir_all(&dir) {
                log::error!("创建脚本录制目录失败: {e}");
                return;
            }
            let label = paths::sanitize_audit_label(&tab.display_name());
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
            let file_name = format!("{label}-{stamp}.cast");
            let path = dir.join(file_name);
            let title = Some(tab.display_name());

            let res = terminal.update(cx, |term, _| term.start_recording(path, title));
            if let Err(e) = res {
                log::error!("启动终端录制失败: {e}");
                self.set_error(rust_i18n::t!("recorder.start_failed").to_string());
            } else {
                self.status = Some(rust_i18n::t!("recorder.started_notice").to_string());
            }
        }
        cx.notify();
    }

    /// 打开回放弹窗选择脚本。
    pub(crate) fn open_replay_modal(
        &mut self,
        tab_id: TabId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dir = paths::recordings_dir();
        let entries = scan_recordings(&dir);
        self.modals.replay = Some(ReplayModalState {
            target_tab_id: tab_id,
            entries,
            selected_index: 0,
        });
        cx.notify();
    }

    /// 在独立的只读会话回放器标签页中打开录制文件。
    pub(crate) fn open_cast_player(
        &mut self,
        path: PathBuf,
        title: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (header, events) = match parse_cast_file(&path) {
            Ok(res) => res,
            Err(e) => {
                log::error!("解析会话录制文件失败，路径: {:?}，原因: {e}", path);
                self.set_error(rust_i18n::t!("recorder.parse_failed").to_string());
                cx.notify();
                return;
            }
        };

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("session.cast")
            .to_string();
        let display_title = title
            .or_else(|| {
                header
                    .title
                    .as_deref()
                    .filter(|t| !t.trim().is_empty())
                    .map(ToString::to_string)
            })
            .unwrap_or(file_name);

        let tab_title =
            rust_i18n::t!("recorder.player_tab_title", name = display_title).to_string();

        let mut profile = self.settings.terminal_profile.clone();
        profile.palette = self.settings.terminal_profile.palette.clone();
        profile.background_transparency = self.settings.window.window_transparency;

        let player = cx.new(|cx| {
            CastPlayerView::new(
                CastPlayerParams {
                    path,
                    title: Some(display_title),
                    header,
                    events,
                    profile,
                },
                window,
                cx,
            )
        });

        let tab_id = TabId::new();
        let mut dummy_session = Session::new(
            tab_title.clone(),
            Connection::Local(transom_core::LocalConnection {
                shell: None,
                working_dir: None,
            }),
        );
        let session_id = dummy_session.id;
        dummy_session.terminal = self.settings.terminal_profile.clone();

        let tab = TerminalTab::player_tab(tab_id, dummy_session, tab_title, player);

        self.create_group(tab_id, session_id);
        self.workspace.tabs.push(tab);
        self.activate_tab(tab_id, window, cx);
        cx.notify();
    }

    /// 打开外部 `.cast` 文件并在独立播放器中打开。
    pub(crate) fn pick_and_play_external_cast(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
    pub(crate) fn pick_and_replay_external_cast(
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
    pub(crate) fn execute_replay_file(
        &mut self,
        tab_id: TabId,
        path: &Path,
        cx: &mut Context<Self>,
    ) {
        let (header, events) = match parse_cast_file(path) {
            Ok(res) => res,
            Err(e) => {
                log::error!("解析回放脚本失败: {e}");
                self.set_error(rust_i18n::t!("recorder.parse_failed").to_string());
                cx.notify();
                return;
            }
        };

        let steps = extract_replay_inputs(&events, 1.5);
        if steps.is_empty() {
            self.status = Some(rust_i18n::t!("recorder.no_inputs_in_cast").to_string());
            cx.notify();
            return;
        }

        let Some(tab) = self.workspace.tab(tab_id) else {
            return;
        };
        let Some(terminal) = tab.terminal() else {
            return;
        };

        let step_count = steps.len();
        self.status = Some(
            rust_i18n::t!(
                "recorder.replaying_start",
                title = header.title.unwrap_or_else(|| "脚本".into()),
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
                app.status = Some(rust_i18n::t!("recorder.replay_completed").to_string());
                cx.notify();
            });
        })
        .detach();

        cx.notify();
    }
}
