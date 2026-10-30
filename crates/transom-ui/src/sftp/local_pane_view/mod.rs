//! SFTP 本地文件浏览器左栏视图渲染。
//!
//! 设计原理（Why）：
//! - 实现初衷：为本地文件系统操作提供响应式虚拟文件列表与多选、拖拽支持；
//! - 核心优势：
//!   1. 支持 Shift/Ctrl 连续多选与主项标记；
//!   2. 支持悬浮快速预览；
//!   3. 支持作为原生拖拽源（跨远端上传）。

mod context_menu;
mod panel_header;
mod toolbar;

use self::context_menu::{
    LocalContextMenuProps, build_local_blank_context_menu, build_local_context_menu,
};
use self::toolbar::{LocalToolbarProps, render_local_toolbar};
use super::row_interaction::{SftpRowContext, SftpRowPath, SftpSide};
use super::side_remote::SftpDragItem;
use super::{LocalFileEntry, LocalQuickPaths, SftpTransferRequest};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::base::v_flex;
use gpui_kit::component::input::InputState;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{VirtualListScrollHandle, v_virtual_list};
use gpui_kit::*;
use transom_core::TabId;
use transom_core::TransferDirection;

pub(crate) struct SftpLocalPaneProps {
    pub(crate) tab_id: TabId,
    pub(crate) show_filter: bool,
    pub(crate) show_hidden: bool,
    pub(crate) local_path_input: Entity<InputState>,
    pub(crate) local_filter_input: Entity<InputState>,
    pub(crate) local_entries: Vec<LocalFileEntry>,
    pub(crate) local_scroll_handle: VirtualListScrollHandle,
    /// 主目录/桌面/下载等快捷路径；目录不存在时对应按钮禁用。
    pub(crate) quick_paths: LocalQuickPaths,
    /// 本地终端模式没有远端，上传入口必须隐藏。
    pub(crate) remote_available: bool,
}

impl AppShell {
    pub(crate) fn render_sftp_local_pane(
        &mut self,
        props: SftpLocalPaneProps,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab_id = props.tab_id;
        let show_filter = props.show_filter;
        let show_hidden = props.show_hidden;
        let local_path_input = props.local_path_input;
        let local_filter_input = props.local_filter_input;
        let local_entries = props.local_entries;
        let local_scroll_handle = props.local_scroll_handle;
        let quick_paths = props.quick_paths;
        let remote_available = props.remote_available;

        let local_content = if local_entries.is_empty() {
            div()
                .p_3()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("sftp.empty").to_string())
                .into_any_element()
        } else {
            let local_sizes = std::rc::Rc::new(vec![size(px(0.), px(28.)); local_entries.len()]);
            let visible_paths: std::rc::Rc<Vec<SftpRowPath>> = std::rc::Rc::new(
                local_entries
                    .iter()
                    .map(|entry| SftpRowPath::Local(entry.path.clone()))
                    .collect(),
            );
            let local_entries_rc = std::rc::Rc::new(local_entries);
            let p = p.clone();
            let app_entity = cx.entity().clone();
            v_virtual_list(
                cx.entity().clone(),
                format!("sftp-local-virtual-list-{tab_id}"),
                local_sizes,
                move |this, range, _window, cx| {
                    let (local_selection, local_primary) = this
                        .workspace
                        .tab(tab_id)
                        .and_then(|tab| tab.sftp())
                        .map(|panel| (panel.local.selection.clone(), panel.local.selected.clone()))
                        .unwrap_or_default();
                    let app_entity = app_entity.clone();
                    range
                        .filter_map(|ix| {
                            let entry = local_entries_rc.get(ix)?;
                            let entry_name = entry.name.clone();
                            let is_dir = entry.is_dir;
                            let is_parent_entry = entry_name == "..";
                            let is_selected = local_selection.contains(&entry.path)
                                || local_primary.as_ref() == Some(&entry.path);
                            let entry_path_cm = entry.path.clone();
                            let app_cm = app_entity.clone();
                            let row = this.sftp_file_row(
                                SftpRowContext {
                                    tab_id,
                                    side: SftpSide::Local,
                                    index: ix,
                                    path: SftpRowPath::Local(entry.path.clone()),
                                    name: entry.name.clone(),
                                    is_dir,
                                    is_parent: is_parent_entry,
                                    size: entry.size,
                                    permissions: None,
                                    selected: is_selected,
                                    visible_paths: visible_paths.clone(),
                                    source_connection_id: None,
                                },
                                &p,
                                cx,
                            );

                            let row = row.context_menu(move |menu, _window, _cx| {
                                build_local_context_menu(
                                    menu,
                                    app_cm.clone(),
                                    LocalContextMenuProps {
                                        tab_id,
                                        entry_path: entry_path_cm.clone(),
                                        is_parent_entry,
                                        remote_available,
                                    },
                                )
                            });
                            Some(row.into_any_element())
                        })
                        .collect()
                },
            )
            .track_scroll(&local_scroll_handle)
            .into_any_element()
        };

