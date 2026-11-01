use super::file_row::{insert_parent_local_entry, insert_parent_remote_entry};
use super::filter::filter_named_entries;
use super::{LocalFileEntry, SftpRemotePaneState};
use crate::app::AppState;
use transom_core::RemoteFile;

impl AppState {
    /// 过滤单个远端列的条目：隐藏开关、排除规则与列内过滤词共同生效。
    pub(crate) fn filter_remote_entries(
        pane: &SftpRemotePaneState,
        show_hidden: bool,
        filter_query: &str,
        hidden_patterns: &str,
    ) -> Vec<RemoteFile> {
        let mut remote_entries = filter_named_entries(
            &pane.entries,
            |entry| entry.name.as_str(),
            show_hidden,
            filter_query,
            hidden_patterns,
        );
        insert_parent_remote_entry(&mut remote_entries, &pane.path, filter_query);
        remote_entries
    }

    /// 过滤本地列条目：共享面板与本地单栏面板共用同一套规则与父目录注入。
    pub(crate) fn filter_local_entries(
        panel: &super::SftpPanelState,
        show_hidden: bool,
        filter_query: &str,
        hidden_patterns: &str,
    ) -> Vec<LocalFileEntry> {
        let mut local_entries = filter_named_entries(
            &panel.local.entries,
            |entry| entry.name.as_str(),
            show_hidden,
            filter_query,
            hidden_patterns,
        );
        insert_parent_local_entry(&mut local_entries, &panel.local.path, filter_query);
        local_entries
    }
}
