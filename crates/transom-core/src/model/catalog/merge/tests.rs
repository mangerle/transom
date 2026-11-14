use super::*;

/// 既有用例默认按本机偏好裁决，语义与改造前一致。
fn merge_local(
    base: Option<&SessionCatalog>,
    local: &SessionCatalog,
    remote: &SessionCatalog,
) -> (SessionCatalog, SyncMergeReport) {
    merge_catalogs(base, local, remote, MergePreference::Local)
}

use crate::ids::{FolderId, IdentityId, SessionId};
use crate::model::catalog::SessionFolder;
use crate::model::identity::Identity;
use crate::model::session::{AuthMethod, Connection, Session, SshConnection};

// 让编译器确认键类型与目录条目字段一致（改字段时会在这里报错）。
fn _key_types(
    _: SessionId,
    _: FolderId,
    _: IdentityId,
    _: &Session,
    _: &SessionFolder,
    _: &Identity,
) {
}

fn session(name: &str, host: &str, updated_at: u64) -> Session {
    let mut session = Session::new(
        name,
        Connection::Ssh(SshConnection {
            host: host.into(),
            port: 22,
            username: "deploy".into(),
            auth: AuthMethod::Agent,
            options: Default::default(),
            proxy_jump: None,
        }),
    );
    session.updated_at = updated_at;
    session
}

fn catalog(sessions: Vec<Session>) -> SessionCatalog {
    SessionCatalog {
        sessions,
        ..Default::default()
    }
}

#[test]
fn remote_only_and_local_only_sessions_are_kept() {
    let local = catalog(vec![session("web", "web.local", 10)]);
    let remote = catalog(vec![session("db", "db.remote", 20)]);
    let (merged, report) = merge_local(None, &local, &remote);
    let names: Vec<&str> = merged.sessions.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["web", "db"]);
    assert_eq!(report.sessions_added, 1);
}

#[test]
fn unchanged_local_takes_remote_update() {
    let ancestor = session("web", "old", 10);
    let local = catalog(vec![ancestor.clone()]);
    let mut remote_session = ancestor.clone();
    remote_session.name = "web-renamed".into();
    remote_session.updated_at = 20;
    let remote = catalog(vec![remote_session]);
    let (merged, report) = merge_local(Some(&catalog(vec![ancestor])), &local, &remote);
    assert_eq!(merged.sessions[0].name, "web-renamed");
    assert_eq!(report.sessions_updated, 1);
    assert_eq!(report.conflicts, 0);
}

#[test]
fn local_change_wins_when_remote_is_unchanged() {
    let ancestor = session("web", "old", 10);
    let mut local_session = ancestor.clone();
    local_session.name = "local-name".into();
    let local = catalog(vec![local_session]);
    let remote = catalog(vec![ancestor.clone()]);
    let (merged, _) = merge_local(Some(&catalog(vec![ancestor])), &local, &remote);
    assert_eq!(merged.sessions[0].name, "local-name");
}

#[test]
fn both_changed_prefers_newer_timestamp() {
    let ancestor = session("web", "old", 10);
    let mut local_session = ancestor.clone();
    local_session.name = "local".into();
    local_session.updated_at = 50;
    let mut remote_session = ancestor.clone();
    remote_session.name = "remote".into();
    remote_session.updated_at = 60;
    let (merged, report) = merge_local(
        Some(&catalog(vec![ancestor])),
        &catalog(vec![local_session]),
        &catalog(vec![remote_session]),
    );
    assert_eq!(merged.sessions[0].name, "remote");
    assert_eq!(report.conflicts, 1);
}

#[test]
fn both_changed_equal_timestamp_keeps_local() {
    let ancestor = session("web", "old", 10);
    let mut local_session = ancestor.clone();
    local_session.name = "local".into();
    local_session.updated_at = 50;
    let mut remote_session = ancestor.clone();
    remote_session.name = "remote".into();
    remote_session.updated_at = 50;
    let (merged, _) = merge_local(
        Some(&catalog(vec![ancestor])),
        &catalog(vec![local_session]),
        &catalog(vec![remote_session]),
    );
    assert_eq!(merged.sessions[0].name, "local");
}

#[test]
fn deletion_applies_only_to_unmodified_side() {
    let ancestor = session("web", "old", 10);
    // 本机未改、远端删除：应从合并结果中消失。
    let local = catalog(vec![ancestor.clone()]);
    let remote = catalog(vec![]);
    let (merged, report) = merge_local(Some(&catalog(vec![ancestor.clone()])), &local, &remote);
    assert!(merged.sessions.is_empty());
    assert_eq!(report.sessions_removed, 1);

    // 本机改过、远端删除：保留本机修改。
    let mut changed = ancestor.clone();
    changed.name = "kept".into();
    let local = catalog(vec![changed]);
    let (merged, _) = merge_local(Some(&catalog(vec![ancestor])), &local, &remote);
    assert_eq!(merged.sessions[0].name, "kept");
}

