use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::connection::http::UnaryClient;
use crate::error::AppError;

/// One row of `session/list` (PROTOCOL.md §5). Titles are not part of this payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: String,
    #[serde(default)]
    pub updated_at: Option<Value>,
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub blank: bool,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub agent_available: Option<bool>,
    #[serde(default)]
    pub parent_session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SessionListValue {
    #[serde(default)]
    items: Vec<SessionSummary>,
}

/// `session/list` requires its single parameter to be named `_request` (measured).
pub async fn session_list(client: &UnaryClient) -> Result<Vec<SessionSummary>, AppError> {
    let value = client
        .call("session/list", serde_json::json!({ "_request": {} }))
        .await?;
    let page: SessionListValue = serde_json::from_value(value)?;
    Ok(page.items)
}

/// Answer one `approval/request` waterfall with `allowed-once` or `rejected`.
pub async fn respond_approval(
    client: &UnaryClient,
    client_id: &str,
    event_id: &str,
    outcome: &str,
) -> Result<(), AppError> {
    let args = serde_json::json!({
        "clientId": client_id,
        "eventId": event_id,
        "outcome": { "kind": "result", "value": outcome },
    });
    client.call("$events/result", args).await?;
    Ok(())
}

/// Queue one text prompt on a session (M2).
pub async fn send_prompt(
    client: &UnaryClient,
    session_id: &str,
    request_id: &str,
    text: &str,
) -> Result<(), AppError> {
    let args = serde_json::json!({
        "request": {
            "requestId": request_id,
            "sessionId": session_id,
            "mode": "queue",
            "content": [{ "type": "text", "text": text }],
        }
    });
    client.call("session/prompt", args).await?;
    Ok(())
}

/// Create a session, optionally rooted at `cwd`, and return its id.
pub async fn create_session(
    client: &UnaryClient,
    cwd: Option<&str>,
) -> Result<String, AppError> {
    let request = match cwd {
        Some(cwd) => serde_json::json!({ "cwd": cwd }),
        None => serde_json::json!({}),
    };
    let args = serde_json::json!({ "request": request });
    let value = client.call("session/create", args).await?;
    value
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| AppError::Decode("session/create returned no sessionId".to_string()))
}

/// Lightweight readiness probe that every dsh 0.2.x build answers.
pub async fn settings_describe(client: &UnaryClient) -> Result<Value, AppError> {
    client.call("settings/describe", serde_json::json!({})).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_list_rows() {
        let value = serde_json::json!({
            "items": [{
                "sessionId": "sess-1",
                "updatedAt": 1750000000000_i64,
                "running": false,
                "blank": true,
                "cwd": "/tmp/work"
            }]
        });
        let page: SessionListValue = serde_json::from_value(value).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].session_id, "sess-1");
        assert_eq!(page.items[0].cwd.as_deref(), Some("/tmp/work"));
        assert!(page.items[0].parent_session_id.is_none());
    }

    #[test]
    fn tolerates_unknown_fields() {
        let value = serde_json::json!({
            "items": [{"sessionId": "s", "projections": {"kind": "cached"}, "future": 1}]
        });
        let page: SessionListValue = serde_json::from_value(value).unwrap();
        assert_eq!(page.items[0].session_id, "s");
    }
}
