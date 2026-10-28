use super::*;
use crate::error::ErrorCode;
use crate::model::session::{Connection, LocalConnection};

fn session(catalog: &mut SessionCatalog, name: &str) -> SessionId {
    let session = Session::new(name, Connection::Local(LocalConnection::default()));
    let id = session.id;
    catalog.sessions.push(session);
    id
}

fn session_ids(catalog: &SessionCatalog) -> Vec<SessionId> {
    catalog.sessions.iter().map(|session| session.id).collect()
}

fn folder_ids(catalog: &SessionCatalog) -> Vec<FolderId> {
    catalog.folders.iter().map(|folder| folder.id).collect()
}

#[test]
fn ensure_path_creates_and_reuses_nested_folders() {
    let mut catalog = SessionCatalog::default();
    let leaf = catalog.ensure_folder_path("生产/数据库").unwrap();
    assert_eq!(catalog.folder_names(leaf), vec!["生产", "数据库"]);
    assert_eq!(catalog.ensure_folder_path("生产/数据库"), Some(leaf));
    assert_eq!(catalog.folders.len(), 2);
}

#[test]
fn folder_names_reject_slashes() {
    let mut catalog = SessionCatalog::default();
    assert_eq!(
        catalog.add_folder("A/B", None),
        Err(CatalogError::InvalidFolderName)
    );
    let id = catalog.add_folder("A", None).unwrap();
    assert_eq!(
        catalog.rename_folder(id, "A/B"),
        Err(CatalogError::InvalidFolderName)
    );
}

#[test]
fn move_folder_rejects_cycle() {
    let mut catalog = SessionCatalog::default();
    let parent = catalog.ensure_folder_path("A").unwrap();
    let child = catalog.ensure_folder_path("A/B/C").unwrap();
    assert_eq!(
        catalog.move_folder(parent, Some(child)),
        Err(CatalogError::FolderCycle(parent))
    );
    assert!(catalog.move_folder(child, Some(parent)).is_ok());
}

/// 数组顺序即显示顺序：向前/向后插入必须精确改变同级先后。
#[test]
fn reorder_session_places_relative_to_anchor() {
    let mut catalog = SessionCatalog::default();
    let folder = catalog.ensure_folder_path("A").unwrap();
    let first = session(&mut catalog, "first");
    let second = session(&mut catalog, "second");
    let third = session(&mut catalog, "third");
    for id in [first, second, third] {
        catalog.move_session(id, Some(folder)).unwrap();
    }
    assert_eq!(
        session_ids(&catalog),
        vec![first, second, third],
        "放入同一文件夹应按移动先后排列"
    );

    catalog
        .reorder_session(third, first, DropSide::After)
        .unwrap();
    assert_eq!(session_ids(&catalog), vec![first, third, second]);

    catalog
        .reorder_session(first, second, DropSide::After)
        .unwrap();
    assert_eq!(session_ids(&catalog), vec![third, second, first]);

    catalog
        .reorder_session(second, third, DropSide::Before)
        .unwrap();
    assert_eq!(session_ids(&catalog), vec![second, third, first]);
}

/// 跨分组拖到某条会话上时，父级跟随锚点，一次操作完成“移动 + 定位”。
#[test]
fn reorder_session_follows_anchor_folder() {
    let mut catalog = SessionCatalog::default();
    let left = catalog.ensure_folder_path("A").unwrap();
    let right = catalog.ensure_folder_path("B").unwrap();
    let alpha = session(&mut catalog, "alpha");
    let beta = session(&mut catalog, "beta");
    let gamma = session(&mut catalog, "gamma");
    catalog.move_session(alpha, Some(left)).unwrap();
    catalog.move_session(beta, Some(left)).unwrap();
    catalog.move_session(gamma, Some(right)).unwrap();

    catalog
        .reorder_session(gamma, beta, DropSide::After)
        .unwrap();

    assert_eq!(catalog.sessions[2].id, gamma);
    assert_eq!(catalog.sessions[2].folder_id, Some(left));
    assert_eq!(session_ids(&catalog), vec![alpha, beta, gamma]);
}

