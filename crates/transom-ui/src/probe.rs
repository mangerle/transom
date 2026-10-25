//! 会话列表的行内拨测：目标收集、状态点渲染与到期清理。
//!
//! 设计原理（Why）：
//! - 实现初衷：原先一次拨测弹一张结果表，用户得在弹窗和列表之间来回对行才知道
//!   "哪台不通"；结果现在直接落在会话行尾的彩色小点上，一眼看完、看完即走。
//!   入口也从右键菜单挪到了行上（分组表头一次问整组，会话行只问自己）。
//! - 核心优势：动作仍在控制平面执行（跨批次共享信号量限流 + 2 秒握手超时），
//!   界面只维护一张"会话 → 状态点"的小表；每个会话至多挂一个到期任务，任务句柄
//!   存在表里，写入新状态时覆盖旧句柄即取消旧定时器，不会出现上一批的到期把
//!   新一批结果顺手抹掉（沿用 `sftp::set_sftp_transient_status` 的做法）。
//! - 代价与局限：状态点是纯瞬时反馈，不落盘也不留历史，到期即消失；"可达"只
//!   说明 TCP 握手成功，不代表能登录。

use crate::app::AppShell;
use crate::palette::{Palette, localized_error};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use std::collections::HashSet;
use std::time::Duration;
use transom_core::{
    AppCommand, FolderId, HostAction, HostOutcome, Session, SessionCatalog, SessionId,
};

/// 可达（绿点）停留时间：确认"通了"就够，不该长期占着行尾。
const REACHABLE_TTL: Duration = Duration::from_secs(3);

/// 不可达（红点）停留更久：失败原因需要时间读，tooltip 里带完整上下文。
const UNREACHABLE_TTL: Duration = Duration::from_secs(8);

/// 状态点直径，与状态栏连接点同规格，避免列表里再引入第二种尺寸的点。
const BADGE_DOT_SIZE: Pixels = px(8.0);

/// 会话行尾的拨测状态点。
#[derive(Clone)]
pub(crate) enum ProbeBadge {
    /// 已派发、结果未回：弱色点占位，免得用户以为没点到而连按。
    Pending,
    /// 可达，tooltip 给出目标地址与握手耗时。
    Reachable { tooltip: SharedString },
    /// 不可达，tooltip 给出目标地址与失败原因。
    Unreachable { tooltip: SharedString },
}

impl ProbeBadge {
    /// 状态点颜色：只用品类色，不跟着主题另调一档。
    fn color(&self, p: &Palette) -> Hsla {
        match self {
            Self::Pending => p.muted_foreground,
            Self::Reachable { .. } => p.success,
            Self::Unreachable { .. } => p.danger,
        }
    }

    /// 悬停文案：在途态现取（避免派发时就固化一条可能过期的字符串）。
    fn tooltip(&self) -> SharedString {
        match self {
            Self::Pending => rust_i18n::t!("probe.pending").into(),
            Self::Reachable { tooltip } | Self::Unreachable { tooltip } => tooltip.clone(),
        }
    }
}

impl AppShell {
    /// 会话行按钮入口：只拨测这一台。
    pub(crate) fn probe_session(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        self.dispatch_probes(&[session_id], cx);
    }

    /// 分组表头按钮入口：对整个文件夹（含子文件夹）发起拨测。
    ///
    /// 口径含子树：一个分组几百台机器时分片挂子文件夹很常见，只测直接子级会让人
    /// 误判"整组都通了"。
    pub(crate) fn probe_folder(&mut self, folder_id: FolderId, cx: &mut Context<Self>) {
        let targets = self
            .catalog
            .sessions
            .iter()
            .filter(|session| session_in_folder(&self.catalog, session, folder_id))
            .map(|session| session.id)
            .collect::<Vec<_>>();
        self.dispatch_probes(&targets, cx);
    }

