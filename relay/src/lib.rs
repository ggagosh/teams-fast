use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
pub struct RegisterClient {
    pub installation_id: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Registration {
    pub installation_id: String,
    pub notification_url: String,
    pub client_state: String,
    pub cursor: u64,
    /// Per-registration bearer for `events`/`unregister`; registering needs the signed-in user's ID token.
    #[serde(default)]
    pub client_key: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Change {
    pub chat_id: Option<String>,
    pub message_id: Option<String>,
    pub kind: String,
    /// Graph's encrypted resource data (rich notification). Only the desktop holds the key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<EncryptedContent>,
    /// The batch's validation tokens (JWTs), checked by the desktop before trusting `content`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tokens: Vec<String>,
}

/// Graph's `encryptedContent` shape; field names match Graph's camelCase on both hops.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedContent {
    pub data: String,
    pub data_signature: String,
    pub data_key: String,
    pub encryption_certificate_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Changes {
    pub cursor: u64,
    pub reset: bool,
    pub changes: Vec<Change>,
}

#[cfg(feature = "server")]
mod identity;
#[cfg(feature = "server")]
pub mod server;
