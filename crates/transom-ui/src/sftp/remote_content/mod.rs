//! SFTP 远程内容区：空态提示与虚拟列表行（选择、双击、右键与拖拽）。
//!
//! 设计原理（Why）：行渲染与交互从主面板抽出后，主函数只负责布局编排；
//! 上下文菜单与行操作分别下沉至子模块，参数以对象模式显式收敛。

mod context_menu;

use self::context_menu::{RemoteContextMenuProps, build_remote_context_menu};
use super::row_interaction::{SftpRowContext, SftpRowPath, SftpSide};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::{VirtualListScrollHandle, v_virtual_list};
use gpui_kit::*;
use transom_core::{ConnectionId, RemoteFile, TabId};

/// 远程内容区的渲染输入：条目、加载态、标签归属与滚动句柄。
pub(crate) struct RemoteContentProps {
    pub(crate) remote_entries: Vec<RemoteFile>,
    pub(crate) loading: bool,
    pub(crate) tab_id: TabId,
    pub(crate) tab_conn_id: Option<ConnectionId>,
    pub(crate) remote_scroll_handle: VirtualListScrollHandle,
}

impl AppShell {
    /// 远程文件内容区：空态提示或虚拟列表（行内承载选择、双击、右键与拖拽）。
    pub(crate) fn render_remote_content(
        &self,
        props: RemoteContentProps,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let RemoteContentProps {
            remote_entries,
            loading,
            tab_id,
            tab_conn_id,
            remote_scroll_handle,
        } = props;
        if remote_entries.is_empty() && !loading {
            div()
                .p_3()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("sftp.empty").to_string())
                .into_any_element()
        } else if remote_entries.is_empty() && loading {
            div()
                .p_3()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("sftp.loading").to_string())
                .into_any_element()
        } else {
            let remote_sizes = std::rc::Rc::new(vec![size(px(0.), px(28.)); remote_entries.len()]);
            let visible_paths: std::rc::Rc<Vec<SftpRowPath>> = std::rc::Rc::new(
                remote_entries
                    .iter()
                    .map(|entry| SftpRowPath::Remote(entry.path.clone()))
                    .collect(),
            );
            let remote_entries_rc = std::rc::Rc::new(remote_entries);
            let p = p.clone();
            let app_entity = cx.entity().clone();
            v_virtual_list(
                cx.entity().clone(),
                format!("sftp-remote-virtual-list-{tab_id}"),
                remote_sizes,
                move |this, range, _window, cx| {
                    let (remote_selection, remote_primary) = this
                        .workspace
                        .tab(tab_id)
                        .and_then(|tab| tab.sftp())
                        .map(|panel| {
                            (
                                panel.remote.selection.clone(),
                                panel.remote.selected.clone(),
                            )
                        })
                        .unwrap_or_default();
                    let app_entity = app_entity.clone();
                    range
                        .filter_map(|ix| {
                            let entry = remote_entries_rc.get(ix)?;
                            let entry_name = entry.name.clone();
                            let entry_is_dir = entry.is_dir;
                            let is_parent_entry = entry_name == "..";
                            let is_selected = remote_selection.contains(&entry.path)
                                || remote_primary.as_ref() == Some(&entry.path);
                            let entry_path_cm = entry.path.clone();
                            let app_cm = app_entity.clone();
                            let row = this.sftp_file_row(
                                SftpRowContext {
                                    tab_id,
                                    side: SftpSide::Remote,
                                    index: ix,
                                    path: SftpRowPath::Remote(entry.path.clone()),
                                    name: entry.name.clone(),
                                    is_dir: entry_is_dir,
                                    is_parent: is_parent_entry,
                                    size: entry.size,
                                    permissions: entry.permissions,
                                    selected: is_selected,
                                    visible_paths: visible_paths.clone(),
                                    source_connection_id: tab_conn_id,
                                },
                                &p,
                                cx,
                            );

                            let row = row.context_menu(move |menu, _window, _cx| {
                                build_remote_context_menu(
                                    menu,
                                    RemoteContextMenuProps {
                                        app: app_cm.clone(),
                                        tab_id,
                                        entry_path: entry_path_cm.clone(),
                                        is_dir: entry_is_dir,
                                        is_parent_entry,
                                    },
                                )
                            });
                            Some(row.into_any_element())
                        })
                        .collect()
                },
            )
            .track_scroll(&remote_scroll_handle)
            .into_any_element()
        }
    }
}
