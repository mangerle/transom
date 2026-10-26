//! 终端交互脚本录制与回放管理模块。
//!
//! 设计说明（Why）：
//! - 实现初衷：为用户提供标准 asciinema v2 格式的交互会话录制、管理与一键自动化回放能力；
//! - 核心优势：
//!   1. 录制文件统一安全保存在 `config/transom/recordings` 下，格式为 `.cast`，可脱机使用任何工具播放；
//!   2. 采用异步任务（GPUI 后台执行器定时器）依时间轴分步把输入回放到对应终端视图；
//!   3. 支持从回放弹窗中浏览已有脚本、查看时长与步数，并支持选择外部文件播放；
//!   4. 录制进行时在底栏醒目标记红点与时长，防误操作与防遗忘。

use crate::app::{AppShell, TabStatus, TerminalTab};
use crate::cast_player::CastPlayerView;
use crate::palette::Palette;
use crate::status_bar::BADGE_HEIGHT;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use std::path::{Path, PathBuf};
use std::time::Duration;
use transom_core::{Connection, Session, TabId, paths};
use transom_terminal::{extract_replay_inputs, parse_cast_file};

/// 录制元数据条目。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RecordingEntry {
    pub file_name: String,
    pub path: PathBuf,
    pub title: String,
    pub width: u16,
    pub height: u16,
    pub duration_secs: f64,
    pub step_count: usize,
    pub modified_display: String,
}

/// 回放管理弹窗状态。
pub(crate) struct ReplayModalState {
    pub target_tab_id: TabId,
    pub entries: Vec<RecordingEntry>,
    pub selected_index: usize,
}

impl AppShell {
    /// 触发当前活动会话的录制开关（录制中则停止，未录制则启动）。
    pub(crate) fn toggle_recording_for_active_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(active_id) = self.active_tab else {
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
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        let Some(terminal) = tab.terminal.as_ref() else {
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
                self.error = Some(rust_i18n::t!("recorder.start_failed").to_string());
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
        self.replay_modal = Some(ReplayModalState {
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
                self.error = Some(rust_i18n::t!("recorder.parse_failed").to_string());
                cx.notify();
                return;
            }
        };

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("session.cast")
            .to_string();
        let display_title = title.unwrap_or_else(|| {
            if header.title.as_ref().is_some_and(|t| !t.trim().is_empty()) {
                header.title.clone().unwrap()
            } else {
                file_name.clone()
            }
        });

        let tab_title =
            rust_i18n::t!("recorder.player_tab_title", name = display_title).to_string();

        let mut profile = self.settings.terminal_profile.clone();
        profile.palette = self.settings.terminal_profile.palette.clone();
        profile.background_transparency = self.settings.window.window_transparency;

        let player = cx.new(|cx| {
            CastPlayerView::new(
                path,
                Some(display_title),
                header,
                events,
                profile,
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

        let mut tab = TerminalTab::pending(tab_id, dummy_session);
        tab.title = tab_title;
        tab.status = TabStatus::Connected;
        tab.player = Some(player);

        self.create_group(tab_id, session_id);
        self.tabs.push(tab);
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
                self.error = Some(rust_i18n::t!("recorder.parse_failed").to_string());
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

        let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
            return;
        };
        let Some(terminal) = tab.terminal.clone() else {
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

    /// 渲染底栏录制指示器。
    pub(crate) fn render_recording_badge(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active_tab_id = self.active_tab;
        let active_is_recording = active_tab_id
            .and_then(|id| self.tabs.iter().find(|t| t.id == id))
            .and_then(|t| t.terminal.as_ref())
            .is_some_and(|term| term.read(cx).is_recording());

        let app_entity = cx.entity().clone();

        let badge = if !active_is_recording {
            Button::new("start-recording-btn")
                .ghost()
                .xsmall()
                .icon(IconName::Disc)
                .tooltip(rust_i18n::t!("recorder.start_btn").to_string())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_recording_for_active_tab(window, cx);
                }))
                .into_any_element()
        } else {
            let stop_tip = rust_i18n::t!("recorder.stop_btn").to_string();
            // 正在录制：红色圆点 + 闪烁/高亮提示
            h_flex()
                .id("recording-indicator")
                .gap_1p5()
                .items_center()
                .px_2()
                .h(rems(BADGE_HEIGHT))
                .rounded(p.radius.md)
                .bg(p.danger.opacity(0.15))
                .border_1()
                .border_color(p.danger)
                .cursor_pointer()
                .tooltip(move |_window, cx| Tooltip::new(stop_tip.clone()).build(_window, cx))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_recording_for_active_tab(window, cx);
                }))
                .child(div().size(px(7.0)).rounded_full().bg(p.danger))
                .child(
                    div()
                        .text_xs()
                        .text_color(p.danger)
                        .font_weight(FontWeight::MEDIUM)
                        .child(rust_i18n::t!("recorder.recording").to_string()),
                )
                .into_any_element()
        };

