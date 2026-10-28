//! 本地 PTY 终端嵌入演示样例。
//!
//! 演示如何通过 `portable-pty` 拉起本地 Shell 子进程，
//! 并接入 `TerminalView` 虚拟终端视图进行键盘输入、屏幕渲染与大小重设。

use gpui_kit::*;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::thread;
use transom_core::{
    TerminalControl, TerminalControlHandle, TerminalInteraction, TerminalIo, TerminalProfile,
};
use transom_terminal::TerminalView;

struct Demo {
    terminal: Entity<TerminalView>,
    _child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl Demo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("创建本地伪终端配对失败");

        let mut command =
            CommandBuilder::new(std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string()));
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        let child = pair
            .slave
            .spawn_command(command)
            .expect("拉起本地 Shell 子进程失败");
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .expect("复制伪终端读取句柄失败");
        let writer = pair.master.take_writer().expect("提取伪终端写入句柄失败");
        let (control, mut control_rx) = TerminalControlHandle::channel(16);
        let master = pair.master;
        thread::Builder::new()
            .name("transom-pty-control".into())
            .spawn(move || {
                while let Some(message) = control_rx.blocking_recv() {
                    match message {
                        TerminalControl::Resize(size) => {
                            let _ = master.resize(PtySize {
                                rows: size.rows,
                                cols: size.cols,
                                pixel_width: 0,
                                pixel_height: 0,
                            });
                        }
                        TerminalControl::Close => break,
                    }
                }
            })
            .expect("启动伪终端控制线程失败");

        let io = TerminalIo::new(reader, writer, control);
        let terminal = cx.new(|cx| {
            TerminalView::new(
                window,
                cx,
                transom_terminal::TerminalViewParams {
                    io,
                    profile: TerminalProfile::default(),
                    interaction: TerminalInteraction::default(),
                    key_filter: None,
                    audit_path: None,
                },
            )
        });
        let focus_handle = terminal.read(cx).focus_handle().clone();
        window.focus(&focus_handle, cx);

        Self {
            terminal,
            _child: child,
        }
    }
}

impl Render for Demo {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.terminal.clone())
    }
}

fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Demo::new(window, cx))
        })
        .expect("创建并打开演示窗口失败");
    });
}
