use serde::{Deserialize, Serialize};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use ts_rs::TS;

pub const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";
pub const DEFAULT_HISTORY_BUDGET: usize = 24_000;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub ollama_url: String,
    pub openrouter_api_key: Option<String>,
    pub history_char_budget: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ollama_url: DEFAULT_OLLAMA_URL.into(),
            openrouter_api_key: None,
            history_char_budget: DEFAULT_HISTORY_BUDGET,
        }
    }
}

/// The key is redacted so it can never end up in logs.
impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("ollama_url", &self.ollama_url)
            .field(
                "openrouter_api_key",
                &self.openrouter_api_key.as_ref().map(|_| "<set>"),
            )
            .field("history_char_budget", &self.history_char_budget)
            .finish()
    }
}

/// What the API returns: the key is masked.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SettingsView {
    pub ollama_url: String,
    pub openrouter_api_key: Option<String>,
    #[ts(type = "number")]
    pub history_char_budget: usize,
}

/// Absent field = unchanged. `openrouter_api_key: ""` clears the key.
#[derive(Debug, Default, Deserialize, TS)]
#[ts(export)]
pub struct SettingsPatch {
    pub ollama_url: Option<String>,
    pub openrouter_api_key: Option<String>,
    #[ts(type = "number | undefined")]
    pub history_char_budget: Option<usize>,
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("ollama_url must start with http:// or https://")]
    BadUrl,
    #[error("history_char_budget must be positive")]
    BadBudget,
    #[error("settings file: {0}")]
    Io(#[from] std::io::Error),
    #[error("settings file is not valid JSON")]
    Parse,
}

pub fn mask_key(key: &str) -> String {
    let n = key.chars().count();
    if n < 12 {
        return "...".into();
    }
    let head: String = key.chars().take(3).collect();
    let tail: String = key.chars().skip(n - 4).collect();
    format!("{head}...{tail}")
}

impl Settings {
    pub fn view(&self) -> SettingsView {
        SettingsView {
            ollama_url: self.ollama_url.clone(),
            openrouter_api_key: self.openrouter_api_key.as_deref().map(mask_key),
            history_char_budget: self.history_char_budget,
        }
    }

    /// Validates the whole patch before changing anything.
    pub fn apply(&mut self, p: SettingsPatch) -> Result<(), SettingsError> {
        let url = match p.ollama_url {
            Some(u) => {
                let u = u.trim().trim_end_matches('/').to_string();
                if !(u.starts_with("http://") || u.starts_with("https://")) || u.len() < 9 {
                    return Err(SettingsError::BadUrl);
                }
                Some(u)
            }
            None => None,
        };
        if p.history_char_budget == Some(0) {
            return Err(SettingsError::BadBudget);
        }
        if let Some(u) = url {
            self.ollama_url = u;
        }
        if let Some(k) = p.openrouter_api_key {
            let k = k.trim();
            self.openrouter_api_key = (!k.is_empty()).then(|| k.to_string());
        }
        if let Some(b) = p.history_char_budget {
            self.history_char_budget = b;
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, SettingsError> {
        match std::fs::read(path) {
            Ok(b) => serde_json::from_slice(&b).map_err(|_| SettingsError::Parse),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Writes atomically (temp file with mode 0600, then rename).
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(self).map_err(|_| SettingsError::Parse)?;
        let _ = std::fs::remove_file(&tmp);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}
