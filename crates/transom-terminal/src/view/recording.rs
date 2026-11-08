//! 终端录制控制：开始、停止与句柄查询。
use super::*;

impl TerminalView {
    /// 获取录制器句柄。
    pub fn recorder(&self) -> &RecorderHandle {
        &self.recorder
    }

    /// 查询当前窗格是否正在录制。
    pub fn is_recording(&self) -> bool {
        self.recorder.is_recording()
    }

    /// 开始录制当前终端会话到 `.cast` 文件。
    pub fn start_recording(
        &mut self,
        path: std::path::PathBuf,
        title: Option<String>,
    ) -> std::io::Result<()> {
        let term = self.state.engine.lock();
        let cols = term.term().grid().columns();
        let rows = term.term().grid().screen_lines();
        drop(term);
        self.recorder.start(path, cols as u16, rows as u16, title)
    }

    /// 停止录制并返回文件路径。
    pub fn stop_recording(&mut self) -> Option<std::path::PathBuf> {
        self.recorder.stop()
    }
}