/// 锚点就是自身、或锚点不存在时，不得破坏既有顺序。
#[test]
fn reorder_session_rejects_invalid_anchor() {
    let mut catalog = SessionCatalog::default();
    let alpha = session(&mut catalog, "alpha");
    let beta = session(&mut catalog, "beta");

    catalog
        .reorder_session(alpha, alpha, DropSide::Before)
        .unwrap();
    assert_eq!(session_ids(&catalog), vec![alpha, beta]);

    let missing = SessionId::new();
    assert_eq!(
        catalog.reorder_session(alpha, missing, DropSide::Before),
        Err(CatalogError::SessionNotFound(missing))
    );
    assert_eq!(session_ids(&catalog), vec![alpha, beta]);
}

/// 文件夹重排同样只改同级先后，并且禁止把父级插到自己子级之间。
#[test]
fn reorder_folder_orders_siblings_and_rejects_cycle() {
    let mut catalog = SessionCatalog::default();
    let a = catalog.ensure_folder_path("A").unwrap();
    let b = catalog.ensure_folder_path("B").unwrap();
    let c = catalog.ensure_folder_path("C").unwrap();
    assert_eq!(folder_ids(&catalog), vec![a, b, c]);

    catalog.reorder_folder(c, a, DropSide::After).unwrap();
    assert_eq!(folder_ids(&catalog), vec![a, c, b]);

    let child_of_a = catalog.ensure_folder_path("A/D").unwrap();
    assert_eq!(
        catalog.reorder_folder(a, child_of_a, DropSide::Before),
        Err(CatalogError::FolderCycle(a))
    );

    // 锚点位于另一个层级时，父级跟随锚点。
    catalog
        .reorder_folder(b, child_of_a, DropSide::Before)
        .unwrap();
    assert_eq!(catalog.folder(b).unwrap().parent, Some(a));
    assert_eq!(folder_ids(&catalog), vec![a, c, b, child_of_a]);
}

#[test]
fn delete_folder_promotes_contents_to_parent() {
    let mut catalog = SessionCatalog::default();
    let parent = catalog.ensure_folder_path("A").unwrap();
    let child = catalog.ensure_folder_path("A/B").unwrap();
    let session_id = session(&mut catalog, "db");
    catalog.move_session(session_id, Some(child)).unwrap();

    assert_eq!(catalog.delete_folder(child).unwrap(), "B");
    assert_eq!(catalog.folder(child), None);
    assert_eq!(
        catalog.sessions[0].folder_id,
        Some(parent),
        "会话应提升到被删文件夹的父级"
    );
}

#[test]
fn normalize_breaks_cycles_and_missing_parents() {
    let mut catalog = SessionCatalog::default();
    let a = FolderId::new();
    let b = FolderId::new();
    catalog.folders.push(SessionFolder {
        id: a,
        name: "A".into(),
        parent: Some(b),
        collapsed: false,
        ..Default::default()
    });
    catalog.folders.push(SessionFolder {
        id: b,
        name: "B".into(),
        parent: Some(a),
        collapsed: false,
        ..Default::default()
    });
    let orphan = session(&mut catalog, "lost");
    catalog.sessions[0].folder_id = Some(FolderId::new());

    catalog.normalize();

    assert!(catalog.folder(a).unwrap().parent.is_none());
    assert!(catalog.folder(b).unwrap().parent.is_none());
    assert_eq!(catalog.sessions[0].id, orphan);
    assert_eq!(catalog.sessions[0].folder_id, None);
}

