//! 模型供应商、模型能力与 AI 全局配置契约。
//!
//! 设计原理（Why）：
//! - 采用“供应商（连接与协议）-> 模型列表（能力与窗口）”两层结构，同一中转站下的多模型共享地址与密钥；
//! - API Key 绝不以明文存入配置文件，统一通过稳定的供应商 ID 引用 OS Keyring（由 `transom-store` 托管），
//!   仅在运行时构造 [`ModelTarget`] 时注入。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AiError;
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
    #[serde(default)]
    pub models: Vec<ModelConfig>,
}

/// 生成不依赖展示名称的新供应商标识。
///
/// 显示名称允许重复和修改，不能作为 Keyring 与模型命名空间的主键；UUID
/// 提供跨重启稳定且无需查询现有配置即可避免碰撞的身份。代价是标识不可读，
/// 因此界面始终使用 [`ProviderConfig::name`] 展示。
pub fn new_provider_id() -> String {
    format!("provider-{}", Uuid::new_v4())
}

/// 按供应商与请求模型名生成模型稳定标识。
///
/// 模型请求名允许在不同供应商间重复，因此命名空间必须包含供应商标识。
pub fn provider_model_id(provider_id: &str, model_name: &str) -> String {
    format!("{provider_id}:{model_name}")
}

pub use crate::config_presets::{
    AcpAgentConfig, PRESETS, PresetModel, ProviderPreset, default_acp_presets, provider_from_preset,
};

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

#[cfg(test)]
mod tests {
    use super::{
        AiSettings, DEFAULT_MAX_TOKENS, PRESETS, new_provider_id, provider_from_preset,
        provider_model_id,
    };
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

    #[test]
    fn provider_ids_are_stable_namespaces_independent_of_display_names() {
        let first = new_provider_id();
        let second = new_provider_id();

        assert_ne!(first, second, "每次创建供应商都必须获得独立标识");
        assert!(first.starts_with("provider-"), "供应商标识应带稳定前缀");
        assert_eq!(
            provider_model_id(&first, "shared-model"),
            format!("{first}:shared-model")
        );
    }

    #[test]
    fn legacy_unimplemented_mcp_servers_field_is_ignored() {
        let settings: AiSettings = serde_json::from_value(serde_json::json!({
            "mcp_servers": [{
                "id": "legacy",
                "transport": { "type": "stdio", "command": "legacy-mcp" }
            }]
        }))
        .expect("旧配置中的未实现 MCP 字段不应阻止配置加载");

        assert!(settings.providers.is_empty());
    }
}
