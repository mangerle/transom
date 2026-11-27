//! 模型供应商、模型能力与 AI 全局配置契约。
//!
//! 设计原理（Why）：
//! - 采用“供应商（连接与协议）-> 模型列表（能力与窗口）”两层结构，同一中转站下的多模型共享地址与密钥；
//! - API Key 绝不以明文存入配置文件，统一通过 `secret_key_id` 引用 OS Keyring（由 `transom-store` 托管），
//!   仅在运行时构造 [`ModelTarget`] 时注入。

use serde::{Deserialize, Serialize};

use crate::error::AiError;
use crate::mcp::McpServerConfig;
use crate::policy::ExecutionMode;
use crate::thinking::{ThinkingEffort, ThinkingSwitch, fallback_effort};

/// Anthropic Messages 协议必填 `max_tokens`，未填写时使用的安全默认值。
pub const DEFAULT_MAX_TOKENS: u64 = 8_192;

/// 默认单轮提问最大工具调用步数上限（防死循环烧 Token）。
pub const DEFAULT_MAX_TURNS: usize = 15;

/// 默认单次命令或工具执行超时时间（秒）。
pub const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 60;

/// 模型供应商通信协议。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderProtocol {
    /// OpenAI Chat Completions 协议（`/chat/completions`）。
    #[default]
    OpenAi,
    /// OpenAI Responses 协议（`/responses`）。
    #[serde(rename = "responses")]
    OpenAiResponses,
    /// Anthropic Messages 协议（`/v1/messages`）。
    Anthropic,
}

impl ProviderProtocol {
    pub const ALL: [Self; 3] = [Self::OpenAi, Self::OpenAiResponses, Self::Anthropic];

    /// 展示用名称。
    pub fn label(&self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI Chat Completions",
            Self::OpenAiResponses => "OpenAI Responses",
            Self::Anthropic => "Anthropic Messages",
        }
    }

    /// 短标识符。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::OpenAiResponses => "responses",
            Self::Anthropic => "anthropic",
        }
    }
}

/// 单个模型的配置与能力声明。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelConfig {
    #[serde(default)]
    pub id: String,
    /// 菜单展示名称（为空时回退使用 `name`）。
    #[serde(default)]
    pub display_name: String,
    /// 发给接口的模型标识名（如 `gpt-4o`、`deepseek-chat`）。
    #[serde(default)]
    pub name: String,
    /// 是否支持视觉输入（开启后允许剪贴板贴图）。
    #[serde(default)]
    pub vision: bool,
    /// 支持的思考强度档位；为空表示不发送思考参数。
    #[serde(default)]
    pub efforts: Vec<ThinkingEffort>,
    /// 最大输出 tokens；为空时 Anthropic 自动补 [`DEFAULT_MAX_TOKENS`]。
    #[serde(default)]
    pub max_tokens: Option<u64>,
    /// 上下文窗口大小估算（用于 Compaction 触发阈值判断）。
    #[serde(default)]
    pub context_window: Option<u32>,
}

impl ModelConfig {
    /// 返回菜单与底部状态条展示用的模型名称（未填显示名时回退实际请求模型名）。
    pub fn display_label(&self) -> &str {
        if self.display_name.trim().is_empty() {
            &self.name
        } else {
            &self.display_name
        }
    }

    /// 解析该模型实际生效的思考强度。
    pub fn resolve_effort(&self, wanted: Option<ThinkingEffort>) -> Option<ThinkingEffort> {
        fallback_effort(&self.efforts, wanted)
    }
}

/// 单个供应商配置（不保存明文 API Key）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub protocol: ProviderProtocol,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub thinking_switch: ThinkingSwitch,
    /// Keyring 中的凭证键名；为空时默认使用 `ai-provider:<id>`。
    #[serde(default)]
    pub secret_key_id: String,
    #[serde(default)]
    pub models: Vec<ModelConfig>,
}

impl ProviderConfig {
    /// 获取此供应商在 Keyring 中的标准存储键名。
    pub fn keyring_key(&self) -> String {
        if self.secret_key_id.trim().is_empty() {
            format!("ai-provider:{}", self.id)
        } else {
            self.secret_key_id.clone()
        }
    }
}

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

pub use crate::policy::RunningMessageBehavior;

/// 持久化存储的 AI 助手完整配置。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiSettings {
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub default_model: String,
    #[serde(default)]
    pub default_effort: Option<ThinkingEffort>,
    #[serde(default)]
    pub execution_mode: ExecutionMode,
    #[serde(default = "default_max_turns")]
    pub max_turns: usize,
    #[serde(default = "default_command_timeout")]
    pub command_timeout_secs: u64,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub acp_agents: Vec<AcpAgentConfig>,
    #[serde(default)]
    pub active_acp_agent_id: Option<String>,
    #[serde(default = "default_response_language")]
    pub response_language: String,
    #[serde(default)]
    pub disabled_skills: Vec<String>,
    #[serde(default)]
    pub exec_policy_rules: Vec<crate::exec_policy::ExecPolicyRule>,
    #[serde(default)]
    pub hooks: Vec<crate::hooks::HookRule>,
    #[serde(default)]
    pub running_message_behavior: RunningMessageBehavior,
}

fn default_response_language() -> String {
    "zh".to_owned()
}

/// 根据用户选择的回复语言代码生成给模型的语言指令。
pub fn language_instruction(lang: &str) -> Option<&'static str> {
    match lang.trim().to_ascii_lowercase().as_str() {
        "zh" => Some("请始终使用中文进行思考、分析与最终回复。"),
        "en" => Some("Please think, analyze, and respond in English."),
        _ => None,
    }
}

