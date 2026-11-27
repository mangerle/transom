//! 领域 6（容器工作区）与领域 7（Git 工作区）。

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};
use transom_ai::ToolError;
use transom_container::{Engine, discover_all};
use transom_core::ContainerLifecycleAction;
use transom_git::{LogQuery, Repo, Side};

use super::ControllerAppActionBridge;
use super::container_deploy;

pub(super) async fn handle_container(
    bridge: &ControllerAppActionBridge,
    action: &str,
    args: &Value,
) -> Result<String, ToolError> {
    if action == "open_exec_terminal" {
        return bridge
            .request_ui_action("container_workspace", action, args)
            .await;
    }
    if action == "create_and_start_container" {
        return container_deploy::create_and_start_container(bridge, args).await;
    }
    if action == "deploy_compose_yaml" {
        return container_deploy::deploy_compose_yaml(bridge, args).await;
    }
    if action == "discover_engines" {
        let (engines, failures) = discover_all().await;
        let list: Vec<Value> = engines
            .iter()
            .map(|e| {
                json!({
                    "kind": e.info.engine.kind.label(),
                    "endpoint": e.info.engine.endpoint.describe(),
                    "version": e.info.version,
                    "platform": e.info.platform,
                    "cli_backend": e.info.cli_backend,
                })
            })
            .collect();
        return serde_json::to_string_pretty(&json!({ "engines": list, "failures": failures }))
            .map_err(|e| ToolError::ExecutionFailed {
                tool: "container_workspace",
                reason: e.to_string(),
            });
    }
    if matches!(action, "compose_up" | "compose_down") {
        return container_deploy::handle_compose_lifecycle(bridge, action, args).await;
    }

    let engine = pick_engine(args["engine_id"].as_str()).await?;
    dispatch_engine_action(&engine, action, args).await
}

async fn pick_engine(wanted: Option<&str>) -> Result<Engine, ToolError> {
    let (engines, failures) = discover_all().await;
    if let Some(id) = wanted.filter(|s| !s.is_empty())
        && let Some(found) = engines.into_iter().find(|e| {
            e.info.engine.endpoint.describe().contains(id)
                || e.info.engine.kind.label().eq_ignore_ascii_case(id)
        })
    {
        return Ok(found);
    }
    let (engines_retry, _) = discover_all().await;
    engines_retry
        .into_iter()
        .next()
        .ok_or_else(|| ToolError::ExecutionFailed {
            tool: "container_workspace",
            reason: if failures.is_empty() {
                "本机未发现可用的 Docker 或 Podman 引擎".to_owned()
            } else {
                failures.join("；")
            },
        })
}

async fn dispatch_engine_action(
    engine: &Engine,
    action: &str,
    args: &Value,
) -> Result<String, ToolError> {
    match action {
        "list_containers" => {
            let mut list = engine.list_containers().await.map_err(map_container_err)?;
            if !args["all"].as_bool().unwrap_or(true) {
                list.retain(|c| format!("{:?}", c.state) == "Running");
            }
            to_json_str(&list)
        }
        "inspect_container" => {
            let cid = req_arg(args, "container_id", "container_workspace")?;
            let detail = engine
                .inspect_container(cid)
                .await
                .map_err(map_container_err)?;
            to_json_str(&detail)
        }
        "container_logs" => {
            let cid = req_arg(args, "container_id", "container_workspace")?;
            let tail = args["tail"].as_u64().unwrap_or(200).clamp(10, 2000) as usize;
            let cli_bin = match engine.info.engine.kind {
                transom_core::ContainerEngineKind::Podman => "podman",
                _ => "docker",
            };
            let raw = tokio::process::Command::new(cli_bin)
                .args(["logs", "--tail", &tail.to_string(), cid])
                .output()
                .await
                .map_err(|e| ToolError::ExecutionFailed {
                    tool: "container_workspace",
                    reason: e.to_string(),
                })?;
            Ok(format!(
                "{}{}",
                String::from_utf8_lossy(&raw.stdout),
                String::from_utf8_lossy(&raw.stderr)
            ))
        }
        "start_container" | "stop_container" | "restart_container" | "remove_container" => {
            let cid = req_arg(args, "container_id", "container_workspace")?;
            let act = match action {
                "start_container" => ContainerLifecycleAction::Start,
                "stop_container" => ContainerLifecycleAction::Stop,
                "restart_container" => ContainerLifecycleAction::Restart,
                _ => ContainerLifecycleAction::Remove {
                    force: args["force"].as_bool().unwrap_or(false),
                },
            };
            engine
                .lifecycle(cid, act)
                .await
                .map_err(map_container_err)?;
            Ok(format!("容器操作完成: {action} -> {cid}"))
        }
        "list_images" => to_json_str(&engine.list_images().await.map_err(map_container_err)?),
        "pull_image" => {
            let img = req_arg(args, "image", "container_workspace")?;
            engine
                .pull_image(img, &mut |_, _| {})
                .await
                .map_err(map_container_err)?;
            Ok(format!("已拉取镜像: {img}"))
        }
        "remove_image" => {
            let img = req_arg(args, "image", "container_workspace")?;
            let force = args["force"].as_bool().unwrap_or(false);
            engine
                .remove_image(img, force)
                .await
                .map_err(map_container_err)?;
            Ok(format!("已移除镜像: {img}"))
        }
        "list_networks" => to_json_str(&engine.list_networks().await.map_err(map_container_err)?),
        "list_volumes" => to_json_str(&engine.list_volumes().await.map_err(map_container_err)?),
        other => Err(ToolError::InvalidArguments {
            tool: "container_workspace",
            reason: format!("未知容器操作: {other}"),
        }),
    }
}

