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
