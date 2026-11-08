//! 认证凭据的暂存与落盘。
//!
//! 弹窗体（SSH 鉴权问答与首连主机指纹确认）已经迁入
//! `connection/prompts_view.rs` 与 `connection/prompts_render.rs`；本文件只保留
//! “认证通过后才写钥匙串”的凭据生命周期：待保存意图、归属解析、成功后落盘与
//! 连接失败时丢弃。
//!
//! 设计原理（Why）：口令被拒绝或连接取消时不能留下半条凭据，否则下次连接会
//! 用一个已失效的旧口令反复失败。保存意图因此只暂存在内存里，必须等
//! `TerminalReady` 确认连接真正建立后才写入钥匙串。
//!
//! # 代价与局限
//! 待保存凭据同一时刻只有一条：新一次鉴权问答会作废旧意图，保留旧值可能把
//! 已被拒绝的口令写进钥匙串。

use super::secrets::enable_auth_save;
use crate::app::AppShell;
use crate::app::AppState;
use crate::palette::localized_error;
use gpui_kit::*;
use transom_core::{Connection, ConnectionId, ErrorCode, SecretKey, SessionId};

impl AppState {
    pub(crate) fn discard_pending_credential(&mut self, connection_id: ConnectionId) {
        let matched = self
            .modals
            .connection
            .pending_credential
            .as_ref()
            .is_some_and(|pending| pending.connection_id == connection_id);
        if matched {
            self.modals.connection.pending_credential = None;
        }
    }

    /// 会话口令归属：引用凭据模板时归模板。
    ///
    /// 外壳在消费 [`crate::connection::ConnectionPromptsEvent::CredentialToSave`]
    /// 时调用：实体只上报会话标识，不接触目录，归属解析必须先回到目录侧。
    pub(crate) fn secret_owner_for(&self, session_id: SessionId) -> transom_core::SecretOwner {
        self.catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .map_or(transom_core::SecretOwner::Session(session_id), |session| {
                session.secret_owner()
            })
    }

    /// 打开"凭据保存到钥匙串"标记，使目录记录与钥匙串实际状态一致。
    fn mark_credential_saved(&mut self, owner: transom_core::SecretOwner) {
        let flipped = match &owner {
            transom_core::SecretOwner::Session(session_id) => self
                .catalog
                .sessions
                .iter_mut()
                .find(|session| session.id == *session_id)
                .and_then(|session| match &mut session.connection {
                    Connection::Ssh(ssh) => Some(ssh),
                    _ => None,
                })
                .is_some_and(|ssh| enable_auth_save(&mut ssh.auth)),
            transom_core::SecretOwner::Identity(identity_id) => self
                .catalog
                .identities
                .iter_mut()
                .find(|identity| identity.id == *identity_id)
                .is_some_and(|identity| enable_auth_save(&mut identity.auth)),
            // 仓库凭据没有目录侧的"已保存"标记。
            transom_core::SecretOwner::Registry(_) => false,
        };
        if !flipped {
            return;
        }
        self.refresh_tab_sessions();
        self.persist_catalog();
    }
}

impl AppShell {
    /// 连接成功建立后，把用户勾选保存的凭据写入钥匙串。
    pub(crate) fn flush_pending_credential(
        &mut self,
        session_id: SessionId,
        connection_id: ConnectionId,
        cx: &mut Context<Self>,
    ) {
        let matched = self
            .state
            .modals
            .connection
            .pending_credential
            .as_ref()
            .is_some_and(|pending| {
                pending.session_id == session_id && pending.connection_id == connection_id
            });
        if !matched {
            return;
        }
        let Some(pending) = self.state.modals.connection.pending_credential.take() else {
            return;
        };
        let key = SecretKey::new(pending.owner.clone(), pending.kind);
        self.state.mark_credential_saved(pending.owner.clone());
        let secret_store = self.state.services.secret_store.clone();
        cx.spawn(async move |this, cx| {
            let error = cx
                .background_executor()
                .spawn(async move {
                    secret_store
                        .set(&key, pending.value.as_str())
                        .err()
                        .filter(|error| error.code != ErrorCode::SecretStoreUnavailable)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(error) = error {
                    this.state.set_error(localized_error(&error));
                }
                cx.notify();
            });
        })
        .detach();
    }
}
