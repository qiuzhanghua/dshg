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
