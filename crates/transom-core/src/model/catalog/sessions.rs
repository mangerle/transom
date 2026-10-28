//! 会话模型与凭据模板在目录中的映射、移动与 ProxyJump 链路解析。

use super::{CatalogError, MAX_PROXY_JUMP_DEPTH, SessionCatalog};
use crate::error::{AppError, ErrorCode};
use crate::ids::{FolderId, SessionId};
use crate::model::session::{Connection, Session};
use std::collections::HashSet;

impl SessionCatalog {
    /// 将会话移动到指定文件夹（`None` 表示移到根层级），并排到该层级末尾。
    ///
    /// # Errors
    /// 会话或目标文件夹不存在时返回错误。
    pub fn move_session(
        &mut self,
        session_id: SessionId,
        folder: Option<FolderId>,
    ) -> Result<(), CatalogError> {
        if let Some(folder_id) = folder
            && self.folder(folder_id).is_none()
        {
            return Err(CatalogError::FolderNotFound(folder_id));
        }
        let index = self
            .sessions
            .iter()
            .position(|session| session.id == session_id)
            .ok_or(CatalogError::SessionNotFound(session_id))?;
        let mut session = self.sessions.remove(index);
        session.folder_id = folder;
        self.sessions.push(session);
        Ok(())
    }

    /// 按 ID 查找凭据模板。
    pub fn identity(
        &self,
        id: crate::ids::IdentityId,
    ) -> Option<&crate::model::identity::Identity> {
        self.identities.iter().find(|identity| identity.id == id)
    }

    /// 仍引用该模板的会话数；删除前的闸门，也是界面提示的口径来源。
    pub fn identity_users(&self, id: crate::ids::IdentityId) -> usize {
        self.sessions
            .iter()
            .filter(|session| session.identity_id == Some(id))
            .count()
    }

    /// 删除凭据模板；仍被引用时拒绝并返回引用数。
    pub fn delete_identity(&mut self, id: crate::ids::IdentityId) -> Result<(), CatalogError> {
        if !self.identities.iter().any(|identity| identity.id == id) {
            return Err(CatalogError::IdentityNotFound(id));
        }
        let users = self.identity_users(id);
        if users > 0 {
            return Err(CatalogError::IdentityInUse(users));
        }
        self.identities.retain(|identity| identity.id != id);
        Ok(())
    }

    /// 把文件夹继承属性与引用模板的用户名与认证方式叠加到会话副本上。
    ///
    /// 继承优先级：凭据模板 > 会话显式配置 > 文件夹链条继承。
    #[must_use]
    pub fn resolved_session(&self, session: &Session) -> Session {
        let mut resolved = session.clone();
        if let Some(folder_id) = session.folder_id {
            let defaults = self.inherited_folder_defaults(folder_id);
            if let Connection::Ssh(ssh) = &mut resolved.connection {
                if ssh.port == 22
                    && let Some(port) = defaults.port
                {
                    ssh.port = port;
                }
                if ssh.username.trim().is_empty()
                    && let Some(username) = &defaults.username
                {
                    ssh.username = username.clone();
                }
                if let crate::model::session::AuthMethod::PrivateKey { path, .. } = &mut ssh.auth
                    && path.as_os_str().is_empty()
                    && let Some(key_path) = &defaults.key_path
                {
                    *path = key_path.clone();
                }
            }
        }
        let Some(Some(identity)) = session.identity_id.map(|id| self.identity(id)) else {
            return resolved;
        };
        if let Connection::Ssh(ssh) = &mut resolved.connection {
            ssh.username = identity.username.clone();
            ssh.auth = identity.auth.clone();
        }
        resolved
    }

    /// 解析 ProxyJump 链路，返回从最外层跳板机到最内层跳板机的有序会话副本。
    ///
    /// # Errors
    /// 引用的跳板会话不存在、指向非 SSH 会话、超过层数上限或存在环形依赖时返回错误。
    pub fn proxy_jump_chain(&self, session: &Session) -> Result<Vec<Session>, AppError> {
        let mut chain = Vec::new();
        let Connection::Ssh(ssh) = &session.connection else {
            return Ok(chain);
        };
        let mut visited: HashSet<SessionId> = HashSet::with_capacity(4);
        visited.insert(session.id);
        let mut cursor = ssh.proxy_jump.as_ref().map(|jump| jump.jump_session_id);
        while let Some(id) = cursor {
            if !visited.insert(id) {
                return Err(AppError::with_detail(
                    ErrorCode::Configuration,
                    format!(
                        "跳板机链路存在环形依赖，会话: {}，重复引用: {id}",
                        session.name
                    ),
                ));
            }
            if chain.len() >= MAX_PROXY_JUMP_DEPTH {
                return Err(AppError::with_detail(
                    ErrorCode::Configuration,
                    format!(
                        "跳板机链路超过 {MAX_PROXY_JUMP_DEPTH} 层上限，会话: {}",
                        session.name
                    ),
                ));
            }
            let Some(hop) = self.sessions.iter().find(|candidate| candidate.id == id) else {
                return Err(AppError::with_detail(
                    ErrorCode::Configuration,
                    format!(
                        "引用的跳板机会话已不存在，会话: {}，跳板机: {id}",
                        session.name
                    ),
                ));
            };
            let Connection::Ssh(hop_ssh) = &hop.connection else {
                return Err(AppError::with_detail(
                    ErrorCode::Configuration,
                    format!(
                        "跳板机会话不是 SSH 连接，会话: {}，跳板机: {}",
                        session.name, hop.name
                    ),
                ));
            };
            chain.push(self.resolved_session(hop));
            cursor = hop_ssh.proxy_jump.as_ref().map(|jump| jump.jump_session_id);
        }
        chain.reverse();
        Ok(chain)
    }
}
