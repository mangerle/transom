//! SFTP 文件属性查看：远端异步 stat 请求与本地同步属性读取。

use crate::app::AppShell;
use chrono::{Local, TimeZone};
use gpui_kit::*;
use std::path::Path;
use transom_core::{RemoteMetadata, SftpOperation, TabId};

impl AppShell {
    /// 请求查看远端路径的详细属性（异步读取，结果经 `AppEvent::SftpMetadata` 回传）
    pub(crate) fn sftp_request_remote_properties(
        &mut self,
        tab_id: TabId,
        path: String,
        cx: &mut Context<Self>,
    ) {
        self.send_sftp_op(tab_id, SftpOperation::Stat { path }, cx);
    }

    /// 读取并直接展示本地路径属性。
    ///
    /// 设计原理（Why）：本地元数据读取为微秒级同步系统调用，远小于一次渲染帧预算，
    /// 无需绕行后台命令通道与跨线程事件，直接在 UI 线程读取可避免额外状态机开销。
    pub(crate) fn sftp_show_local_properties(
        &mut self,
        tab_id: TabId,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        match local_file_properties(&path) {
            Ok(properties) => {
                self.sftp_modals.properties = Some(SftpPropertiesModalState {
                    scope: "本地".to_string(),
                    properties,
                });
            }
            Err(err) => {
                if let Some(panel) = self
                    .workspace
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == tab_id)
                    .and_then(|t| t.sftp_mut())
                {
                    panel.local_error = Some(format!("读取本地文件属性失败: {err}"));
                }
            }
        }
        cx.notify();
    }
}

/// 通用文件属性快照（远端与本地共用）。
///
/// 设计原理（Why）：
/// - 实现初衷：远端与本地属性来源不同（SFTP lstat / 本地 lstat），但展示结构一致，
///   统一为同一快照类型可让弹窗渲染逻辑只维护一份；
/// - 核心优势：所有差异化字段均为 `Option`，缺失时展示层跳过对应行，
///   不用为不同来源编写分支渲染；
/// - 代价与局限：快照为纯展示数据，不保留原始元数据，如需二次操作（如 chmod）
///   必须重新读取。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FileProperties {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) kind: String,
    pub(crate) size: Option<u64>,
    pub(crate) permissions: Option<u32>,
    pub(crate) owner: Option<String>,
    pub(crate) group: Option<String>,
    pub(crate) modified: Option<u32>,
    pub(crate) accessed: Option<u32>,
    pub(crate) link_target: Option<String>,
}

/// 文件属性查看弹窗状态（只读展示，无输入控件）。
pub(crate) struct SftpPropertiesModalState {
    /// 属性来源描述（「远端」/「本地」），用于弹窗副标题
    pub(crate) scope: String,
    pub(crate) properties: FileProperties,
}

/// 由远端属性契约转换为展示层属性快照
pub(crate) fn remote_file_properties(metadata: &RemoteMetadata) -> FileProperties {
    let name = metadata
        .path
        .trim_end_matches('/')
        .rsplit_once('/')
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| metadata.path.clone());
    FileProperties {
        name,
        path: metadata.path.clone(),
        kind: file_kind_text(metadata.is_symlink, metadata.is_dir).to_string(),
        size: if metadata.is_dir {
            None
        } else {
            Some(metadata.size)
        },
        permissions: metadata.permissions,
        owner: identity_text(metadata.user.as_deref(), metadata.uid),
        group: identity_text(metadata.group.as_deref(), metadata.gid),
        modified: metadata.mtime,
        accessed: metadata.atime,
        link_target: metadata.link_target.clone(),
    }
}

/// 读取本地文件系统属性快照（lstat 语义，不跟随符号链接）
pub(crate) fn local_file_properties(path: &Path) -> Result<FileProperties, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    let is_symlink = metadata.file_type().is_symlink();
    let link_target = if is_symlink {
        std::fs::read_link(path)
            .ok()
            .map(|target| target.display().to_string())
    } else {
        None
    };
    #[cfg(unix)]
    let (owner, group) = {
        use std::os::unix::fs::MetadataExt;
        (
            identity_text(
                local_user_name(metadata.uid()).as_deref(),
                Some(metadata.uid()),
            ),
            identity_text(
                local_group_name(metadata.gid()).as_deref(),
                Some(metadata.gid()),
            ),
        )
    };
    #[cfg(not(unix))]
    let (owner, group): (Option<String>, Option<String>) = (None, None);
    Ok(FileProperties {
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string()),
        path: path.display().to_string(),
        kind: file_kind_text(is_symlink, metadata.is_dir()).to_string(),
        size: if metadata.is_dir() {
            None
        } else {
            Some(metadata.len())
        },
        permissions: local_permissions(&metadata),
        owner,
        group,
        modified: unix_seconds(metadata.modified().ok()),
        accessed: unix_seconds(metadata.accessed().ok()),
        link_target,
    })
}

/// 文件类型的中文描述（符号链接优先于目录/文件判定）
fn file_kind_text(is_symlink: bool, is_dir: bool) -> &'static str {
    if is_symlink {
        "符号链接"
    } else if is_dir {
        "文件夹"
    } else {
        "文件"
    }
}

