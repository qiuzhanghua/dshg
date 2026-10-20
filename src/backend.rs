use std::sync::OnceLock;
use std::time::Duration;

use crate::connection::auth::{self, AuthTarget};
use crate::error::AppError;
use crate::secrets::SessionCookie;

/// Shared tokio runtime for all network work; the GPUI thread never blocks on it.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to start the network runtime")
    })
}

pub fn http_client() -> Result<reqwest::Client, AppError> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()?)
}

/// Parse the printed URL and exchange its token for a cookie.
pub async fn authenticate(input: &str) -> Result<(AuthTarget, SessionCookie), AppError> {
    let client = http_client()?;
    let target = auth::parse_auth_url(input)?;
    let cookie = auth::exchange_token(&client, &target).await?;
    Ok((target, cookie))
}
