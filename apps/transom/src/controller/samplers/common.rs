//! 采样调度与事件投递通用工具定义。

use transom_core::AppEvent;

/// 投递可覆盖状态事件：通道满时丢弃本条样本（下一轮会带最新值重来），
/// 通道断开返回 `false` 让采样任务退出。
///
/// 设计原理（Why）：流量、系统统计都是可覆盖的单调状态量，样本丢失没有
/// 一致性损失（终值由后续样本兜底）。若沿用 send_async 反压，UI 消费停滞
/// 时这些样本会占满 1024 深的事件通道，把控制平面的状态迁移事件（连接
/// 建立/断开、失败、转发表更）一并挤住——那才是不能丢的事件。
/// 通道满按 debug 记录：UI 停滞属可自愈的异常，持续刷 warn 只会污染日志。
pub(super) fn send_overwritable(tx: &flume::Sender<AppEvent>, event: AppEvent, what: &str) -> bool {
    match tx.try_send(event) {
        Ok(()) => true,
        Err(flume::TrySendError::Full(_)) => {
            log::debug!("{what}样本被丢弃：事件队列已满，界面消费停滞");
            true
        }
        Err(flume::TrySendError::Disconnected(_)) => false,
    }
}
