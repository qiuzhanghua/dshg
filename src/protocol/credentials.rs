//! Credential references (P1): describe/set/unset write-only secrets.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::connection::http::UnaryClient;
use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CredentialInfo {
    #[serde(default)]
    pub configured: bool,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub writable: bool,
}

pub async fn describe(
    client: &UnaryClient,
    refs: Vec<String>,
) -> Result<HashMap<String, CredentialInfo>, AppError> {
    let value = client
        .call("credentials/describe", serde_json::json!({ "refs": refs }))
        .await?;
    Ok(serde_json::from_value(value)?)
}

pub async fn set(client: &UnaryClient, reference: &str, secret: &str) -> Result<(), AppError> {
    client
        .call(
            "credentials/set",
            serde_json::json!({ "ref": reference, "value": secret }),
        )
        .await?;
    Ok(())
}

pub async fn unset(client: &UnaryClient, reference: &str) -> Result<(), AppError> {
    client
        .call("credentials/unset", serde_json::json!({ "ref": reference }))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_credential_info() {
        let map: HashMap<String, CredentialInfo> = serde_json::from_str(
            r#"{"DEEPSEEK_API_KEY":{"configured":true,"source":"user","writable":true},"OTHER":{"configured":false}}"#,
        )
        .unwrap();
        assert!(map["DEEPSEEK_API_KEY"].configured);
        assert_eq!(map["DEEPSEEK_API_KEY"].source.as_deref(), Some("user"));
        assert!(!map["OTHER"].configured);
    }
}
