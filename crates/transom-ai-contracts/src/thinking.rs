//! 思考强度与各协议请求体参数适配。
//!
//! 设计原理（Why）：
//! - 不同大模型厂商对推理开关与强度的参数字段写法差异很大（如千问使用 `enable_thinking`，
//!   DeepSeek/智谱使用 `thinking.type`，OpenAI 官方对未定义扩展字段会直接返回 400）。
//! - 通过 [`ThinkingSwitch`] 与 [`ThinkingEffort`] 显式建模各协议差异，避免硬编码猜测模型名。

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::ProviderProtocol;

/// 思考强度档位：按界面展示顺序排列，从关闭到最高。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingEffort {
    None,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl ThinkingEffort {
    /// 全部可选档位。
    pub const ALL: [Self; 6] = [
        Self::None,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::XHigh,
        Self::Max,
    ];

    /// 协议与界面共用的标准标识。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }

    /// 从字符串解析思考强度；无法识别时返回 `None`。
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::XHigh),
            "max" => Some(Self::Max),
            _ => None,
        }
    }
}

/// Chat Completions 兼容端点下的思考开关写法。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingSwitch {
    /// 不额外注入思考开关字段（如 OpenAI 官方端点）。
    #[default]
    None,
    /// 使用 `enable_thinking: bool`（如阿里云千问兼容模式）。
    EnableThinking,
    /// 使用 `thinking: { type: "enabled" | "disabled" }`（如 DeepSeek、智谱、豆包）。
    ThinkingType,
}

/// 当模型不支持目标档位时，向最接近的可用档位平滑回退。
pub fn fallback_effort(
    supported: &[ThinkingEffort],
    wanted: Option<ThinkingEffort>,
) -> Option<ThinkingEffort> {
    if supported.is_empty() {
        return None;
    }
    let target = wanted?;
    if supported.contains(&target) {
        return Some(target);
    }
    // 优先向低于等于 target 的最高档退避，其次取高于 target 的最低档。
    supported
        .iter()
        .copied()
        .filter(|effort| *effort <= target)
        .max()
        .or_else(|| supported.iter().copied().min())
}

/// 构造平铺进请求体的思考扩展参数（配合 `rig` 的 `additional_params` 使用）。
pub fn request_params(
    protocol: ProviderProtocol,
    switch: ThinkingSwitch,
    effort: Option<ThinkingEffort>,
) -> Option<Value> {
    let effort = effort?;
    match protocol {
        ProviderProtocol::OpenAi => {
            let mut map = serde_json::Map::new();
            let enabled = effort != ThinkingEffort::None;
            match switch {
                ThinkingSwitch::None => {}
                ThinkingSwitch::EnableThinking => {
                    map.insert("enable_thinking".to_owned(), Value::Bool(enabled));
                }
                ThinkingSwitch::ThinkingType => {
                    let kind = if enabled { "enabled" } else { "disabled" };
                    map.insert("thinking".to_owned(), json!({ "type": kind }));
                }
            }
            if enabled {
                map.insert(
                    "reasoning_effort".to_owned(),
                    Value::String(effort.as_str().to_owned()),
                );
            }
            if map.is_empty() {
                None
            } else {
                Some(Value::Object(map))
            }
        }
        ProviderProtocol::OpenAiResponses => Some(json!({
            "reasoning": {
                "effort": effort.as_str(),
                "summary": "auto"
            }
        })),
        ProviderProtocol::Anthropic => {
            if effort == ThinkingEffort::None {
                Some(json!({
                    "thinking": { "type": "disabled" }
                }))
            } else {
                Some(json!({
                    "thinking": { "type": "adaptive" },
                    "output_config": { "effort": effort.as_str() }
                }))
            }
        }
    }
}

/// 构造包含思考参数与 Anthropic Prompt Caching 断点的请求体扩展字段。
pub fn build_request_params(
    protocol: ProviderProtocol,
    switch: ThinkingSwitch,
    effort: Option<ThinkingEffort>,
    enable_prompt_cache: bool,
) -> Option<Value> {
    let mut base = request_params(protocol, switch, effort);
    if protocol == ProviderProtocol::Anthropic && enable_prompt_cache {
        let cache_val = json!({ "type": "ephemeral" });
        match &mut base {
            Some(Value::Object(map)) => {
                map.insert("cache_control".to_owned(), cache_val);
            }
            _ => {
                base = Some(json!({ "cache_control": cache_val }));
            }
        }
    }
    base
}

#[cfg(test)]
mod tests {
    use super::{
        ThinkingEffort, ThinkingSwitch, build_request_params, fallback_effort, request_params,
    };
    use crate::config::ProviderProtocol;

    #[test]
    fn fallback_picks_nearest_supported_effort() {
        let supported = [ThinkingEffort::Low, ThinkingEffort::High];
        assert_eq!(
            fallback_effort(&supported, Some(ThinkingEffort::Medium)),
            Some(ThinkingEffort::Low)
        );
        assert_eq!(
            fallback_effort(&supported, Some(ThinkingEffort::None)),
            Some(ThinkingEffort::Low)
        );
        assert_eq!(fallback_effort(&[], Some(ThinkingEffort::High)), None);
    }

    #[test]
    fn request_params_respects_protocol_and_switch() {
        let qwen = request_params(
            ProviderProtocol::OpenAi,
            ThinkingSwitch::EnableThinking,
            Some(ThinkingEffort::Medium),
        )
        .expect("千问参数不应为空");
        assert_eq!(qwen["enable_thinking"], true);
        assert_eq!(qwen["reasoning_effort"], "medium");

        let anthropic = request_params(
            ProviderProtocol::Anthropic,
            ThinkingSwitch::None,
            Some(ThinkingEffort::None),
        )
        .expect("Anthropic 关闭思考应生成 disabled");
        assert_eq!(anthropic["thinking"]["type"], "disabled");

        let cached = build_request_params(
            ProviderProtocol::Anthropic,
            ThinkingSwitch::None,
            None,
            true,
        )
        .expect("开启 Prompt Cache 时不应为空");
        assert_eq!(cached["cache_control"]["type"], "ephemeral");
    }
}
