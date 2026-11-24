//! 动态分辨率（MS-RDPEDISP DisplayControl）尾沿防抖与偶数对齐状态机。
//!
//! 设计原理（Why）：
//! 1. **真正的尾沿防抖（Trailing-edge Debounce）**：用户连续拖拽窗口缩放时，仅在尺寸
//!    稳定 250ms 后发送**最终目标分辨率**，彻底消除旧版节流实现丢弃拖拽结束最终尺寸的 Bug；
//! 2. **协议规范对齐**：Windows RDP DisplayControl 规范要求宽度为偶数，且分辨率须位于
//!    合法范围内（640x480 ..= 4096x2560）；
//! 3. **通道就绪容错**：若请求发出时 DisplayControl 动态虚拟通道尚未完成握手，保留挂起
//!    目标并按有限次数退避重试，避免首次对齐静默丢失。

use std::time::{Duration, Instant};

/// 动态调整分辨率尾沿防抖间隔（毫秒）。
pub(crate) const RESIZE_DEBOUNCE_INTERVAL: Duration = Duration::from_millis(250);

/// 动态分辨率安全范围限制。
pub const MIN_DESKTOP_WIDTH: u16 = 640;
pub const MAX_DESKTOP_WIDTH: u16 = 4096;
pub const MIN_DESKTOP_HEIGHT: u16 = 480;
pub const MAX_DESKTOP_HEIGHT: u16 = 2560;

/// 通道未就绪时的最大退避重试次数（20 * 250ms = 5 秒，覆盖慢速 WAN DVC 握手）。
const MAX_CHANNEL_WAIT_RETRIES: u8 = 20;

/// 规范化目标分辨率：钳制到合法范围并执行 MS-RDPEDISP 偶数对齐。
pub(crate) fn normalize_resolution(width: u16, height: u16) -> (u16, u16) {
    let w = (width.clamp(MIN_DESKTOP_WIDTH, MAX_DESKTOP_WIDTH)) & !1;
    let h = (height.clamp(MIN_DESKTOP_HEIGHT, MAX_DESKTOP_HEIGHT)) & !1;
    (w.max(MIN_DESKTOP_WIDTH), h.max(MIN_DESKTOP_HEIGHT))
}

/// 动态分辨率请求尾沿防抖控制器。
pub(crate) struct ResizeController {
    current: (u16, u16),
    pending: Option<(u16, u16)>,
    deadline: Option<Instant>,
    retries_left: u8,
}

impl ResizeController {
    /// 以初始协商分辨率构造控制器。
    pub(crate) fn new(initial: (u16, u16)) -> Self {
        Self {
            current: initial,
            pending: None,
            deadline: None,
            retries_left: 0,
        }
    }

    /// 远端完成重激活后更新当前生效分辨率。
    pub(crate) fn set_current(&mut self, resolution: (u16, u16)) {
        self.current = resolution;
        if self.pending == Some(resolution) {
            self.pending = None;
            self.deadline = None;
            self.retries_left = 0;
        }
    }

    /// 记录新的目标尺寸并顺延尾沿防抖截止时间。
    pub(crate) fn schedule(&mut self, width: u16, height: u16, now: Instant) {
        let target = normalize_resolution(width, height);
        if target == self.current {
            self.pending = None;
            self.deadline = None;
            self.retries_left = 0;
            return;
        }
        self.pending = Some(target);
        self.deadline = Some(now + RESIZE_DEBOUNCE_INTERVAL);
        self.retries_left = MAX_CHANNEL_WAIT_RETRIES;
    }

    /// 返回下一次需要触发防抖检查的截止时间（供 `select!` 定时器使用）。
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// 若已到达截止时间，取出待发送的目标分辨率。
    pub(crate) fn take_due(&mut self, now: Instant) -> Option<(u16, u16)> {
        let deadline = self.deadline?;
        if now < deadline {
            return None;
        }
        self.deadline = None;
        self.pending.take()
    }

    /// 当 DisplayControl 通道尚未就绪时，按剩余次数顺延重试，避免静默丢失。
    pub(crate) fn requeue_if_channel_not_ready(&mut self, target: (u16, u16), now: Instant) {
        if self.retries_left > 0 && target != self.current {
            self.retries_left -= 1;
            self.pending = Some(target);
            self.deadline = Some(now + RESIZE_DEBOUNCE_INTERVAL);
        } else if target != self.current {
            log::debug!(
                "DisplayControl 动态分辨率通道等待超时，放弃本次请求: {}x{}",
                target.0,
                target.1
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_clamps_and_aligns_even() {
        assert_eq!(normalize_resolution(1365, 767), (1364, 766));
        assert_eq!(normalize_resolution(200, 100), (640, 480));
        assert_eq!(normalize_resolution(9999, 9999), (4096, 2560));
    }

    #[test]
    fn trailing_edge_debounce_delivers_final_size() {
        let t0 = Instant::now();
        let mut ctrl = ResizeController::new((1920, 1080));

        // 连续拖动窗口，中间尺寸被覆盖，截止时间顺延
        ctrl.schedule(1400, 900, t0);
        assert!(ctrl.take_due(t0 + Duration::from_millis(100)).is_none());

        let t1 = t0 + Duration::from_millis(150);
        ctrl.schedule(1601, 901, t1);
        assert!(ctrl.take_due(t1 + Duration::from_millis(200)).is_none());

        // 稳定满 250ms 后交付最终偶数对齐尺寸
        let due = ctrl
            .take_due(t1 + RESIZE_DEBOUNCE_INTERVAL)
            .expect("到期后应交付最终尺寸");
        assert_eq!(due, (1600, 900));
        assert!(ctrl.next_deadline().is_none());
    }

    #[test]
    fn requeues_when_channel_not_ready() {
        let t0 = Instant::now();
        let mut ctrl = ResizeController::new((1920, 1080));
        ctrl.schedule(1280, 720, t0);

        let due = ctrl
            .take_due(t0 + RESIZE_DEBOUNCE_INTERVAL)
            .expect("首次到期");
        ctrl.requeue_if_channel_not_ready(due, t0 + RESIZE_DEBOUNCE_INTERVAL);
        assert!(ctrl.next_deadline().is_some());
    }

    #[test]
    fn stops_requeue_after_max_retries_and_skips_matching_current() {
        let mut t = Instant::now();
        let mut ctrl = ResizeController::new((1920, 1080));
        ctrl.schedule(1280, 720, t);

        // 耗尽重试次数后应停止挂起
        for _ in 0..=MAX_CHANNEL_WAIT_RETRIES {
            t += RESIZE_DEBOUNCE_INTERVAL;
            let due = ctrl.take_due(t).expect("应取出待发送目标");
            ctrl.requeue_if_channel_not_ready(due, t);
        }
        assert!(ctrl.next_deadline().is_none());

        // 若目标与当前生效分辨率相同则直接跳过
        ctrl.set_current((1280, 720));
        ctrl.schedule(1280, 720, t);
        assert!(ctrl.next_deadline().is_none());
    }
}
