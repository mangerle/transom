use async_trait::async_trait;
use transom_core::{AppError, ErrorCode, PromptInfo, PromptKind};
use zeroize::Zeroizing;

#[async_trait]
pub trait AuthPrompter: Send + Sync {
    async fn keyboard_interactive(
        &self,
        name: &str,
        instructions: &str,
        prompts: &[PromptInfo],
    ) -> Result<Vec<String>, AppError>;

    /// 首次连接遇到未知主机密钥时，请求用户确认是否信任该指纹（TOFU）。
    ///
    /// 设计原理（Why）：
    /// - 默认实现返回错误而不是自动信任：无界面实现必须显式选择"自动信任"
    ///   或"直接失败"，防止新实现静默继承首连被 MITM 无感通过的漏洞；
    /// - 交互式实现（UI 提示器）覆写为弹出指纹确认，用户同意才返回 `Ok(true)`。
    ///
    /// # Errors
    /// 默认实现恒返回 `ErrorCode::HostKeyUnknown` 错误，表示当前提示器
    /// 不具备主机密钥确认能力。
    async fn confirm_host_key(
        &self,
        _host: &str,
        _port: u16,
        _key_type: &str,
        _fingerprint: &str,
    ) -> Result<bool, AppError> {
        Err(AppError::with_detail(
            ErrorCode::HostKeyUnknown,
            "当前认证提示器不支持主机密钥确认",
        ))
    }
}

pub struct PasswordPrompter {
    password: Zeroizing<String>,
}

impl PasswordPrompter {
    pub fn new(password: impl Into<String>) -> Self {
        Self {
            password: Zeroizing::new(password.into()),
        }
    }
}

#[async_trait]
impl AuthPrompter for PasswordPrompter {
    /// 无界面场景（测试、脚本化连接）按传统 TOFU 默认策略首连自动信任：
    /// 没有用户可问，失败会让所有自动化用例无法建立连接。生产交互路径
    /// 必须走 UI 提示器的确认弹窗，不会到达这里。
    async fn confirm_host_key(
        &self,
        _host: &str,
        _port: u16,
        _key_type: &str,
        _fingerprint: &str,
    ) -> Result<bool, AppError> {
        Ok(true)
    }

    async fn keyboard_interactive(
        &self,
        _name: &str,
        _instructions: &str,
        prompts: &[PromptInfo],
    ) -> Result<Vec<String>, AppError> {
        let mut answers = Vec::with_capacity(prompts.len());
        for prompt in prompts {
            // 优先按语义类别判定；Text 是服务器下发的自由文本，只能继续用关键词匹配。
            let wants_password = match prompt.kind {
                PromptKind::Password => true,
                PromptKind::Passphrase => false,
                PromptKind::Text => {
                    let lower = prompt.prompt.to_ascii_lowercase();
                    !prompt.echo && (lower.contains("password") || lower.contains("passcode"))
                }
            };
            answers.push(if wants_password {
                self.password.to_string()
            } else {
                String::new()
            });
        }
        if answers.iter().all(String::is_empty) {
            return Err(AppError::with_detail(
                ErrorCode::KeyboardInteractiveFailed,
                "server requested interactive answers that require a UI prompt",
            ));
        }
        Ok(answers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn password_prompter_answers_password_prompts() {
        let prompter = PasswordPrompter::new("secret");
        let prompts = vec![
            PromptInfo {
                prompt: "Password:".into(),
                echo: false,
                kind: PromptKind::Text,
            },
            PromptInfo {
                prompt: "Verification code:".into(),
                echo: true,
                kind: PromptKind::Text,
            },
        ];
        let answers = prompter
            .keyboard_interactive("", "", &prompts)
            .await
            .unwrap();
        assert_eq!(answers, vec!["secret".to_string(), String::new()]);
    }

    #[tokio::test]
    async fn password_prompter_rejects_unknown_prompts() {
        let prompter = PasswordPrompter::new("secret");
        let prompts = vec![PromptInfo {
            prompt: "Token:".into(),
            echo: true,
            kind: PromptKind::Text,
        }];
        assert!(
            prompter
                .keyboard_interactive("", "", &prompts)
                .await
                .is_err()
        );
    }
    /// TOFU 语义：未覆写的实现默认拒绝确认，PasswordPrompter 按无界面策略自动信任。
    #[tokio::test]
    async fn host_key_confirmation_semantics() {
        struct BarePrompter;
        #[async_trait]
        impl AuthPrompter for BarePrompter {
            async fn keyboard_interactive(
                &self,
                _: &str,
                _: &str,
                _: &[PromptInfo],
            ) -> Result<Vec<String>, AppError> {
                Ok(Vec::new())
            }
        }
        assert!(
            BarePrompter
                .confirm_host_key("host", 22, "ed25519", "fp")
                .await
                .is_err(),
            "未覆写的实现必须默认拒绝而不是静默信任"
        );

        let headless = PasswordPrompter::new("secret");
        assert!(
            headless
                .confirm_host_key("host", 22, "ed25519", "fp")
                .await
                .expect("无界面提示器应当自动信任"),
        );
    }

    /// 语义类别优先于文本匹配：我们自己生成的密码提示不带任何文本。
    #[tokio::test]
    async fn password_prompter_answers_by_kind_without_text() {
        let prompter = PasswordPrompter::new("secret");
        let prompts = vec![PromptInfo::secret(PromptKind::Password)];
        let answers = prompter
            .keyboard_interactive("", "", &prompts)
            .await
            .unwrap();
        assert_eq!(answers, vec!["secret".to_string()]);

        // 私钥口令不等于登录密码，不能拿密码去顶替。
        let prompts = vec![PromptInfo::secret(PromptKind::Passphrase)];
        assert!(
            prompter
                .keyboard_interactive("", "", &prompts)
                .await
                .is_err()
        );
    }
}
