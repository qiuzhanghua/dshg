//! Plugin inventory and enable/disable (P1).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::connection::http::UnaryClient;
use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginEntry {
    #[serde(default)]
    pub entry_id: Option<String>,
    #[serde(default)]
    pub module_name: Option<String>,
    #[serde(default)]
    pub meta: Option<Value>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub fiber_phase: Option<String>,
}

impl PluginEntry {
    pub fn display_name(&self) -> String {
        self.meta
            .as_ref()
            .and_then(|meta| meta.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| {
                self.module_name
                    .as_deref()
                    .unwrap_or("(unnamed)")
                    .trim_start_matches("@deepseek-ai/")
                    .to_string()
            })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Inventory {
    #[serde(default)]
    pub management_available: Option<bool>,
    #[serde(default)]
    pub entries: Vec<PluginEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChangeResult {
    #[serde(default)]
    pub changed: bool,
    #[serde(default)]
    pub application: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub error: Option<Value>,
}

pub async fn inventory(client: &UnaryClient) -> Result<Inventory, AppError> {
    let value = client
        .call("pluginInventory/list", serde_json::json!({}))
        .await?;
    Ok(serde_json::from_value(value)?)
}

pub async fn set_enabled(
    client: &UnaryClient,
    entry_id: &str,
    enabled: bool,
) -> Result<ChangeResult, AppError> {
    let value = client
        .call(
            "pluginManager/setPluginEnabled",
            serde_json::json!({ "id": entry_id, "enabled": enabled }),
        )
        .await?;
    Ok(serde_json::from_value(value)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_inventory_and_change() {
        let inventory: Inventory = serde_json::from_str(
            r#"{"managementAvailable":true,"entries":[{"entryId":"e1","moduleName":"@deepseek-ai/dsh-tool-bash","meta":{"name":"Bash"},"enabled":true,"fiberPhase":"active"}]}"#,
        )
        .unwrap();
        assert_eq!(inventory.entries.len(), 1);
        assert_eq!(inventory.entries[0].display_name(), "Bash");

        let change: ChangeResult = serde_json::from_str(
            r#"{"changed":true,"application":"restart-required","stage":"enable","target":"e1","enabled":true}"#,
        )
        .unwrap();
        assert!(change.changed);
        assert_eq!(change.application.as_deref(), Some("restart-required"));
    }
}
