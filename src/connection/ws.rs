use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{self, Message};
use url::Url;

use crate::error::AppError;
use crate::secrets::SessionCookie;

/// Facts published in the `$events` ready frame (PROTOCOL.md §1.2).
#[derive(Debug, Clone, PartialEq)]
pub struct EventsReady {
    pub client_id: String,
    pub home: Option<String>,
}

/// Build the single multiplexed stream socket URL: `ws://host:port/api/remote.mux`.
pub fn mux_url(base: &Url) -> Result<Url, AppError> {
    let mut url = base.join("api/remote.mux")?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        other => {
            return Err(AppError::LocalIo(format!(
                "unsupported base scheme {other:?}"
            )));
        }
    };
    url.set_scheme(scheme)
        .map_err(|_| AppError::LocalIo("could not convert to a websocket URL".to_string()))?;
    Ok(url)
}

fn map_ws_error(error: tungstenite::Error) -> AppError {
    match error {
        tungstenite::Error::Http(response) => match response.status() {
            reqwest::StatusCode::UNAUTHORIZED => AppError::Unauthorized,
            reqwest::StatusCode::FORBIDDEN => {
                AppError::TrustFence("websocket upgrade rejected".to_string())
            }
            status => AppError::Transport(format!("websocket upgrade returned HTTP {status}")),
        },
        other => AppError::Transport(format!("websocket: {other}")),
    }
}

/// Open the multiplexed socket, start the `$events` logical stream and wait for its
/// ready frame. Used by M0 T4 to prove cookie-authenticated upgrades work.
pub async fn wait_events_ready(
    base: &Url,
    cookie: &SessionCookie,
    timeout: Duration,
) -> Result<EventsReady, AppError> {
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

    let (mut socket, _response) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(map_ws_error)?;

    let open = json!({
        "type": "open",
        "streamId": "ev1",
        "endpoint": "$events",
        "payload": { "args": {} },
    });
    socket
        .send(Message::Text(open.to_string().into()))
        .await
        .map_err(map_ws_error)?;

    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let message = tokio::time::timeout_at(deadline, socket.next())
            .await
            .map_err(|_| AppError::Transport("timed out waiting for the events stream".to_string()))?;
        let message = message
            .ok_or_else(|| AppError::Transport("websocket closed before ready".to_string()))?
            .map_err(map_ws_error)?;
        match message {
            Message::Text(text) => {
                let frame: Value = serde_json::from_str(text.as_str())?;
                if let Some(ready) = parse_ready(&frame) {
                    return Ok(ready);
                }
                tracing::debug!(frame = %frame, "ignoring pre-ready frame");
            }
            Message::Close(_) => {
                return Err(AppError::Transport(
                    "websocket closed before ready".to_string(),
                ));
            }
            _ => {}
        }
    }
}

/// Recognize `{"type":"item","streamId":…,"value":{"type":"ready",…}}`.
pub fn parse_ready(frame: &Value) -> Option<EventsReady> {
    if frame.get("type")?.as_str()? != "item" {
        return None;
    }
    let value = frame.get("value")?;
    if value.get("type")?.as_str()? != "ready" {
        return None;
    }
    let client_id = value.get("clientId")?.as_str()?.to_string();
    let home = value
        .get("host")
        .and_then(|host| host.get("home"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(EventsReady { client_id, home })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_mux_url() {
        let url = mux_url(&Url::parse("http://127.0.0.1:52631/").unwrap()).unwrap();
        assert_eq!(url.as_str(), "ws://127.0.0.1:52631/api/remote.mux");
    }

    #[test]
    fn parses_measured_ready_frame() {
        let frame = json!({
            "type": "item",
            "streamId": "ev1",
            "value": {
                "type": "ready",
                "clientId": "779bb156-636a-4c82-abab-798bb9be4bb7",
                "host": { "home": "/Users/daniel" }
            }
        });
        let ready = parse_ready(&frame).unwrap();
        assert_eq!(ready.client_id, "779bb156-636a-4c82-abab-798bb9be4bb7");
        assert_eq!(ready.home.as_deref(), Some("/Users/daniel"));
    }

    #[test]
    fn ignores_other_frames() {
        assert!(parse_ready(&json!({"type": "item", "value": {"type": "emit"}})).is_none());
        assert!(parse_ready(&json!({"type": "open"})).is_none());
    }
}
