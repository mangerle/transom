//! 密钥向导与已知主机弹窗的状态定义。
//!
//! 设计原理（Why）：这些状态只随弹窗生死，渲染与动作在 keys 模块内完成；
//! 与外壳动作分离后，状态字段变更不会牵动外壳文件。
//!
//! 已知主机弹窗已独立为 [`super::KnownHostsView`] 实体：记录、搜索与删除
//! 动作都归实体；密钥向导仍以状态结构由 `ModalHub` 持有，待后续迁移。

use super::keygen::{KeyAlgorithm, PublicKeyInfo};
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{ConnectionId, HostKeyRecord, TabId};

/// 已知主机指纹管理弹窗：记录快照与搜索框。
pub(super) struct KnownHostsModalState {
    pub(super) open: bool,
    pub(super) records: Vec<HostKeyRecord>,
    pub(super) search: Entity<InputState>,
}

/// 可部署公钥的已连接 SSH 会话。
#[derive(Debug, Clone)]
pub(crate) struct KeyTarget {
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) title: String,
}

#[derive(Clone)]
pub(crate) struct KeyWizardState {
    pub(crate) ssh_dir: PathBuf,
    pub(crate) algorithm: KeyAlgorithm,
    pub(crate) comment: Entity<InputState>,
    pub(crate) path: Entity<InputState>,
    pub(crate) passphrase: Entity<InputState>,
    pub(crate) keys: Vec<PublicKeyInfo>,
    pub(crate) selected: Option<usize>,
    pub(crate) generating: bool,
    pub(crate) message: Option<String>,
    pub(crate) error: Option<String>,
}

impl KeyWizardState {
    pub(super) fn new(window: &mut Window, cx: &mut Context<impl Sized>, ssh_dir: PathBuf) -> Self {
        let default_path = ssh_dir.join(KeyAlgorithm::Ed25519.default_file_name());
        let default_path = default_path.display().to_string();
        let comment_placeholder = rust_i18n::t!("keys.comment_placeholder").to_string();
        let comment_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(comment_placeholder));
        let path_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(default_path)
                .placeholder("~/.ssh/id_ed25519")
        });
        let passphrase_placeholder = rust_i18n::t!("keys.passphrase_placeholder").to_string();
        let passphrase_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(passphrase_placeholder)
        });
        Self {
            ssh_dir,
            algorithm: KeyAlgorithm::Ed25519,
            comment: comment_input,
            path: path_input,
            passphrase: passphrase_input,
            keys: Vec::new(),
            selected: None,
            generating: false,
            message: None,
            error: None,
        }
    }
}
