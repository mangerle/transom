//! 凭据模板模块单元测试。

use super::types::{build_identity, BuiltIdentity, IdentityForm};
use crate::session_form::AuthChoice;
use std::path::Path;
use transom_core::{AuthMethod, IdentityId, SecretKind, SecretOwner};

/// 常用形态的表单快照：其余字段取中性值，让每个用例只表达它关心的差异。
fn form(auth: AuthChoice, save: bool, password: &str, passphrase: &str) -> BuiltIdentity {
    let built = build_identity(&IdentityForm {
        name: "ops",
        username: "ops-user",
        auth,
        key_path: "/home/ops/.ssh/id_ed25519",
        save_password: save,
        save_passphrase: save,
        password,
        passphrase,
        base: None,
    });
    built.expect("构造模板应通过校验")
}

fn blank(
    auth: AuthChoice,
    name: &str,
    username: &str,
    key_path: &str,
) -> Result<BuiltIdentity, String> {
    build_identity(&IdentityForm {
        name,
        username,
        auth,
        key_path,
        save_password: false,
        save_passphrase: false,
        password: "",
        passphrase: "",
        base: None,
    })
}

#[test]
fn rejects_blank_fields() {
    assert!(blank(AuthChoice::Agent, "  ", "u", "").is_err());
    assert!(blank(AuthChoice::Agent, "ops", " ", "").is_err());
    // 私钥认证必须给路径，否则连不上还看不出原因。
    assert!(blank(AuthChoice::PrivateKey, "ops", "u", "  ").is_err());
    assert!(blank(AuthChoice::PrivateKey, "ops", "u", "/k").is_ok());
}

/// 模板口令按 IdentityId 归档，槽位与会话口令互不干扰。
#[test]
fn password_secret_is_keyed_by_identity() {
    let built = form(AuthChoice::Password, true, "p@ss", "");
    assert_eq!(built.writes.len(), 1);
    let (key, value) = &built.writes[0];
    assert!(matches!(key.owner, SecretOwner::Identity(_)));
    assert_eq!(key.kind, SecretKind::Password);
    assert_eq!(value.as_deref(), Some("p@ss"));
    // 账号串带 identity/ 前缀，与会话槽位的 `<uuid>/<kind>` 形状区分开。
    assert!(key.account().starts_with("identity/"));
}

/// 未勾选保存、或勾了保存但内容为空，都必须落到"删除"语义。
#[test]
fn unsaved_secret_maps_to_delete() {
    let unsaved = form(AuthChoice::Password, false, "p@ss", "");
    assert_eq!(unsaved.writes.len(), 1);
    assert!(unsaved.writes[0].1.is_none());

    let cleared = form(AuthChoice::Password, true, "", "");
    assert!(cleared.writes[0].1.is_none());
}

#[test]
fn agent_auth_produces_no_secret_writes() {
    let built = form(AuthChoice::Agent, true, "ignored", "");
    assert!(built.writes.is_empty());
    assert!(matches!(built.identity.auth, AuthMethod::Agent));
}

/// 编辑必须沿用原 id：换新 id 会让所有引用它的会话突然变成"未引用"。
#[test]
fn editing_keeps_the_original_id() {
    let id = IdentityId::new();
    let built = build_identity(&IdentityForm {
        name: "ops2",
        username: "ops",
        auth: AuthChoice::Agent,
        key_path: "",
        save_password: false,
        save_passphrase: false,
        password: "",
        passphrase: "",
        base: Some(id),
    })
    .unwrap();
    assert_eq!(built.identity.id, id);
}

/// 模板里的私钥路径与连接参数一同规范化，清单显示的是真实路径。
#[test]
fn key_path_is_expanded() {
    let built = form(AuthChoice::PrivateKey, true, "", "phrase");
    match &built.identity.auth {
        AuthMethod::PrivateKey { path, .. } => {
            assert!(!path.to_string_lossy().starts_with('~'));
            assert_eq!(path, Path::new("/home/ops/.ssh/id_ed25519"));
        }
        _ => panic!("应为私钥认证"),
    }
    assert_eq!(built.writes[0].0.kind, SecretKind::KeyPassphrase);
    assert_eq!(built.writes[0].1.as_deref(), Some("phrase"));
}

/// `~` 必须展开成绝对路径：与会话表单同口径，连接层不必理解家目录简写。
#[test]
fn tilde_in_key_path_is_expanded_to_home() {
    let built = build_identity(&IdentityForm {
        name: "ops",
        username: "ops",
        auth: AuthChoice::PrivateKey,
        key_path: "~/.ssh/id_ed25519",
        save_password: false,
        save_passphrase: false,
        password: "",
        passphrase: "",
        base: None,
    })
    .unwrap();
    match &built.identity.auth {
        AuthMethod::PrivateKey { path, .. } => {
            assert!(!path.to_string_lossy().starts_with('~'));
            assert!(path.is_absolute(), "实际: {path:?}");
        }
        _ => panic!("应为私钥认证"),
    }
}