pub(super) async fn handle_git(
    bridge: &ControllerAppActionBridge,
    action: &str,
    args: &Value,
) -> Result<String, ToolError> {
    if action == "list_projects" {
        let store = bridge.deps.git_projects.clone();
        let reg = tokio::task::spawn_blocking(move || store.load())
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool: "git_workspace",
                reason: e.to_string(),
            })?
            .map_err(|e| ToolError::ExecutionFailed {
                tool: "git_workspace",
                reason: e.to_string(),
            })?;
        let list: Vec<Value> = reg
            .projects
            .iter()
            .map(|p| {
                json!({
                    "id": p.id.to_string(),
                    "label": p.label,
                    "path": p.path.display().to_string(),
                })
            })
            .collect();
        return to_json_str(&list);
    }

    let path = resolve_git_path(bridge, args).await?;
    let cfg = Arc::new(bridge.deps.git_config.read().clone());
    let repo = Repo::with_config(path, cfg);

    match action {
        "status" => Ok(format!(
            "{:#?}",
            repo.status(false).await.map_err(map_git_err)?
        )),
        "log" => {
            let limit = args["limit"].as_u64().unwrap_or(30).clamp(1, 200) as usize;
            let q = LogQuery {
                limit,
                ..LogQuery::default()
            };
            Ok(format!("{:#?}", repo.log(&q).await.map_err(map_git_err)?))
        }
        "diff" => {
            let p = req_arg(args, "path", "git_workspace")?;
            let diff = repo
                .diff(&Side::Commit("HEAD".to_owned()), &Side::Worktree, p)
                .await
                .map_err(map_git_err)?;
            Ok(format!("{diff:#?}"))
        }
        "branches" => Ok(format!(
            "{:#?}",
            repo.branches().await.map_err(map_git_err)?
        )),
        "stage" => {
            let paths = str_array(args, "paths");
            repo.stage(&paths).await.map_err(map_git_err)?;
            Ok(format!("已暂存 {} 个路径", paths.len()))
        }
        "stage_all" => {
            repo.stage_all().await.map_err(map_git_err)?;
            Ok("已暂存全部改动".to_owned())
        }
        "unstage" => {
            let paths = str_array(args, "paths");
            repo.unstage(&paths).await.map_err(map_git_err)?;
            Ok(format!("已取消暂存 {} 个路径", paths.len()))
        }
        "discard" => {
            let paths = str_array(args, "paths");
            repo.discard(&paths).await.map_err(map_git_err)?;
            Ok(format!("已丢弃 {} 个路径的工作区改动", paths.len()))
        }
        "delete_untracked" => {
            let paths = str_array(args, "paths");
            repo.discard_untracked(&paths).await.map_err(map_git_err)?;
            Ok(format!("已删除 {} 个未跟踪文件", paths.len()))
        }
        "commit" => {
            let msg = req_arg(args, "message", "git_workspace")?;
            let amend = args["amend"].as_bool().unwrap_or(false);
            repo.commit(msg, amend).await.map_err(map_git_err)?;
            Ok("Git 提交完成".to_owned())
        }
        "checkout" => {
            let br = req_arg(args, "branch", "git_workspace")?;
            repo.checkout(br).await.map_err(map_git_err)?;
            Ok(format!("已切换到分支: {br}"))
        }
        "create_branch" => {
            let br = req_arg(args, "branch", "git_workspace")?;
            repo.create_branch(br, None, true)
                .await
                .map_err(map_git_err)?;
            Ok(format!("已创建并切换到分支: {br}"))
        }
        "rename_branch" => {
            let old = req_arg(args, "branch", "git_workspace")?;
            let new = req_arg(args, "new_branch", "git_workspace")?;
            repo.rename_branch(old, new).await.map_err(map_git_err)?;
            Ok(format!("已重命名分支: {old} -> {new}"))
        }
        "pull" => {
            repo.pull(false).await.map_err(map_git_err)?;
            Ok("Git pull 完成".to_owned())
        }
        "fetch" => {
            repo.fetch().await.map_err(map_git_err)?;
            Ok("Git fetch 完成".to_owned())
        }
        "push" => {
            let br = args["branch"].as_str();
            repo.push(br, false).await.map_err(map_git_err)?;
            Ok("Git push 完成".to_owned())
        }
        "stash_list" => Ok(format!("{:#?}", repo.stashes().await.map_err(map_git_err)?)),
        "stash_save" => {
            let msg = args["message"].as_str().unwrap_or("AI 暂存");
            repo.stash_save(msg, true).await.map_err(map_git_err)?;
            Ok("已保存 Stash".to_owned())
        }
        "stash_pop" => {
            repo.stash_pop("stash@{0}").await.map_err(map_git_err)?;
            Ok("已弹出最近一次 Stash".to_owned())
        }
        other => Err(ToolError::InvalidArguments {
            tool: "git_workspace",
            reason: format!("未知 Git 动作: {other}"),
        }),
    }
}

