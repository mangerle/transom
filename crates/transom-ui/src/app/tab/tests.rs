//! 标签模型不变量测试。

use super::{TabCloseAction, TabContent, TabGroupOwner, TabStatus, WorkspaceTab};
use transom_core::{AuthMethod, Connection, Session, SshConnection, TabId};

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