    /// 派生动作并点亮在途状态点；没有可拨测主机时只给一次提示。
    ///
    /// `targets` 传会话 id 而不是会话本体：调用方要先从目录里挑目标，
    /// 借用的目录不能横跨本方法的可变借用。
    fn dispatch_probes(&mut self, targets: &[SessionId], cx: &mut Context<Self>) {
        let requested: HashSet<SessionId> = targets.iter().copied().collect();
        // 本地 Shell 与串口没有网络地址，派生即落空，静默跳过。
        let probeable: Vec<HostAction> = self
            .catalog
            .sessions
            .iter()
            .filter(|session| requested.contains(&session.id))
            .filter_map(HostAction::probe_for)
            .collect();
        if probeable.is_empty() {
            log::warn!(
                "批量拨测没有可探测的网络主机，请求会话数: {}",
                requested.len()
            );
            self.error = Some(rust_i18n::t!("probe.no_target").to_string());
            cx.notify();
            return;
        }
        // 已在途的会话不再重复派发：控制器还没回报就再发一次，只会多占并发额度。
        let fresh: Vec<HostAction> = probeable
            .into_iter()
            .filter(|action| !self.is_probe_pending(action.session_id()))
            .collect();
        if fresh.is_empty() {
            return;
        }
        let dispatched: Vec<SessionId> = fresh.iter().map(HostAction::session_id).collect();
        log::info!("派发批量拨测，目标数: {}", dispatched.len());
        if !self.send_command(AppCommand::RunHostActions { actions: fresh }) {
            // 命令通道已关闭：不点亮状态点，否则在途态永远等不到回报。
            return;
        }
        for session_id in dispatched {
            self.set_probe_badge(session_id, ProbeBadge::Pending, None, cx);
        }
    }

    /// 控制器回报的单条结果：换成带停留时间的结果点，并挂上到期清理任务。
    pub(crate) fn handle_probe_result(
        &mut self,
        session_id: SessionId,
        outcome: HostOutcome,
        cx: &mut Context<Self>,
    ) {
        // tooltip 里带上目标地址：列表里同名会话不少，光说"可达"定位不到是哪台。
        let target = self.session_target(session_id);
        let label = self.session_label(session_id);
        let (badge, ttl) = match outcome {
            HostOutcome::Reachable { latency_ms } => {
                // 可达按 `debug` 记：一次分组拨测上百台，逐条 info 会把有意义
                // 的生命周期日志冲走；不通的那条才值得 warn。
                log::debug!("拨测可达，会话: {label}，目标: {target}，耗时 {latency_ms} 毫秒");
                (
                    ProbeBadge::Reachable {
                        tooltip: format!(
                            "{target} · {}",
                            rust_i18n::t!("probe.latency", ms = latency_ms)
                        )
                        .into(),
                    },
                    REACHABLE_TTL,
                )
            }
            HostOutcome::Unreachable { error } => {
                let reason = localized_error(&error);
                log::warn!("拨测不可达，会话: {label}，目标: {target}，原因: {reason}");
                (
                    ProbeBadge::Unreachable {
                        tooltip: format!("{target} · {reason}").into(),
                    },
                    UNREACHABLE_TTL,
                )
            }
        };
        self.set_probe_badge(session_id, badge, Some(ttl), cx);
    }

    /// 会话名（日志定位用）；查不到时退回 id，保证每条日志都能追到一行记录。
    fn session_label(&self, session_id: SessionId) -> String {
        self.catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .map(|session| session.name.clone())
            .unwrap_or_else(|| format!("会话 {session_id}"))
    }

    /// 该会话是否有在途拨测。
    fn is_probe_pending(&self, session_id: SessionId) -> bool {
        matches!(
            self.probe_badges.get(&session_id),
            Some(ProbeBadge::Pending)
        )
    }

    /// 会话的 `host:port` 描述；会话已删除时退回空串，tooltip 只剩结果本身。
    fn session_target(&self, session_id: SessionId) -> String {
        self.catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .and_then(HostAction::probe_for)
            .map(|action| match action {
                HostAction::Probe { host, port, .. } => format!("{host}:{port}"),
            })
            .unwrap_or_default()
    }

