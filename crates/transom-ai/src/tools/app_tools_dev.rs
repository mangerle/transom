//! 软件领域全局工具（第 6~10 领域：容器工作区、Git 工作区、API Client、设置/备份/同步、AI 记忆与技能）。

use rig_agent::tool::{Tool, ToolContext, ToolExecutionError};
use serde_json::{Value, json};

use super::ToolRuntimeContext;
use super::app_tools::{AppActionArgs, define_app_tool, invoke_app_tool};
use transom_ai_contracts::policy::ToolRisk;

define_app_tool! {
    /// 领域 6：容器工作区管理（本地 Docker/Podman 引擎、容器/镜像/网络/卷/Compose 与进入终端）。
    pub struct ContainerWorkspaceTool {
        tool_name: "container_workspace",
        actions: [
            "discover_engines", "list_containers", "inspect_container",
            "container_logs", "start_container", "stop_container",
            "restart_container", "remove_container",
            "create_and_start_container", "deploy_compose_yaml",
            "list_images", "pull_image", "remove_image",
            "list_networks", "list_volumes",
            "compose_up", "compose_down", "open_exec_terminal"
        ],
        description: "管理本地或远端引擎中的容器工作区：discover_engines / list_containers / inspect_container / \
         container_logs / start_container / stop_container / restart_container / remove_container / \
         create_and_start_container / deploy_compose_yaml / list_images / pull_image / remove_image / \
         list_networks / list_volumes / compose_up / compose_down / open_exec_terminal。",
        schema_props: json!({
            "target": { "type": "string", "description": "可选目标：省略或 'local' 操作本机，传入远端会话名/ID 则自动连接并在远端引擎执行" },
            "engine_id": { "type": "string", "description": "引擎标识，省略则自动探测首个可用引擎" },
            "container_id": { "type": "string" },
            "name": { "type": "string", "description": "容器名称或 Compose 项目名称" },
            "image": { "type": "string", "description": "镜像名或 ID" },
            "ports": { "type": "array", "items": { "type": "string" }, "description": "端口映射，如 [\"8080:80\", \"443:443/tcp\"]" },
            "env": { "type": "array", "items": { "type": "string" }, "description": "环境变量 KEY=VALUE 列表" },
            "volumes": { "type": "array", "items": { "type": "string" }, "description": "挂载列表 host_path:container_path[:ro]" },
            "restart": { "type": "string", "enum": ["no", "on-failure", "always", "unless-stopped"] },
            "network": { "type": "string", "description": "容器网络名称" },
            "command": { "type": "array", "items": { "type": "string" }, "description": "覆盖镜像启动命令" },
            "yaml": { "type": "string", "description": "deploy_compose_yaml 传入的 docker-compose.yml 内容" },
            "pull": { "type": "boolean", "description": "创建或部署前是否先拉取镜像" },
            "project_dir": { "type": "string", "description": "Compose 项目目录" },
            "tail": { "type": "integer", "description": "日志行数，默认 200" },
            "all": { "type": "boolean", "description": "list_containers 是否包含已停止容器" },
            "force": { "type": "boolean" }
        }),
        risk: |act| match act {
            "discover_engines" | "list_containers" | "inspect_container" | "container_logs"
            | "list_images" | "list_networks" | "list_volumes" => ToolRisk::SafeRead,
            "remove_container" | "remove_image" | "compose_down" => ToolRisk::Destructive,
            _ => ToolRisk::Write,
        }
    }
}

