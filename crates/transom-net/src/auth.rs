use async_trait::async_trait;
use transom_core::{AppError, ErrorCode, PromptInfo};
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
            let lower = prompt.prompt.to_ascii_lowercase();
            if !prompt.echo && (lower.contains("password") || lower.contains("passcode")) {
                answers.push(self.password.to_string());
            } else {
                answers.push(String::new());
            }
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
            },
            PromptInfo {
                prompt: "Verification code:".into(),
                echo: true,
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
        }];
        assert!(
            prompter
                .keyboard_interactive("", "", &prompts)
                .await
                .is_err()
        );
    }
}