        h_flex()
            .id("recording-badge-wrapper")
            .items_center()
            .child(badge)
            .context_menu(move |menu, _window, _cx| {
                let app = app_entity.clone();
                let toggle_label = if active_is_recording {
                    rust_i18n::t!("recorder.stop_btn").to_string()
                } else {
                    rust_i18n::t!("recorder.start_btn").to_string()
                };

                let mut menu = menu.item(
                    PopupMenuItem::new(toggle_label)
                        .icon(if active_is_recording {
                            IconName::Square
                        } else {
                            IconName::Disc
                        })
                        .on_click({
                            let app = app.clone();
                            move |_, window, cx| {
                                app.update(cx, |this, cx| {
                                    this.toggle_recording_for_active_tab(window, cx);
                                });
                            }
                        }),
                );

                if let Some(tab_id) = active_tab_id {
                    menu = menu.item(
                        PopupMenuItem::new(rust_i18n::t!("recorder.replay_menu").to_string())
                            .icon(IconName::Play)
                            .on_click({
                                let app = app.clone();
                                move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.open_replay_modal(tab_id, window, cx);
                                    });
                                }
                            }),
                    );
                }

                menu
            })
            .into_any_element()
    }

    /// 渲染回放脚本选择弹窗。
    pub(crate) fn render_replay_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.replay_modal.as_ref() else {
            return div().into_any_element();
        };

        let target_tab_id = modal.target_tab_id;
        let selected_idx = modal.selected_index;
        let selected_entry = modal.entries.get(selected_idx).cloned();

        let mut list = v_flex()
            .id("replay-script-list")
            .gap_1()
            .overflow_y_scroll();
        if modal.entries.is_empty() {
            list = list.child(
                v_flex()
                    .p_4()
                    .gap_2()
                    .items_center()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("recorder.no_recordings").to_string())
                    .child(
                        div()
                            .text_center()
                            .child(rust_i18n::t!("recorder.no_recordings_hint").to_string()),
                    ),
            );
        } else {
            for (idx, entry) in modal.entries.iter().enumerate() {
                let is_sel = idx == selected_idx;
                let bg = if is_sel {
                    p.muted
                } else {
                    gpui::transparent_black()
                };
                let border = if is_sel {
                    p.primary
                } else {
                    gpui::transparent_black()
                };
                let title = if entry.title.is_empty() {
                    entry.file_name.clone()
                } else {
                    format!("{} ({})", entry.title, entry.file_name)
                };

                let entry_path = entry.path.clone();
                let entry_title = entry.title.clone();
                let item_play_path = entry.path.clone();
                let item_play_title = entry.title.clone();

                list = list.child(
                    v_flex()
                        .id(ElementId::Name(format!("replay-item-{idx}").into()))
                        .p_2()
                        .rounded(p.radius.sm)
                        .bg(bg)
                        .border_1()
                        .border_color(border)
                        .cursor_pointer()
                        .hover(|s| s.bg(p.muted.opacity(0.4)))
                        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                            if event.click_count() >= 2 {
                                this.replay_modal = None;
                                this.open_cast_player(
                                    entry_path.clone(),
                                    Some(entry_title.clone()),
                                    window,
                                    cx,
                                );
                            } else if let Some(m) = this.replay_modal.as_mut() {
                                m.selected_index = idx;
                                cx.notify();
                            }
                        }))
                        .child(
                            h_flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(p.foreground)
                                        .truncate()
                                        .child(title),
                                )
                                .child(
                                    Button::new(ElementId::Name(
                                        format!("play-entry-btn-{idx}").into(),
                                    ))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Disc)
                                    .tooltip(rust_i18n::t!("recorder.open_player").to_string())
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            this.replay_modal = None;
                                            this.open_cast_player(
                                                item_play_path.clone(),
                                                Some(item_play_title.clone()),
                                                window,
                                                cx,
                                            );
                                        },
                                    )),
                                ),
                        )
                        .child(
                            h_flex()
                                .justify_between()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(format!(
                                    "{:.1}s · {} 步",
                                    entry.duration_secs, entry.step_count
                                ))
                                .child(entry.modified_display.clone()),
                        ),
                );
            }
        }

        let body = v_flex()
            .gap_3()
            .h(px(320.0))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("recorder.double_click_play_hint").to_string()),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Button::new("pick-external-play-btn")
                                    .outline()
                                    .small()
                                    .icon(IconName::Disc)
                                    .label(rust_i18n::t!("recorder.open_player").to_string())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.replay_modal = None;
                                        this.pick_and_play_external_cast(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("pick-external-cast-btn")
                                    .ghost()
                                    .small()
                                    .icon(IconName::FolderOpen)
                                    .label(rust_i18n::t!("recorder.pick_external").to_string())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.replay_modal = None;
                                        this.pick_and_replay_external_cast(
                                            target_tab_id,
                                            window,
                                            cx,
                                        );
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .p_1()
                    .rounded(p.radius.md)
                    .bg(p.muted.opacity(0.2))
                    .border_1()
                    .border_color(p.border)
                    .child(list),
            );

        let play_entry = selected_entry.clone();
        let footer = h_flex()
            .justify_between()
            .items_center()
            .child(
                Button::new("cancel-replay-modal")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.replay_modal = None;
                        cx.notify();
                    })),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("start-replay-selected-btn")
                            .outline()
                            .disabled(selected_entry.is_none())
                            .icon(IconName::Play)
                            .label(rust_i18n::t!("recorder.start_replay").to_string())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(entry) = selected_entry.as_ref() {
                                    let path = entry.path.clone();
                                    this.replay_modal = None;
                                    this.execute_replay_file(target_tab_id, &path, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("open-in-player-btn")
                            .primary()
                            .disabled(play_entry.is_none())
                            .icon(IconName::Disc)
                            .label(rust_i18n::t!("recorder.open_player").to_string())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(entry) = play_entry.as_ref() {
                                    let path = entry.path.clone();
                                    let title = entry.title.clone();
                                    this.replay_modal = None;
                                    this.open_cast_player(path, Some(title), window, cx);
                                }
                            })),
                    ),
            );

        let dialog = Self::dialog_frame(
            "replay-cast-modal",
            rust_i18n::t!("recorder.replay_title").to_string(),
            Some(rust_i18n::t!("recorder.format_notice").to_string()),
            body,
            Some(footer),
            px(560.0),
            p,
        );
        Self::overlay(dialog, p)
    }
}

/// 扫描指定目录下全部 `.cast` 脚本。
fn scan_recordings(dir: &Path) -> Vec<RecordingEntry> {
    let mut entries = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return entries;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("cast") {
            continue;
        }

        let Ok((header, events)) = parse_cast_file(&path) else {
            continue;
        };

        let duration = events.last().map_or(0.0, |e| e.time);
        let step_count = events
            .iter()
            .filter(|e| e.event_type == transom_terminal::CastEventType::Input)
            .count();

        let modified_display = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|time| {
                let datetime: chrono::DateTime<chrono::Local> = time.into();
                datetime.format("%Y-%m-%d %H:%M").to_string()
            })
            .unwrap_or_default();

        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();

        entries.push(RecordingEntry {
            file_name,
            path,
            title: header.title.unwrap_or_default(),
            width: header.width,
            height: header.height,
            duration_secs: duration,
            step_count,
            modified_display,
        });
    }

    entries.sort_by(|a, b| b.file_name.cmp(&a.file_name));
    entries
}
