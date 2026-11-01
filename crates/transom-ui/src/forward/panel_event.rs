//! 转发面板发给外壳的意图事件。
//!
//! 面板不持有外壳句柄：需要外壳启停规则、写会话目录或关闭面板时发事件，
//! 外壳在创建面板时订阅一次。事件自带全部数据，外壳处理时不得回读面板。

use transom_core::{ForwardId, ForwardRule, TabId};

/// 转发面板向外壳发出的意图。
#[derive(Clone)]
pub(crate) enum ForwardPanelEvent {
    /// 提交一条已通过校验的转发规则。
    SubmitRule { tab_id: TabId, rule: ForwardRule },
    /// 启动一条规则。
    StartRule { tab_id: TabId, rule_id: ForwardId },
    /// 停止一条规则。
    StopRule { tab_id: TabId, rule_id: ForwardId },
    /// 删除一条规则。
    DeleteRule { tab_id: TabId, rule_id: ForwardId },
    /// 关闭面板；句柄由外壳回收。
    ClosePanel,
}