#[test]
fn merge_adds_new_and_updates_same_ids() {
    let mut local = SessionCatalog::default();
    let folder = local.add_folder("本地", None).unwrap();
    local.set_folder_collapsed(folder, true).unwrap();
    let local_session = session(&mut local, "same");

    let mut remote = SessionCatalog::default();
    remote.folders.push(SessionFolder {
        id: folder,
        name: "远端".into(),
        parent: None,
        collapsed: false,
        ..Default::default()
    });
    let mut same = Session::new("same", Connection::Local(LocalConnection::default()));
    same.id = local_session;
    let new_session = Session::new("new", Connection::Local(LocalConnection::default()));
    remote.sessions = vec![same, new_session];

    let summary = local.merge(remote);
    assert_eq!(summary.added_sessions, 1);
    assert_eq!(summary.updated_sessions, 1);
    assert_eq!(summary.updated_folders, 1);
    assert_eq!(local.folders[0].name, "远端");
    assert!(local.folders[0].collapsed, "折叠状态应保留本机偏好");
    assert_eq!(local.sessions.len(), 2);
}

#[test]
fn normalize_drops_duplicate_folder_ids() {
    let mut catalog = SessionCatalog::default();
    let id = FolderId::new();
    catalog.folders.push(SessionFolder::new("one", None));
    catalog.folders[0].id = id;
    catalog.folders.push(SessionFolder {
        id,
        name: "duplicate".into(),
        parent: None,
        collapsed: false,
        ..Default::default()
    });
    catalog.normalize();
    assert_eq!(catalog.folders.len(), 1);
    assert_eq!(catalog.folders[0].name, "one");
}

fn ssh_session(catalog: &mut SessionCatalog, name: &str) -> SessionId {
    let session = Session::new(
        name,
        crate::model::session::Connection::Ssh(crate::model::session::SshConnection::new(
            "h",
            "inline-user",
        )),
    );
    let id = session.id;
    catalog.sessions.push(session);
    id
}

fn ops(catalog: &mut SessionCatalog) -> crate::ids::IdentityId {
    let identity = crate::model::identity::Identity::new(
        "ops",
        "ops-user",
        crate::model::session::AuthMethod::Password { save: true },
    );
    let id = identity.id;
    catalog.identities.push(identity);
    id
}

fn set_jump(catalog: &mut SessionCatalog, session: SessionId, jump: SessionId) {
    let session = catalog
        .sessions
        .iter_mut()
        .find(|candidate| candidate.id == session)
        .unwrap();
    let crate::model::session::Connection::Ssh(ssh) = &mut session.connection else {
        panic!("测试会话必须是 SSH 连接");
    };
    ssh.proxy_jump = Some(crate::model::session::ProxyJumpConfig {
        jump_session_id: jump,
    });
}

/// 多层跳板链必须按"最外层 -> 最内层"返回，建连方按此顺序逐级打洞。
#[test]
fn proxy_jump_chain_is_ordered_outermost_first() {
    let mut catalog = SessionCatalog::default();
    let outer = ssh_session(&mut catalog, "outer");
    let inner = ssh_session(&mut catalog, "inner");
    let target = ssh_session(&mut catalog, "target");
    set_jump(&mut catalog, inner, outer);
    set_jump(&mut catalog, target, inner);

    let session = catalog
        .sessions
        .iter()
        .find(|candidate| candidate.id == target)
        .unwrap();
    let chain = catalog.proxy_jump_chain(session).unwrap();
    let names: Vec<&str> = chain.iter().map(|hop| hop.name.as_str()).collect();
    assert_eq!(names, vec!["outer", "inner"]);
    // 链路副本同样叠加了模板/文件夹继承，与直连目标保持同一口径。
    assert!(matches!(
        chain[0].connection,
        crate::model::session::Connection::Ssh(_)
    ));
}

#[test]
fn direct_connection_has_empty_proxy_chain() {
    let mut catalog = SessionCatalog::default();
    let id = ssh_session(&mut catalog, "direct");
    let session = catalog
        .sessions
        .iter()
        .find(|candidate| candidate.id == id)
        .unwrap();
    assert!(catalog.proxy_jump_chain(session).unwrap().is_empty());
}

