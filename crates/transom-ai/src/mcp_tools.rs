//! 反向 MCP Server 对外暴露的工具 Schema 清单。

use serde_json::{Value, json};

/// 返回全部向外部 ACP / MCP 客户端暴露的工具定义。
pub(crate) fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "list_sessions",
            "description": "列出当前 Transom 中已连接的 SSH 会话（脱敏元数据，不含密钥与密码）。",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "remote_exec",
            "description": "在本机（target=\"local\"）或指定远端主机上执行一条 Shell 命令（支持 session_id 或 target，未连接时自动连接）。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "target": { "type": "string", "description": "\"local\" 或服务器名/session_id/IP" },
                    "command": { "type": "string" },
                    "cwd": { "type": "string" },
                    "timeout_secs": { "type": "integer" }
                },
                "required": ["command"]
            }
        }),
        json!({
            "name": "remote_exec_bulk",
            "description": "在多个已连接 session_id 上并发执行同一条 Shell 命令（写操作强制每次人工审批）。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_ids": { "type": "array", "items": { "type": "string" } },
                    "command": { "type": "string" }
                },
                "required": ["session_ids", "command"]
            }
        }),
        json!({
            "name": "remote_fs",
            "description": "操作本机（target=\"local\"）或远端主机文件系统：action 可选 list / read / search / edit / mkdir / rename / delete / chmod。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "target": { "type": "string" },
                    "action": { "type": "string", "enum": ["list", "read", "search", "edit", "mkdir", "rename", "delete", "chmod"] },
                    "path": { "type": "string" },
                    "content": { "type": "string" },
                    "pattern": { "type": "string" },
                    "target_path": { "type": "string" },
                    "mode": { "type": "string" },
                    "recursive": { "type": "boolean" }
                },
                "required": ["action", "path"]
            }
        }),
        json!({
            "name": "transfer_file",
            "description": "在本机与远端主机之间通过 SFTP 上传或下载文件（upload 支持整个目录，未连接时自动连接 target）。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "direction": { "type": "string", "enum": ["upload", "download"] },
                    "session_id": { "type": "string" },
                    "target": { "type": "string" },
                    "local_path": { "type": "string" },
                    "remote_path": { "type": "string" },
                    "overwrite": { "type": "boolean" }
                },
                "required": ["direction", "local_path", "remote_path"]
            }
        }),
        json!({
            "name": "inspect_host_runtime",
            "description": "只读采集指定主机的 CPU、内存、磁盘占用与运行中容器概览。",
            "inputSchema": {
                "type": "object",
                "properties": { "session_id": { "type": "string" } },
                "required": ["session_id"]
            }
        }),
        json!({
            "name": "container_ops",
            "description": "查看或操作指定主机上的 Docker/Podman 容器（list / inspect / logs / stats / restart）。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "action": { "type": "string", "enum": ["list", "inspect", "logs", "stats", "restart"] },
                    "container_id": { "type": "string" },
                    "tail": { "type": "integer" }
                },
                "required": ["session_id", "action"]
            }
        }),
        json!({
            "name": "get_recent_blocks",
            "description": "读取指定终端最近的 OSC 133 命令执行记录。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "limit": { "type": "integer" }
                },
                "required": ["session_id"]
            }
        }),
        json!({
            "name": "read_terminal",
            "description": "读取指定终端当前屏幕末尾若干行输出（敏感只读，需审批）。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "lines": { "type": "integer" }
                },
                "required": ["session_id"]
            }
        }),
        json!({
            "name": "read_tool_output",
            "description": "按行偏移或关键字分页读取超长工具输出句柄（output_id）。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "output_id": { "type": "string" },
                    "offset_line": { "type": "integer" },
                    "limit_lines": { "type": "integer" },
                    "pattern": { "type": "string" }
                },
                "required": ["output_id"]
            }
        }),
        // 以下为全软件 10 大领域的全局控制工具：
        json!({
            "name": "session_catalog",
            "description": "管理 Transom 已保存会话目录与受信任主机：list / create / update / delete / create_folder / delete_folder / list_known_hosts / remove_known_host / deploy_public_key。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string" },
                    "session_id": { "type": "string" },
                    "name": { "type": "string" },
                    "kind": { "type": "string" },
                    "host": { "type": "string" },
                    "port": { "type": "integer" },
                    "username": { "type": "string" },
                    "private_key_path": { "type": "string" },
                    "folder_id": { "type": "string" },
                    "parent_id": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "protection": { "type": "string" },
                    "jump_session_id": { "type": "string" },
                    "public_key_path": { "type": "string" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "workspace_control",
            "description": "控制 Transom 主窗口标签页与面板：list_tabs / connect_session / open_local_terminal / switch_tab / close_tab / split_pane / open_panel / send_terminal_input。注意：仅在用户明确要求打开终端或必须人工交互时才调用 connect_session；常规远端巡检、命令与文件操作请直接传 target 参数后台静默连接。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string" },
                    "session_id": { "type": "string" },
                    "name": { "type": "string" },
                    "tab_id": { "type": "string" },
                    "cwd": { "type": "string" },
                    "axis": { "type": "string" },
                    "panel": { "type": "string" },
                    "input": { "type": "string" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "sftp_transfer",
            "description": "管理 SFTP 传输队列与目录同步：list_transfers / cancel_transfer / sync_preview / sync_apply。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string" },
                    "session_id": { "type": "string" },
                    "tab_id": { "type": "string" },
                    "transfer_id": { "type": "string" },
                    "local_dir": { "type": "string" },
                    "remote_dir": { "type": "string" },
                    "direction": { "type": "string" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "host_monitor",
            "description": "主机监控与进程控制：snapshot / list_processes / kill_process / toggle_sampling / probe_host。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string" },
                    "session_id": { "type": "string" },
                    "pid": { "type": "integer" },
                    "signal": { "type": "string" },
                    "enabled": { "type": "boolean" },
                    "host": { "type": "string" },
                    "port": { "type": "integer" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "port_forward_ops",
            "description": "管理 SSH 端口转发规则：list / create / start / stop / delete / probe。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string" },
                    "session_id": { "type": "string" },
                    "forward_id": { "type": "string" },
                    "kind": { "type": "string" },
                    "listen_host": { "type": "string" },
                    "listen_port": { "type": "integer" },
                    "target_host": { "type": "string" },
                    "target_port": { "type": "integer" },
                    "name": { "type": "string" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "container_workspace",
            "description": "管理本地或远端引擎容器工作区：discover_engines / list_containers / inspect_container / container_logs / start_container / stop_container / restart_container / remove_container / create_and_start_container / deploy_compose_yaml / list_images / pull_image / remove_image / list_networks / list_volumes / compose_up / compose_down / open_exec_terminal。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string" },
                    "target": { "type": "string" },
                    "engine_id": { "type": "string" },
                    "container_id": { "type": "string" },
                    "name": { "type": "string" },
                    "image": { "type": "string" },
                    "ports": { "type": "array", "items": { "type": "string" } },
                    "env": { "type": "array", "items": { "type": "string" } },
                    "volumes": { "type": "array", "items": { "type": "string" } },
                    "restart": { "type": "string" },
                    "network": { "type": "string" },
                    "command": { "type": "array", "items": { "type": "string" } },
                    "yaml": { "type": "string" },
                    "pull": { "type": "boolean" },
                    "project_dir": { "type": "string" },
                    "tail": { "type": "integer" },
                    "all": { "type": "boolean" },
                    "force": { "type": "boolean" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "git_workspace",
            "description": "操作 Transom Git 工作区仓库：list_projects / status / log / diff / branches / stage / stage_all / unstage / discard / delete_untracked / commit / checkout / create_branch / rename_branch / pull / fetch / push / stash_list / stash_save / stash_pop。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string" },
                    "repo_path": { "type": "string" },
                    "project_id": { "type": "string" },
                    "paths": { "type": "array", "items": { "type": "string" } },
                    "path": { "type": "string" },
                    "message": { "type": "string" },
                    "amend": { "type": "boolean" },
                    "branch": { "type": "string" },
                    "new_branch": { "type": "string" },
                    "remote": { "type": "string" },
                    "limit": { "type": "integer" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "api_client",
            "description": "管理 API Client 项目集合并发起测试请求：list_projects / list_requests / get_request / create_request / delete_request / send_request。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string" },
                    "project_id": { "type": "string" },
                    "request_id": { "type": "string" },
                    "name": { "type": "string" },
                    "method": { "type": "string" },
                    "url": { "type": "string" },
                    "headers": { "type": "object" },
                    "body": { "type": "string" },
                    "environment_id": { "type": "string" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "app_settings_ops",
            "description": "管理快捷指令、应用设置、云同步与加密备份：list_snippets / save_snippet / delete_snippet / get_settings / update_settings / sync_status / sync_push / sync_pull / export_backup。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string" },
                    "snippet_id": { "type": "string" },
                    "name": { "type": "string" },
                    "command": { "type": "string" },
                    "group": { "type": "string" },
                    "language": { "type": "string" },
                    "theme": { "type": "string" },
                    "output_path": { "type": "string" },
                    "passphrase": { "type": "string" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "ai_memory_skills",
            "description": "读写 AI 长期记忆与自定义 Slash 技能：read_memory / append_memory / replace_memory / list_skills / save_skill / delete_skill。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string" },
                    "scope": { "type": "string" },
                    "host_id": { "type": "string" },
                    "content": { "type": "string" },
                    "skill_name": { "type": "string" },
                    "title": { "type": "string" },
                    "description": { "type": "string" }
                },
                "required": ["action"]
            }
        }),
    ]
}
