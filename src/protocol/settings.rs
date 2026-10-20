//! Read-only settings surface (M4 step 1).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::connection::http::UnaryClient;
use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecretSlot {
    #[serde(default)]
    pub path: Vec<String>,
    #[serde(default)]
    pub set: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceView {
    pub ns: String,
    #[serde(default)]
    pub value: Value,
    #[serde(default)]
    pub applies: String,
    #[serde(default)]
    pub revision: i64,
    #[serde(default)]
    pub secrets: Vec<SecretSlot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SettingsDescribe {
    #[serde(default)]
    pub writable: bool,
    #[serde(default)]
    pub namespaces: Vec<NamespaceView>,
}

pub async fn describe(client: &UnaryClient) -> Result<SettingsDescribe, AppError> {
    let value = client.call("settings/describe", serde_json::json!({})).await?;
    Ok(serde_json::from_value(value)?)
}

pub async fn open_document(client: &UnaryClient) -> Result<(), AppError> {
    client
        .call("settings/openSettingsDocument", serde_json::json!({}))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_describe_shape() {
        let raw = serde_json::json!({
            "writable": true,
            "hasDocument": true,
            "namespaces": [{
                "ns": "llm-deepseek",
                "applies": "live",
                "revision": 3,
                "value": {"model": "deepseek-v4-pro"},
                "secrets": [{"path": ["apiKey"], "set": true}]
            }]
        });
        let describe: SettingsDescribe = serde_json::from_value(raw).unwrap();
        assert!(describe.writable);
        assert_eq!(describe.namespaces.len(), 1);
        assert_eq!(describe.namespaces[0].ns, "llm-deepseek");
        assert_eq!(describe.namespaces[0].revision, 3);
        assert_eq!(describe.namespaces[0].secrets[0].path, vec!["apiKey"]);
        assert!(describe.namespaces[0].secrets[0].set);
    }
}
