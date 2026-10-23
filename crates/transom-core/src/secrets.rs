use crate::ids::SessionId;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    Password,
    KeyPassphrase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretKey {
    pub session_id: SessionId,
    pub kind: SecretKind,
}

impl SecretKey {
    pub fn new(session_id: SessionId, kind: SecretKind) -> Self {
        Self { session_id, kind }
    }

    pub fn account(&self) -> String {
        let kind = match self.kind {
            SecretKind::Password => "password",
            SecretKind::KeyPassphrase => "key_passphrase",
        };
        format!("{}/{kind}", self.session_id)
    }
}

#[derive(Default)]
pub struct ConnectionSecrets {
    pub password: Option<Zeroizing<String>>,
    pub key_passphrase: Option<Zeroizing<String>>,
}
