//! 主机批量拨测与 Wake-on-LAN 的界面侧：目标收集、结果表与弹窗渲染。
//!
//! 设计原理（Why）：
//! - 实现初衷：Termius / WindTerm 的主机列表能一次看清整组机器"通不通、多久"，
//!   排查网络与关机问题时不必逐台点开。入口挂在会话行与文件夹行的右键菜单，
//!   因为"选中一批→问这批"天然是列表操作，另开独立面板要多一次上下文切换；
//! - 核心优势：动作在控制平面执行（并发与超时都在 tokio 侧），界面只维护
//!   "每条一行、逐条落结果"的表格状态，握手超时不会拖住输入框与终端；
//! - 代价与局限：唤醒只能确认"魔法包已发出"，界面据此显示"已发送"而不是
//!   "已开机"；结果不落盘，关掉弹窗即消失——拨测是瞬时诊断，留存反而误导。

use crate::app::AppShell;
use crate::palette::{Palette, localized_error};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use std::collections::HashSet;
use transom_core::{
    AppCommand, FolderId, HostAction, HostOutcome, Session, SessionCatalog, SessionId,
};

/// 结果行的状态图标尺寸：与表格文字同级，不抢会话名的视觉权重。
const OUTCOME_ICON_SIZE: Pixels = px(14.0);

/// 一次批量动作的类别，决定标题、目标派生方式与"无可执行目标"时的提示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostActionKind {
    Probe,
    Wake,
}

/// 主机动作弹窗状态：一次批次的类别、逐行结果与未回报数。
pub(crate) struct HostActionsModal {
    pub(crate) kind: HostActionKind,
    pub(crate) rows: Vec<HostActionRow>,
    /// 尚未回报的动作数；归零即整批结束。控制器保证每条动作恰好回报一次，
    /// 因此这个计数不会悬着不退。
    pub(crate) pending: usize,
}

/// 弹窗里的一行：目标在派发时定格，结果逐条回填。
#[derive(Clone)]
pub(crate) struct HostActionRow {
    pub(crate) session_id: SessionId,
    pub(crate) label: String,
    pub(crate) target: String,
    pub(crate) outcome: Option<HostOutcome>,
}

impl HostActionKind {
    fn title(self, count: usize) -> String {
        match self {
            Self::Probe => rust_i18n::t!("probe.title", count = count).to_string(),
            Self::Wake => rust_i18n::t!("probe.wake_title", count = count).to_string(),
        }
    }
}

impl AppShell {
    /// 会话行右键入口：落在选区内则对整批生效，否则只针对这一行。
    ///
    /// 与文件管理器的惯例一致：右键点在选区外意味着"我要问的是这一台"，
    /// 若沿用上次多选悄悄扩大影响面，唤醒类动作的代价就不可接受了。
    pub(crate) fn run_host_actions_for_sessions(
        &mut self,
        kind: HostActionKind,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) {
        let targets: HashSet<SessionId> = if self.selected_sessions.contains(&session_id) {
            self.selected_sessions.clone()
        } else {
            [session_id].into_iter().collect()
        };
        self.dispatch_host_actions(kind, &targets.into_iter().collect::<Vec<_>>(), cx);
    }

    /// 文件夹行右键入口：对整个文件夹（含子文件夹）发起动作。
    pub(crate) fn run_host_actions_for_folder(
        &mut self,
        kind: HostActionKind,
        folder_id: FolderId,
        cx: &mut Context<Self>,
    ) {
        let targets = self
            .catalog
            .sessions
            .iter()
            .filter_map(|session| {
                session_in_folder(&self.catalog, session, folder_id).then_some(session.id)
            })
            .collect::<Vec<_>>();
        self.dispatch_host_actions(kind, &targets, cx);
    }

