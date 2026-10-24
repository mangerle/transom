//! 会话树行模型、拖放落点与拖放处理。
//!
//! 行元素拼装见 `render` 子模块，扁平化行构建见 `rows` 子模块。

mod render;
mod rows;

pub(crate) use rows::build_rows;

use crate::app::AppShell;
use gpui_kit::*;
use transom_core::{CatalogError, DropSide, FolderId, Session, SessionId};

/// 行槽位高度：虚拟列表按它分配空间，渲染时必须严格一致，否则行会错位或裁切。
const ROW_HEIGHT: Pixels = px(38.0);
/// 内层高亮条高度：小于行槽位，留出上下间隙，同时把背景限制在内容区，缩进区保持透明。
const BAND_HEIGHT: Pixels = px(34.0);
/// 缩进基准与每级步长（逻辑像素）。
const INDENT_BASE: f32 = 6.0;
const INDENT_STEP: f32 = 18.0;
/// 行内图标尺寸：与 34px 内容条匹配，默认的 14px 图标在这个高度下显得偏弱。
const ROW_ICON_SIZE: Pixels = px(18.0);
/// 折叠箭头尺寸：比行图标小一档，避免与文件夹图标抢视觉重心。
const ROW_CHEVRON_SIZE: Pixels = px(14.0);
/// 行背景深浅梯度：常态最轻，选中居中，悬浮与活动会话改用实心强调色。
///
/// 深色主题下侧边栏底色与强调色本身就很接近（`#0a0a0a` 与 `#262626`），
/// 再对强调色叠低透明度只会得到几乎不可辨的色差，因此悬浮态不再走透明度梯度，
/// 而是直接铺满实色，与 SFTP 文件行的悬浮表现保持一致。
const TINT_IDLE: f32 = 0.14;
const TINT_SELECTED: f32 = 0.42;
/// 文件夹行整体比会话行更重一档，用于区分层级容器。
const TINT_FOLDER_IDLE: f32 = 0.22;
/// 拖拽悬停在文件夹行中段时的“将放入此分组”提示底色。
const TINT_FOLDER_INTO: f32 = 0.6;

/// 按层级计算行左内边距，文件夹与会话共用，保证同层对齐。
fn indent_padding(depth: usize) -> Pixels {
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
    pub(crate) fn height(&self) -> Pixels {
        // 文件夹与会话保持统一行高；内层内容条再压缩，兼顾两行信息与紧凑度。
        ROW_HEIGHT
    }
}

impl AppShell {
    /// 折叠或展开文件夹，并持久化收纳状态。
    pub(crate) fn toggle_folder_collapsed(&mut self, id: FolderId, cx: &mut Context<Self>) {
        let collapsed = self.catalog.folder(id).map(|folder| !folder.collapsed);
        if let Some(collapsed) = collapsed {
            let _ = self.catalog.set_folder_collapsed(id, collapsed);
            self.persist_catalog();
        }
        cx.notify();
    }

    /// 处理会话/文件夹拖放：换父级或调整同级顺序。
    ///
    /// 目标文件夹被折叠时自动展开，保证拖入的条目立即可见；
    /// 非法移动（拖入自身子树）由领域层拒绝并显示中文原因。
    pub(crate) fn handle_sidebar_drop(
        &mut self,
        drag: SidebarDrag,
        target: SidebarTarget,
        cx: &mut Context<Self>,
    ) {
        match self.apply_sidebar_drop(drag, target) {
            Ok(landed) => {
                if let Some(folder_id) = landed
                    && let Some(folder) =
                        self.catalog.folders.iter_mut().find(|f| f.id == folder_id)
                {
                    folder.collapsed = false;
                }
                self.persist_catalog();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    /// 把落点翻译成领域操作，返回条目最终所属文件夹（`None` 表示根层级）。
    ///
    /// 跨类型落点按“能落进容器”解释：会话落在文件夹行上下沿即放入该文件夹，
    /// 文件夹落在会话行上下沿即放入该会话所在文件夹；两者都不需要用户二次操作。
    fn apply_sidebar_drop(
        &mut self,
        drag: SidebarDrag,
        target: SidebarTarget,
    ) -> Result<Option<FolderId>, CatalogError> {
        match (drag, target) {
            (SidebarDrag::Session(session_id), SidebarTarget::Root) => {
                self.catalog.move_session(session_id, None)?;
                Ok(None)
            }
            (SidebarDrag::Folder(folder_id), SidebarTarget::Root) => {
                self.catalog.move_folder(folder_id, None)?;
                Ok(None)
            }
            (SidebarDrag::Session(session_id), SidebarTarget::Folder(folder_id)) => {
                self.catalog.move_session(session_id, Some(folder_id))?;
                Ok(Some(folder_id))
            }
            (SidebarDrag::Folder(moved_id), SidebarTarget::Folder(parent_id)) => {
                if moved_id == parent_id {
                    return Ok(Some(parent_id));
                }
                self.catalog.move_folder(moved_id, Some(parent_id))?;
                Ok(Some(parent_id))
            }
            (drag, SidebarTarget::Relative { anchor, side }) => match (drag, anchor) {
                (SidebarDrag::Session(session_id), SidebarDrag::Session(anchor_id)) => {
                    self.catalog.reorder_session(session_id, anchor_id, side)?;
                    Ok(self
                        .catalog
                        .folder_of_session(anchor_id)
                        .map(|folder| folder.id))
                }
                (SidebarDrag::Folder(folder_id), SidebarDrag::Folder(anchor_id)) => {
                    self.catalog.reorder_folder(folder_id, anchor_id, side)?;
                    Ok(self
                        .catalog
                        .folder(folder_id)
                        .and_then(|folder| folder.parent))
                }
                (SidebarDrag::Session(session_id), SidebarDrag::Folder(anchor_id)) => {
                    self.catalog.move_session(session_id, Some(anchor_id))?;
                    Ok(Some(anchor_id))
                }
                (SidebarDrag::Folder(folder_id), SidebarDrag::Session(anchor_id)) => {
                    let parent = self
                        .catalog
                        .folder_of_session(anchor_id)
                        .map(|folder| folder.id);
                    self.catalog.move_folder(folder_id, parent)?;
                    Ok(parent)
                }
            },
        }
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
