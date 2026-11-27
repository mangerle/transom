//! 领域 8：API Client 项目集合管理与 HTTP 测试请求。

use serde_json::{Value, json};
use transom_ai::ToolError;
use transom_api::{ApiEnvironment, ApiHeader, ApiMethod, ApiProject, ApiRequest};

use super::ControllerAppActionBridge;

pub(super) async fn handle_api_client(
    bridge: &ControllerAppActionBridge,
    action: &str,
    args: &Value,
) -> Result<String, ToolError> {
    match action {
        "list_projects" => {
            let reg = load_api_registry(bridge).await?;
            let list: Vec<Value> = reg
                .projects
                .iter()
                .map(|p| {
                    json!({
                        "id": p.id.to_string(),
                        "name": p.name,
                        "request_count": p.requests.len(),
                    })
                })
                .collect();
            to_json_str(&list)
        }
        "list_requests" => {
            let pid = args["project_id"].as_str().unwrap_or("");
            let reg = load_api_registry(bridge).await?;
            let rows: Vec<Value> = reg
                .projects
                .iter()
                .filter(|p| pid.is_empty() || p.id.to_string() == pid)
                .flat_map(|p| {
                    p.requests.iter().map(|r| {
                        json!({
                            "project_id": p.id.to_string(),
                            "request_id": r.id.to_string(),
                            "name": r.name,
                            "method": format!("{:?}", r.method),
                            "url": r.url,
                        })
                    })
                })
                .collect();
            to_json_str(&rows)
        }
        "get_request" => {
            let rid = req_arg(args, "request_id")?;
            let reg = load_api_registry(bridge).await?;
            let found = reg
                .projects
                .iter()
                .flat_map(|p| p.requests.iter())
                .find(|r| r.id.to_string() == rid)
                .ok_or_else(|| ToolError::ExecutionFailed {
                    tool: "api_client",
                    reason: format!("未找到请求: {rid}"),
                })?;
            let safe_headers: Vec<Value> = found
                .headers
                .iter()
                .map(|h| {
                    let lower = h.name.to_ascii_lowercase();
                    let val = if lower.contains("auth")
                        || lower.contains("token")
                        || lower.contains("key")
                    {
                        "***".to_owned()
                    } else {
                        h.value.clone()
                    };
                    json!({ "name": h.name, "value": val, "enabled": h.enabled })
                })
                .collect();
            to_json_str(&json!({
                "id": found.id.to_string(),
                "name": found.name,
                "method": format!("{:?}", found.method),
                "url": found.url,
                "headers": safe_headers,
                "body": found.body,
            }))
        }
        "create_request" => create_api_request(bridge, args).await,
        "delete_request" => delete_api_request(bridge, args).await,
        "send_request" => send_api_request(bridge, args).await,
        other => Err(ToolError::InvalidArguments {
            tool: "api_client",
            reason: format!("未知 API Client 动作: {other}"),
        }),
    }
}

async fn create_api_request(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let pid = req_arg(args, "project_id")?.to_owned();
    let name = req_arg(args, "name")?.to_owned();
    let url = req_arg(args, "url")?.to_owned();
    let method = parse_method(args["method"].as_str().unwrap_or("GET"));
    let body = args["body"].as_str().unwrap_or("").to_owned();
    let store = bridge.deps.api_projects.clone();
    let created_id = tokio::task::spawn_blocking(move || {
        let mut reg = store.load()?;
        let proj = reg
            .projects
            .iter_mut()
            .find(|p| p.id.to_string() == pid)
            .ok_or_else(|| {
                transom_core::AppError::with_detail(
                    transom_core::ErrorCode::Configuration,
                    format!("未找到 API 项目: {pid}"),
                )
            })?;
        let mut req = ApiRequest::new(name, None);
        req.method = method;
        req.url = url;
        req.body = body;
        let id = req.id.to_string();
        proj.requests.push(req);
        store.save(&reg)?;
        Ok::<_, transom_core::AppError>(id)
    })
    .await
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "api_client",
        reason: e.to_string(),
    })?
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "api_client",
        reason: e.to_string(),
    })?;
    Ok(format!("已创建 API 请求，request_id: {created_id}"))
}

