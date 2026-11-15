//! 会话树行模型、拖放落点与拖放处理。
//!
//! 行元素拼装见 `render` 子模块，扁平化行构建见 `rows` 子模块。

mod folder_row;
pub(crate) use folder_row::render_folder_row;
pub(crate) mod reorder;
pub(crate) use reorder::SidebarDragPreview;
mod rows;
mod session_row;
pub(crate) use session_row::{SessionRowRenderParams, render_session_row};

pub(crate) use rows::build_rows;

use crate::theme::tokens::size;
use gpui_kit::*;
use transom_core::{DropSide, FolderId, Session, SessionId};

/// 行槽位高度（rem）：虚拟列表按它分配空间，渲染时必须严格一致。
pub(crate) const ROW_HEIGHT_REM: f32 = size::TREE_ROW_HEIGHT;
/// 缩进基准与每级步长（逻辑像素）。
const INDENT_BASE: f32 = 6.0;
const INDENT_STEP: f32 = 18.0;
/// 行内图标尺寸：与内容条及活动栏图标匹配，随密度等比缩放。
pub(crate) const ROW_ICON_SIZE: Rems = rems(size::TREE_ROW_ICON);
/// 折叠箭头尺寸：比行图标小一档，避免与文件夹图标抢视觉重心。
pub(crate) const ROW_CHEVRON_SIZE: Rems = rems(size::TREE_CHEVRON);

/// 按层级计算行左内边距，文件夹与会话共用，保证同层对齐。
pub(crate) fn indent_padding(depth: usize) -> Pixels {
    px(INDENT_BASE + depth as f32 * INDENT_STEP)
}

/// 侧边栏拖动载荷：会话与文件夹共用一条拖放通道，同时充当落点锚点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SidebarDrag {
    Session(SessionId),
    Folder(FolderId),
}

/// 侧边栏拖放落点。
///
/// `Root`/`Folder` 表示放入容器末尾；`Relative` 表示插到某一行前半区或后半区
/// （会话行整行二分，文件夹行上下各三分之一、中段仍归放入分组），
/// 父级跟随锚点行，因此跨分组拖一次就完成了“移动 + 排序”，
/// 不需要用户先把条目移进目标分组再排第二次。
#[derive(Debug, Clone, Copy)]
pub(crate) enum SidebarTarget {
    Root,
    Folder(FolderId),
    Relative { anchor: SidebarDrag, side: DropSide },
}

#[derive(Clone)]
pub(crate) struct FolderRow {
    pub(crate) id: FolderId,
    pub(crate) name: String,
    pub(crate) depth: usize,
    pub(crate) collapsed: bool,
    pub(crate) session_count: usize,
    pub(crate) folder_count: usize,
}

#[derive(Clone)]
pub(crate) struct SessionRow {
    pub(crate) session: Box<Session>,
    pub(crate) depth: usize,
}

#[derive(Clone)]
pub(crate) enum SidebarRow {
    Folder(FolderRow),
    Session(SessionRow),
}

impl SidebarRow {
    pub(crate) fn height(&self, rem_size: Pixels) -> Pixels {
        // 文件夹与会话保持统一行高；内层内容条再压缩，兼顾两行信息与紧凑度。
        rems(ROW_HEIGHT_REM).to_pixels(rem_size)
    }
}

#[cfg(test)]
mod tests {
    use super::{SidebarRow, build_rows};
    use transom_core::{Connection, FolderId, LocalConnection, Session, SessionCatalog};

    fn local_session(name: &str, folder: Option<FolderId>) -> Session {
        let mut session = Session::new(name, Connection::Local(LocalConnection::default()));
        session.folder_id = folder;
        session
    }

    fn row_names(catalog: &SessionCatalog, query: &str) -> Vec<String> {
        build_rows(catalog, query)
            .into_iter()
            .map(|row| match row {
                SidebarRow::Folder(row) => format!("folder:{}", row.name),
                SidebarRow::Session(row) => format!("session:{}", row.session.name),
            })
            .collect()
    }

    /// 同级条目严格按目录数组顺序输出：一旦改回字典序，拖拽排序的结果会被静默覆盖。
    #[test]
    fn rows_follow_catalog_order_not_name_collation() {
        let mut catalog = SessionCatalog::default();
        let prod = catalog.ensure_folder_path("生产").unwrap();
        catalog.add_folder("Beta", None).unwrap();
        catalog.add_folder("Alpha", None).unwrap();
        catalog.sessions.push(local_session("zeta", Some(prod)));
        catalog.sessions.push(local_session("alpha", Some(prod)));

        assert_eq!(
            row_names(&catalog, ""),
            vec![
                "folder:生产",
                "session:zeta",
                "session:alpha",
                "folder:Beta",
                "folder:Alpha",
            ]
        );
    }

    #[test]
    fn nested_folders_render_depth_first_with_counts() {
        let mut catalog = SessionCatalog::default();
        let prod = catalog.ensure_folder_path("生产").unwrap();
        let db = catalog.ensure_folder_path("生产/数据库").unwrap();
        catalog.sessions.push(local_session("db-01", Some(db)));
        catalog.sessions.push(local_session("root", None));

        let rows = build_rows(&catalog, "");
        assert_eq!(
            row_names(&catalog, ""),
            vec![
                "folder:生产",
                "folder:数据库",
                "session:db-01",
                "session:root"
            ]
        );
        match &rows[0] {
            SidebarRow::Folder(row) => {
                assert_eq!(row.session_count, 1);
                assert_eq!(row.folder_count, 1);
            }
            _ => panic!("首行应为文件夹"),
        }
        match &rows[1] {
            SidebarRow::Folder(row) => {
                assert_eq!(row.depth, 1);
                assert_eq!(row.session_count, 1);
            }
            _ => panic!("第二行应为子文件夹"),
        }
        assert_eq!(prod, catalog.folders[0].id);
    }

    #[test]
    fn collapsed_folder_hides_descendants() {
        let mut catalog = SessionCatalog::default();
        let prod = catalog.ensure_folder_path("生产").unwrap();
        let db = catalog.ensure_folder_path("生产/数据库").unwrap();
        catalog.sessions.push(local_session("db-01", Some(db)));
        catalog.set_folder_collapsed(prod, true).unwrap();

        assert_eq!(row_names(&catalog, ""), vec!["folder:生产"]);
    }

    #[test]
    fn filter_keeps_ancestors_and_ignores_collapsed_state() {
        let mut catalog = SessionCatalog::default();
        let prod = catalog.ensure_folder_path("生产").unwrap();
        let db = catalog.ensure_folder_path("生产/数据库").unwrap();
        catalog.sessions.push(local_session("db-01", Some(db)));
        catalog.sessions.push(local_session("web-01", Some(prod)));
        catalog.set_folder_collapsed(prod, true).unwrap();

        assert_eq!(
            row_names(&catalog, "db-01"),
            vec!["folder:生产", "folder:数据库", "session:db-01"]
        );
    }

    #[test]
    fn folder_name_match_shows_entire_subtree() {
        let mut catalog = SessionCatalog::default();
        let prod = catalog.ensure_folder_path("生产").unwrap();
        let db = catalog.ensure_folder_path("生产/数据库").unwrap();
        catalog.sessions.push(local_session("db-01", Some(db)));
        catalog.sessions.push(local_session("web-01", Some(prod)));

        assert_eq!(
            row_names(&catalog, "生产"),
            vec![
                "folder:生产",
                "folder:数据库",
                "session:db-01",
                "session:web-01"
            ]
        );
    }
}
