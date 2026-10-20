use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// On-disk client config. Versioned so later milestones can migrate in place.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub connection: ConnectionConfig,
    pub ui: UiConfig,
    pub recent: RecentConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ConnectionConfig {
    pub base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiConfig {
    pub locale: String,
    pub theme: String,
    pub window: WindowConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WindowConfig {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RecentConfig {
    pub last_session_id: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            connection: ConnectionConfig::default(),
            ui: UiConfig::default(),
            recent: RecentConfig::default(),
        }
    }
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:3080".to_string(),
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            locale: "zh-CN".to_string(),
            theme: "system".to_string(),
            window: WindowConfig::default(),
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 1200.0,
            height: 800.0,
        }
    }
}

impl Default for RecentConfig {
    fn default() -> Self {
        Self {
            last_session_id: None,
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        if let Ok(dir) = std::env::var("DSHG_CONFIG_DIR") {
            return Path::new(&dir).join("config.json");
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Path::new(&home)
            .join("Library/Application Support/dshg")
            .join("config.json")
    }

    /// Load the config, tolerating a missing or corrupt file (fall back to defaults).
    pub fn load_or_default(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Config>(&text) {
                Ok(config) => config,
                Err(error) => {
                    tracing::warn!(%error, path = %path.display(), "ignoring invalid config");
                    Config::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "could not read config");
                Config::default()
            }
        }
    }

    /// Write atomically: temp file next to the target, then rename over it.
    pub fn save(&self, path: &Path) -> Result<(), AppError> {
        let parent = path
            .parent()
            .ok_or_else(|| AppError::LocalIo(format!("no parent for {}", path.display())))?;
        std::fs::create_dir_all(parent)?;
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("dshg-config-test-{name}-{}.json", std::process::id()))
    }

    #[test]
    fn defaults_roundtrip() {
        let path = temp_path("roundtrip");
        let config = Config::default();
        config.save(&path).expect("save");
        let loaded = Config::load_or_default(&path);
        assert_eq!(config, loaded);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        let path = temp_path("unknown");
        std::fs::write(
            &path,
            r#"{"version":1,"connection":{"baseUrl":"http://127.0.0.1:9"},"future":{"x":1}}"#,
        )
        .unwrap();
        let loaded = Config::load_or_default(&path);
        assert_eq!(loaded.connection.base_url, "http://127.0.0.1:9");
        assert_eq!(loaded.ui.locale, "zh-CN");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(Config::load_or_default(&path), Config::default());
        let _ = std::fs::remove_file(&path);
    }
}