/// 拼接身份展示文本：名称与数字 ID 同时可用时一并展示，只有其一则单独展示
fn identity_text(name: Option<&str>, id: Option<u32>) -> Option<String> {
    match (name, id) {
        (Some(name), Some(id)) => Some(format!("{name} ({id})")),
        (Some(name), None) => Some(name.to_string()),
        (None, Some(id)) => Some(id.to_string()),
        (None, None) => None,
    }
}

/// 将 Unix 秒级时间戳格式化为本地时区 `YYYY-MM-DD HH:MM:SS`。
///
/// 设计原理（Why）：属性弹窗的时间需要与用户在服务器上 `ls -l` 看到的
/// 本地时间对齐（而非 UTC），故借助已在依赖树内的 chrono 做本地时区转换。
pub(crate) fn format_unix_time(seconds: u32) -> String {
    match Local.timestamp_opt(i64::from(seconds), 0).single() {
        Some(datetime) => datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => format!("{seconds} (Unix 时间戳)"),
    }
}

/// 将系统时间转换为 Unix 秒（超出 u32 可表示范围时饱和截断）
fn unix_seconds(time: Option<std::time::SystemTime>) -> Option<u32> {
    let seconds = time?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(seconds.min(u64::from(u32::MAX)) as u32)
}

#[cfg(unix)]
fn local_permissions(metadata: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode())
}

#[cfg(not(unix))]
fn local_permissions(_metadata: &std::fs::Metadata) -> Option<u32> {
    None
}

/// 从 `/etc/passwd` 或 `/etc/group` 内容中解析指定 ID 对应的名称。
///
/// 设计原理（Why）：本地属性展示需要把 uid/gid 映射为可读名称；在不引入
/// libc 依赖的前提下，直接解析系统账号文件是最轻量且可单测验证的方式。
fn parse_id_name(content: &str, id: u32) -> Option<String> {
    for line in content.lines() {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let _password_placeholder = fields.next();
        if fields.next().and_then(|field| field.parse::<u32>().ok()) == Some(id) {
            return Some(name.to_string());
        }
    }
    None
}

#[cfg(unix)]
fn local_user_name(uid: u32) -> Option<String> {
    let content = std::fs::read_to_string("/etc/passwd").ok()?;
    parse_id_name(&content, uid)
}

#[cfg(unix)]
fn local_group_name(gid: u32) -> Option<String> {
    let content = std::fs::read_to_string("/etc/group").ok()?;
    parse_id_name(&content, gid)
}

#[cfg(test)]
mod tests {
    use super::{
        file_kind_text, format_unix_time, identity_text, parse_id_name, remote_file_properties,
    };
    use transom_core::RemoteMetadata;

    #[test]
    fn test_parse_id_name() {
        let passwd = "root:x:0:0:root:/root:/bin/bash\nalice:x:1000:1000::/home/alice:/bin/sh\n";
        assert_eq!(parse_id_name(passwd, 0).as_deref(), Some("root"));
        assert_eq!(parse_id_name(passwd, 1000).as_deref(), Some("alice"));
        assert_eq!(parse_id_name(passwd, 42), None);
        assert_eq!(parse_id_name("", 0), None);
        assert_eq!(parse_id_name("broken-line", 0), None);
    }

    #[test]
    fn test_identity_text() {
        assert_eq!(
            identity_text(Some("root"), Some(0)).as_deref(),
            Some("root (0)")
        );
        assert_eq!(identity_text(Some("root"), None).as_deref(), Some("root"));
        assert_eq!(identity_text(None, Some(1000)).as_deref(), Some("1000"));
        assert_eq!(identity_text(None, None), None);
    }

    #[test]
    fn test_file_kind_text() {
        assert_eq!(file_kind_text(true, true), "符号链接");
        assert_eq!(file_kind_text(true, false), "符号链接");
        assert_eq!(file_kind_text(false, true), "文件夹");
        assert_eq!(file_kind_text(false, false), "文件");
    }

    #[test]
    fn test_format_unix_time_shape() {
        let text = format_unix_time(0);
        assert_eq!(text.len(), 19);
        assert_eq!(&text[4..5], "-");
        assert_eq!(&text[7..8], "-");
        assert_eq!(&text[10..11], " ");
        assert_eq!(&text[13..14], ":");
        assert_eq!(&text[16..17], ":");
    }

    #[test]
    fn test_remote_file_properties_name_and_size() {
        let metadata = RemoteMetadata {
            path: "/var/log/app.log".to_string(),
            is_dir: false,
            is_symlink: false,
            size: 2048,
            uid: Some(0),
            user: Some("root".to_string()),
            gid: Some(0),
            group: None,
            permissions: Some(0o100644),
            atime: Some(1),
            mtime: Some(2),
            link_target: None,
        };
        let properties = remote_file_properties(&metadata);
        assert_eq!(properties.name, "app.log");
        assert_eq!(properties.kind, "文件");
        assert_eq!(properties.size, Some(2048));
        assert_eq!(properties.owner.as_deref(), Some("root (0)"));
        assert_eq!(properties.group.as_deref(), Some("0"));
    }
}
