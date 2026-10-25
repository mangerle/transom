//! 凭据模板（Identity）管理页：清单、新建/编辑与删除。
//!
//! 设计原理（Why）：
//! - 实现初衷：几十台机器共用一个运维账号时，改密要逐台重填。模板把"谁登录"
//!   独立出来，会话只引用它，改一处全生效；
//! - 核心优势：口令仍然只进系统钥匙串（模板里只记"用哪种认证、存不存"），
//!   本模块多用的只是一个按 IdentityId 归档的槽位，与会话槽位互不干扰；
//!   被引用的模板不允许删除，避免留下"会话突然退回内联认证"的悬空引用；
//! - 代价与局限：模板与会话同库同文件，因此跟着备份与云同步一起走；模板口令
//!   只有在导出时勾选"包含口令"才会进备份文件，跨机器迁移身份必须走那一步。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::palette::localized_error;
use crate::session_form::{AuthChoice, text_input};
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::InputState;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{
    AppError, AuthMethod, ErrorCode, Identity, IdentityId, SecretKey, SecretKind, SecretOwner,
};

/// 模板编辑器状态；`base` 为 `None` 表示新建。
///
/// 派生 `Clone` 是为了渲染期能把它从 `Context` 里取成局部值再交给按钮闭包：
/// 字段全是 `Entity`，克隆只是引用计数，不复制输入内容。
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
///
/// 收敛成结构体而不是九个平铺参数：字段多且同类型（四个 `&str`、两个 `bool`），
/// 平铺传参最容易把"保存密码"与"保存口令"的顺序传反。
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
///
/// 与界面渲染分开是为了能单测非法输入；空值语义与会话表单一致：
/// 勾选保存却把内容清空，就是"删掉这条凭据"。
fn build_identity(form: &IdentityForm<'_>) -> Result<BuiltIdentity, String> {
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
            // 与会话表单同口径：落盘展开后的绝对路径，连接层不必理解 `~`。
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
    // 按认证方式决定要动哪个槽位：agent 方式没有可保存的凭据，不产生写入。
    // 写入值存在即 Set，为 `None` 即 Delete——取消勾选或清空都要清掉旧口令，
    // 否则用户会以为已经删除，钥匙串里却还留着。
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
        // 未来新增认证方式时不必到这里补分支：无口令即不写槽位。
        _ => {}
    }
    Ok(BuiltIdentity { identity, writes })
}

/// 模板清单里的一行摘要。
fn describe_auth(auth: &AuthMethod) -> String {
    match auth {
        AuthMethod::Password { .. } => rust_i18n::t!("identity.auth_password").to_string(),
        AuthMethod::PrivateKey { path, .. } => format!(
            "{} · {}",
            rust_i18n::t!("identity.auth_key"),
            path.display()
        ),
        AuthMethod::Agent => rust_i18n::t!("identity.auth_agent").to_string(),
        // 未来新增认证方式时清单不至于显示空白。
        _ => rust_i18n::t!("identity.auth_other").to_string(),
    }
}

impl AppShell {
    /// 打开模板编辑器；编辑时后台读取已保存口令回填输入框。
    pub(crate) fn open_identity_editor(
        &mut self,
        id: Option<IdentityId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = id.and_then(|id| self.catalog.identity(id).cloned());
        let (auth, key_path) = match existing.as_ref().map(|identity| &identity.auth) {
            Some(AuthMethod::PrivateKey { path, .. }) => {
                (AuthChoice::PrivateKey, path.display().to_string())
            }
            Some(AuthMethod::Password { .. }) => (AuthChoice::Password, String::new()),
            _ => (AuthChoice::Agent, String::new()),
        };
        let save_password = matches!(
            existing.as_ref().map(|identity| &identity.auth),
            Some(AuthMethod::Password { save: true })
        );
        let save_passphrase = matches!(
            existing.as_ref().map(|identity| &identity.auth),
            Some(AuthMethod::PrivateKey {
                save_passphrase: true,
                ..
            })
        );
        let owner = existing
            .as_ref()
            .map(|identity| SecretOwner::Identity(identity.id));
        let state = IdentityEditState {
            base: existing.as_ref().map(|identity| identity.id),
            name: text_input(
                window,
                cx,
                &existing
                    .as_ref()
                    .map(|i| i.name.clone())
                    .unwrap_or_default(),
                "",
            ),
            username: text_input(
                window,
                cx,
                &existing
                    .as_ref()
                    .map(|i| i.username.clone())
                    .unwrap_or_default(),
                "",
            ),
            auth,
            key_path: text_input(window, cx, &key_path, ""),
            password: text_input(
                window,
                cx,
                "",
                &rust_i18n::t!("identity.password_placeholder"),
            ),
            passphrase: text_input(
                window,
                cx,
                "",
                &rust_i18n::t!("identity.passphrase_placeholder"),
            ),
            save_password,
            save_passphrase,
            error: None,
        };
        self.settings_modal.identity_edit = Some(state);
        if let Some(owner) = owner {
            self.prefill_identity_secrets(owner, window, cx);
        }
        cx.notify();
    }

