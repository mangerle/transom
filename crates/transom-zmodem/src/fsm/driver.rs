//! ZMODEM 驱动线程主循环与事件驱动调度。

use std::sync::atomic::Ordering;

use super::state::Fsm;
use super::types::{DECIDE_TIMEOUT, IDLE_TIMEOUT, Mode, TRANSFER_TIMEOUT, UPLOAD_WAIT};

/// 驱动线程主循环：按会话阶段选择等待超时，超时行为见各分支。
pub(crate) fn fsm_loop(mut fsm: Fsm) {
    loop {
        if let Ok(command) = fsm.ctrl_rx.try_recv() {
            fsm.handle_command(command);
        }
        let alive = match fsm.mode {
            // 待命期等待对端首块；迟迟没有合法帧说明标记只是二进制噪声，
            // 超时后静默退还过滤层，避免终端永久无输出无输入。
            Mode::Idle => match fsm.raw_rx.recv_timeout(IDLE_TIMEOUT) {
                Ok(block) => {
                    fsm.consume_block(block);
                    true
                }
                Err(flume::RecvTimeoutError::Timeout) => {
                    fsm.abandon_unstarted("等待对端首帧超时");
                    true
                }
                Err(flume::RecvTimeoutError::Disconnected) => false,
            },
            Mode::Deciding => match fsm.raw_rx.recv_timeout(DECIDE_TIMEOUT) {
                Ok(block) => {
                    fsm.consume_block(block);
                    true
                }
                Err(flume::RecvTimeoutError::Timeout) => {
                    // sz 未跟进：视为等待 ZFILE 的 rz 变体，转上传决策。
                    fsm.enter_await_upload(0);
                    true
                }
                Err(flume::RecvTimeoutError::Disconnected) => false,
            },
            Mode::Download => match fsm.raw_rx.recv_timeout(TRANSFER_TIMEOUT) {
                Ok(block) => {
                    fsm.consume_block(block);
                    true
                }
                Err(flume::RecvTimeoutError::Timeout) => {
                    let actions = fsm.receiver.cancel();
                    fsm.run_recv(actions);
                    true
                }
                Err(flume::RecvTimeoutError::Disconnected) => false,
            },
            Mode::AwaitUpload => match fsm.raw_rx.recv_timeout(UPLOAD_WAIT) {
                Ok(block) => {
                    fsm.consume_block(block);
                    true
                }
                Err(flume::RecvTimeoutError::Timeout) => {
                    // 用户未响应：静默放弃，对端 rz 轮询超时自行退出。
                    fsm.mode = Mode::Idle;
                    fsm.shared.active.store(false, Ordering::Release);
                    while let Ok(block) = fsm.raw_rx.try_recv() {
                        let _ = fsm.replay_tx.try_send(block);
                    }
                    true
                }
                Err(flume::RecvTimeoutError::Disconnected) => false,
            },
            Mode::Upload => {
                // ZACK 确认窗口按发送端退避策略等待；其余阶段沿用固定静默上限。
                let timeout = fsm.sender.ack_timeout().unwrap_or(TRANSFER_TIMEOUT);
                match fsm.raw_rx.recv_timeout(timeout) {
                    Ok(block) => {
                        fsm.consume_block(block);
                        true
                    }
                    Err(flume::RecvTimeoutError::Timeout) => {
                        let actions = fsm.sender.on_timeout();
                        fsm.run_send(actions);
                        true
                    }
                    Err(flume::RecvTimeoutError::Disconnected) => false,
                }
            }
        };
        if !alive {
            fsm.abort_in_flight();
            return;
        }
    }
}
