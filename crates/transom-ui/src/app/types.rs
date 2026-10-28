//! AppShell 依赖的各类模态弹窗与输入状态契约定义。

use crate::forward::ForwardPanelState;
use crate::pane_tree::LayoutNode;
use crate::sftp::SftpPanelState;
use gpui_kit::*;
use std::collections::{HashMap, VecDeque};
use transom_core::{
    Connection, ConnectionId, ForwardId, ForwardStatus, GroupId, NetworkTraffic, Session,
    SessionId, SystemStats, TabId,
};
use transom_terminal::TerminalView;

pub(crate) enum TabStatus {
    Connecting,
    Connected,
    Failed(transom_core::AppError),
    Disconnected,
}

/// 标签组：一个标签栏条目下挂载的一棵分割树。
pub(crate) struct TabGroup {
    pub(crate) id: GroupId,
    pub(crate) session_id: SessionId,
    pub(crate) serial: usize,
    pub(crate) root: LayoutNode,
    pub(crate) active_pane: TabId,
    pub(crate) zoomed: Option<TabId>,
}

impl TabGroup {
    pub(crate) fn new(pane: TabId, session_id: SessionId, serial: usize) -> Self {
        Self {
            id: GroupId::new(),
            session_id,
            serial,
            root: LayoutNode::pane(pane),
            active_pane: pane,
            zoomed: None,
        }
    }

    pub(crate) fn pane_ids(&self) -> Vec<TabId> {
        self.root.pane_ids()
    }

    pub(crate) fn contains(&self, tab: TabId) -> bool {
        self.root.contains(tab)
    }

    pub(crate) fn pane_count(&self) -> usize {
        self.root.pane_count()
    }
}

/// 终端标签窗格的具体内容形态。
///
/// 遵循“非法状态不可表示 (Make Illegal States Unrepresentable)”设计原则：
/// 标签页要么是承载实时通信连接的会话窗格（终端视图、SFTP、端口转发等），
/// 要么是独立的录像回放播放器，二者在类型系统中严格互斥。
pub(crate) enum TabContent {
    /// 实时通信会话窗格（包含终端引擎与伴随子面板）。
    Session(Box<SessionPaneState>),
    /// 录像回放播放器。
    Player(Entity<crate::cast_player::CastPlayerView>),
}

/// 实时通信会话窗格关联的所有运行期状态。
#[derive(Default)]
pub(crate) struct SessionPaneState {
    pub(crate) terminal: Option<Entity<TerminalView>>,
    pub(crate) sftp: Option<SftpPanelState>,
    pub(crate) forwards_panel: Option<ForwardPanelState>,
    pub(crate) forward_statuses: HashMap<ForwardId, ForwardStatus>,
    pub(crate) system_stats: Option<SystemStats>,
    pub(crate) network_traffic: Option<NetworkTraffic>,
    pub(crate) _startup_task: Option<Task<()>>,
    pub(crate) _subscription: Option<Subscription>,
    pub(crate) reconnect_attempts: u32,
    pub(crate) reconnect_task: Option<Task<()>>,
    pub(crate) zmodem: Option<crate::zmodem::ZmodemState>,
}

pub(crate) struct TerminalTab {
    pub(crate) id: TabId,
    pub(crate) session_id: SessionId,
    pub(crate) session: Session,
    pub(crate) connection_id: Option<ConnectionId>,
    pub(crate) title: String,
    pub(crate) status: TabStatus,
    pub(crate) content: TabContent,
}

impl TerminalTab {
    /// 是否为录像播放器标签。
    pub(crate) fn is_player(&self) -> bool {
        matches!(self.content, TabContent::Player(_))
    }

    /// 获取关联的录像播放器视图句柄（若存在）。
    pub(crate) fn player(&self) -> Option<Entity<crate::cast_player::CastPlayerView>> {
        match &self.content {
            TabContent::Player(player) => Some(player.clone()),
            TabContent::Session(_) => None,
        }
    }

    /// 获取会话状态不可变引用（若当前为会话标签）。
    pub(crate) fn session_pane(&self) -> Option<&SessionPaneState> {
        match &self.content {
            TabContent::Session(pane) => Some(pane),
            TabContent::Player(_) => None,
        }
    }

    /// 获取会话状态可变引用（若当前为会话标签）。
    pub(crate) fn session_pane_mut(&mut self) -> Option<&mut SessionPaneState> {
        match &mut self.content {
            TabContent::Session(pane) => Some(pane),
            TabContent::Player(_) => None,
        }
    }

    /// 快捷访问终端视图引用。
    pub(crate) fn terminal(&self) -> Option<Entity<TerminalView>> {
        self.session_pane().and_then(|p| p.terminal.clone())
    }

    /// 快捷访问 SFTP 面板引用。
    pub(crate) fn sftp(&self) -> Option<&SftpPanelState> {
        self.session_pane().and_then(|p| p.sftp.as_ref())
    }

    /// 快捷访问 SFTP 面板可变引用。
    pub(crate) fn sftp_mut(&mut self) -> Option<&mut SftpPanelState> {
        self.session_pane_mut().and_then(|p| p.sftp.as_mut())
    }

    /// 快捷访问端口转发面板引用。
    pub(crate) fn forwards_panel(&self) -> Option<&ForwardPanelState> {
        self.session_pane().and_then(|p| p.forwards_panel.as_ref())
    }

    /// 快捷访问端口转发面板可变引用。
    pub(crate) fn forwards_panel_mut(&mut self) -> Option<&mut ForwardPanelState> {
        self.session_pane_mut()
            .and_then(|p| p.forwards_panel.as_mut())
    }

    /// 快捷访问 Zmodem 传输状态。
    pub(crate) fn zmodem(&self) -> Option<&crate::zmodem::ZmodemState> {
        self.session_pane().and_then(|p| p.zmodem.as_ref())
    }

    /// 快捷访问系统资源监控数据。
    pub(crate) fn system_stats(&self) -> Option<&SystemStats> {
        self.session_pane().and_then(|p| p.system_stats.as_ref())
    }

    /// 快捷访问网络流量监控数据。
    pub(crate) fn network_traffic(&self) -> Option<&NetworkTraffic> {
        self.session_pane().and_then(|p| p.network_traffic.as_ref())
    }

    pub(crate) fn display_name(&self) -> String {
        if self.is_player() {
            return self.title.clone();
        }
        let name = self.session.name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        match &self.session.connection {
            Connection::Ssh(ssh) => format!("{}@{}", ssh.username, ssh.host),
            Connection::Telnet(telnet) => format!("{}:{}", telnet.host, telnet.port),
            Connection::Serial(serial) => serial.port.clone(),
            Connection::Local(local) => local.shell.clone().unwrap_or_else(|| "shell".to_string()),
            _ => self.title.clone(),
        }
    }

    /// 构造处于连接等待中的实时通信终端标签。
    pub(crate) fn pending(id: TabId, session: Session) -> Self {
        let title = session.name.clone();
        Self {
            id,
            session_id: session.id,
            session,
            connection_id: None,
            title,
            status: TabStatus::Connecting,
            content: TabContent::Session(Box::default()),
        }
    }

    /// 构造已就绪的录像回放播放器标签。
    pub(crate) fn player_tab(
        id: TabId,
        session: Session,
        title: String,
        player: Entity<crate::cast_player::CastPlayerView>,
    ) -> Self {
        Self {
            id,
            session_id: session.id,
            session,
            connection_id: None,
            title,
            status: TabStatus::Connected,
            content: TabContent::Player(player),
        }
    }
}

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
