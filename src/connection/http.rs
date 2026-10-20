use std::time::Duration;

use serde_json::Value;
use url::Url;

use crate::connection::envelope::{ClientRequest, SERVER_RESPONSE, ServerResponse};
use crate::error::AppError;
use crate::secrets::SessionCookie;

/// Minimal unary RPC client for the dsh Client API (PROTOCOL.md §2.1).
pub struct UnaryClient {
    client: reqwest::Client,
    base: Url,
    cookie: Option<SessionCookie>,
}

impl UnaryClient {
    pub fn new(base: Url, cookie: Option<SessionCookie>) -> Result<Self, AppError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self {
            client,
            base,
            cookie,
        })
    }

    pub fn with_cookie(mut self, cookie: SessionCookie) -> Self {
        self.cookie = Some(cookie);
        self
    }

    pub fn base(&self) -> &Url {
        &self.base
    }

    /// Call `POST /api/<method>` with `{args}` and return the business value.
    pub async fn call(&self, method: &str, args: Value) -> Result<Value, AppError> {
        let url = self.base.join(&format!("api/{method}")).map_err(|error| {
            AppError::Decode(format!("could not build endpoint URL for {method}: {error}"))
        })?;
        let rpc_id = uuid::Uuid::new_v4().to_string();
        let request = ClientRequest::new(rpc_id.clone(), method, args);
        let mut builder = self.client.post(url).json(&request);
        if let Some(cookie) = &self.cookie {
            builder = builder.header(reqwest::header::COOKIE, cookie.header());
        }
        let response = builder.send().await?;
        let status = response.status();
        match status {
            reqwest::StatusCode::UNAUTHORIZED => return Err(AppError::Unauthorized),
            reqwest::StatusCode::FORBIDDEN => {
                return Err(AppError::TrustFence(format!(
                    "{method} rejected by the trust fence"
                )));
            }
            reqwest::StatusCode::NOT_FOUND => {
                return Err(AppError::EndpointMissing(method.to_string()));
            }
            status if !status.is_success() => {
                return Err(AppError::Transport(format!(
                    "{method} returned HTTP {status}"
                )));
            }
            _ => {}
        }
        let text = response.text().await?;
        let envelope: ServerResponse = serde_json::from_str(&text)
            .map_err(|error| AppError::Decode(format!("{method}: {error}")))?;
        if envelope.kind != SERVER_RESPONSE {
            return Err(AppError::Decode(format!(
                "{method}: unexpected envelope type {:?}",
                envelope.kind
            )));
        }
        if envelope.rpc_id != rpc_id {
            return Err(AppError::Decode(format!(
                "{method}: rpcId mismatch (sent {rpc_id}, got {})",
                envelope.rpc_id
            )));
        }
        envelope.into_value()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_loopback_base() {
        let client = UnaryClient::new(
            Url::parse("http://127.0.0.1:3080/").unwrap(),
            None,
        )
        .expect("client");
        assert_eq!(client.base().as_str(), "http://127.0.0.1:3080/");
    }
}
