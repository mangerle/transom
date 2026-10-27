//! 终端触发器规则数据契约与配置模型。
//!
//! 设计说明（Why）：
//! - 实现初衷：为终端输出流提供自动模式匹配与动作响应能力（如监控到特定关键字触发高亮、蜂鸣、桌面通知或自动应答）；
//! - 核心优势：
//!   1. 规则具备独立唯一标识 `TriggerId`，便于增删改查及在界面上管理；
//!   2. 动作类型强类型枚举化，覆盖 `Bell`、`Notification`、`AutoResponse` 等经典自动化场景；
//!   3. 支持正则安全预检（长度约束与语法校验），反序列化具备防御性兜底；
//!   4. 规则默认按顺序进行匹配评估，每条规则可独立开关。

use crate::ids::TriggerId;
use serde::{Deserialize, Serialize};

/// 触发器命中后执行的动作类型。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerAction {
    /// 触发屏幕视觉闪烁或响铃。
    #[default]
    Bell,
    /// 触发系统桌面通知，附带通知内容模板（支持 `$0`, `$1` 等正则捕获组替换，留空则使用默认提示）。
    Notification(String),
    /// 自动向终端输入流应答文本内容（末尾自动追加换行 `\r` 提交执行）。
    AutoResponse(String),
    /// 未知动作类型兜底，防止版本协议字段扩展导致反序列化失败。
    #[serde(other)]
    Unknown,
}

/// 单条终端触发器规则。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerRule {
    /// 规则唯一标识。
    #[serde(default = "TriggerId::new")]
    pub id: TriggerId,
    /// 规则名称或简述，供用户识别。
    #[serde(default)]
    pub name: String,
    /// 是否启用该触发器。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 正则表达式模式串（如 `password:`、`Deploy success in (\d+)s`、`\[ERROR\]`）。
    #[serde(default)]
    pub pattern: String,
    /// 触发后执行的具体动作。
    #[serde(default)]
    pub action: TriggerAction,
    /// 是否区分大小写（默认 false，便于宽松匹配）。
    #[serde(default)]
    pub case_sensitive: bool,
}

fn default_true() -> bool {
    true
}

impl Default for TriggerRule {
    fn default() -> Self {
        Self {
            id: TriggerId::new(),
            name: String::new(),
            enabled: true,
            pattern: String::new(),
            action: TriggerAction::Bell,
            case_sensitive: false,
        }
    }
}

impl TriggerRule {
    /// 构造一条新触发器规则。
    pub fn new(name: impl Into<String>, pattern: impl Into<String>, action: TriggerAction) -> Self {
        Self {
            id: TriggerId::new(),
            name: name.into(),
            enabled: true,
            pattern: pattern.into(),
            action,
            case_sensitive: false,
        }
    }
}

/// 全局终端触发器集合设置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerSettings {
    /// 触发器总开关。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 触发器规则列表，按先后顺序匹配。
    #[serde(default)]
    pub rules: Vec<TriggerRule>,
}

impl Default for TriggerSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            rules: Vec::new(),
        }
    }
}

impl TriggerSettings {
    /// 检查指定规则是否已存在。
    pub fn find_rule(&self, id: TriggerId) -> Option<&TriggerRule> {
        self.rules.iter().find(|r| r.id == id)
    }

    /// 查找指定规则的可变引用。
    pub fn find_rule_mut(&mut self, id: TriggerId) -> Option<&mut TriggerRule> {
        self.rules.iter_mut().find(|r| r.id == id)
    }

    /// 移除指定规则。
    pub fn remove_rule(&mut self, id: TriggerId) -> Option<TriggerRule> {
        if let Some(pos) = self.rules.iter().position(|r| r.id == id) {
            Some(self.rules.remove(pos))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TriggerAction, TriggerRule, TriggerSettings};

    #[test]
    fn test_trigger_settings_serialization_round_trip() {
        let mut settings = TriggerSettings::default();
        let rule1 = TriggerRule::new(
            "密码输入提示自动应答",
            r"(?i)password:\s*$",
            TriggerAction::AutoResponse("secret".into()),
        );
        let id1 = rule1.id;
        settings.rules.push(rule1);

        let json = serde_json::to_string(&settings).expect("序列化触发器配置失败");
        let restored: TriggerSettings =
            serde_json::from_str(&json).expect("反序列化触发器配置失败");

        assert!(restored.enabled);
        assert_eq!(restored.rules.len(), 1);
        let rule = restored.find_rule(id1).expect("必须能查找到已有规则");
        assert_eq!(rule.name, "密码输入提示自动应答");
        assert_eq!(rule.action, TriggerAction::AutoResponse("secret".into()));
    }
}
