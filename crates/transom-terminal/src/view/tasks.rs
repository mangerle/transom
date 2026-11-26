//! 终端内部后台事件处理与 I/O 驱动任务。

use super::{READ_BUFFER_BYTES, READ_CHANNEL_CAPACITY, TerminalView};
use crate::audit::AuditSink;
use crate::event::TerminalEvent;
use crate::osc::{OscEvent, TitleAction};
use crate::record::RecorderHandle;
use alacritty_terminal::event::{Event, WindowSize};
use alacritty_terminal::vte::ansi::Rgb as AnsiRgb;
use gpui_kit::*;
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::OnceLock;
use std::thread;
use std::thread::JoinHandle;
use std::time::Instant;
use transom_core::Rgb as PaletteColor;

/// 终端读、写线程句柄；随 TerminalView 存活并在销毁或重连时交给回收器。
pub(super) struct TerminalIoThreads {
    pub(super) reader: Option<JoinHandle<()>>,
    pub(super) writer: Option<JoinHandle<()>>,
}

/// 启动终端 I/O 后交回视图的全部拥有型资源。
pub(super) struct TerminalIoRuntime {
    pub(super) input_tx: flume::Sender<Vec<u8>>,
    pub(super) output_rx: flume::Receiver<Vec<u8>>,
    pub(super) reader_task: Task<()>,
    pub(super) threads: TerminalIoThreads,
}

impl TerminalIoThreads {
    /// 线程创建失败时保留空槽位，视图仍按已退出路径降级。
    fn empty() -> Self {
        Self {
            reader: None,
            writer: None,
        }
    }
}

/// 终端线程回收队列容量；关闭窗格时不能无限堆积待 join 的句柄。
const TERMINAL_THREAD_QUEUE_CAPACITY: usize = 64;
/// 两个回收线程避免单个阻塞读永久占住全部回收能力。
const TERMINAL_REAPER_THREADS: usize = 2;

struct TerminalThreadReaper {
    sender: flume::Sender<JoinHandle<()>>,
    _workers: Vec<JoinHandle<()>>,
}

impl TerminalThreadReaper {
    fn start() -> Option<Self> {
        let (sender, receiver) = flume::bounded(TERMINAL_THREAD_QUEUE_CAPACITY);
        let mut workers = Vec::with_capacity(TERMINAL_REAPER_THREADS);
        for index in 0..TERMINAL_REAPER_THREADS {
            let receiver = receiver.clone();
            match thread::Builder::new()
                .name(format!("transom-terminal-reaper-{index}"))
                .spawn(move || run_terminal_reaper(receiver))
            {
                Ok(handle) => workers.push(handle),
                Err(error) => {
                    log::error!("启动终端线程回收器失败，序号: {index}，原因: {error}");
                }
            }
        }
        (!workers.is_empty()).then_some(Self {
            sender,
            _workers: workers,
        })
    }
}

fn terminal_reaper() -> Option<&'static TerminalThreadReaper> {
    static REAPER: OnceLock<Option<TerminalThreadReaper>> = OnceLock::new();
    REAPER.get_or_init(TerminalThreadReaper::start).as_ref()
}

fn run_terminal_reaper(receiver: flume::Receiver<JoinHandle<()>>) {
    while let Ok(handle) = receiver.recv() {
        if handle.join().is_err() {
            log::warn!("终端 I/O 线程异常退出，相关窗格可能已经关闭");
        }
    }
}

/// 把终端读写线程移交给进程级有界回收器，避免句柄在 view Drop 中丢失。
pub(super) fn reap_io_threads(threads: TerminalIoThreads) {
    let TerminalIoThreads { reader, writer } = threads;
    let handles = [reader, writer];
    let Some(reaper) = terminal_reaper() else {
        log::error!("终端线程回收器不可用，读写线程句柄只能随进程退出释放");
        return;
    };
    for handle in handles.into_iter().flatten() {
        if let Err(error) = reaper.sender.try_send(handle) {
            log::warn!("终端线程回收队列已满，线程句柄只能随进程退出释放: {error}");
        }
    }
}

/// 终端 I/O 线程派生任务参数结构体。
pub(super) struct TerminalIoTaskParams<'a> {
    pub(super) reader: Box<dyn Read + Send>,
    pub(super) writer: Box<dyn Write + Send>,
    pub(super) audit_path: Option<std::path::PathBuf>,
    pub(super) recorder: &'a RecorderHandle,
    pub(super) event_tx: &'a flume::Sender<Event>,
    pub(super) state: &'a Arc<super::TerminalRenderState>,
}

