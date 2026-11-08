//! 密钥生成与管理向导：实体、渲染、外壳职责与部署目标查询。

mod entity;
mod keygen;
mod known_hosts_view;
mod paths;
mod render;
mod shell;
mod state;

pub(crate) use entity::{KeyWizardEvent, KeyWizardView};
pub(crate) use known_hosts_view::{KnownHostsEvent, KnownHostsView};
pub(crate) use paths::{default_ssh_dir, expand_home};

use crate::app::{AppState, TabStatus};
use transom_core::Connection;

impl AppState {
    pub(crate) fn key_targets(&self) -> Vec<state::KeyTarget> {
        self.workspace
            .tabs
            .iter()
            .filter(|tab| {
                matches!(tab.status, TabStatus::Connected)
                    && matches!(tab.session.connection, Connection::Ssh(_))
            })
            .filter_map(|tab| {
                Some(state::KeyTarget {
                    tab_id: tab.id,
                    connection_id: tab.connection_id?,
                    title: tab.display_name(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
