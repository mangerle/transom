//! 会话录制与回放管理的数据模型与文件扫描工具。

use std::path::{Path, PathBuf};
use transom_core::TabId;
use transom_terminal::parse_cast_file;

/// 录制元数据条目。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RecordingEntry {
    pub file_name: String,
    pub path: PathBuf,
    pub title: String,
    pub width: u16,
    pub height: u16,
    pub duration_secs: f64,
    pub step_count: usize,
    pub modified_display: String,
}

/// 回放管理弹窗状态。
pub(crate) struct ReplayModalState {
    pub target_tab_id: TabId,
    pub entries: Vec<RecordingEntry>,
    pub selected_index: usize,
}

/// 扫描指定目录下全部 `.cast` 脚本。
pub(crate) fn scan_recordings(dir: &Path) -> Vec<RecordingEntry> {
    let mut entries = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return entries;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("cast") {
            continue;
        }

        let Ok((header, events)) = parse_cast_file(&path) else {
            continue;
        };

        let duration = events.last().map_or(0.0, |e| e.time);
        let step_count = events
            .iter()
            .filter(|e| e.event_type == transom_terminal::CastEventType::Input)
            .count();

        let modified_display = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|time| {
                let datetime: chrono::DateTime<chrono::Local> = time.into();
                datetime.format("%Y-%m-%d %H:%M").to_string()
            })
            .unwrap_or_default();

        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();

        entries.push(RecordingEntry {
            file_name,
            path,
            title: header.title.unwrap_or_default(),
            width: header.width,
            height: header.height,
            duration_secs: duration,
            step_count,
            modified_display,
        });
    }

    entries.sort_by(|a, b| b.file_name.cmp(&a.file_name));
    entries
}