define_app_tool! {
    /// 领域 7：Git 工作区操作（项目列表、状态、日志、Diff、暂存、提交、分支、远程同步、Stash）。
    pub struct GitWorkspaceTool {
        tool_name: "git_workspace",
        actions: [
            "list_projects", "status", "log", "diff", "branches",
            "stage", "stage_all", "unstage", "discard", "delete_untracked",
            "commit", "checkout", "create_branch", "rename_branch",
            "pull", "fetch", "push", "stash_list", "stash_save", "stash_pop"
        ],
        description: "操作 Transom Git 工作区仓库：list_projects / status / log / diff / branches / \
         stage / stage_all / unstage / discard / delete_untracked / commit / checkout / \
         create_branch / rename_branch / pull / fetch / push / stash_list / stash_save / stash_pop。",
        schema_props: json!({
            "repo_path": { "type": "string", "description": "仓库本地绝对路径，或从 list_projects 获取" },
            "project_id": { "type": "string", "description": "已保存 Git 项目 ID" },
            "paths": { "type": "array", "items": { "type": "string" } },
            "path": { "type": "string" },
            "message": { "type": "string", "description": "commit 或 stash_save 说明" },
            "amend": { "type": "boolean" },
            "branch": { "type": "string" },
            "new_branch": { "type": "string" },
            "remote": { "type": "string" },
            "limit": { "type": "integer" }
        }),
        risk: |act| match act {
            "list_projects" | "status" | "log" | "diff" | "branches" | "stash_list" => {
                ToolRisk::SafeRead
            }
            "discard" | "delete_untracked" => ToolRisk::Destructive,
            _ => ToolRisk::Write,
        }
    }
}

define_app_tool! {
    /// 领域 8：API Client 管理与测试请求。
    pub struct ApiClientTool {
        tool_name: "api_client",
        actions: [
            "list_projects", "list_requests", "get_request",
            "create_request", "delete_request", "send_request"
        ],
        description: "管理 API Client 项目集合并发起测试请求：list_projects / list_requests / \
         get_request / create_request / delete_request / send_request。",
        schema_props: json!({
            "project_id": { "type": "string" },
            "request_id": { "type": "string" },
            "name": { "type": "string" },
            "method": { "type": "string", "enum": ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"] },
            "url": { "type": "string" },
            "headers": { "type": "object" },
            "body": { "type": "string" },
            "environment_id": { "type": "string" }
        }),
        risk: |act| match act {
            "list_projects" | "list_requests" | "get_request" => ToolRisk::SafeRead,
            "delete_request" => ToolRisk::Destructive,
            _ => ToolRisk::Write,
        }
    }
}

define_app_tool! {
    /// 领域 9：快捷指令（Snippets）、设置、云同步与加密备份。
    pub struct AppSettingsOpsTool {
        tool_name: "app_settings_ops",
        actions: [
            "list_snippets", "save_snippet", "delete_snippet",
            "get_settings", "update_settings",
            "sync_status", "sync_push", "sync_pull", "export_backup"
        ],
        description: "管理快捷指令、应用设置、云同步与加密备份：list_snippets / save_snippet / delete_snippet / \
         get_settings / update_settings / sync_status / sync_push / sync_pull / export_backup。",
        schema_props: json!({
            "snippet_id": { "type": "string" },
            "name": { "type": "string" },
            "command": { "type": "string" },
            "group": { "type": "string" },
            "language": { "type": "string" },
            "theme": { "type": "string" },
            "output_path": { "type": "string", "description": "export_backup 目标文件路径" },
            "passphrase": { "type": "string", "description": "加密备份口令（仅用于本地加密，不写入日志）" }
        }),
        risk: |act| match act {
            "list_snippets" | "get_settings" | "sync_status" => ToolRisk::SafeRead,
            "delete_snippet" => ToolRisk::Destructive,
            _ => ToolRisk::Write,
        }
    }
}

define_app_tool! {
    /// 领域 10：AI 长期记忆与自定义 Slash 技能管理。
    pub struct AiMemorySkillsTool {
        tool_name: "ai_memory_skills",
        actions: [
            "read_memory", "append_memory", "replace_memory",
            "list_skills", "save_skill", "delete_skill"
        ],
        description: "读写 AI 长期记忆与自定义 Slash 技能：read_memory / append_memory / replace_memory / \
         list_skills / save_skill / delete_skill。",
        schema_props: json!({
            "scope": { "type": "string", "enum": ["global", "host"], "description": "默认 global" },
            "host_id": { "type": "string", "description": "scope=host 时的主机/会话 ID" },
            "content": { "type": "string", "description": "写入的 Markdown 记忆或技能正文" },
            "skill_name": { "type": "string", "description": "Slash 指令名称（如 deploy-check）" },
            "title": { "type": "string" },
            "description": { "type": "string" }
        }),
        risk: |act| match act {
            "read_memory" | "list_skills" => ToolRisk::SafeRead,
            "delete_skill" => ToolRisk::Destructive,
            _ => ToolRisk::Write,
        }
    }
}
