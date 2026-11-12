//! SFTP 远程内容区：空态提示与虚拟列表行（选择、双击、右键与拖拽）。
//!
//! 设计原理（Why）：行渲染与交互从主面板抽出后，主函数只负责布局编排；
//! 上下文菜单与行操作分别下沉至子模块，参数以对象模式显式收敛。

mod context_menu;

pub(super) use self::context_menu::build_remote_blank_context_menu;
use self::context_menu::{RemoteContextMenuProps, build_remote_context_menu};
use super::row_interaction::{SftpRowContext, SftpRowPath, SftpSide};
use crate::palette::Palette;
use crate::sftp::SftpRemotePaneRef;
use crate::sftp::entity::SftpPanelView;
use crate::theme::tokens::{rem_px, size as token_size};
use gpui_kit::base::v_flex;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{VirtualListScrollHandle, v_virtual_list};
use gpui_kit::*;
use transom_core::{ConnectionId, RemoteFile, TabId};
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

/// 远程内容区的渲染输入：条目、加载态、目标列与滚动句柄。
pub(crate) struct RemoteContentProps {
    pub(crate) remote_entries: Vec<RemoteFile>,
    pub(crate) loading: bool,
    /// 列级错误：并列远端列的读取/操作错误只落在本列。
    pub(crate) error: Option<String>,
    pub(crate) tab_id: TabId,
    pub(crate) rem_size: Pixels,
    pub(crate) target: SftpRemotePaneRef,
    pub(crate) connection_id: ConnectionId,
    pub(crate) remote_scroll_handle: VirtualListScrollHandle,
    /// 渲染所需的选中集快照：虚拟列表回调在实体租借期间执行，不能再读实体。
    pub(crate) remote_selection: std::collections::BTreeSet<String>,
    pub(crate) remote_primary: Option<String>,
}

/// 构建远端虚拟列表所需的参数集合。
struct RemoteListParams {
    remote_entries: Vec<RemoteFile>,
    tab_id: TabId,
    rem_size: Pixels,
    target: SftpRemotePaneRef,
    connection_id: ConnectionId,
    remote_scroll_handle: VirtualListScrollHandle,
    remote_selection: std::collections::BTreeSet<String>,
    remote_primary: Option<String>,
}

impl SftpPanelView {
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
            error,
            tab_id,
            rem_size,
            target,
            connection_id,
            remote_scroll_handle,
            remote_selection,
            remote_primary,
        } = props;

        let content = if remote_entries.is_empty() {
            remote_placeholder(loading, p)
        } else {
            self.build_remote_virtual_list(
                RemoteListParams {
                    remote_entries: remote_entries.clone(),
                    tab_id,
                    rem_size,
                    target,
                    connection_id,
                    remote_scroll_handle,
                    remote_selection,
                    remote_primary,
                },
                p,
                cx,
            )
        };

        // 加载态视觉过渡：已有条目上覆盖轻量半透明遮罩与微型动态 Spinner，消除假死顿挫感
        let content = if loading && !remote_entries.is_empty() {
            div()
                .relative()
                .size_full()
                .child(content)
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(p.background.opacity(0.45))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(Spinner::new().color(p.primary))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.foreground)
                                .child(rust_i18n::t!("sftp.loading").to_string()),
                        ),
                )
                .into_any_element()
        } else {
            content
        };

        // 列级错误：保留已有列表可继续操作，只在上方补一条提示条。
        if let Some(error) = error {
            v_flex()
                .size_full()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(p.danger)
                        .child(error),
                )
                .child(div().flex_1().overflow_hidden().child(content))
                .into_any_element()
        } else {
            content
        }
    }

    /// 构建远程文件虚拟滚动列表。
    fn build_remote_virtual_list(
        &self,
        params: RemoteListParams,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let RemoteListParams {
            remote_entries,
            tab_id,
            rem_size,
            target,
            connection_id,
            remote_scroll_handle,
            remote_selection,
            remote_primary,
        } = params;
        let row_height = rem_px(token_size::FILE_ROW_HEIGHT, rem_size);
        let remote_sizes = std::rc::Rc::new(vec![size(px(0.), row_height); remote_entries.len()]);
        let visible_paths: std::rc::Rc<Vec<SftpRowPath>> = std::rc::Rc::new(
            remote_entries
                .iter()
                .map(|entry| SftpRowPath::Remote(entry.path.clone()))
                .collect(),
        );
        let remote_entries_rc = std::rc::Rc::new(remote_entries);
        // 右键菜单只需要"回到自己"的句柄：直接取当前实体，不读外壳状态。
        let menu_app = cx.entity().clone();
        let remote_selection = std::rc::Rc::new(remote_selection);
        let remote_primary = std::rc::Rc::new(remote_primary);
        let p = p.clone();
        let has_main_remote = self.state.main_connection.is_some();

        v_virtual_list(
            cx.entity().clone(),
            format!("sftp-remote-virtual-list-{tab_id}-{target:?}"),
            remote_sizes,
            move |this, range, _window, cx| {
                range
                    .filter_map(|ix| {
                        let entry = remote_entries_rc.get(ix)?;
                        let entry_name = entry.name.clone();
                        let entry_is_dir = entry.is_dir;
                        let is_parent_entry = entry_name == "..";
                        let is_selected = remote_selection.contains(&entry.path)
                            || remote_primary.as_deref() == Some(entry.path.as_str());
                        let entry_path_cm = entry.path.clone();
                        let app_cm = menu_app.clone();
                        let row = this.file_row(
                            SftpRowContext {
                                tab_id,
                                side: SftpSide::Remote(target),
                                index: ix,
                                path: SftpRowPath::Remote(entry.path.clone()),
                                name: entry.name.clone(),
                                is_dir: entry_is_dir,
                                is_parent: is_parent_entry,
                                size: entry.size,
                                permissions: entry.permissions,
                                selected: is_selected,
                                visible_paths: visible_paths.clone(),
                                source_connection_id: Some(connection_id),
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
                                    target,
                                    entry_path: entry_path_cm.clone(),
                                    is_dir: entry_is_dir,
                                    is_parent_entry,
                                    has_main_remote,
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

/// 空目录或加载中的占位内容。
fn remote_placeholder(loading: bool, p: &Palette) -> AnyElement {
    if !loading {
        return crate::widgets::empty_state(
            AppIcon::Ui(UiIcon::LIST_FILES),
            rust_i18n::t!("sftp.empty").to_string(),
            rust_i18n::t!("sftp.empty_hint").to_string(),
            p,
        );
    }
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new().color(p.primary))
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("sftp.loading").to_string()),
        )
        .into_any_element()
}
