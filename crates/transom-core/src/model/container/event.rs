//! 容器引擎原生实时事件数据契约（GET /events / docker events）。

use serde::{Deserialize, Serialize};

/// 容器引擎原生实时事件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineEvent {
    /// 资源类型（如 container, image, volume, network 等）。
    pub resource_type: String,
    /// 触发的具体动作（如 start, die, stop, destroy, oom, kill, prune 等）。
    pub action: String,
    /// 发生事件的主体资源 ID。
    pub id: String,
    /// 发生事件的资源名称（若有）。
    pub name: Option<String>,
    /// 容器所使用的镜像标识（若事件为主体容器）。
    pub image: Option<String>,
    /// 容器退出码（容器 die 退出时提供）。
    pub exit_code: Option<i32>,
    /// 事件发生的 Unix 秒级时间戳。
    pub timestamp: i64,
}

impl EngineEvent {
    /// 判断该事件是否表示容器非正常异常退出（例如崩溃退出或 OOM）。
    pub fn is_abnormal_container_exit(&self) -> bool {
        if self.resource_type != "container" {
            return false;
        }
        if self.action == "oom" {
            return true;
        }
        if self.action == "die" {
            return self.exit_code.is_some_and(|code| code != 0);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_event_abnormal_exit() {
        let normal_exit = EngineEvent {
            resource_type: "container".to_string(),
            action: "die".to_string(),
            id: "abc".to_string(),
            name: Some("test".to_string()),
            image: Some("alpine".to_string()),
            exit_code: Some(0),
            timestamp: 123456,
        };
        assert!(!normal_exit.is_abnormal_container_exit());

        let crash_exit = EngineEvent {
            exit_code: Some(137),
            ..normal_exit.clone()
        };
        assert!(crash_exit.is_abnormal_container_exit());

        let oom_exit = EngineEvent {
            action: "oom".to_string(),
            exit_code: None,
            ..normal_exit
        };
        assert!(oom_exit.is_abnormal_container_exit());
    }
}
