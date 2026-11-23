//! RDP 图形管线与 FastPath 帧分片组装适配。
//!
//! 设计原理（Why）：RDP 协议的图形更新采用分片与表面命令机制。当网络传输
//! 发生分片时，必须在边界处进行完整包重组后方可送入解码器。

use ironrdp_pdu::geometry::InclusiveRectangle;

/// 图像帧更新指令。
#[derive(Debug, Clone)]
pub struct GraphicsFrameEvent {
    /// 桌面总像素宽度。
    pub width: u16,
    /// 桌面总像素高度。
    pub height: u16,
    /// 发生更新的矩形损伤区域集合。
    pub dirty_rectangles: Vec<InclusiveRectangle>,
}

/// 图形流状态跟踪。
#[derive(Default)]
pub struct GraphicsPipeline {
    current_width: u16,
    current_height: u16,
    pending_dirty: Vec<InclusiveRectangle>,
}

impl GraphicsPipeline {
    /// 构造新的图形管线适配器。
    pub fn new(initial_width: u16, initial_height: u16) -> Self {
        Self {
            current_width: initial_width,
            current_height: initial_height,
            pending_dirty: Vec::with_capacity(32),
        }
    }

    /// 重设视口分辨率。
    pub fn resize(&mut self, width: u16, height: u16) {
        self.current_width = width;
        self.current_height = height;
        self.pending_dirty.clear();
        // 分辨率改变后全屏标记为脏
        if width > 0 && height > 0 {
            self.pending_dirty.push(InclusiveRectangle {
                left: 0,
                top: 0,
                right: width - 1,
                bottom: height - 1,
            });
        }
    }

    /// 添加局部损伤矩形。
    pub fn record_damage(&mut self, rect: InclusiveRectangle) {
        self.pending_dirty.push(rect);
    }

    /// 产生一个提交帧事件，并清空当前损伤累积。
    pub fn commit_frame(&mut self) -> Option<GraphicsFrameEvent> {
        if self.pending_dirty.is_empty() {
            return None;
        }

        let dirty = std::mem::take(&mut self.pending_dirty);
        Some(GraphicsFrameEvent {
            width: self.current_width,
            height: self.current_height,
            dirty_rectangles: dirty,
        })
    }
}
