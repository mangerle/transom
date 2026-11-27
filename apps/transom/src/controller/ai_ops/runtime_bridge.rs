//! 远端主机运行状态与容器信息的只读采集桥接。

use async_trait::async_trait;
use transom_ai::{RuntimeInspector, ToolError};
use transom_net::SshRemote;

/// 采集远端主机运行状态与容器信息的轻量只读桥接。
pub(super) struct HostRuntimeBridge {
    pub(super) remote: SshRemote,
}

#[async_trait]
impl RuntimeInspector for HostRuntimeBridge {
    async fn host_runtime_snapshot(&self) -> Result<String, ToolError> {
        let script = "uname -a; uptime; free -m; df -hT -x tmpfs -x devtmpfs; \
            (command -v docker >/dev/null 2>&1 && docker ps --format '{{.Names}}\t{{.Status}}\t{{.Image}}' || true)";
        let out = self
            .remote
            .exec_command_capture(script)
            .await
            .map_err(|err| ToolError::ExecutionFailed {
                tool: "inspect_host_runtime",
                reason: err.to_string(),
            })?;
        Ok(out.stdout)
    }

    async fn container_action(
        &self,
        action: &str,
        container_id: Option<&str>,
        tail: Option<usize>,
    ) -> Result<String, ToolError> {
        let target = container_id.map(shell_quote).unwrap_or_default();
        let cmd = match action {
            "list" => "docker ps -a --format 'table {{.ID}}\t{{.Names}}\t{{.Status}}\t{{.Image}}'"
                .to_owned(),
            "inspect" => format!("docker inspect {target}"),
            "logs" => {
                let n = tail.unwrap_or(100).clamp(10, 500);
                format!("docker logs --tail {n} {target}")
            }
            "stats" => "docker stats --no-stream".to_owned(),
            "restart" => format!("docker restart {target}"),
            other => {
                return Err(ToolError::InvalidArguments {
                    tool: "container_ops",
                    reason: format!("不支持的容器动作: {other}"),
                });
            }
        };
        let out = self
            .remote
            .exec_command_capture(&cmd)
            .await
            .map_err(|err| ToolError::ExecutionFailed {
                tool: "container_ops",
                reason: err.to_string(),
            })?;
        if out.exit_status == Some(0) {
            Ok(out.stdout)
        } else {
            Ok(format!("{}\n{}", out.stdout, out.stderr))
        }
    }
}

pub(super) fn shell_quote(raw: &str) -> String {
    let escaped = raw.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}
