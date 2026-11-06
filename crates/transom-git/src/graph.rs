//! 提交图的列（lane）分配。
//!
//! 设计原理（Why）：
//! - 界面要画的是 IntelliJ 那种「每行一个提交、竖向若干条泳道」的图，而 git
//!   自己的 `--graph` 输出是 ASCII 折线，无法直接映射到矢量绘制；因此这里
//!   只做一件事——把「提交 → 父提交」的关系折算成行号与列号；
//! - 输入必须是拓扑序（见 `log.rs` 的 `--topo-order`），否则父提交可能还没
//!   出现就被分配列，图会画错；
//! - [`GraphState`] 可跨分页携带：日志是分页追加的，若每页从零开始算列，
//!   页面交界处会出现凭空消失或凭空出现的泳道。携带状态后下一页接得上。
//!
//! 代价与局限：列数等于同时活跃的分支数，极端分支拓扑下会撑得很宽，
//! 界面按可视宽度裁剪并显示省略，不做自动合并。

use crate::types::CommitEntry;

/// 泳道分配的跨页状态。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphState {
    /// 每一列上「下一个应当出现在这里的提交号」；`None` 表示该列空闲。
    lanes: Vec<Option<String>>,
}

impl GraphState {
    /// 当前活跃列数（去掉尾部空闲列后的宽度）。
    pub fn width(&self) -> usize {
        self.lanes.len()
    }
}

/// 一行提交在图上的位置与连线。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow {
    /// 本行提交圆点所在列。
    pub lane: usize,
    /// 各父提交将被放到的列，顺序与 [`CommitEntry::parents`] 一致。
    pub parents: Vec<usize>,
    /// 本行穿过但不属于本提交的列（其他分支的竖向连线）。
    pub crossing: Vec<usize>,
}

/// 为一页提交计算图行，并就地更新泳道状态。
///
/// # Panics
/// 不会 panic：全部索引访问都经由 `position` 得到的合法下标，
/// 新列追加后立即取 `len() - 1`，`lanes` 非空前提由代码结构保证。
pub fn assign_lanes(state: &mut GraphState, entries: &[CommitEntry]) -> Vec<GraphRow> {
    let mut rows = Vec::with_capacity(entries.len());
    for entry in entries {
        let lane = match take_lane(&state.lanes, &entry.oid) {
            Some(existing) => existing,
            None => {
                state.lanes.push(Some(entry.oid.clone()));
                state.lanes.len() - 1
            }
        };
        // 先记录穿过本行的其他列，再把本行腾空给父提交。
        let crossing = state
            .lanes
            .iter()
            .enumerate()
            .filter(|(index, occupant)| occupant.is_some() && *index != lane)
            .map(|(index, _)| index)
            .collect();
        state.lanes[lane] = None;
        let parents = entry
            .parents
            .iter()
            .map(|parent| occupy_parent(&mut state.lanes, parent))
            .collect();
        rows.push(GraphRow {
            lane,
            parents,
            crossing,
        });
    }
    compact(state);
    rows
}

/// 找到某个提交已经占用的列。
fn take_lane(lanes: &[Option<String>], oid: &str) -> Option<usize> {
    lanes.iter().position(|lane| lane.as_deref() == Some(oid))
}

/// 为父提交占列：已被别的子提交预约则复用，否则就近取空列或追加。
fn occupy_parent(lanes: &mut Vec<Option<String>>, oid: &str) -> usize {
    if let Some(existing) = take_lane(lanes, oid) {
        return existing;
    }
    match lanes.iter().position(Option::is_none) {
        Some(free) => {
            lanes[free] = Some(oid.to_string());
            free
        }
        None => {
            lanes.push(Some(oid.to_string()));
            lanes.len() - 1
        }
    }
}

/// 收回尾部的空闲列，避免只增不减。
fn compact(state: &mut GraphState) {
    while matches!(state.lanes.last(), Some(None)) {
        state.lanes.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::{GraphState, assign_lanes};
    use crate::types::CommitEntry;

    fn commit(oid: &str, parents: &[&str]) -> CommitEntry {
        CommitEntry {
            oid: oid.to_string(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author_name: String::new(),
            author_email: String::new(),
            author_time: 0,
            subject: String::new(),
            body: String::new(),
            refs: Vec::new(),
        }
    }

    #[test]
    fn linear_history_stays_in_one_column() {
        let entries = [
            commit("c3", &["c2"]),
            commit("c2", &["c1"]),
            commit("c1", &[]),
        ];
        let mut state = GraphState::default();
        let rows = assign_lanes(&mut state, &entries);
        assert_eq!(
            rows.iter().map(|row| row.lane).collect::<Vec<_>>(),
            vec![0, 0, 0]
        );
        assert_eq!(state.width(), 0, "线性历史结束后不残留泳道");
    }

    #[test]
    fn merge_reuses_the_waiting_branch_column() {
        // c3 在 1 列等待被合并；合并提交 m 把 0 列与 1 列连起来。
        let entries = [
            commit("m", &["c2", "c3"]),
            commit("c2", &["c1"]),
            commit("c3", &["c1"]),
            commit("c1", &[]),
        ];
        let mut state = GraphState::default();
        let rows = assign_lanes(&mut state, &entries);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(
            rows[0].parents,
            vec![0, 1],
            "第一父留在 0 列，分支父占 1 列"
        );
        assert_eq!(rows[2].lane, 1, "分支提交应落在被预约的 1 列");
        assert_eq!(rows[1].crossing, vec![1], "c2 这一行有分支连线穿过");
    }

    #[test]
    fn state_carries_across_pages() {
        let mut state = GraphState::default();
        let first_page = [commit("c3", &["c2"])];
        assign_lanes(&mut state, &first_page);
        let second_page = [commit("c2", &["c1"]), commit("c1", &[])];
        let rows = assign_lanes(&mut state, &second_page);
        assert_eq!(rows[0].lane, 0, "续页应接上前一页的列");
        assert_eq!(state.width(), 0);
    }
}