    /// 写入状态点，并按需挂到期任务；`ttl` 为 `None` 表示在途态，不自动消失。
    fn set_probe_badge(
        &mut self,
        session_id: SessionId,
        badge: ProbeBadge,
        ttl: Option<Duration>,
        cx: &mut Context<Self>,
    ) {
        // 先摘掉旧的到期句柄：dropping `Task` 即取消任务，新一批结果必须活满
        // 自己的 TTL，不能被上一批的定时器抹掉。
        self.probe_expiry_tasks.remove(&session_id);
        self.probe_badges.insert(session_id, badge);
        if let Some(ttl) = ttl {
            let task = cx.spawn(async move |this, cx| {
                cx.background_executor().timer(ttl).await;
                let _ = this.update(cx, |this, cx| {
                    this.probe_badges.remove(&session_id);
                    this.probe_expiry_tasks.remove(&session_id);
                    cx.notify();
                });
            });
            self.probe_expiry_tasks.insert(session_id, task);
        }
        cx.notify();
    }

    /// 渲染会话行尾的状态点；调用方已确认该会话有状态点。
    pub(crate) fn render_probe_badge(
        session_id: SessionId,
        badge: &ProbeBadge,
        p: &Palette,
    ) -> AnyElement {
        let tooltip = badge.tooltip();
        // 悬浮样式必须落在带 id 的元素上，否则 GPUI 不分配悬浮状态、悬停不重绘。
        div()
            .id(ElementId::Name(format!("probe-badge-{session_id}").into()))
            .flex_none()
            .size(BADGE_DOT_SIZE)
            .rounded_full()
            .bg(badge.color(p))
            .tooltip(move |_window, cx| Tooltip::new(tooltip.clone()).build(_window, cx))
            .into_any_element()
    }
}

/// 会话是否落在指定文件夹（含其所有子文件夹）内。
fn session_in_folder(catalog: &SessionCatalog, session: &Session, folder_id: FolderId) -> bool {
    session
        .folder_id
        .is_some_and(|current| current == folder_id || catalog.is_descendant_of(current, folder_id))
}

#[cfg(test)]
mod tests {
    // 不用 `use super::*`：本模块 glob 导入了 gpui，而 gpui 里有同名属性宏
    // `test`（gpui::test），glob 进来会把标准 `#[test]` 劫持成它，导致宏递归。
    use super::session_in_folder;
    use transom_core::{Connection, LocalConnection, Session, SessionCatalog, SshConnection};

    fn ssh_session(name: &str, host: &str) -> Session {
        Session::new(name, Connection::Ssh(SshConnection::new(host, "deploy")))
    }

    /// 文件夹口径含子文件夹，且根层级会话不算在内：表头一次问清整组是本功能
    /// 的主要用法，漏掉子树会让人误判"整组都通了"。
    #[test]
    fn folder_scope_includes_descendants_only() {
        let mut catalog = SessionCatalog::default();
        let prod = catalog.add_folder("prod", None).unwrap();
        let db = catalog.add_folder("db", Some(prod)).unwrap();

        let mut nested = ssh_session("pg", "10.0.0.9");
        nested.folder_id = Some(db);
        let mut direct = ssh_session("web", "10.0.0.8");
        direct.folder_id = Some(prod);
        let root = ssh_session("solo", "10.0.0.7");
        catalog.sessions = vec![nested.clone(), direct.clone(), root.clone()];

        assert!(session_in_folder(&catalog, &direct, prod));
        assert!(session_in_folder(&catalog, &nested, prod));
        assert!(!session_in_folder(&catalog, &root, prod));
        // 子文件夹单独拨测时不含父级其他会话。
        assert!(!session_in_folder(&catalog, &direct, db));
    }

    /// 本地会话不可拨测：分组聚合后不应为它生成动作，也不该为它点亮状态点。
    #[test]
    fn local_session_yields_no_action() {
        let local = Session::new("local", Connection::Local(LocalConnection::default()));
        assert!(transom_core::HostAction::probe_for(&local).is_none());
    }
}
