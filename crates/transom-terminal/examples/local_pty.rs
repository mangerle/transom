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
            .expect("failed to open pty");

        let mut command =
            CommandBuilder::new(std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string()));
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        let child = pair
            .slave
            .spawn_command(command)
            .expect("failed to spawn shell");
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .expect("failed to clone pty reader");
        let writer = pair
            .master
            .take_writer()
            .expect("failed to take pty writer");
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
            .expect("failed to spawn pty control thread");

        let io = TerminalIo::new(reader, writer, control);
        let terminal = cx.new(|cx| {
            TerminalView::new(
                window,
                cx,
                io,
                TerminalProfile::default(),
                TerminalInteraction::default(),
                None,
                None,
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
        .expect("failed to open window");
    });
}
