//! 凭据模板（Identity）数据模型、表单输入与校验逻辑。

use crate::session_form::AuthChoice;
use gpui_kit::Entity;
use gpui_kit::component::input::InputState;
use transom_core::{AuthMethod, Identity, IdentityId, SecretKey, SecretKind, SecretOwner};

/// 模板编辑器状态；`base` 为 `None` 表示新建。
#[derive(Clone)]
pub(crate) struct IdentityEditState {
    pub(crate) base: Option<IdentityId>,
    pub(crate) name: Entity<InputState>,
    pub(crate) username: Entity<InputState>,
    pub(crate) auth: AuthChoice,
    pub(crate) key_path: Entity<InputState>,
    pub(crate) password: Entity<InputState>,
    pub(crate) passphrase: Entity<InputState>,
    pub(crate) save_password: bool,
    pub(crate) save_passphrase: bool,
    pub(crate) error: Option<String>,
}

/// 校验并构造模板的结果，连同要落进钥匙串的口令变更。
pub(crate) struct BuiltIdentity {
    pub(crate) identity: Identity,
    /// `(槽位, 新值)`；`None` 表示删除该槽位。
    pub(crate) writes: Vec<(SecretKey, Option<String>)>,
}

/// 模板表单的一次提交内容。
#[derive(Clone, Copy)]
pub(crate) struct IdentityForm<'a> {
    pub(crate) name: &'a str,
    pub(crate) username: &'a str,
    pub(crate) auth: AuthChoice,
    pub(crate) key_path: &'a str,
    pub(crate) save_password: bool,
    pub(crate) save_passphrase: bool,
    pub(crate) password: &'a str,
    pub(crate) passphrase: &'a str,
    pub(crate) base: Option<IdentityId>,
}

/// 校验并构造模板。
pub(crate) fn build_identity(form: &IdentityForm<'_>) -> Result<BuiltIdentity, String> {
    let IdentityForm {
        name,
        username,
        auth,
        key_path,
        save_password,
        save_passphrase,
        password,
        passphrase,
        base,
    } = *form;
    let name = name.trim();
    if name.is_empty() {
        return Err(rust_i18n::t!("identity.error_name").to_string());
    }
    let username = username.trim();
    if username.is_empty() {
        return Err(rust_i18n::t!("identity.error_username").to_string());
    }
    let method = match auth {
        AuthChoice::Password => AuthMethod::Password {
            save: save_password,
        },
        AuthChoice::PrivateKey => {
            let path = key_path.trim();
            if path.is_empty() {
                return Err(rust_i18n::t!("identity.error_key").to_string());
            }
            AuthMethod::PrivateKey {
                path: crate::keys::expand_home(path).to_path_buf(),
                cert_path: None,
                save_passphrase,
            }
        }
        AuthChoice::Agent => AuthMethod::Agent,
    };
    let identity = match base {
        Some(id) => Identity {
            id,
            name: name.to_string(),
            username: username.to_string(),
            auth: method,
        },
        None => Identity::new(name, username, method),
    };
    let owner = SecretOwner::Identity(identity.id);
    let mut writes = Vec::new();
    match &identity.auth {
        AuthMethod::Password { save } => {
            let value = (*save && !password.is_empty()).then(|| password.to_string());
            writes.push((SecretKey::new(owner, SecretKind::Password), value));
        }
        AuthMethod::PrivateKey {
            save_passphrase, ..
        } => {
            let value =
                (*save_passphrase && !passphrase.is_empty()).then(|| passphrase.to_string());
            writes.push((SecretKey::new(owner, SecretKind::KeyPassphrase), value));
        }
        AuthMethod::Agent => {}
        _ => {}
    }
    Ok(BuiltIdentity { identity, writes })
}

/// 模板清单里的一行摘要。
pub(crate) fn describe_auth(auth: &AuthMethod) -> String {
    match auth {
        AuthMethod::Password { .. } => rust_i18n::t!("identity.auth_password").to_string(),
        AuthMethod::PrivateKey { path, .. } => format!(
            "{} · {}",
            rust_i18n::t!("identity.auth_key"),
            path.display()
        ),
        AuthMethod::Agent => rust_i18n::t!("identity.auth_agent").to_string(),
        _ => rust_i18n::t!("identity.auth_other").to_string(),
    }
}