    /// 派生动作并打开结果弹窗；没有可用目标时只提示，不开空表。
    /// `targets` 是会话 id 而不是会话本体：调用方要先把目录里的目标挑出来，
    /// 借用的目录不能再跨进本方法的可变借用。
    fn dispatch_host_actions(
        &mut self,
        kind: HostActionKind,
        targets: &[SessionId],
        cx: &mut Context<Self>,
    ) {
        let mut actions: Vec<HostAction> = Vec::with_capacity(targets.len());
        let mut rows: Vec<HostActionRow> = Vec::with_capacity(targets.len());
        for session in self
            .catalog
            .sessions
            .iter()
            .filter(|session| targets.contains(&session.id))
        {
            // 派生失败即静默跳过：拨测跳过本地/串口，唤醒跳过未配 MAC 的会话，
            // 全部跳过时才给一次提示（见下）。
            let action = match kind {
                HostActionKind::Probe => HostAction::probe_for(session),
                HostActionKind::Wake => HostAction::wake_for(session),
            };
            let Some(action) = action else { continue };
            rows.push(HostActionRow {
                session_id: session.id,
                label: session.name.clone(),
                target: describe_target(&action),
                outcome: None,
            });
            actions.push(action);
        }
        if actions.is_empty() {
            self.error = Some(match kind {
                HostActionKind::Probe => rust_i18n::t!("probe.no_target").to_string(),
                HostActionKind::Wake => rust_i18n::t!("probe.no_mac").to_string(),
            });
            cx.notify();
            return;
        }
        let pending = actions.len();
        if !self.send_command(AppCommand::RunHostActions { actions }) {
            return;
        }
        self.host_actions = Some(HostActionsModal {
            kind,
            rows,
            pending,
        });
        cx.notify();
    }

    /// 控制器逐条回报的结果：定位对应行写入状态并退一位计数。
    pub(crate) fn handle_host_action_result(
        &mut self,
        session_id: SessionId,
        outcome: HostOutcome,
        cx: &mut Context<Self>,
    ) {
        let Some(modal) = self.host_actions.as_mut() else {
            return;
        };
        let Some(row) = modal
            .rows
            .iter_mut()
            .find(|row| row.session_id == session_id)
        else {
            return;
        };
        if row.outcome.replace(outcome).is_none() {
            modal.pending = modal.pending.saturating_sub(1);
        }
        cx.notify();
    }

    pub(crate) fn close_host_actions(&mut self, cx: &mut Context<Self>) {
        self.host_actions = None;
        cx.notify();
    }

    pub(crate) fn render_host_actions_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.host_actions.as_ref() else {
            return div().into_any_element();
        };
        // 渲染期把状态取成局部值：闭包要求 'static，且下面的按钮回调还要
        // 可变借用 self，不能让不可变借用横跨整段构造。
        let kind = modal.kind;
        let total = modal.rows.len();
        let pending = modal.pending;
        let rows = modal.rows.clone();

        // 结果表限高滚动：一个文件夹几百台机器时不能把弹窗撑出屏幕。
        let list = v_flex()
            .id(ElementId::Name("host-actions-list".into()))
            .gap_1()
            .max_h(px(360.0))
            .overflow_y_scroll()
            .children(rows.into_iter().map(|row| outcome_row(&row, p)));
        let content =
            if pending > 0 {
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Progress::new("host-actions-progress").loading(true))
                            .child(div().text_xs().text_color(p.muted_foreground).child(
                                rust_i18n::t!("probe.running", pending = pending).to_string(),
                            )),
                    )
                    .child(list)
                    .into_any_element()
            } else {
                list.into_any_element()
            };

        let footer = Button::new("close-host-actions")
            .ghost()
            .label(rust_i18n::t!("app.close").to_string())
            .on_click(cx.listener(|this, _, _, cx| this.close_host_actions(cx)));

        let dialog = Self::dialog_frame(
            "host-actions-modal",
            kind.title(total),
            Some(rust_i18n::t!("probe.done_hint").to_string()),
            content,
            Some(footer),
            px(560.0),
            p,
        );
        Self::overlay(dialog, p)
    }
}

/// 会话是否落在指定文件夹（含其所有子文件夹）内。
fn session_in_folder(catalog: &SessionCatalog, session: &Session, folder_id: FolderId) -> bool {
    session
        .folder_id
        .is_some_and(|current| current == folder_id || catalog.is_descendant_of(current, folder_id))
}

/// 目标列的静态描述：主机:端口 或 网卡地址。
fn describe_target(action: &HostAction) -> String {
    match action {
        HostAction::Probe { host, port, .. } => format!("{host}:{port}"),
        HostAction::Wake { mac, .. } => mac.clone(),
    }
}

