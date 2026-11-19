//! AppShell 依赖的各类模态弹窗与输入状态契约定义。

use std::collections::VecDeque;

/// 应用级错误通知的有界待发队列。
///
/// 设计原理（Why）：
/// - 错误写入点分布在事件处理、订阅回调与命令入口，多数不持有窗口句柄，
///   无法在写入时直接推送通知；
/// - 旧实现用单槽 `Option<String>` 暂存、渲染阶段 `take` 后投递，同一帧内
///   多次失败只剩最后一条，且通知投递被绑在绘制节奏上；
/// - 改为有界队列后，写入与展示解耦，每条错误都能到达通知层。上限 8 条，
///   超出时保留最早出现的根因、丢弃后续衍生错误——错误风暴里第一条通常
///   才是需要用户处理的那条。
///
/// 代价与局限：队列只在渲染帧被消费；窗口完全停止重绘期间错误会滞留，
/// 但不会丢失，恢复绘制后按入队顺序补发。
pub(crate) struct PendingNotices {
    items: VecDeque<String>,
}

impl PendingNotices {
    /// 单帧待发错误上限。
    const LIMIT: usize = 8;

    pub(crate) fn new() -> Self {
        Self {
            items: VecDeque::with_capacity(Self::LIMIT),
        }
    }

    /// 入队一条错误；队列已满时静默丢弃本条（保留已有根因）。
    pub(crate) fn push(&mut self, message: impl Into<String>) {
        if self.items.len() < Self::LIMIT {
            self.items.push_back(message.into());
        }
    }

    /// 取走全部待发错误，保留队列容量供下一轮复用。
    pub(crate) fn drain(&mut self) -> Vec<String> {
        self.items.drain(..).collect()
    }
}

impl Default for PendingNotices {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::PendingNotices;

    #[test]
    fn pending_notices_keep_earliest_errors() {
        let mut notices = PendingNotices::new();
        for index in 0..(PendingNotices::LIMIT + 4) {
            notices.push(format!("错误 {index}"));
        }
        let drained = notices.drain();
        assert_eq!(drained.len(), PendingNotices::LIMIT);
        assert_eq!(drained[0], "错误 0", "队列满时应保留最早的根因");
        assert_eq!(drained[PendingNotices::LIMIT - 1], "错误 7");
        assert!(notices.drain().is_empty(), "取走后不得重复投递");
    }
}