#[test]
fn proxy_jump_chain_rejects_self_cycle_and_dangling_references() {
    let mut catalog = SessionCatalog::default();
    let lone = ssh_session(&mut catalog, "lone");
    set_jump(&mut catalog, lone, lone);
    let session = catalog
        .sessions
        .iter()
        .find(|candidate| candidate.id == lone)
        .unwrap();
    let error = catalog.proxy_jump_chain(session).unwrap_err();
    assert_eq!(error.code, ErrorCode::Configuration);
    assert!(error.detail.as_deref().unwrap_or_default().contains("环形"));

    let mut catalog = SessionCatalog::default();
    let first = ssh_session(&mut catalog, "first");
    let second = ssh_session(&mut catalog, "second");
    set_jump(&mut catalog, first, second);
    set_jump(&mut catalog, second, first);
    let session = catalog
        .sessions
        .iter()
        .find(|candidate| candidate.id == first)
        .unwrap();
    assert!(catalog.proxy_jump_chain(session).is_err());

    let mut catalog = SessionCatalog::default();
    let dangling = ssh_session(&mut catalog, "dangling");
    set_jump(&mut catalog, dangling, SessionId::new());
    let session = catalog
        .sessions
        .iter()
        .find(|candidate| candidate.id == dangling)
        .unwrap();
    let error = catalog.proxy_jump_chain(session).unwrap_err();
    assert!(
        error
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("不存在")
    );
}

/// 跳板机只能是 SSH 会话：串口/本地终端没有可复用的加密隧道。
#[test]
fn proxy_jump_chain_rejects_non_ssh_jump() {
    let mut catalog = SessionCatalog::default();
    let local = Session::new(
        "local",
        crate::model::session::Connection::Local(crate::model::session::LocalConnection::default()),
    );
    let local_id = local.id;
    catalog.sessions.push(local);
    let target = ssh_session(&mut catalog, "target");
    set_jump(&mut catalog, target, local_id);

    let session = catalog
        .sessions
        .iter()
        .find(|candidate| candidate.id == target)
        .unwrap();
    let error = catalog.proxy_jump_chain(session).unwrap_err();
    assert!(
        error
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("不是 SSH")
    );
}

/// 模板叠加只改身份（用户名 + 认证方式），主机、端口与机器级选项保持不变。
#[test]
fn resolved_session_overrides_identity_fields_only() {
    let mut catalog = SessionCatalog::default();
    let identity_id = ops(&mut catalog);
    let session_id = ssh_session(&mut catalog, "web");
    catalog
        .sessions
        .iter_mut()
        .find(|session| session.id == session_id)
        .unwrap()
        .identity_id = Some(identity_id);

    let session = catalog
        .sessions
        .iter()
        .find(|session| session.id == session_id)
        .unwrap();
    let resolved = catalog.resolved_session(session);
    match &resolved.connection {
        crate::model::session::Connection::Ssh(ssh) => {
            assert_eq!(ssh.username, "ops-user");
            assert_eq!(ssh.host, "h");
            assert_eq!(ssh.port, 22);
            assert!(matches!(
                ssh.auth,
                crate::model::session::AuthMethod::Password { save: true }
            ));
        }
        _ => panic!("应仍是 SSH 连接"),
    }
    // 原会话不被修改：叠加发生在连接派发时的副本上。
    match &session.connection {
        crate::model::session::Connection::Ssh(ssh) => assert_eq!(ssh.username, "inline-user"),
        _ => panic!("应仍是 SSH 连接"),
    }
}

/// 未引用模板时逐字段原样返回，保证老会话行为不变。
#[test]
fn resolved_session_passes_through_without_reference() {
    let catalog = SessionCatalog::default();
    let session = Session::new(
        "web",
        crate::model::session::Connection::Ssh(crate::model::session::SshConnection::new("h", "u")),
    );
    let resolved = catalog.resolved_session(&session);
    match &resolved.connection {
        crate::model::session::Connection::Ssh(ssh) => {
            assert_eq!(ssh.username, "u");
            assert!(matches!(ssh.auth, crate::model::session::AuthMethod::Agent));
        }
        _ => panic!("应仍是 SSH 连接"),
    }
    assert_eq!(resolved.id, session.id);
}

