use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// A dsh browser-session cookie, bound to the engine authority.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionCookie {
    pub authority: String,
    pub name: String,
    pub value: String,
    pub expires_at: i64,
}

impl SessionCookie {
    pub fn header(&self) -> String {
        format!("{}={}", self.name, self.value)
    }

    pub fn is_expired(&self, now: i64) -> bool {
        self.expires_at <= now
    }
}

pub trait SecretStore {
    fn load(&self) -> Result<Option<SessionCookie>, AppError>;
    fn save(&self, cookie: &SessionCookie) -> Result<(), AppError>;
    fn delete(&self) -> Result<(), AppError>;
}

const SERVICE: &str = "dshg";
const ACCOUNT: &str = "session-cookie";

/// Keychain-backed store (the production path on macOS).
pub struct KeychainStore {
    service: String,
    account: String,
}

impl Default for KeychainStore {
    fn default() -> Self {
        Self::new(SERVICE, ACCOUNT)
    }
}

impl KeychainStore {
    pub fn new(service: &str, account: &str) -> Self {
        Self {
            service: service.to_string(),
            account: account.to_string(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry, AppError> {
        Ok(keyring::Entry::new(&self.service, &self.account)?)
    }
}

impl SecretStore for KeychainStore {
    fn load(&self) -> Result<Option<SessionCookie>, AppError> {
        match self.entry()?.get_password() {
            Ok(text) => Ok(Some(serde_json::from_str(&text)?)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn save(&self, cookie: &SessionCookie) -> Result<(), AppError> {
        let text = serde_json::to_string(cookie)?;
        self.entry()?.set_password(&text)?;
        Ok(())
    }

    fn delete(&self) -> Result<(), AppError> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

/// In-memory store used when the Keychain is unavailable and in tests.
#[derive(Default)]
pub struct MemoryStore {
    cookie: Mutex<Option<SessionCookie>>,
}

impl SecretStore for MemoryStore {
    fn load(&self) -> Result<Option<SessionCookie>, AppError> {
        Ok(self.cookie.lock().expect("poisoned").clone())
    }

    fn save(&self, cookie: &SessionCookie) -> Result<(), AppError> {
        *self.cookie.lock().expect("poisoned") = Some(cookie.clone());
        Ok(())
    }

    fn delete(&self) -> Result<(), AppError> {
        *self.cookie.lock().expect("poisoned") = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SessionCookie {
        SessionCookie {
            authority: "127.0.0.1:3080".to_string(),
            name: "dsh-auth-abc".to_string(),
            value: "v1.payload.sig".to_string(),
            expires_at: 1_800_000_000,
        }
    }

    #[test]
    fn header_shape() {
        assert_eq!(sample().header(), "dsh-auth-abc=v1.payload.sig");
    }

    #[test]
    fn memory_store_roundtrip() {
        let store = MemoryStore::default();
        assert!(store.load().unwrap().is_none());
        store.save(&sample()).unwrap();
        assert_eq!(store.load().unwrap(), Some(sample()));
        store.delete().unwrap();
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn expiry() {
        let cookie = sample();
        assert!(!cookie.is_expired(1_700_000_000));
        assert!(cookie.is_expired(1_800_000_001));
    }
}