impl TerminalView {
    /// 启动后台读取线程与写入线程，返回输入发送端、输出接收端副本与任务句柄。
    ///
    /// 输出接收端要额外交回一份：粘贴前视图需要先把已到达的输出并入引擎，
    /// 才能按最新的终端模式决定粘贴编码（见 `TerminalView::encode_paste`）。
    pub(super) fn spawn_io_tasks(
        params: TerminalIoTaskParams<'_>,
        cx: &mut Context<Self>,
    ) -> TerminalIoRuntime {
        let TerminalIoTaskParams {
            mut reader,
            mut writer,
            audit_path,
            recorder,
            event_tx,
            state,
        } = params;
        let recorder_for_reader = recorder.clone();
        let filter_state = state.clone();
        let (bytes_tx, bytes_rx) = flume::bounded::<Vec<u8>>(READ_CHANNEL_CAPACITY);
        let exit_tx = event_tx.clone();

        let reader_spawn = thread::Builder::new()
            .name("transom-terminal-reader".into())
            .spawn(move || {
                let mut audit = audit_path.map(AuditSink::new);
                let mut buffer = [0u8; READ_BUFFER_BYTES];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(count) => {
                            // 注入脚本的回显在进入网格前摘掉：命令照常执行，
                            // 但用户不会先看到整段脚本闪一下。
                            let filtered = filter_state.silent_echo.lock().filter(&buffer[..count]);
                            if filtered.is_empty() {
                                continue;
                            }
                            let slice = filtered.as_slice();
                            if let Some(audit) = audit.as_mut() {
                                audit.record(slice);
                            }
                            recorder_for_reader.record_output(slice);
                            if bytes_tx.send(filtered).is_err() {
                                break;
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(error) => {
                            log::warn!("读取终端输出失败，按已退出处理，原因: {error}");
                            break;
                        }
                    }
                }
                if let Some(audit) = audit.as_mut() {
                    audit.finish();
                }
            });
        let reader_thread = match reader_spawn {
            Ok(handle) => Some(handle),
            Err(_) => {
                log::error!("创建终端读取线程失败，该窗格按已退出降级处理");
                let _ = exit_tx.try_send(Event::Exit);
                None
            }
        };

        let (input_tx, input_rx) = flume::bounded::<Vec<u8>>(256);
        let writer_exit_tx = event_tx.clone();
        let writer_spawn = thread::Builder::new()
            .name("transom-terminal-writer".into())
            .spawn(move || {
                while let Ok(bytes) = input_rx.recv() {
                    if let Err(error) = writer.write_all(&bytes) {
                        log::warn!("写入终端输入失败，按已退出处理，原因: {error}");
                        let _ = writer_exit_tx.try_send(Event::Exit);
                        break;
                    }
                    let _ = writer.flush();
                }
            });
        let writer_thread = match writer_spawn {
            Ok(handle) => Some(handle),
            Err(_) => {
                log::error!("创建终端写入线程失败，该窗格按已退出降级处理");
                let _ = exit_tx.try_send(Event::Exit);
                None
            }
        };
        let threads = if reader_thread.is_none() && writer_thread.is_none() {
            TerminalIoThreads::empty()
        } else {
            TerminalIoThreads {
                reader: reader_thread,
                writer: writer_thread,
            }
        };

        let reader_rx = bytes_rx.clone();
        let reader_task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            while let Ok(bytes) = reader_rx.recv_async().await {
                if this
                    .update(cx, |view, cx| view.apply_output(&bytes, cx))
                    .is_err()
                {
                    return;
                }
            }
            let _ = this.update(cx, |view, cx| view.mark_exited(cx));
        });

        TerminalIoRuntime {
            input_tx,
            output_rx: bytes_rx,
            reader_task,
            threads,
        }
    }

    /// 把一批终端输出并入引擎，并处理触发器命中。
    ///
    /// 读取任务与粘贴前的排空共用这一条路径：两边各写一份的话，触发器与
    /// 引擎的处理顺序会分叉，排空时吞掉的字节就再也不会触发规则。
    pub(super) fn apply_output(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let matches = self.state.trigger_engine.lock().process_bytes(bytes);
        self.state.engine.lock().process(bytes);
        for m in matches {
            match &m.action {
                transom_core::TriggerAction::AutoResponse(reply) => {
                    let mut to_send = reply.clone();
                    if !to_send.ends_with('\r') && !to_send.ends_with('\n') {
                        to_send.push('\r');
                    }
                    self.write_input(to_send.as_bytes());
                }
                transom_core::TriggerAction::Bell => {
                    cx.emit(TerminalEvent::Bell);
                }
                transom_core::TriggerAction::Notification(body) => {
                    cx.emit(TerminalEvent::DesktopNotification {
                        title: m.name.clone(),
                        body: body.clone(),
                    });
                }
                _ => {}
            }
            cx.emit(TerminalEvent::TriggerFired {
                rule_name: m.name,
                action: m.action,
            });
        }
        cx.notify();
    }

    pub(super) fn process_events(&mut self, cx: &mut Context<Self>) {
        self.process_osc_events(cx);
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                // 提示符自己的 OSC 标题不覆盖标签；只有命令运行期间的程序标题才发布。
                Event::Title(title) => {
                    if self.title_gate.on_title(&title) == TitleAction::Publish {
                        cx.emit(TerminalEvent::Title(title));
                    }
                }
                Event::ResetTitle => {
                    if self.title_gate.on_reset() == TitleAction::Clear {
                        cx.emit(TerminalEvent::ResetTitle);
                    }
                }
                Event::Bell => cx.emit(TerminalEvent::Bell),
                // OSC 52 由旁路扫描器接管，内核侧策略已关闭；这条分支只在
                // 未来重新启用内核通道时兜底，仍然遵守同一套会话授权。
                Event::ClipboardStore(_, text) => self.on_remote_clipboard_store(&text, cx),
                // 本地剪贴板进入远端属于更高一级的隐私让渡，当前一律拒绝：
                // 不回写、不提示，行为与主流终端默认档一致。
                Event::ClipboardLoad(_, _) => {
                    log::debug!("已拒绝远端 OSC 52 剪贴板读取请求");
                }
                Event::ColorRequest(index, formatter) => {
                    let rgb = self.ansi_rgb(index);
                    let bytes = formatter(rgb);
                    self.write_input(bytes.as_bytes());
                }
                Event::TextAreaSizeRequest(formatter) => {
                    let metrics = *self.state.metrics.lock();
                    let size = self.state.engine.lock().size();
                    let window_size = WindowSize {
                        num_lines: size.rows,
                        num_cols: size.cols,
                        cell_width: f32::from(metrics.cell_width) as u16,
                        cell_height: f32::from(metrics.cell_height) as u16,
                    };
                    let bytes = formatter(window_size);
                    self.write_input(bytes.as_bytes());
                }
                Event::PtyWrite(text) => {
                    self.write_input(text.as_bytes());
                }
                Event::Exit | Event::ChildExit(_) => self.mark_exited(cx),
                Event::CursorBlinkingChange | Event::MouseCursorDirty | Event::Wakeup => {}
            }
        }
    }

    /// 把旁路扫到的 OSC 序列翻成对外的终端事件。
    pub(super) fn process_osc_events(&mut self, cx: &mut Context<Self>) {
        let events = self.state.engine.lock().take_osc_events();
        if events.is_empty() {
            return;
        }
        let now = Instant::now();
        for event in events {
            // 命令边界决定标题归属：进入命令时允许运行期标题，离开时清除。
            if self.title_gate.on_boundary(&event) == TitleAction::Clear {
                cx.emit(TerminalEvent::ResetTitle);
            }
            match event {
                OscEvent::Cwd { path } => cx.emit(TerminalEvent::Cwd(path)),
                OscEvent::Notify { title, body } => {
                    cx.emit(TerminalEvent::DesktopNotification { title, body });
                }
                OscEvent::Progress { state, progress } => {
                    cx.emit(TerminalEvent::Progress { state, progress });
                }
                OscEvent::ClipboardStore { text } => {
                    self.on_remote_clipboard_store(&text, cx);
                }
                OscEvent::CommandRecorded { command } => {
                    cx.emit(TerminalEvent::CommandRecorded { command });
                }
                // 集成握手不进命令归并：它标记的是"脚本装好了"，不是"命令跑完了"。
                OscEvent::ShellReport { shell } => self.on_integration_signal(
                    crate::shell_integration::IntegrationSignal::ShellReport { shell },
                    cx,
                ),
                OscEvent::IntegrationReady { shell } => self.on_integration_signal(
                    crate::shell_integration::IntegrationSignal::Ready { shell },
                    cx,
                ),
                event => {
                    if let Some(tracked) = self.command_tracker.on_event(&event, now) {
                        cx.emit(TerminalEvent::CommandFinished {
                            exit_code: tracked.exit_code,
                            duration_ms: tracked.duration_ms,
                        });
                    }
                }
            }
        }
    }

    /// 处理远端剪贴板写入：已授权直接落盘，未授权转成宿主确认请求。
    ///
    /// 为什么由视图而不是宿主直接写：授权状态是"每个连接一份"的运行期事实，
    /// 视图生命周期与连接一致；宿主只负责在首次请求时收集用户的决定。
    fn on_remote_clipboard_store(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.remote_clipboard_allowed {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
            cx.emit(TerminalEvent::ClipboardStored);
        } else {
            cx.emit(TerminalEvent::ClipboardWriteRequest {
                text: text.to_string(),
            });
        }
    }

    pub(super) fn ansi_rgb(&self, index: usize) -> AnsiRgb {
        let palette = &self.profile.palette;
        let engine = self.state.engine.lock();
        let colors = engine.term().colors();
        if index < 256
            && let Some(rgb) = colors[index]
        {
            return AnsiRgb {
                r: rgb.r,
                g: rgb.g,
                b: rgb.b,
            };
        }
        let color = if index < 16 {
            palette.ansi[index]
        } else if index == 256 {
            palette.foreground
        } else if index == 257 {
            palette.background
        } else if index == 258 {
            palette.cursor
        } else if index < 232 {
            let index = index - 16;
            let r = (index / 36) as u8;
            let g = ((index % 36) / 6) as u8;
            let b = (index % 6) as u8;
            let channel = |value: u8| if value == 0 { 0 } else { value * 40 + 55 };
            PaletteColor::new(channel(r), channel(g), channel(b))
        } else {
            let shade = index.saturating_sub(232).min(23) as u8;
            let level = 8 + shade * 10;
            PaletteColor::new(level, level, level)
        };
        AnsiRgb {
            r: color.r,
            g: color.g,
            b: color.b,
        }
    }
}
