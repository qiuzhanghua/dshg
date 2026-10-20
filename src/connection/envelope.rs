use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CLIENT_REQUEST: &str = "client-request";
pub const SERVER_RESPONSE: &str = "server-response";

/// Unary RPC request envelope (PROTOCOL.md §2.1).
#[derive(Debug, Clone, Serialize)]
pub struct ClientRequest<'a> {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(rename = "rpcId")]
    pub rpc_id: String,
    pub method: &'a str,
    pub payload: Value,
}

impl<'a> ClientRequest<'a> {
    pub fn new(rpc_id: impl Into<String>, method: &'a str, args: Value) -> Self {
        Self {
            kind: CLIENT_REQUEST,
            rpc_id: rpc_id.into(),
            method,
            payload: serde_json::json!({ "args": args }),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerResponse {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "rpcId")]
    pub rpc_id: String,
    pub result: RpcResult,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RpcResult {
    pub ok: bool,
    #[serde(default)]
    pub value: Option<Value>,
    #[serde(default)]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: Option<Value>,
}

impl ServerResponse {
    /// Unwrap the business value, mapping `ok:false` to [`crate::error::AppError::Rpc`].
    pub fn into_value(self) -> Result<Value, crate::error::AppError> {
        if self.result.ok {
            Ok(self.result.value.unwrap_or(Value::Null))
        } else {
            let error = self.result.error.unwrap_or(RpcError {
                code: "unknown".to_string(),
                message: "engine returned an error without details".to_string(),
                details: None,
            });
            Err(crate::error::AppError::Rpc(format!(
                "{}: {}",
                error.code, error.message
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_shape_matches_wire() {
        let request = ClientRequest::new(
            "r1",
            "session/list",
            serde_json::json!({ "_request": {} }),
        );
        let text = serde_json::to_string(&request).unwrap();
        assert_eq!(
            text,
            r#"{"type":"client-request","rpcId":"r1","method":"session/list","payload":{"args":{"_request":{}}}}"#
        );
    }

    #[test]
    fn response_ok() {
        let text = r#"{"type":"server-response","rpcId":"s1","result":{"ok":true,"value":{"items":[]}}}"#;
        let response: ServerResponse = serde_json::from_str(text).unwrap();
        assert_eq!(response.rpc_id, "s1");
        assert_eq!(response.into_value().unwrap(), serde_json::json!({"items": []}));
    }

    #[test]
    fn response_error() {
        let text = r#"{"type":"server-response","rpcId":"s2","result":{"ok":false,"error":{"code":"gateway/bad-request","message":"nope","details":{"endpoint":"x"}}}}"#;
        let error = serde_json::from_str::<ServerResponse>(text)
            .unwrap()
            .into_value()
            .unwrap_err();
        assert_eq!(error.code(), "rpc");
        assert!(error.to_string().contains("gateway/bad-request"));
    }
}