fn default_max_turns() -> usize {
    DEFAULT_MAX_TURNS
}

fn default_command_timeout() -> u64 {
    DEFAULT_COMMAND_TIMEOUT_SECS
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            default_model: String::new(),
            default_effort: None,
            execution_mode: ExecutionMode::default(),
            max_turns: DEFAULT_MAX_TURNS,
            command_timeout_secs: DEFAULT_COMMAND_TIMEOUT_SECS,
            mcp_servers: Vec::new(),
            acp_agents: Vec::new(),
            active_acp_agent_id: None,
            response_language: default_response_language(),
            disabled_skills: Vec::new(),
            exec_policy_rules: Vec::new(),
            hooks: Vec::new(),
            running_message_behavior: RunningMessageBehavior::default(),
        }
    }
}

impl AiSettings {
    /// 遍历所有已配置模型及其所属供应商。
    pub fn models(&self) -> impl Iterator<Item = (&ProviderConfig, &ModelConfig)> {
        self.providers
            .iter()
            .flat_map(|provider| provider.models.iter().map(move |model| (provider, model)))
    }

    /// 按模型 ID 查找精确匹配。
    pub fn find_model(&self, model_id: &str) -> Option<(&ProviderConfig, &ModelConfig)> {
        self.models().find(|(_, model)| model.id == model_id)
    }

    /// 按“指定模型 -> 默认模型 -> 首个可用模型”顺序解析模型。
    pub fn resolve_model(&self, model_id: &str) -> Option<(&ProviderConfig, &ModelConfig)> {
        self.find_model(model_id)
            .or_else(|| self.find_model(&self.default_model))
            .or_else(|| self.models().next())
    }

    /// 构造运行期目标模型；API Key 由外部凭证加载函数按需注入。
    pub fn build_target(
        &self,
        model_id: &str,
        effort: Option<ThinkingEffort>,
        api_key: String,
    ) -> Result<ModelTarget, AiError> {
        let Some((provider, model)) = self.resolve_model(model_id) else {
            return Err(AiError::ModelNotFound {
                model_id: model_id.to_owned(),
            });
        };
        let target = ModelTarget {
            protocol: provider.protocol,
            base_url: provider.base_url.clone(),
            api_key,
            thinking_switch: provider.thinking_switch,
            model: model.name.clone(),
            vision: model.vision,
            effort: model.resolve_effort(effort),
            max_tokens: model.max_tokens,
            context_window: model.context_window.unwrap_or(128_000),
        };
        target.validate_ready()?;
        Ok(target)
    }
}

/// 一场 Agent 运行实际消费的完整模型连接信息（仅在内存流转）。
#[derive(Clone, Default)]
pub struct ModelTarget {
    pub protocol: ProviderProtocol,
    pub base_url: String,
    pub api_key: String,
    pub thinking_switch: ThinkingSwitch,
    pub model: String,
    pub vision: bool,
    pub effort: Option<ThinkingEffort>,
    pub max_tokens: Option<u64>,
    pub context_window: u32,
}

impl std::fmt::Debug for ModelTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelTarget")
            .field("protocol", &self.protocol)
            .field("base_url", &self.base_url)
            .field("api_key", &"<已脱敏>")
            .field("model", &self.model)
            .field("vision", &self.vision)
            .field("effort", &self.effort)
            .finish()
    }
}

impl ModelTarget {
    /// 获取请求时实际携带的 `max_tokens`。
    pub fn request_max_tokens(&self) -> Option<u64> {
        self.max_tokens.or(match self.protocol {
            ProviderProtocol::Anthropic => Some(DEFAULT_MAX_TOKENS),
            ProviderProtocol::OpenAi | ProviderProtocol::OpenAiResponses => None,
        })
    }

    /// 检查是否具备发起请求所需的最小必要条件。
    pub fn validate_ready(&self) -> Result<(), AiError> {
        if self.api_key.trim().is_empty() {
            return Err(AiError::ConfigNotReady { field: "API Key" });
        }
        if self.model.trim().is_empty() {
            return Err(AiError::ConfigNotReady {
                field: "模型名称"
            });
        }
        Ok(())
    }
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
            id: format!("{provider_id}:{}", model.name),
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
        secret_key_id: format!("ai-provider:{provider_id}"),
        models,
    }
}

#[cfg(test)]
mod tests {
    use super::{AiSettings, DEFAULT_MAX_TOKENS, PRESETS, provider_from_preset};
    use crate::error::AiError;

    #[test]
    fn preset_resolves_target_and_masks_debug_secret() {
        let mut settings = AiSettings::default();
        let provider = provider_from_preset(&PRESETS[1], "test");
        let model_id = provider.models[0].id.clone();
        settings.providers.push(provider);

        let target = settings
            .build_target(&model_id, None, "sk-secret-123".to_owned())
            .expect("目标模型应构建成功");
        assert_eq!(target.request_max_tokens(), Some(DEFAULT_MAX_TOKENS));
        let debug_text = format!("{target:?}");
        assert!(!debug_text.contains("sk-secret-123"));
        assert!(debug_text.contains("已脱敏"));
    }

    #[test]
    fn missing_api_key_returns_config_not_ready() {
        let mut settings = AiSettings::default();
        let provider = provider_from_preset(&PRESETS[0], "1");
        let model_id = provider.models[0].id.clone();
        settings.providers.push(provider);

        let err = settings
            .build_target(&model_id, None, "   ".to_owned())
            .expect_err("空 API Key 应返回错误");
        assert!(matches!(err, AiError::ConfigNotReady { .. }));
    }
}
