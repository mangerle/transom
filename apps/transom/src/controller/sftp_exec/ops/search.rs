//! SFTP 远程递归文件搜索（find）与内容全文检索（grep）执行处理。
//!
//! 设计原理（Why）：
//! - 实现初衷：为远端主机提供高效、深度的文件检索能力。SFTP 协议逐层扫描深层目录延迟极大，
//!   通过复用底层 SSH 通道直接调用远端系统内建的 `find` 与 `grep` 命令，毫秒级返回匹配结果；
//! - 核心优势：
//!   1. 检索路径与模式全量进行 POSIX 单引号转义闭合，彻底阻断 Shell 命令注入；
//!   2. 自动注入 `head -n` 熔断行数保护与目录剪枝（排除 `.git/`、`node_modules/`），避免大仓库爆内存；
//!   3. 区分文件名与文件内容两种模式，解析器健壮容错，忽略畸变行；
//! - 代价与局限：依赖远端宿主环境为标准 Linux/Unix 环境且具备 `find` 或 `grep` 工具。

use super::SftpExecutionContext;
use super::archive::posix_shell_quote;
use crate::controller::sftp_finish::finish_sftp_none;
use transom_core::{AppError, ErrorCode, RemoteSearchQuery, RemoteSearchResultItem, SftpAppEvent};

/// 构建远程检索 Shell 命令。
pub(crate) fn build_search_command(query: &RemoteSearchQuery) -> String {
    let quoted_path = posix_shell_quote(&query.path);
    let max_results = query.max_results.clamp(1, 1000);

    if query.is_content {
        let case_flag = if query.case_sensitive { "" } else { "-i " };
        let depth_flag = match query.max_depth {
            Some(depth) => format!("--max-depth={depth} "),
            None => String::new(),
        };
        let quoted_pattern = posix_shell_quote(&query.pattern);
        format!(
            "grep -rnI --exclude-dir={{.git,node_modules,target}} {case_flag}{depth_flag}-e {quoted_pattern} {quoted_path} 2>/dev/null | head -n {max_results}"
        )
    } else {
        let name_flag = if query.case_sensitive {
            "-name"
        } else {
            "-iname"
        };
        let depth_flag = match query.max_depth {
            Some(depth) => format!("-maxdepth {depth} "),
            None => String::new(),
        };
        let pattern_with_wildcards = format!("*{}*", query.pattern);
        let quoted_pattern = posix_shell_quote(&pattern_with_wildcards);
        format!(
            "find {quoted_path} {depth_flag}{name_flag} {quoted_pattern} 2>/dev/null | head -n {max_results}"
        )
    }
}

/// 解析检索进程的标准输出为结构化结果项列表。
pub(crate) fn parse_search_output(stdout: &str, is_content: bool) -> Vec<RemoteSearchResultItem> {
    let mut results = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if is_content {
            // grep -rnI 格式：path:line:content
            if let Some((path, rest)) = trimmed.split_once(':')
                && let Some((line_no_str, content)) = rest.split_once(':')
                && let Ok(line_no) = line_no_str.trim().parse::<usize>()
            {
                results.push(RemoteSearchResultItem::new_content(path, line_no, content));
            } else {
                results.push(RemoteSearchResultItem::new_path(trimmed));
            }
        } else {
            results.push(RemoteSearchResultItem::new_path(trimmed));
        }
    }
    results
}

/// 执行远程文件与内容检索。
pub(super) async fn handle_search(ctx: &SftpExecutionContext, query: RemoteSearchQuery) {
    let result = execute_search(ctx, query).await;
    finish_sftp_none(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}

async fn execute_search(
    ctx: &SftpExecutionContext,
    query: RemoteSearchQuery,
) -> Result<(), AppError> {
    let Some(ref remote) = ctx.remote else {
        return Err(AppError::with_detail(
            ErrorCode::SftpFailed,
            "当前会话缺少底层 SSH 执行通道，无法进行远程检索",
        ));
    };

    let command = build_search_command(&query);
    log::info!("正在执行远程检索命令: {command}");
    let output = remote.exec_command_capture(&command).await?;

    let items = parse_search_output(&output.stdout, query.is_content);
    log::info!("远程检索完成，找到 {} 条匹配结果", items.len());

    let _ = ctx
        .event_tx
        .send_async(
            SftpAppEvent::SftpSearchResults {
                tab_id: ctx.tab_id,
                connection_id: ctx.connection_id,
                op_id: ctx.op_id,
                items,
            }
            .into(),
        )
        .await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{build_search_command, parse_search_output};
    use transom_core::RemoteSearchQuery;

    #[test]
    fn test_build_search_command_name_case_insensitive() {
        let query = RemoteSearchQuery {
            path: "/var/log".to_string(),
            pattern: "nginx".to_string(),
            is_content: false,
            case_sensitive: false,
            max_depth: Some(3),
            max_results: 50,
        };
        let cmd = build_search_command(&query);
        assert_eq!(
            cmd,
            "find '/var/log' -maxdepth 3 -iname '*nginx*' 2>/dev/null | head -n 50"
        );
    }

    #[test]
    fn test_build_search_command_content() {
        let query = RemoteSearchQuery {
            path: "/etc".to_string(),
            pattern: "127.0.0.1".to_string(),
            is_content: true,
            case_sensitive: true,
            max_depth: None,
            max_results: 100,
        };
        let cmd = build_search_command(&query);
        assert_eq!(
            cmd,
            "grep -rnI --exclude-dir={.git,node_modules,target} -e '127.0.0.1' '/etc' 2>/dev/null | head -n 100"
        );
    }

    #[test]
    fn test_parse_search_output() {
        let name_out = "/var/log/nginx.log\n/var/log/nginx.error.log\n";
        let items = parse_search_output(name_out, false);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].path, "/var/log/nginx.log");
        assert_eq!(items[0].line_number, None);

        let grep_out = "/etc/hosts:1:127.0.0.1 localhost\n/etc/hosts:2:127.0.1.1 hostname\n";
        let items = parse_search_output(grep_out, true);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].path, "/etc/hosts");
        assert_eq!(items[0].line_number, Some(1));
        assert_eq!(
            items[0].line_content.as_deref(),
            Some("127.0.0.1 localhost")
        );
    }
}
