//! 标签模型不变量测试。

use super::model::TabContentKind;
use super::{TabCloseAction, TabContent, TabGroupOwner, TabStatus, WorkspaceTab};
use crate::api_client::{ApiWorkspace, ApiWorkspaceServices};
use crate::app::install_app_settings;
use crate::cast_player::{CastPlayerParams, CastPlayerView};
use crate::container::{ContainerWorkspace, ContainerWorkspaceState};
use crate::git::{GitWorkspace, GitWorkspaceState};
use crate::workspace::WorkspaceState;
use gpui_kit::component::Root;
use gpui_kit::{
    App, AppContext, Context, IntoElement, Render, Size, Styled, TestAppContext, VisualTestContext,
    Window, div, init, px,
};
use std::path::PathBuf;
use transom_api::{ApiHttpService, ApiMockService, ApiProject, ApiWebSocketService};
use transom_core::{
    AppSettings, AuthMethod, Connection, ContainerEngineKind, ContainerEngineProfile,
    ContainerEngineRef, Session, SessionCatalog, SshConnection, TabId, TerminalProfile,
};
use transom_terminal::CastHeader;

struct TabProbeHost;

impl Render for TabProbeHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

fn probe_window(cx: &mut TestAppContext) -> &mut VisualTestContext {
    cx.update(|cx| {
        init(cx);
        install_app_settings(cx, AppSettings::default());
        cx.set_reduce_motion(true);
    });
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|_| TabProbeHost);
        Root::new(view, window, cx)
    });
    cx.simulate_resize(Size {
        width: px(1000.),
        height: px(800.),
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx
}

fn new_git_tab(
    window: &mut Window,
    cx: &mut App,
    project: transom_core::GitProject,
) -> WorkspaceTab {
    let title = project.display_name();
    let (command_tx, _) = flume::bounded(16);
    let workspace = cx.new(|cx| {
        let state = GitWorkspaceState::new(project, window, cx);
        GitWorkspace::new(state, TabId::new(), command_tx)
    });
    WorkspaceTab::new_git(TabId::new(), title, workspace)
}

fn new_container_tab(
    window: &mut Window,
    cx: &mut App,
    profile: ContainerEngineProfile,
) -> WorkspaceTab {
    let title = profile.display_name();
    let (command_tx, _) = flume::bounded(16);
    let workspace = cx.new(|cx| {
        let state = ContainerWorkspaceState::new(profile.clone(), window, cx);
        ContainerWorkspace::new(state, TabId::new(), command_tx)
    });
    WorkspaceTab::new_container(TabId::new(), title, profile, workspace)
}

fn assert_no_session_side_effects(tab: &WorkspaceTab) {
    assert!(tab.terminal().is_none());
    assert!(tab.sftp_view().is_none());
    assert!(tab.forwards_view().is_none());
    assert!(tab.zmodem().is_none());
    assert!(tab.system_stats().is_none());
    assert!(tab.network_traffic().is_none());
}

fn ssh_session() -> Session {
    Session::new(
        "生产主机",
        Connection::Ssh(SshConnection {
            host: "10.0.0.8".to_string(),
            port: 22,
            username: "deploy".to_string(),
            auth: AuthMethod::Agent,
            options: Default::default(),
            proxy_jump: None,
        }),
    )
}

#[test]
fn session_tab_owns_connection_state_and_session_owner() {
    let session = ssh_session();
    let session_id = session.id;
    let tab = WorkspaceTab::new_session(TabId::new(), session);

    assert!(matches!(tab.content, TabContent::Session(_)));
    assert_eq!(tab.session_id(), Some(session_id));
    assert_eq!(tab.connection_id(), None);
    assert_eq!(tab.group_owner(), TabGroupOwner::Session(session_id));
    assert!(tab.can_split());
    assert!(matches!(
        tab.close_action(),
        TabCloseAction::CloseTerminal { .. }
    ));
}

#[test]
fn session_tab_title_preserves_runtime_override_and_fallback() {
    let mut tab = WorkspaceTab::new_session(TabId::new(), ssh_session());
    assert_eq!(tab.display_name(), "生产主机");
    assert_eq!(tab.tab_label(), "生产主机");

    tab.title = "app.rs - nvim".to_string();
    assert_eq!(tab.tab_label(), "app.rs - nvim");
    assert_eq!(tab.display_name(), "生产主机");

    tab.title = "app.rs\n- nvim".to_string();
    assert_eq!(tab.tab_label(), "app.rs - nvim");
}

#[test]
fn session_status_is_presentational_only() {
    let tab = WorkspaceTab::new_session(TabId::new(), ssh_session());
    assert!(matches!(tab.status, TabStatus::Connecting));
}

#[test]
fn tab_group_owner_keeps_session_tabs_together() {
    let session = ssh_session();
    let first = WorkspaceTab::new_session(TabId::new(), session.clone());
    let second = WorkspaceTab::new_session(TabId::new(), session);
    assert_eq!(first.group_owner(), second.group_owner());
    assert_eq!(
        first.group_owner(),
        TabGroupOwner::Session(first.session_id().unwrap())
    );
}

#[test]
fn closing_container_tab_sends_container_disconnect() {
    let tab_id = TabId::new();
    let action = TabCloseAction::for_kind(TabContentKind::Container, tab_id);
    assert!(matches!(
        action,
        TabCloseAction::DisconnectContainer { tab_id: actual } if actual == tab_id
    ));
}

#[test]
fn closing_non_session_tab_does_not_send_terminal_close() {
    let tab_id = TabId::new();
    for kind in [
        TabContentKind::Player,
        TabContentKind::Monitor,
        TabContentKind::Container,
        TabContentKind::Git,
    ] {
        let action = TabCloseAction::for_kind(kind, tab_id);
        assert!(!matches!(action, TabCloseAction::CloseTerminal { .. }));
    }
}

#[test]
fn non_session_tab_cannot_split_or_receive_session_events() {
    assert!(TabContentKind::Session.supports_split());
    assert!(TabContentKind::Session.is_session());
    for kind in [
        TabContentKind::Player,
        TabContentKind::Monitor,
        TabContentKind::Container,
        TabContentKind::Git,
    ] {
        assert!(!kind.supports_split());
        assert!(!kind.is_session());
    }
}

#[test]
fn monitor_target_is_a_session_tab() {
    let target_session = ssh_session();
    let target_session_id = target_session.id;
    let target_tab_id = TabId::new();
    let monitor = WorkspaceTab::new_monitor(
        TabId::new(),
        "监控".to_string(),
        target_tab_id,
        target_session,
        None,
    );

    assert!(monitor.session().is_none(), "监控标签本身不是会话标签");
    assert_no_session_side_effects(&monitor);
    assert_eq!(
        monitor.group_owner(),
        TabGroupOwner::Session(target_session_id)
    );
    assert_eq!(
        monitor.monitor().map(|state| state.target_tab_id),
        Some(target_tab_id)
    );
}

#[test]
fn monitor_target_selection_ignores_git_tabs() {
    let mut app = TestAppContext::single();
    let cx = probe_window(&mut app);
    let (git_tab, session_tab) = cx.update(|window, cx| {
        let git_project = transom_core::GitProject::new(PathBuf::from("/tmp/transom-monitor-git"));
        let git_tab = new_git_tab(window, cx, git_project);
        (
            git_tab,
            WorkspaceTab::new_session(TabId::new(), ssh_session()),
        )
    });
    let mut workspace = WorkspaceState::new();
    workspace.tabs.push(git_tab);
    workspace.tabs.push(session_tab);
    let session_id = workspace.tabs[1].id;

    assert_eq!(
        workspace.monitor_target_tab(Some(workspace.tabs[0].id)),
        Some(session_id)
    );
    assert_eq!(
        workspace.monitor_target_tab(Some(session_id)),
        Some(session_id)
    );
}

#[test]
fn restore_skips_unknown_session_keys() {
    let mut catalog = SessionCatalog::default();
    let session = ssh_session();
    let session_key = session.id.to_string();
    catalog.sessions.push(session);
    assert!(crate::app::tab_coordination::saved_session_for_key(&catalog, "missing").is_none());
    assert!(
        crate::app::tab_coordination::saved_session_for_key(&catalog, "placeholder-git-tab")
            .is_none()
    );
    assert!(crate::app::tab_coordination::saved_session_for_key(&catalog, &session_key).is_some());
}

#[test]
fn git_tab_open_and_close_are_typed() {
    let mut app = TestAppContext::single();
    let cx = probe_window(&mut app);
    let tab = cx.update(|window, cx| {
        new_git_tab(
            window,
            cx,
            transom_core::GitProject::new(PathBuf::from("/tmp/transom-tab-test")),
        )
    });

    assert!(tab.session().is_none());
    assert!(tab.connection_id().is_none());
    assert!(tab.session_pane().is_none());
    assert_eq!(tab.group_owner(), TabGroupOwner::Workspace(tab.id));
    assert!(matches!(tab.close_action(), TabCloseAction::DropOnly));
    assert_no_session_side_effects(&tab);
    assert!(tab.git_workspace().is_some());
    let workspace = tab.git_workspace().expect("Git 标签必须持有工作区实体");
    cx.update(|window, cx| {
        workspace.update(cx, |view, cx| {
            let _ = view.render(window, cx);
        });
    });
}

#[test]
fn git_project_reuses_existing_tab_id() {
    let mut app = TestAppContext::single();
    let cx = probe_window(&mut app);
    let (project, first, second) = cx.update(|window, cx| {
        let project = transom_core::GitProject::new(PathBuf::from("/tmp/transom-tab-reuse"));
        let first = new_git_tab(window, cx, project.clone());
        let second = new_git_tab(window, cx, project.clone());
        (project, first, second)
    });
    let first_id = first.id;
    let second_id = second.id;
    let mut workspace = WorkspaceState::new();
    workspace.tabs.push(first);
    workspace.tabs.push(second);

    let found = cx.update(|_, app| workspace.git_tab_for_project(project.id, app));
    assert_eq!(found, Some(first_id));
    assert_ne!(first_id, second_id);
}

#[test]
fn container_tab_open_and_close_are_typed() {
    let mut app = TestAppContext::single();
    let cx = probe_window(&mut app);
    let tab = cx.update(|window, cx| {
        let profile = ContainerEngineProfile::new(
            "测试引擎",
            ContainerEngineRef::auto(ContainerEngineKind::Docker),
        );
        new_container_tab(window, cx, profile)
    });

    assert!(tab.session().is_none());
    assert!(tab.connection_id().is_none());
    assert!(tab.session_pane().is_none());
    assert_eq!(tab.group_owner(), TabGroupOwner::Workspace(tab.id));
    assert!(matches!(
        tab.close_action(),
        TabCloseAction::DisconnectContainer { .. }
    ));
    assert_no_session_side_effects(&tab);
    assert!(tab.container_workspace().is_some());
    assert!(tab.container_profile().is_some());
}

#[test]
fn container_profile_reuses_existing_tab_id() {
    let mut app = TestAppContext::single();
    let cx = probe_window(&mut app);
    let (profile, first, second) = cx.update(|window, cx| {
        let profile = ContainerEngineProfile::new(
            "复用引擎",
            ContainerEngineRef::auto(ContainerEngineKind::Docker),
        );
        let first = new_container_tab(window, cx, profile.clone());
        let second = new_container_tab(window, cx, profile.clone());
        (profile, first, second)
    });
    let first_id = first.id;
    let second_id = second.id;
    let mut workspace = WorkspaceState::new();
    workspace.tabs.push(first);
    workspace.tabs.push(second);

    assert_eq!(
        workspace.container_tab_for_profile(profile.id),
        Some(first_id)
    );
    assert_ne!(first_id, second_id);
}

#[test]
fn player_tab_open_and_close_are_typed() {
    let mut app = TestAppContext::single();
    let cx = probe_window(&mut app);
    let tab = cx.update(|window, cx| {
        let player = cx.new(|cx| {
            CastPlayerView::new(
                CastPlayerParams {
                    path: PathBuf::from("/tmp/transom-tab-test.cast"),
                    title: Some("回放".to_string()),
                    header: CastHeader {
                        version: 2,
                        width: 80,
                        height: 24,
                        timestamp: None,
                        title: Some("回放".to_string()),
                        env: None,
                    },
                    events: Vec::new(),
                    profile: TerminalProfile::default(),
                },
                window,
                cx,
            )
        });
        WorkspaceTab::new_player(TabId::new(), "回放".to_string(), player)
    });

    assert!(tab.session().is_none());
    assert!(tab.connection_id().is_none());
    assert!(tab.session_pane().is_none());
    assert_eq!(tab.group_owner(), TabGroupOwner::Workspace(tab.id));
    assert!(matches!(tab.close_action(), TabCloseAction::DropOnly));
    assert_no_session_side_effects(&tab);
    assert!(tab.player().is_some());
    let player = tab.player().expect("播放器标签必须持有实体");
    cx.update(|window, cx| {
        player.update(cx, |view, cx| {
            let _ = view.render(window, cx);
        });
    });
}

#[test]
fn api_client_tab_uses_typed_content_without_session_fields() {
    let mut app = TestAppContext::single();
    let cx = probe_window(&mut app);
    let runtime = tokio::runtime::Runtime::new().expect("API 测试运行时应能创建");
    let services = ApiWorkspaceServices {
        http: ApiHttpService::new(runtime.handle().clone()).expect("API HTTP 服务应能初始化"),
        websocket: ApiWebSocketService::new(runtime.handle().clone()),
        mock: ApiMockService::new(runtime.handle().clone()),
    };
    let project = ApiProject::new("测试 API");
    let tab = cx.update(|window, cx| {
        let workspace = cx.new(|cx| ApiWorkspace::new(project.clone(), services, window, cx));
        WorkspaceTab::new_api_client(
            TabId::new(),
            project.display_name().to_string(),
            project.id,
            workspace,
        )
    });

    assert!(tab.session().is_none());
    assert!(tab.connection_id().is_none());
    assert!(tab.session_pane().is_none());
    assert_eq!(tab.group_owner(), TabGroupOwner::Workspace(tab.id));
    assert!(matches!(tab.close_action(), TabCloseAction::DropOnly));
    assert!(tab.api_project_id().is_some());
    assert!(tab.api_workspace().is_some());
    assert_no_session_side_effects(&tab);
}