async fn resolve_git_path(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<PathBuf, ToolError> {
    if let Some(p) = args["repo_path"].as_str().filter(|s| !s.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    if let Some(pid) = args["project_id"].as_str().filter(|s| !s.is_empty()) {
        let store = bridge.deps.git_projects.clone();
        let pid_owned = pid.to_owned();
        let reg = tokio::task::spawn_blocking(move || store.load())
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool: "git_workspace",
                reason: e.to_string(),
            })?
            .map_err(|e| ToolError::ExecutionFailed {
                tool: "git_workspace",
                reason: e.to_string(),
            })?;
        if let Some(proj) = reg.projects.iter().find(|p| p.id.to_string() == pid_owned) {
            return Ok(proj.path.clone());
        }
    }
    Err(ToolError::InvalidArguments {
        tool: "git_workspace",
        reason: "请提供 repo_path 或有效的 project_id".to_owned(),
    })
}

fn req_arg<'a>(args: &'a Value, key: &str, tool: &'static str) -> Result<&'a str, ToolError> {
    args[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments {
            tool,
            reason: format!("缺少必要参数 {key}"),
        })
}

fn str_array(args: &Value, key: &str) -> Vec<String> {
    args[key]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn to_json_str<T: serde::Serialize>(val: &T) -> Result<String, ToolError> {
    serde_json::to_string_pretty(val).map_err(|e| ToolError::ExecutionFailed {
        tool: "app_bridge",
        reason: e.to_string(),
    })
}

fn map_container_err(err: transom_container::ContainerError) -> ToolError {
    ToolError::ExecutionFailed {
        tool: "container_workspace",
        reason: err.to_string(),
    }
}

fn map_git_err(err: transom_git::GitError) -> ToolError {
    ToolError::ExecutionFailed {
        tool: "git_workspace",
        reason: err.to_string(),
    }
}
