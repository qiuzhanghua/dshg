//! Long-lived `$events` stream: approvals (waterfall) and forwarded emits.

use async_channel::Sender;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use url::Url;

use crate::connection::ws::{map_ws_error, mux_url};
use crate::error::AppError;
use crate::secrets::SessionCookie;

#[derive(Debug, Clone, PartialEq)]
pub enum EventsMessage {
    Ready { client_id: String },
    Approval {
        event_id: String,
        tool_name: String,
        reason: Option<String>,
    },
    Emit { event: String },
}

/// Reconnect forever (except for auth failures) and forward decoded messages.
pub async fn events_loop(
    base: &Url,
    cookie: &SessionCookie,
    tx: Sender<EventsMessage>,
) -> Result<(), AppError> {
    loop {
        match events_once(base, cookie, &tx).await {
            Ok(()) => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
            Err(AppError::Unauthorized) | Err(AppError::TrustFence(_)) => {
                return Err(AppError::Unauthorized);
            }
            Err(error) => {
                tracing::warn!(%error, "events stream dropped; reconnecting");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
}

async fn events_once(
    base: &Url,
    cookie: &SessionCookie,
    tx: &Sender<EventsMessage>,
) -> Result<(), AppError> {
    let ws_url = mux_url(base)?;
    let mut request = ws_url
        .as_str()
        .into_client_request()
        .map_err(|error| AppError::Transport(format!("bad websocket request: {error}")))?;
    request.headers_mut().insert(
        "cookie",
        cookie
            .header()
            .parse()
            .map_err(|error| AppError::LocalIo(format!("bad cookie header: {error}")))?,
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(map_ws_error)?;
    let open = json!({
        "type": "open",
        "streamId": "events",
        "endpoint": "$events",
        "payload": { "args": {} },
    });
    socket
        .send(Message::Text(open.to_string().into()))
        .await
        .map_err(map_ws_error)?;

    loop {
        let message = match socket.next().await {
            Some(Ok(message)) => message,
            Some(Err(error)) => return Err(map_ws_error(error)),
            None => return Ok(()),
        };
        if let Message::Text(text) = message {
            let frame: Value = serde_json::from_str(text.as_str())?;
            if frame.get("type").and_then(Value::as_str) == Some("item")
                && let Some(value) = frame.get("value")
                && let Some(message) = parse_events_value(value)
                && tx.send(message).await.is_err()
            {
                return Ok(());
            }
        }
    }
}

pub fn parse_events_value(value: &Value) -> Option<EventsMessage> {
    match value.get("type").and_then(Value::as_str)? {
        "ready" => Some(EventsMessage::Ready {
            client_id: value.get("clientId")?.as_str()?.to_string(),
        }),
        "waterfall" => {
            let event = value.get("event")?.as_str()?;
            if event != "approval/request" {
                return None;
            }
            let request = value.get("request")?.clone();
            let tool_name = request
                .get("toolName")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            let reason = request
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    request
                        .get("displayReason")
                        .and_then(|display| display.get("en"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                });
            Some(EventsMessage::Approval {
                event_id: value.get("eventId")?.as_str()?.to_string(),
                tool_name,
                reason,
            })
        }
        "emit" => Some(EventsMessage::Emit {
            event: value.get("event")?.as_str()?.to_string(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_ready_and_approval() {
        let ready = json!({"type":"ready","clientId":"c1","host":{"home":"/tmp"}});
        assert_eq!(
            parse_events_value(&ready),
            Some(EventsMessage::Ready {
                client_id: "c1".to_string()
            })
        );

        let approval = json!({
            "type": "waterfall",
            "event": "approval/request",
            "eventId": "e1",
            "agentId": "a1",
            "request": {
                "toolName": "bash",
                "reason": "run ls",
                "displayReason": { "en": "run ls" }
            }
        });
        assert_eq!(
            parse_events_value(&approval),
            Some(EventsMessage::Approval {
                event_id: "e1".to_string(),
                tool_name: "bash".to_string(),
                reason: Some("run ls".to_string()),
            })
        );
    }
}
