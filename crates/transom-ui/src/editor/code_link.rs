//! 终端 OSC 8 代码位置链接的远端路径解析。

/// 绝对路径原样使用；相对路径以远端当前目录为基准拼接。
///
/// `..` 组件在终端侧已被拒绝，这里只需处理目录末尾斜杠与根目录拼接。
pub(in crate::editor) fn resolve_code_path(path: &str, remote_dir: Option<&str>) -> Option<String> {
    if path.starts_with('/') {
        return Some(path.to_string());
    }
    let dir = remote_dir?;
    if dir == "/" {
        Some(format!("/{path}"))
    } else {
        Some(format!("{}/{}", dir.trim_end_matches('/'), path))
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_code_path;

    #[test]
    fn absolute_paths_ignore_remote_dir() {
        assert_eq!(
            resolve_code_path("/etc/nginx/nginx.conf", Some("/home/dev")),
            Some("/etc/nginx/nginx.conf".to_string())
        );
    }

    #[test]
    fn relative_paths_join_remote_cwd() {
        assert_eq!(
            resolve_code_path("src/main.rs", Some("/home/dev/app")),
            Some("/home/dev/app/src/main.rs".to_string())
        );
        assert_eq!(
            resolve_code_path("src/main.rs", Some("/")),
            Some("/src/main.rs".to_string())
        );
    }

    #[test]
    fn relative_paths_without_cwd_are_rejected() {
        assert_eq!(resolve_code_path("src/main.rs", None), None);
    }
}