#[test]
fn merge_reports_additions_and_removals_together() {
    let web = session("web", "old", 10);
    let ancestor = catalog(vec![web.clone()]);
    let local = catalog(vec![web]);
    let remote = catalog(vec![session("db", "db.remote", 20)]);
    let (merged, report) = merge_local(Some(&ancestor), &local, &remote);
    assert_eq!(report.sessions_removed, 1, "本机未改的旧会话应真正删除");
    assert_eq!(report.sessions_added, 1, "远端新会话应并入");
    assert_eq!(merged.sessions.len(), 1);
    assert_eq!(merged.sessions[0].name, "db");
}

#[test]
fn no_base_prefers_newer_timestamp_and_counts_as_update() {
    let origin = session("web", "local", 10);
    let mut local_session = origin.clone();
    local_session.name = "local-name".into();
    let mut remote_session = origin;
    remote_session.name = "remote-name".into();
    remote_session.updated_at = 20;
    let (merged, report) = merge_local(
        None,
        &catalog(vec![local_session]),
        &catalog(vec![remote_session]),
    );
    assert_eq!(
        merged.sessions[0].name, "remote-name",
        "无共同祖先时按时间戳取较新者"
    );
    assert_eq!(report.sessions_updated, 1);
    assert_eq!(report.conflicts, 0, "采用远端版本应计为更新而非冲突");
}

#[test]
fn equal_session_timestamp_follows_preference() {
    let base = session("web", "old", 10);
    let mut remote = base.clone();
    remote.name = "remote".into();
    let local = catalog(vec![base]);
    let remote = catalog(vec![remote]);

    let (merged, report) = merge_catalogs(None, &local, &remote, MergePreference::Remote);
    assert_eq!(merged.sessions[0].name, "remote", "平局时拉取取远端");
    assert_eq!(report.sessions_updated, 1);
    assert_eq!(report.conflicts, 0);
    let (merged, report) = merge_catalogs(None, &local, &remote, MergePreference::Local);
    assert_eq!(merged.sessions[0].name, "web", "平局时推送保留本机");
    assert_eq!(report.conflicts, 1, "平局保留本机时计入冲突");
}

#[test]
fn folder_conflict_follows_preference_and_keeps_collapse() {
    let id = FolderId::new();
    let mut local = catalog(Vec::new());
    local.folders.push(SessionFolder {
        id,
        name: "local".into(),
        collapsed: true,
        ..Default::default()
    });
    let mut remote = catalog(Vec::new());
    remote.folders.push(SessionFolder {
        id,
        name: "remote".into(),
        collapsed: false,
        ..Default::default()
    });

    let (merged, report) = merge_catalogs(None, &local, &remote, MergePreference::Remote);
    assert_eq!(merged.folders[0].name, "remote");
    assert!(
        merged.folders[0].collapsed,
        "覆盖远端文件夹时保留本机折叠态"
    );
    assert_eq!(report.folders_updated, 1);
    assert_eq!(report.conflicts, 0);

    let (merged, report) = merge_catalogs(None, &local, &remote, MergePreference::Local);
    assert_eq!(
        merged.folders[0].name, "local",
        "推送时本机文件夹不被远端覆盖"
    );
    assert_eq!(report.conflicts, 1);
}

#[test]
fn pull_prefers_newer_local_session_over_remote() {
    let mut local_session = session("local", "h", 20);
    let mut remote_session = local_session.clone();
    remote_session.name = "remote".into();
    remote_session.updated_at = 10;
    local_session.name = "local".into();
    let local = catalog(vec![local_session]);
    let remote = catalog(vec![remote_session]);

    let (merged, report) = merge_catalogs(None, &local, &remote, MergePreference::Remote);
    assert_eq!(
        merged.sessions[0].name, "local",
        "拉取时本机较新的编辑不得被旧远端覆盖"
    );
    assert_eq!(report.conflicts, 1, "保留本机较新版本属需要用户关注的差异");
}

#[test]
fn engine_profiles_merge_by_newer_timestamp() {
    use crate::model::container::{
        ContainerEngineKind, ContainerEngineProfile, ContainerEngineRef,
    };

    let mut local_profile = ContainerEngineProfile::new(
        "本机 Docker",
        ContainerEngineRef::auto(ContainerEngineKind::Docker),
    );
    local_profile.updated_at = 10;
    let mut remote_profile = local_profile.clone();
    remote_profile.name = "远端 Docker".into();
    remote_profile.updated_at = 20;
    let local = SessionCatalog {
        engines: vec![local_profile],
        ..Default::default()
    };
    let remote = SessionCatalog {
        engines: vec![remote_profile],
        ..Default::default()
    };

    let (merged, report) = merge_catalogs(None, &local, &remote, MergePreference::Local);
    assert_eq!(
        merged.engines[0].name, "远端 Docker",
        "配置合并与会话一致：时间戳较新者胜"
    );
    assert_eq!(report.engines_updated, 1);
}