/// 一行结果：会话名 + 目标 + 状态图标与文案。
fn outcome_row(row: &HostActionRow, p: &Palette) -> AnyElement {
    let (icon, text, color) = describe_outcome(row.outcome.as_ref(), &row.target, p);
    h_flex()
        .id(ElementId::Name(
            format!("host-action-row-{}", row.session_id).into(),
        ))
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.border.opacity(0.5))
        .child(Icon::new(icon).with_size(OUTCOME_ICON_SIZE))
        .child(div().flex_1().text_sm().child(row.label.clone()))
        .child(div().text_xs().text_color(color).child(text))
        .into_any_element()
}

/// 单行结果对应的图标、文案与颜色。
fn describe_outcome(
    outcome: Option<&HostOutcome>,
    target: &str,
    p: &Palette,
) -> (IconName, String, Hsla) {
    match outcome {
        // 未回报的行显示目标本身，用户能看出"这台还在等"。
        None => (IconName::Clock, target.to_string(), p.muted_foreground),
        Some(HostOutcome::Reachable { latency_ms }) => (
            IconName::Check,
            format!(
                "{target} · {}",
                rust_i18n::t!("probe.latency", ms = latency_ms)
            ),
            p.success,
        ),
        Some(HostOutcome::Unreachable { error }) => {
            (IconName::CircleAlert, localized_error(error), p.danger)
        }
        Some(HostOutcome::WakeSent) => (
            IconName::ArrowUpRight,
            format!("{target} · {}", rust_i18n::t!("probe.wake_sent")),
            p.success,
        ),
    }
}

#[cfg(test)]
mod tests {
    // 不用 `use super::*`：本模块 glob 导入了 gpui，而 gpui 里有同名属性宏
    // `test`（gpui::test），glob 进来会把标准 `#[test]` 劫持成它，导致宏递归。
    use super::{describe_target, session_in_folder};
    use transom_core::{
        Connection, HostAction, LocalConnection, Session, SessionCatalog, SshConnection,
    };

    fn ssh_session(name: &str, host: &str, mac: Option<&str>) -> Session {
        let mut ssh = SshConnection::new(host, "deploy");
        ssh.options.wol_mac = mac.map(str::to_string);
        Session::new(name, Connection::Ssh(ssh))
    }

    /// 目标列在派发时定格：结果回来后仍要能看出探测的是哪个地址。
    #[test]
    fn describe_target_covers_both_actions() {
        let session = ssh_session("web", "10.0.0.8", Some("aa:bb:cc:dd:ee:ff"));
        let probe = HostAction::probe_for(&session).expect("SSH 会话应可拨测");
        assert_eq!(describe_target(&probe), "10.0.0.8:22");
        let wake = HostAction::wake_for(&session).expect("已配 MAC 应可唤醒");
        assert_eq!(describe_target(&wake), "aa:bb:cc:dd:ee:ff");
    }

    /// 文件夹口径含子文件夹，且根层级会话不算在内：右键文件夹一次问完整组
    /// 机器是本功能的主要用法，漏掉子树会让人误判"整组都通了"。
    #[test]
    fn folder_scope_includes_descendants_only() {
        let mut catalog = SessionCatalog::default();
        let prod = catalog.add_folder("prod", None).unwrap();
        let db = catalog.add_folder("db", Some(prod)).unwrap();

        let mut nested = ssh_session("pg", "10.0.0.9", None);
        nested.folder_id = Some(db);
        let mut direct = ssh_session("web", "10.0.0.8", None);
        direct.folder_id = Some(prod);
        let root = ssh_session("solo", "10.0.0.7", None);
        catalog.sessions = vec![nested.clone(), direct.clone(), root.clone()];

        assert!(session_in_folder(&catalog, &direct, prod));
        assert!(session_in_folder(&catalog, &nested, prod));
        assert!(!session_in_folder(&catalog, &root, prod));
        // 子文件夹被单独右键时不含父级其他会话。
        assert!(!session_in_folder(&catalog, &direct, db));
    }

    /// 本地会话既不能拨测也不能唤醒：两条入口都要落空并给出提示。
    #[test]
    fn local_session_yields_no_action() {
        let local = Session::new("local", Connection::Local(LocalConnection::default()));
        assert!(HostAction::probe_for(&local).is_none());
        assert!(HostAction::wake_for(&local).is_none());
    }
}
