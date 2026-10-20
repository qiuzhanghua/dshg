use async_channel::Sender;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

use crate::connection::ws::{map_ws_error, mux_url};
use crate::error::AppError;
use crate::secrets::SessionCookie;

/// Open `session/follow` and forward every frame value to `frames`.
///
/// The caller keeps this future running for the lifetime of the session view;
/// dropping it (aborting the task) closes the socket.
pub async fn follow_session(
    base: &Url,
    cookie: &SessionCookie,
    session_id: &str,
    frames: Sender<Value>,
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

    let (mut socket, _response) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(map_ws_error)?;

    let stream_id = format!("follow-{session_id}");
    let open = json!({
        "type": "open",
        "streamId": stream_id,
        "endpoint": "session/follow",
        "payload": { "args": { "request": {
            "address": { "kind": "session", "sessionId": session_id },
            "assistantStream": true
        } } },
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
        match message {
            Message::Text(text) => {
                let frame: Value = serde_json::from_str(text.as_str())?;
                match frame.get("type").and_then(Value::as_str) {
                    Some("item") => {
                        if let Some(value) = frame.get("value")
                            && frames.send(value.clone()).await.is_err()
                        {
                            return Ok(());
                        }
                    }
                    Some("error") => {
                        let error = frame.get("error").cloned().unwrap_or(Value::Null);
                        return Err(AppError::Rpc(format!(
                            "session/follow failed: {error}"
                        )));
                    }
                    Some("end") => return Ok(()),
                    _ => {}
                }
            }
            Message::Close(_) => return Ok(()),
            _ => {}
        }
    }
}
