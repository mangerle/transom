//! 模型供应商预设表与外部 ACP Agent 开箱即用预设。

use serde::{Deserialize, Serialize};

use crate::config::{ModelConfig, ProviderConfig, ProviderProtocol, provider_model_id};
use crate::thinking::ThinkingEffort;
use crate::thinking::ThinkingSwitch;

/// 外部 ACP（Agent Client Protocol）编程助手配置。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpAgentConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub install_command: String,
    #[serde(default)]
    pub default_model: String,
    #[serde(default)]
    pub available_models: Vec<String>,
    #[serde(default)]
    pub enabled: bool,
}

/// 返回常见外部 ACP Coding Agent 的开箱即用预设。
pub fn default_acp_presets() -> Vec<AcpAgentConfig> {
    vec![
        AcpAgentConfig {
            id: "claude-code".to_owned(),
            name: "Claude Code (ACP)".to_owned(),
            command: "claude-code-acp".to_owned(),
            args: Vec::new(),
            install_command: "mkdir -p \"$HOME/.transom/acp-tools\" && npm install -g --prefix \"$HOME/.transom/acp-tools\" @zed-industries/claude-code-acp".to_owned(),
            default_model: "claude-sonnet-4-5".to_owned(),
            available_models: vec![
                "claude-sonnet-4-5".to_owned(),
                "claude-opus-4-1".to_owned(),
                "claude-haiku-4-5".to_owned(),
            ],
            enabled: true,
        },
        AcpAgentConfig {
            id: "codex".to_owned(),
            name: "Codex (ACP)".to_owned(),
            command: "codex-acp".to_owned(),
            args: Vec::new(),
            install_command: "mkdir -p \"$HOME/.transom/acp-tools\" && npm install -g --prefix \"$HOME/.transom/acp-tools\" @zed-industries/codex-acp".to_owned(),
            default_model: "gpt-5".to_owned(),
            available_models: vec![
                "gpt-5".to_owned(),
                "gpt-5-mini".to_owned(),
                "o3".to_owned(),
            ],
            enabled: true,
        },
        AcpAgentConfig {
            id: "kimi".to_owned(),
            name: "Kimi CLI (ACP)".to_owned(),
            command: "kimi".to_owned(),
            args: vec!["acp".to_owned()],
            install_command: "uv tool install kimi-cli".to_owned(),
            default_model: "kimi-k2".to_owned(),
            available_models: vec![
                "kimi-k2".to_owned(),
                "kimi-k2-thinking".to_owned(),
            ],
            enabled: true,
        },
    ]
}

/// 供应商预设模板（用于设置页一键填充）。
#[derive(Clone, Copy, Debug)]
pub struct ProviderPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub protocol: ProviderProtocol,
    pub base_url: &'static str,
    pub thinking_switch: ThinkingSwitch,
    pub default_models: &'static [PresetModel],
}

/// 预设模型条目。
#[derive(Clone, Copy, Debug)]
pub struct PresetModel {
    pub name: &'static str,
    pub vision: bool,
    pub efforts: &'static [ThinkingEffort],
    pub context_window: u32,
}

/// 主流大模型供应商内置预设表。
pub const PRESETS: [ProviderPreset; 5] = [
    ProviderPreset {
        id: "openai",
        name: "OpenAI",
        protocol: ProviderProtocol::OpenAiResponses,
        base_url: "https://api.openai.com/v1",
        thinking_switch: ThinkingSwitch::None,
        default_models: &[
            PresetModel {
                name: "gpt-4.1",
                vision: true,
                efforts: &[],
                context_window: 1_000_000,
            },
            PresetModel {
                name: "o4-mini",
                vision: true,
                efforts: &[
                    ThinkingEffort::Low,
                    ThinkingEffort::Medium,
                    ThinkingEffort::High,
                ],
                context_window: 200_000,
            },
        ],
    },
    ProviderPreset {
        id: "anthropic",
        name: "Anthropic",
        protocol: ProviderProtocol::Anthropic,
        base_url: "https://api.anthropic.com",
        thinking_switch: ThinkingSwitch::None,
        default_models: &[PresetModel {
            name: "claude-sonnet-4-5",
            vision: true,
            efforts: &[
                ThinkingEffort::None,
                ThinkingEffort::Low,
                ThinkingEffort::Medium,
                ThinkingEffort::High,
            ],
            context_window: 200_000,
        }],
    },
    ProviderPreset {
        id: "deepseek",
        name: "DeepSeek",
        protocol: ProviderProtocol::OpenAi,
        base_url: "https://api.deepseek.com/v1",
        thinking_switch: ThinkingSwitch::ThinkingType,
        default_models: &[
            PresetModel {
                name: "deepseek-chat",
                vision: false,
                efforts: &[],
                context_window: 64_000,
            },
            PresetModel {
                name: "deepseek-reasoner",
                vision: false,
                efforts: &[ThinkingEffort::Medium, ThinkingEffort::High],
                context_window: 64_000,
            },
        ],
    },
    ProviderPreset {
        id: "zhipu",
        name: "智谱 GLM",
        protocol: ProviderProtocol::OpenAi,
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        thinking_switch: ThinkingSwitch::ThinkingType,
        default_models: &[PresetModel {
            name: "glm-4.5",
            vision: true,
            efforts: &[ThinkingEffort::None, ThinkingEffort::High],
            context_window: 128_000,
        }],
    },
    ProviderPreset {
        id: "qwen",
        name: "通义千问",
        protocol: ProviderProtocol::OpenAi,
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        thinking_switch: ThinkingSwitch::EnableThinking,
        default_models: &[PresetModel {
            name: "qwen-plus",
            vision: false,
            efforts: &[ThinkingEffort::None, ThinkingEffort::High],
            context_window: 128_000,
        }],
    },
];

/// 按预设 ID 生成一份可直接编辑的 [`ProviderConfig`]。
pub fn provider_from_preset(preset: &ProviderPreset, unique_suffix: &str) -> ProviderConfig {
    let provider_id = if unique_suffix.is_empty() {
        preset.id.to_owned()
    } else {
        format!("{}-{unique_suffix}", preset.id)
    };
    let models = preset
        .default_models
        .iter()
        .map(|model| ModelConfig {
            id: provider_model_id(&provider_id, model.name),
            display_name: String::new(),
            name: model.name.to_owned(),
            vision: model.vision,
            efforts: model.efforts.to_vec(),
            max_tokens: None,
            context_window: Some(model.context_window),
        })
        .collect();
    ProviderConfig {
        id: provider_id.clone(),
        name: preset.name.to_owned(),
        protocol: preset.protocol,
        base_url: preset.base_url.to_owned(),
        thinking_switch: preset.thinking_switch,
        models,
    }
}
