//! 程序化输入与剪贴板交互：发送文本、粘贴与选区复制。
use super::*;
use crate::input::{KeyEventKind, keystroke_event_to_bytes, paste_bytes};

/// 判断剪贴板条目是否包含图片数据。
///
/// 图片粘贴不走文本粘贴通道：Claude Code 这类 TUI 会绑定 Ctrl+V 并直接读取
/// 系统剪贴板，终端只需把控制字节交给程序。混合条目只要含图片就按图片处理，
/// 避免应用提供的可选文本降级掩盖用户真正要粘贴的图片。
pub(super) fn clipboard_contains_image(item: &ClipboardItem) -> bool {
    item.entries()
        .iter()
        .any(|entry| matches!(entry, ClipboardEntry::Image(_)))
}

/// 图片粘贴应把 Ctrl+V 控制序列交给终端程序，而不是转换成文本粘贴。
///
/// Claude Code 等 TUI 通过 Ctrl+V 事件直接读取系统剪贴板并创建图片附件；终端
/// 不能把图片字节伪装成括号粘贴文本，也不能把图片写入临时路径后代发。使用当前
/// 终端模式编码控制序列，确保 Kitty 键盘协议协商时保持按键语义一致。
pub(super) fn image_paste_bytes(mode: TermMode) -> Option<Vec<u8>> {
    let keystroke = Keystroke {
        modifiers: gpui_kit::Modifiers {
            control: true,
            ..Default::default()
        },
        key: "v".to_string(),
        key_char: None,
    };
    keystroke_event_to_bytes(&keystroke, mode, KeyEventKind::Press)
}

impl TerminalView {
    /// 将程序化文本写入终端输入通道。
    ///
    /// 与键盘输入复用同一条有界队列，保证初始化命令按调用顺序到达 Shell；
    /// 返回 `false` 表示终端写入线程已退出（连接已断开）。
    pub fn send_text(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let sent = self.write_input(text.as_bytes());
        if sent {
            self.state.engine.lock().scroll_to_bottom();
            cx.notify();
        }
        sent
    }

    /// 按粘贴语义把一段文本写入终端输入通道，可选在末尾补回车执行。
    ///
    /// 多行脚本使用终端当前的括号粘贴模式：远端 shell 会把整段文本当作一次
    /// 粘贴，而不是每读到一行就提前执行；对端未开启括号粘贴时退回原始字节。
    /// 与键盘输入复用同一条有界队列，返回 `false` 表示终端写入线程已退出。
    pub fn send_paste(&mut self, text: &str, execute: bool, cx: &mut Context<Self>) -> bool {
        let mut bytes = self.encode_paste(text, cx);
        if execute {
            bytes.push(b'\r');
        }
        let sent = self.write_input(&bytes);
        if sent {
            self.state.engine.lock().scroll_to_bottom();
            cx.notify();
        }
        sent
    }

    /// 先排空待解析输出，再按最新的终端模式编码粘贴内容。
    ///
    /// 设计原理（Why）：引擎的模式位由输出解析得到，而输出要经过读取线程 →
    /// 有界队列 → UI 线程才落地。UI 线程一旦忙于渲染，Shell 刚发出的
    /// `ESC[?2004l` 还躺在队列里，此时按旧状态加括号粘贴标记，就把标记送进了一个
    /// 已经退出括号粘贴的对端：readline 会把这种未绑定的序列静默丢弃，屏幕上并不
    /// 留残字；要紧的是 vim、tmux 这类直接读原始字节的程序，它会把 `ESC[200~` 当成
    /// 按键解释。反向过期更糟：以为没开其实开着，多行内容会被逐行立即执行。粘贴是
    /// 低频且由用户显式触发的事件，值得在这里多花一次解析把判定窗口压到最小。
    ///
    /// 代价与局限：只能吃掉已经进了队列的字节；读取线程尚未读到的输出、以及回车
    /// 与粘贴本身在毫秒级内的真实碰撞仍无法根除——这是任何按 DECSET 2004 加帧的
    /// 终端都存在的固有竞态。另注：屏幕上的 `^[[200~` 残字与模式位过期无关，它来自
    /// 误按 Ctrl+V 后 readline 布置的一次性引用插入，被引用的是标记自己的 ESC。
    pub(super) fn encode_paste(&mut self, text: &str, cx: &mut Context<Self>) -> Vec<u8> {
        let drained = self.drain_pending_output(cx);
        let mode = self.state.engine.lock().mode();
        if drained > 0 {
            log::debug!(
                "粘贴前排空待解析输出 {drained} 批，括号粘贴模式: {}",
                mode.contains(TermMode::BRACKETED_PASTE)
            );
        }
        paste_bytes(mode, text)
    }

    /// 把已到达但尚未解析的输出全部并入引擎，返回排空的批次数量。
    pub(super) fn drain_pending_output(&mut self, cx: &mut Context<Self>) -> usize {
        let mut batches = 0;
        while let Ok(bytes) = self.output_rx.try_recv() {
            self.apply_output(&bytes, cx);
            batches += 1;
        }
        batches
    }

    pub(super) fn mark_exited(&mut self, cx: &mut Context<Self>) {
        if !self.exited {
            self.exited = true;
            cx.emit(TerminalEvent::Exit);
        }
    }

    pub(super) fn copy_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let text = self.state.engine.lock().selection_text();
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            cx.emit(TerminalEvent::ClipboardStored);
            true
        } else {
            false
        }
    }

    pub(super) fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        if clipboard_contains_image(&item) {
            let mode = self.state.engine.lock().mode();
            let Some(bytes) = image_paste_bytes(mode) else {
                return false;
            };
            return if self.write_input(&bytes) {
                self.state.engine.lock().scroll_to_bottom();
                cx.notify();
                true
            } else {
                false
            };
        }
        let Some(text) = item.text() else {
            return false;
        };
        let bytes = self.encode_paste(&text, cx);
        if self.write_input(&bytes) {
            self.state.engine.lock().scroll_to_bottom();
            cx.notify();
            true
        } else {
            false
        }
    }
}
