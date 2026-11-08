//! 鼠标命中测试：屏幕单元格与终端网格坐标。
use super::*;

impl TerminalView {
    pub(super) fn hit_test_screen_cell(&self, position: Point<Pixels>) -> Option<(usize, usize)> {
        let metrics = *self.state.metrics.lock();
        if metrics.cols == 0
            || metrics.rows == 0
            || metrics.cell_width <= px(0.0)
            || metrics.cell_height <= px(0.0)
        {
            return None;
        }
        let x = position.x - metrics.origin.x;
        let y = position.y - metrics.origin.y;
        if x < px(0.0) || y < px(0.0) {
            return None;
        }
        let col = (f32::from(x) / f32::from(metrics.cell_width)).floor() as usize;
        let row = (f32::from(y) / f32::from(metrics.cell_height)).floor() as usize;
        if col >= metrics.cols || row >= metrics.rows {
            return None;
        }
        Some((row, col))
    }

    pub(super) fn hit_test_grid_point(&self, position: Point<Pixels>) -> Option<AlacPoint> {
        let (row, col) = self.hit_test_screen_cell(position)?;
        // 折叠模式下可见行与网格行不再一一对应：命中必须查行映射，
        // 否则选择会落到被隐藏的输出区或错位的行上。
        if let Some(layout) = self.state.block_layout.lock().clone() {
            return layout
                .line_at(row)
                .map(|line| AlacPoint::new(line, Column(col)));
        }
        let offset = self.state.engine.lock().display_offset() as i32;
        Some(AlacPoint::new(Line(row as i32 - offset), Column(col)))
    }
}
