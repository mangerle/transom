//! 提交图的自绘：节点菱形与连线曲线。
//!
//! 设计原理（Why）：
//! - 用 `canvas` 直接画路径，而不是「圆点 div + 竖线 div」的拼装：拼装画不出
//!   跨列的斜连线与合并弧线，分支一多就退化成看不出父子关系的竖线阵，而
//!   IDEA 的图好读靠的正是这些连线；
//! - 每行一张画布（高度固定一行）：整页一张画布要自己算全表偏移与裁剪，
//!   在滚动容器里还会整块重绘；按行切分后与虚拟列表天然兼容。代价是跨行
//!   连线只能在行边界处相接，因此所有连线都严格从本行的上下边缘进出；
//! - 入线取「上一行声明的父列」、出线取「本行声明的父列」：这两者由
//!   `transom_git::assign_lanes` 保证在同一列对齐，画布不参与列分配计算；
//! - 路径坐标在 paint 回调里用 `bounds.origin` 现算：gpui 的 `paint_path`
//!   收视口绝对坐标，构建期拿不到 bounds，事后平移路径顶点既啰嗦又易错。

// 设计令牌例外：提交图 canvas 的列宽、行高、节点半径与线宽属于绘制几何，保持物理像素。
use gpui_kit::*;
use transom_git::GraphRow;

/// 单列宽度与行高（px），与日志表的行尺寸保持一致。
pub(crate) const LANE_WIDTH: f32 = 16.0;
pub(crate) const ROW_HEIGHT: f32 = 28.0;
/// 节点菱形的外接半径（px）。
const NODE_RADIUS: f32 = 4.5;
/// 连线粗细（px）。
const EDGE_WIDTH: f32 = 1.6;

/// 一行的图形态：入线列、节点列、出线列、穿过列。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct GraphCell {
    incoming: Vec<usize>,
    lane: usize,
    outgoing: Vec<usize>,
    crossing: Vec<usize>,
}

impl GraphCell {
    /// 从泳道结果里取出某一行需要的形态。
    pub(crate) fn at(rows: &[GraphRow], index: usize) -> Self {
        let Some(row) = rows.get(index) else {
            return Self::default();
        };
        Self {
            incoming: if index == 0 {
                Vec::new()
            } else {
                rows.get(index - 1)
                    .map(|previous| previous.parents.clone())
                    .unwrap_or_default()
            },
            lane: row.lane,
            outgoing: row.parents.clone(),
            crossing: row.crossing.clone(),
        }
    }
}

/// 画一行的提交图。
pub(crate) fn paint(cell: GraphCell, lane_count: usize, node: Hsla, edge: Hsla) -> AnyElement {
    let width = px(LANE_WIDTH * lane_count.max(1) as f32);
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window: &mut Window, _| {
            let at = |lane: usize, y: f32| {
                point(
                    bounds.origin.x + px(LANE_WIDTH * lane as f32 + LANE_WIDTH / 2.0),
                    bounds.origin.y + px(y),
                )
            };
            let mid = ROW_HEIGHT / 2.0;
            let mut edges = PathBuilder::stroke(px(EDGE_WIDTH));
            // 穿过本行的其他分支：整高竖线。
            for lane in &cell.crossing {
                edges.move_to(at(*lane, 0.0));
                edges.line_to(at(*lane, ROW_HEIGHT));
            }
            // 入线：上一行的父列接到本行节点；同列走直线，跨列走二次曲线。
            for from in &cell.incoming {
                edges.move_to(at(*from, 0.0));
                if *from == cell.lane {
                    edges.line_to(at(cell.lane, mid));
                } else {
                    edges.curve_to(at(cell.lane, mid), at(*from, mid));
                }
            }
            // 出线：本行节点接到下一行的父列。
            for to in &cell.outgoing {
                edges.move_to(at(cell.lane, mid));
                if *to == cell.lane {
                    edges.line_to(at(*to, ROW_HEIGHT));
                } else {
                    edges.curve_to(
                        at(*to, ROW_HEIGHT),
                        at(cell.lane, mid + (ROW_HEIGHT - mid) / 2.0),
                    );
                }
            }
            if let Ok(path) = edges.build() {
                window.paint_path(path, edge);
            }

            let center_x = LANE_WIDTH * cell.lane as f32 + LANE_WIDTH / 2.0;
            let mut diamond = PathBuilder::fill();
            diamond.add_polygon(
                &[
                    point(
                        bounds.origin.x + px(center_x),
                        bounds.origin.y + px(mid - NODE_RADIUS),
                    ),
                    point(
                        bounds.origin.x + px(center_x + NODE_RADIUS),
                        bounds.origin.y + px(mid),
                    ),
                    point(
                        bounds.origin.x + px(center_x),
                        bounds.origin.y + px(mid + NODE_RADIUS),
                    ),
                    point(
                        bounds.origin.x + px(center_x - NODE_RADIUS),
                        bounds.origin.y + px(mid),
                    ),
                ],
                true,
            );
            if let Ok(path) = diamond.build() {
                window.paint_path(path, node);
            }
        },
    )
    .w(width)
    .h(px(ROW_HEIGHT))
    .flex_none()
    .into_any_element()
}