/// 引用模板的会话必须先解除引用才能删模板：静默清引用会让那些会话
/// 突然退回（可能为空的）内联认证，表现为"昨天还能连今天连不上"。
#[test]
fn delete_identity_refuses_while_referenced() {
    let mut catalog = SessionCatalog::default();
    let identity_id = ops(&mut catalog);
    let session_id = ssh_session(&mut catalog, "web");
    catalog
        .sessions
        .iter_mut()
        .find(|session| session.id == session_id)
        .unwrap()
        .identity_id = Some(identity_id);

    assert_eq!(catalog.identity_users(identity_id), 1);
    assert_eq!(
        catalog.delete_identity(identity_id),
        Err(CatalogError::IdentityInUse(1))
    );
    assert_eq!(catalog.identities.len(), 1);

    catalog
        .sessions
        .iter_mut()
        .find(|session| session.id == session_id)
        .unwrap()
        .identity_id = None;
    assert_eq!(catalog.identity_users(identity_id), 0);
    assert!(catalog.delete_identity(identity_id).is_ok());
    assert!(catalog.identities.is_empty());
    assert!(matches!(
        catalog.delete_identity(identity_id),
        Err(CatalogError::IdentityNotFound(_))
    ));
}

/// 悬空模板引用与重复模板 id 都要在归一时收敛：坏数据不能让会话带着
/// 无效指针去连接，也不能让口令存进两个同 id 槽位。
#[test]
fn normalize_clears_dangling_identity_references() {
    let mut catalog = SessionCatalog::default();
    let identity_id = ops(&mut catalog);
    let session_id = ssh_session(&mut catalog, "web");
    catalog
        .sessions
        .iter_mut()
        .find(|session| session.id == session_id)
        .unwrap()
        .identity_id = Some(crate::ids::IdentityId::new());
    let duplicate = catalog.identities[0].clone();
    catalog.identities.push(duplicate);

    catalog.normalize();
    assert_eq!(catalog.identities.len(), 1);
    assert_eq!(catalog.identities[0].id, identity_id);
    assert!(catalog.sessions[0].identity_id.is_none());
}

#[test]
fn folder_defaults_inheritance_and_session_resolution() {
    let mut catalog = SessionCatalog::default();
    let root = catalog.add_folder("root", None).unwrap();
    catalog
        .update_folder(
            root,
            "root",
            Some(2200),
            Some("root_user".to_string()),
            Some(PathBuf::from("/keys/root_id_rsa")),
        )
        .unwrap();

    let sub = catalog.add_folder("sub", Some(root)).unwrap();
    // sub 只覆盖端口，用户名和私钥留空，继承自 root
    catalog
        .update_folder(sub, "sub", Some(2222), None, None)
        .unwrap();

    let defaults = catalog.inherited_folder_defaults(sub);
    assert_eq!(defaults.port, Some(2222));
    assert_eq!(defaults.username.as_deref(), Some("root_user"));
    assert_eq!(
        defaults.key_path.as_deref(),
        Some(std::path::Path::new("/keys/root_id_rsa"))
    );

    // 新建一个属于 sub 的会话，用户名为空，端口为默认 22，私钥为空
    let mut session = Session::new(
        "child_session",
        crate::model::session::Connection::Ssh(crate::model::session::SshConnection {
            host: "server.internal".into(),
            port: 22,
            username: "".into(),
            auth: crate::model::session::AuthMethod::PrivateKey {
                path: PathBuf::new(),
                cert_path: None,
                save_passphrase: false,
            },
            options: crate::model::session::SessionOptions::default(),
            proxy_jump: None,
        }),
    );
    session.folder_id = Some(sub);

    let resolved = catalog.resolved_session(&session);
    if let crate::model::session::Connection::Ssh(ssh) = resolved.connection {
        assert_eq!(ssh.port, 2222);
        assert_eq!(ssh.username, "root_user");
        if let crate::model::session::AuthMethod::PrivateKey { path, .. } = ssh.auth {
            assert_eq!(path, PathBuf::from("/keys/root_id_rsa"));
        } else {
            panic!("应为私钥认证");
        }
    } else {
        panic!("应为 SSH 连接");
    }
}
