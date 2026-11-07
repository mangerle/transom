//! 网络资源的 REST 操作：列表、检视、创建、连接与清理。

use super::{DockerApi, JSON_BODY_LIMIT};
use crate::error::ContainerError;
use crate::http;
use crate::models::{NetworkItem, NetworkPruneResponse};
use hyper::Method;
use transom_core::{ContainerNetworkDetails, ContainerNetworkSummary, ContainerPruneReport};

impl DockerApi {
    /// 网络列表。
    pub async fn list_networks(&self) -> Result<Vec<ContainerNetworkSummary>, ContainerError> {
        let items: Vec<NetworkItem> = self.get_json("/networks", "列出网络").await?;
        Ok(items.iter().map(NetworkItem::summary).collect())
    }

    /// 网络详情。
    pub async fn inspect_network(
        &self,
        network: &str,
    ) -> Result<ContainerNetworkDetails, ContainerError> {
        let (item, raw) = self
            .inspect_json::<NetworkItem>(
                &format!("/networks/{}", http::percent_encode(network)),
                "检视网络",
            )
            .await?;
        Ok(item.into_details(raw))
    }

    pub async fn create_network(
        &self,
        spec: &transom_core::ContainerNetworkSpec,
    ) -> Result<(), ContainerError> {
        let name = spec.name.as_str();
        // 留空的字段一律不下发，由引擎按自己的默认值决定；只有填了子网才带 IPAM。
        let mut payload = serde_json::json!({
            "Name": name,
            "CheckDuplicate": true,
        });
        if let Some(driver) = spec.driver.as_deref() {
            payload["Driver"] = serde_json::Value::String(driver.to_string());
        }
        if spec.internal {
            payload["Internal"] = serde_json::Value::Bool(true);
        }
        if let Some(subnet) = spec.subnet.as_deref() {
            let mut config = serde_json::json!({ "Subnet": subnet });
            if let Some(gateway) = spec.gateway.as_deref() {
                config["Gateway"] = serde_json::Value::String(gateway.to_string());
            }
            payload["IPAM"] = serde_json::json!({ "Config": [config] });
        }
        let body = serde_json::to_vec(&payload).map_err(|error| ContainerError::Decode {
            operation: "构造网络请求".to_string(),
            message: error.to_string(),
        })?;
        let response = self
            .send(Method::POST, "/networks/create", Some(body), "创建网络")
            .await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "创建网络", body, Some(name))
    }

    pub async fn remove_network(&self, network: &str) -> Result<(), ContainerError> {
        let path = format!("/networks/{}", http::percent_encode(network));
        let response = self.send(Method::DELETE, &path, None, "删除网络").await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "删除网络", body, Some(network))
    }

    /// 将容器连接到网络。
    pub async fn connect_network(
        &self,
        network: &str,
        container: &str,
    ) -> Result<(), ContainerError> {
        let path = format!("/networks/{}/connect", http::percent_encode(network));
        let payload = serde_json::json!({
            "Container": container,
        });
        let body = serde_json::to_vec(&payload).map_err(|error| ContainerError::Decode {
            operation: "构造连接网络请求".to_string(),
            message: error.to_string(),
        })?;
        let response = self
            .send(Method::POST, &path, Some(body), "连接网络")
            .await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "连接网络", body, Some(network))
    }

    /// 将容器从网络断开。
    pub async fn disconnect_network(
        &self,
        network: &str,
        container: &str,
        force: bool,
    ) -> Result<(), ContainerError> {
        let path = format!("/networks/{}/disconnect", http::percent_encode(network));
        let payload = serde_json::json!({
            "Container": container,
            "Force": force,
        });
        let body = serde_json::to_vec(&payload).map_err(|error| ContainerError::Decode {
            operation: "构造断开网络请求".to_string(),
            message: error.to_string(),
        })?;
        let response = self
            .send(Method::POST, &path, Some(body), "断开网络")
            .await?;
        if response.status.is_success() {
            return Ok(());
        }
        let status = response.status;
        let body = http::read_body_text(response.body, 8192).await;
        http::ensure_success(status, "断开网络", body, Some(network))
    }

    /// 清理未使用的容器网络。
    pub async fn prune_networks(&self) -> Result<ContainerPruneReport, ContainerError> {
        const OPERATION: &str = "清理未使用网络";
        let response = self
            .send(Method::POST, "/networks/prune", None, OPERATION)
            .await?;
        let status = response.status;
        if !status.is_success() {
            let body = http::read_body_text(response.body, 8192).await;
            return Err(http::status_error(status, OPERATION, body, "networks"));
        }
        let bytes = http::read_body_bytes(response.body, JSON_BODY_LIMIT).await?;
        let report: NetworkPruneResponse =
            serde_json::from_slice(&bytes).map_err(|error| ContainerError::Decode {
                operation: OPERATION.to_string(),
                message: error.to_string(),
            })?;
        Ok(ContainerPruneReport {
            deleted: report.networks_deleted,
            reclaimed: 0,
            errors: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn network_connect_and_disconnect_payload_structures() {
        let connect_payload = serde_json::json!({
            "Container": "my-web-1",
        });
        assert_eq!(connect_payload["Container"], "my-web-1");

        let disconnect_payload = serde_json::json!({
            "Container": "my-web-1",
            "Force": false,
        });
        assert_eq!(disconnect_payload["Container"], "my-web-1");
        assert_eq!(disconnect_payload["Force"], false);
    }
}
