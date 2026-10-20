use thiserror::Error;

/// Error taxonomy shared by the transport, store and UI layers (M0-DESIGN §8).
#[derive(Debug, Error)]
pub enum AppError {
    #[error("engine not running: {0}")]
    EngineNotRunning(String),
    #[error("authentication required")]
    Unauthorized,
    #[error("request rejected by the trust fence: {0}")]
    TrustFence(String),
    #[error("engine endpoint missing: {0}")]
    EndpointMissing(String),
    #[error("transport failure: {0}")]
    Transport(String),
    #[error("engine returned an error: {0}")]
    Rpc(String),
    #[error("could not decode a response: {0}")]
    Decode(String),
    #[error("local storage failure: {0}")]
    LocalIo(String),
}

impl AppError {
    /// Stable machine-readable tag, also used by the UI copy mapping.
    pub fn code(&self) -> &'static str {
        match self {
            AppError::EngineNotRunning(_) => "engine-not-running",
            AppError::Unauthorized => "unauthorized",
            AppError::TrustFence(_) => "trust-fence",
            AppError::EndpointMissing(_) => "endpoint-missing",
            AppError::Transport(_) => "transport",
            AppError::Rpc(_) => "rpc",
            AppError::Decode(_) => "decode",
            AppError::LocalIo(_) => "local-io",
        }
    }
}

impl From<reqwest::Error> for AppError {
    fn from(error: reqwest::Error) -> Self {
        if error.is_connect() {
            AppError::EngineNotRunning(error.to_string())
        } else {
            AppError::Transport(error.to_string())
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        AppError::Decode(error.to_string())
    }
}

impl From<url::ParseError> for AppError {
    fn from(error: url::ParseError) -> Self {
        AppError::LocalIo(format!("invalid URL: {error}"))
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        AppError::LocalIo(error.to_string())
    }
}

impl From<keyring::Error> for AppError {
    fn from(error: keyring::Error) -> Self {
        AppError::LocalIo(format!("keychain: {error}"))
    }
}