        // 对比入口只在"选中单个本地文件且远端有同名文件"时可用；
        // 目录、多选与远端缺失都置灰，避免点击后才发现没有对手方。
        let diff_ready = remote_available && self.diff_entry_ready(tab_id);
        let toolbar = render_local_toolbar(
            LocalToolbarProps {
                tab_id,
                show_filter,
                show_hidden,
                local_path_input: &local_path_input,
                local_filter_input: &local_filter_input,
                quick_paths: &quick_paths,
                remote_available,
                diff_ready,
            },
            p,
            cx,
        );

        let app_local_drop = cx.entity().clone();
        let app_blank_menu = cx.entity().clone();
        let drop_bg = p.sidebar_accent.opacity(0.3);
        let drop_border = p.primary;
        let drop_container = v_flex()
            .id(ElementId::Name("sftp-local-list".into()))
            .relative()
            .flex_1()
            .overflow_hidden()
            .p_2()
            .drag_over::<SftpDragItem>(move |style, _, _, _| {
                style.bg(drop_bg).border_2().border_color(drop_border)
            })
            .on_drop(move |drag: &SftpDragItem, _window, cx| {
                let item = drag.clone();
                app_local_drop.update(cx, |this, cx| {
                    let Some(source_connection_id) = item.source_connection_id else {
                        return;
                    };
                    let Some(local_dir) = this
                        .sftp_panel_mut_tab(tab_id)
                        .map(|panel| panel.local.path.clone())
                    else {
                        return;
                    };
                    for path in &item.paths {
                        let fname = path.rsplit('/').next().unwrap_or(path);
                        let target_local = match transom_core::safe_local_join(&local_dir, fname) {
                            Ok(path) => path,
                            Err(_) => {
                                if let Some(panel) = this.sftp_panel_mut_tab(tab_id) {
                                    panel.set_local_error(
                                        rust_i18n::t!("sftp.invalid_name").to_string(),
                                    );
                                }
                                cx.notify();
                                continue;
                            }
                        };
                        this.sftp_enqueue_transfer(
                            SftpTransferRequest {
                                tab_id,
                                connection_id: source_connection_id,
                                direction: TransferDirection::Download,
                                is_dir: item.is_dir,
                                local_path: target_local,
                                remote_path: path.clone(),
                            },
                            cx,
                        );
                    }
                });
            })
            .child(local_content)
            .vertical_scrollbar(&local_scroll_handle)
            // 空白区右键给目录级动作：行元素上的菜单命中时优先并停止冒泡，不会互抢。
            .context_menu(move |menu, _window, _cx| {
                build_local_blank_context_menu(menu, app_blank_menu.clone(), tab_id)
            });

        v_flex()
            .size_full()
            .overflow_hidden()
            .child(toolbar)
            .child(drop_container)
            .into_any_element()
    }
}