async fn delete_api_request(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let rid = req_arg(args, "request_id")?.to_owned();
    let store = bridge.deps.api_projects.clone();
    tokio::task::spawn_blocking(move || {
        let mut reg = store.load()?;
        let mut removed = false;
        for proj in &mut reg.projects {
            let before = proj.requests.len();
            proj.requests.retain(|r| r.id.to_string() != rid);
            if proj.requests.len() < before {
                removed = true;
            }
        }
        if !removed {
            return Err(transom_core::AppError::with_detail(
                transom_core::ErrorCode::Configuration,
                format!("未找到 API 请求: {rid}"),
            ));
        }
        store.save(&reg)
    })
    .await
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "api_client",
        reason: e.to_string(),
    })?
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "api_client",
        reason: e.to_string(),
    })?;
    Ok("已删除 API 请求".to_owned())
}

async fn send_api_request(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let plan = if let Some(rid) = args["request_id"].as_str().filter(|s| !s.is_empty()) {
        let reg = load_api_registry(bridge).await?;
        let (proj, req) = reg
            .projects
            .iter()
            .find_map(|p| {
                p.requests
                    .iter()
                    .find(|r| r.id.to_string() == rid)
                    .map(|r| (p, r))
            })
            .ok_or_else(|| ToolError::ExecutionFailed {
                tool: "api_client",
                reason: format!("未找到 API 请求: {rid}"),
            })?;
        let env = args["environment_id"]
            .as_str()
            .and_then(|eid| proj.environments.iter().find(|e| e.id.to_string() == eid))
            .cloned()
            .unwrap_or_else(ApiEnvironment::empty);
        proj.execution_plan(req, &env)
    } else {
        let url = req_arg(args, "url")?.to_owned();
        let mut req = ApiRequest::new("AI 临时请求", None);
        req.method = parse_method(args["method"].as_str().unwrap_or("GET"));
        req.url = url;
        req.body = args["body"].as_str().unwrap_or("").to_owned();
        if let Some(map) = args["headers"].as_object() {
            for (k, v) in map {
                if let Some(val) = v.as_str() {
                    req.headers.push(ApiHeader::new(k, val));
                }
            }
        }
        let proj = ApiProject::new("AI 临时集合");
        proj.execution_plan(&req, &ApiEnvironment::empty())
    }
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "api_client",
        reason: e.to_string(),
    })?;

    let resp =
        bridge
            .deps
            .api_http
            .execute(&plan)
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool: "api_client",
                reason: e.to_string(),
            })?;
    to_json_str(&json!({
        "status": resp.status,
        "status_text": resp.status_text,
        "elapsed_ms": resp.elapsed_millis,
        "body_size": resp.body_size,
        "body": resp.body,
    }))
}

async fn load_api_registry(
    bridge: &ControllerAppActionBridge,
) -> Result<transom_api::ApiRegistry, ToolError> {
    let store = bridge.deps.api_projects.clone();
    tokio::task::spawn_blocking(move || store.load())
        .await
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "api_client",
            reason: e.to_string(),
        })?
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "api_client",
            reason: e.to_string(),
        })
}

fn parse_method(raw: &str) -> ApiMethod {
    match raw.to_ascii_uppercase().as_str() {
        "POST" => ApiMethod::Post,
        "PUT" => ApiMethod::Put,
        "DELETE" => ApiMethod::Delete,
        "PATCH" => ApiMethod::Patch,
        "HEAD" => ApiMethod::Head,
        "OPTIONS" => ApiMethod::Options,
        _ => ApiMethod::Get,
    }
}

fn req_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    args[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "api_client",
            reason: format!("缺少必要参数 {key}"),
        })
}

fn to_json_str<T: serde::Serialize>(val: &T) -> Result<String, ToolError> {
    serde_json::to_string_pretty(val).map_err(|e| ToolError::ExecutionFailed {
        tool: "api_client",
        reason: e.to_string(),
    })
}
