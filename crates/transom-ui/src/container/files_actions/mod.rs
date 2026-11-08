//! 容器文件页的浏览、传输与就地编辑动作。
//!
//! 按用户意图拆分为 browse / transfer / prompt / edit / mutate 五个子模块，
//! 方法统一归属 [`ContainerWorkspace`]：文件页状态随标签页存活，动作只读写实体
//! 自己的状态与请求登记，不再经外壳转手。需要窗口才能创建的输入实体由回调把
//! `Window` 传入；外壳只在对话框渲染与渲染期窗口动作中参与。

mod browse;
mod edit;
mod mutate;
mod prompt;
mod transfer;

use super::*;

impl ContainerWorkspace {
    /// 当前文件页操作的远端目标：是否走数据卷接口，以及容器名或卷名。
    ///
    /// 容器与数据卷共用同一套文件页，区别只在命令族；集中判定一次，避免每个
    /// 动作各写一份 `section` 分支。
    fn file_target(&self) -> Option<(bool, String)> {
        let is_volume = self.state.section == ContainerSection::Volumes;
        let target = if is_volume {
            self.state.selected_volume_name()
        } else {
            self.state.selected_container_name()
        }?;
        Some((is_volume, target))
    }
}
