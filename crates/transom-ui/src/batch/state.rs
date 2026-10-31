//! 批量编辑表单状态与属性应用。

use super::common::*;

pub(crate) struct BatchEditForm {
    pub(crate) selected_count: usize,
    pub(crate) change_port: bool,
    pub(crate) port: Entity<InputState>,
    pub(crate) change_username: bool,
    pub(crate) username: Entity<InputState>,
    pub(crate) change_auth: bool,
    pub(crate) auth: AuthChoice,
    pub(crate) key_path: Entity<InputState>,
    pub(crate) save_passphrase: bool,
    pub(crate) error: Option<String>,
}

impl BatchEditForm {
    /// 表单全部文本项的当前值，用作脏判定基线。
    ///
    /// 只比文本：勾选框（改端口/改用户名等）即便被用户改过也不拦取消——
    /// 批量修改只有点保存才会落盘，误按 Esc 不产生任何数据损失。
    pub(crate) fn text_values(&self, cx: &App) -> Vec<String> {
        [&self.port, &self.username, &self.key_path]
            .into_iter()
            .map(|input| input.read(cx).value().to_string())
            .collect()
    }
}

/// 批量更新值；`None` 表示该属性保持不变。
pub(super) struct BatchUpdates {
    pub(super) port: Option<u16>,
    pub(super) username: Option<String>,
    pub(super) auth: Option<AuthMethod>,
}

/// 将批量更新应用到选中的 SSH 会话，返回（已更新，已跳过）数量。
///
/// 本地与串口会话没有端口/用户名/SSH 认证概念，统一跳过而不是静默修改。
pub(super) fn apply_batch_updates(
    catalog: &mut SessionCatalog,
    selected: &HashSet<SessionId>,
    updates: &BatchUpdates,
) -> (usize, usize) {
    let mut updated = 0;
    let mut skipped = 0;
    for session in &mut catalog.sessions {
        if !selected.contains(&session.id) {
            continue;
        }
        let Connection::Ssh(ssh) = &mut session.connection else {
            skipped += 1;
            continue;
        };
        if let Some(port) = updates.port {
            ssh.port = port;
        }
        if let Some(username) = &updates.username {
            ssh.username = username.clone();
        }
        if let Some(auth) = &updates.auth {
            ssh.auth = auth.clone();
        }
        updated += 1;
    }
    (updated, skipped)
}