    /// 读取模板已存口令并回填；钥匙串走 D-Bus 可能挂起秒级，放后台执行。
    fn prefill_identity_secrets(
        &mut self,
        owner: SecretOwner,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self.settings_modal.identity_edit.as_ref() else {
            return;
        };
        let fields = [
            (SecretKind::Password, edit.password.clone()),
            (SecretKind::KeyPassphrase, edit.passphrase.clone()),
        ];
        let secret_store = self.services.secret_store.clone();
        let executor = cx.background_executor().clone();
        let read_fields = fields.clone();
        window
            .spawn(cx, async move |cx| {
                let secrets = executor
                    .spawn(async move {
                        read_fields
                            .iter()
                            .map(|(kind, _)| {
                                secret_store
                                    .get(&SecretKey::new(owner, *kind))
                                    .ok()
                                    .flatten()
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;
                let _ = cx.update(|window, cx| {
                    for ((_, field), secret) in fields.into_iter().zip(secrets) {
                        if let Some(secret) = secret {
                            field.update(cx, |state, cx| {
                                state.set_value(secret.as_str(), window, cx);
                            });
                        }
                    }
                });
            })
            .detach();
    }

    pub(crate) fn cancel_identity_edit(&mut self, cx: &mut Context<Self>) {
        self.settings_modal.identity_edit = None;
        cx.notify();
    }

    /// 保存模板：写目录并后台落口令。
    pub(crate) fn save_identity(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.settings_modal.identity_edit.as_ref() else {
            return;
        };
        // 先把输入取值绑成局部字符串：结构体借用它们活到构造完成，
        // 直接在字面量里临时取值会在语句结束时先被释放。
        let name = edit.name.read(cx).value().to_string();
        let username = edit.username.read(cx).value().to_string();
        let key_path = edit.key_path.read(cx).value().to_string();
        let password = edit.password.read(cx).value().to_string();
        let passphrase = edit.passphrase.read(cx).value().to_string();
        let values = IdentityForm {
            name: &name,
            username: &username,
            auth: edit.auth,
            key_path: &key_path,
            save_password: edit.save_password,
            save_passphrase: edit.save_passphrase,
            password: &password,
            passphrase: &passphrase,
            base: edit.base,
        };
        let built = build_identity(&values);
        let built = match built {
            Ok(built) => built,
            Err(error) => {
                // 校验错误留在表单里而不是全局错误栏：用户正在填这张表。
                if let Some(edit) = self.settings_modal.identity_edit.as_mut() {
                    edit.error = Some(error);
                }
                cx.notify();
                return;
            }
        };

        let identity = built.identity;
        let replaced = match self
            .catalog
            .identities
            .iter_mut()
            .find(|existing| existing.id == identity.id)
        {
            Some(existing) => {
                *existing = identity.clone();
                true
            }
            None => false,
        };
        if !replaced {
            self.catalog.identities.push(identity);
        }
        self.settings_modal.identity_edit = None;
        // 模板身份变了，引用它的会话显示的用户名与认证方式要跟着刷新。
        self.refresh_tab_sessions();
        self.persist_catalog();
        self.status = Some(
            rust_i18n::t!(if replaced {
                "identity.updated"
            } else {
                "identity.created"
            })
            .to_string(),
        );

        let writes = built.writes;
        if !writes.is_empty() {
            let secret_store = self.services.secret_store.clone();
            // 钥匙串写入走 D-Bus 可能挂起秒级：后台执行，失败回填错误栏。
            cx.spawn(async move |this, cx| {
                let error = cx
                    .background_executor()
                    .spawn(async move {
                        let mut last = None;
                        for (key, value) in &writes {
                            let result = match value {
                                Some(value) => secret_store.set(key, value.as_str()),
                                None => secret_store.delete(key),
                            };
                            if let Err(error) = result {
                                last = Some(error);
                            }
                        }
                        last
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if let Some(error) = error {
                        this.error = Some(localized_error(&error));
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    /// 删除模板；仍被会话引用时拒绝并说明引用数，成功后清理其口令槽位。
    pub(crate) fn delete_identity(&mut self, id: IdentityId, cx: &mut Context<Self>) {
        let name = self
            .catalog
            .identity(id)
            .map(|identity| identity.name.clone())
            .unwrap_or_default();
        if let Err(error) = self.catalog.delete_identity(id) {
            self.error = Some(
                AppError::with_detail(ErrorCode::Configuration, error.to_string()).to_string(),
            );
            cx.notify();
            return;
        }
        if self
            .settings_modal
            .identity_edit
            .as_ref()
            .is_some_and(|edit| edit.base == Some(id))
        {
            self.settings_modal.identity_edit = None;
        }
        self.status = Some(rust_i18n::t!("identity.deleted", name = name).to_string());
        self.persist_catalog();

        // 目录里的模板没了，两种可能的口令槽位都要清掉，否则留下无人认领的口令。
        let secret_store = self.services.secret_store.clone();
        cx.spawn(async move |this, cx| {
            let error = cx
                .background_executor()
                .spawn(async move {
                    let mut last = None;
                    for kind in [SecretKind::Password, SecretKind::KeyPassphrase] {
                        if let Err(error) =
                            secret_store.delete(&SecretKey::new(SecretOwner::Identity(id), kind))
                        {
                            last = Some(error);
                        }
                    }
                    last
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(error) = error {
                    this.error = Some(localized_error(&error));
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// 凭据模板设置组渲染。
    pub(crate) fn render_settings_identities(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let identities = self.catalog.identities.clone();
        let edit = self.settings_modal.identity_edit.clone();

        let mut list = v_flex().gap_1();
        if identities.is_empty() {
            list = list.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("identity.empty").to_string()),
            );
        }
        for identity in identities {
            let id = identity.id;
            let users = self.catalog.identity_users(id);
            let row = h_flex()
                .id(ElementId::Name(format!("identity-row-{id}").into()))
                .px_2()
                .py_1()
                .gap_2()
                .items_center()
                .border_b_1()
                .border_color(p.border.opacity(0.5))
                .child(Icon::new(IconName::KeyRound).with_size(px(14.0)))
                .child(div().flex_1().text_sm().child(format!(
                    "{} · {}",
                    identity.name,
                    describe_auth(&identity.auth)
                )))
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("identity.used_by", count = users).to_string()),
                )
                .child(
                    Button::new(ElementId::Name(format!("identity-edit-{id}").into()))
                        .ghost()
                        .xsmall()
                        .label(rust_i18n::t!("common.edit").to_string())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_identity_editor(Some(id), window, cx);
                        })),
                )
                .child(
                    Button::new(ElementId::Name(format!("identity-del-{id}").into()))
                        .ghost()
                        .danger()
                        .xsmall()
                        .label(rust_i18n::t!("common.delete").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.delete_identity(id, cx);
                        })),
                );
            list = list.child(row);
        }

        let new_button = Button::new("identity-new")
            .small()
            .label(rust_i18n::t!("identity.new").to_string())
            .on_click(cx.listener(|this, _, window, cx| {
                this.open_identity_editor(None, window, cx);
            }));

        let body = v_flex().gap_2().child(list).child(new_button);
        match edit {
            Some(edit) => v_flex()
                .gap_3()
                .child(body)
                .child(self.render_identity_editor(edit, p, cx))
                .into_any_element(),
            None => body.into_any_element(),
        }
    }

    fn render_identity_editor(
        &mut self,
        edit: IdentityEditState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let auth = edit.auth;
        let chip = |label: String, choice: AuthChoice, key: &'static str| {
            Button::new(ElementId::Name(key.into()))
                .small()
                .label(label)
                .selected(auth == choice)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(edit) = this.settings_modal.identity_edit.as_mut() {
                        edit.auth = choice;
                    }
                    cx.notify();
                }))
        };
        let mut content = v_flex()
            .gap_2()
            .p_3()
            .rounded(p.radius.md)
            .border_1()
            .border_color(p.border)
            .child(Self::form_field(
                rust_i18n::t!("identity.name").to_string(),
                edit.name.clone(),
                p,
            ))
            .child(Self::form_field(
                rust_i18n::t!("identity.username").to_string(),
                edit.username.clone(),
                p,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(chip(
                        rust_i18n::t!("identity.auth_password").to_string(),
                        AuthChoice::Password,
                        "identity-auth-password",
                    ))
                    .child(chip(
                        rust_i18n::t!("identity.auth_key").to_string(),
                        AuthChoice::PrivateKey,
                        "identity-auth-key",
                    ))
                    .child(chip(
                        rust_i18n::t!("identity.auth_agent").to_string(),
                        AuthChoice::Agent,
                        "identity-auth-agent",
                    )),
            );

        match auth {
            AuthChoice::Password => {
                content = content
                    .child(Self::secret_field(
                        rust_i18n::t!("identity.password").to_string(),
                        edit.password.clone(),
                        p,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .child(rust_i18n::t!("identity.save_password").to_string()),
                            )
                            .child(
                                Switch::new("identity-toggle-save-password")
                                    .small()
                                    .checked(edit.save_password)
                                    .on_click(cx.listener(|this, &checked, _, cx| {
                                        if let Some(edit) =
                                            this.settings_modal.identity_edit.as_mut()
                                        {
                                            edit.save_password = checked;
                                        }
                                        cx.notify();
                                    })),
                            ),
                    );
            }
            AuthChoice::PrivateKey => {
                content = content
                    .child(Self::form_field(
                        rust_i18n::t!("identity.key_path").to_string(),
                        edit.key_path.clone(),
                        p,
                    ))
                    .child(Self::secret_field(
                        rust_i18n::t!("identity.passphrase").to_string(),
                        edit.passphrase.clone(),
                        p,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .child(rust_i18n::t!("identity.save_passphrase").to_string()),
                            )
                            .child(
                                Switch::new("identity-toggle-save-passphrase")
                                    .small()
                                    .checked(edit.save_passphrase)
                                    .on_click(cx.listener(|this, &checked, _, cx| {
                                        if let Some(edit) =
                                            this.settings_modal.identity_edit.as_mut()
                                        {
                                            edit.save_passphrase = checked;
                                        }
                                        cx.notify();
                                    })),
                            ),
                    );
            }
            AuthChoice::Agent => {}
        }

        if let Some(error) = edit.error.clone() {
            content = content.child(div().text_sm().text_color(p.danger).child(error));
        }
        content
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("identity-cancel")
                            .ghost()
                            .label(rust_i18n::t!("common.cancel").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cancel_identity_edit(cx);
                            })),
                    )
                    .child(
                        Button::new("identity-save")
                            .primary()
                            .label(rust_i18n::t!("common.save").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_identity(cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}
#[cfg(test)]
mod tests {
    // 不用 `use super::*`：本模块 glob 导入了 gpui，而 gpui 里有同名属性宏
    // `test`（gpui::test），glob 进来会把标准 `#[test]` 劫持成它，导致宏递归。
    use super::{
        AuthMethod, BuiltIdentity, IdentityForm, IdentityId, SecretKind, SecretOwner,
        build_identity,
    };
    use crate::session_form::AuthChoice;
    use std::path::Path;

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
}
