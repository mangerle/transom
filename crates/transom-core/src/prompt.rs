/// 认证提示的语义类别。
///
/// 为什么不能只靠提示文本：SSH 在密码认证阶段不下发任何提示字符串，`ssh` 里那行
/// "password:" 是客户端自己打的，界面要能把它本地化；而 `echo = false` 只表示
/// "不要回显"，区分不了登录密码与私钥口令。服务器主动下发的 keyboard-interactive
/// 文本则必须原样展示，因此保留 `Text` 变体承载原始字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptKind {
    /// 服务器在 keyboard-interactive 中下发的提示，界面原样显示 `prompt`。
    #[default]
    Text,
    /// 登录密码，由客户端在缺少凭据时索取。
    Password,
    /// 加密私钥的口令，由客户端在私钥需要口令时索取。
    Passphrase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptInfo {
    pub prompt: String,
    pub echo: bool,
    /// 提示语义；`Text` 以外的类别由界面按本地化文案渲染标签。
    pub kind: PromptKind,
}

impl PromptInfo {
    /// 构造一个客户端主动索取的机密输入提示：不回显，文本由界面按 `kind` 生成。
    pub fn secret(kind: PromptKind) -> Self {
        Self {
            prompt: String::new(),
            echo: false,
            kind,
        }
    }
}
