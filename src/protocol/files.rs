//! Read-only workspace file listing and text previews (P1).

use serde::{Deserialize, Serialize};

use crate::connection::http::UnaryClient;
use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DirectoryEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DirectoryListing {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub entries: Vec<DirectoryEntry>,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileText {
    #[serde(default)]
    pub absolute_path: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub lines: i64,
    #[serde(default)]
    pub eof: bool,
}

/// List direct children of `path` (workspace-relative, empty for the root).
pub async fn list(
    client: &UnaryClient,
    session_id: &str,
    path: &str,
) -> Result<DirectoryListing, AppError> {
    let value = client
        .call(
            "workspaceFiles/list",
            serde_json::json!({
                "workspaceFileScopeId": session_id,
                "path": path,
            }),
        )
        .await?;
    Ok(serde_json::from_value(value)?)
}

/// Read a text file by absolute path.
pub async fn read(
    client: &UnaryClient,
    session_id: &str,
    absolute_path: &str,
    offset: Option<i64>,
    limit: Option<i64>,
) -> Result<FileText, AppError> {
    let mut range = serde_json::json!({});
    if let Some(offset) = offset {
        range["offset"] = offset.into();
    }
    if let Some(limit) = limit {
        range["limit"] = limit.into();
    }
    let value = client
        .call(
            "workspaceFiles/read",
            serde_json::json!({
                "workspaceFileScopeId": session_id,
                "path": absolute_path,
                "range": range,
            }),
        )
        .await?;
    Ok(serde_json::from_value(value)?)
}

/// Watch one absolute path; frames are raw `{kind:"ready"|"change"}` values.
pub async fn watch(
    base: &url::Url,
    cookie: &crate::secrets::SessionCookie,
    session_id: &str,
    absolute_path: &str,
    frames: async_channel::Sender<serde_json::Value>,
) -> Result<(), AppError> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    use crate::connection::ws::{map_ws_error, mux_url};

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
    let open = serde_json::json!({
        "type": "open",
        "streamId": "files",
        "endpoint": "workspaceFiles/changes",
        "payload": { "args": {
            "workspaceFileScopeId": session_id,
            "path": absolute_path,
        } },
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
            let frame: serde_json::Value = serde_json::from_str(text.as_str())?;
            if frame.get("type").and_then(serde_json::Value::as_str) == Some("item")
                && let Some(value) = frame.get("value")
                && frames.send(value.clone()).await.is_err()
            {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_listing_and_file_text() {
        let listing: DirectoryListing = serde_json::from_str(
            r#"{"path":"src","entries":[{"name":"main.rs","type":"file","size":42},{"name":"app","type":"directory"}],"truncated":false}"#,
        )
        .unwrap();
        assert_eq!(listing.path, "src");
        assert_eq!(listing.entries.len(), 2);
        assert_eq!(listing.entries[0].size, Some(42));
        assert_eq!(listing.entries[1].kind, "directory");

        let text: FileText = serde_json::from_str(
            r#"{"absolutePath":"/tmp/a.txt","version":"v1","text":"hello","offset":1,"lines":1,"eof":true}"#,
        )
        .unwrap();
        assert_eq!(text.absolute_path, "/tmp/a.txt");
        assert_eq!(text.text, "hello");
        assert!(text.eof);
    }
}
