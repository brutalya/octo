use directories::ProjectDirs;
use std::path::PathBuf;

/// Single source of truth for configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub claude_bin: PathBuf,
    pub agy_bin: PathBuf,
    /// OpenRouter API root; overridable so tests can point at a fake server.
    pub openrouter_base: String,
}

impl Config {
    pub fn settings_path(&self) -> PathBuf {
        self.data_dir.join("settings.json")
    }

    /// Default binaries looked up in PATH; `data_dir` overrides the platform default.
    pub fn new(data_dir: Option<PathBuf>) -> Self {
        let data_dir = data_dir.unwrap_or_else(|| {
            ProjectDirs::from("", "", "octo")
                .map(|d| d.data_dir().to_path_buf())
                .unwrap_or_else(|| PathBuf::from("octo-data"))
        });
        Self {
            data_dir,
            claude_bin: "claude".into(),
            agy_bin: "agy".into(),
            openrouter_base: "https://openrouter.ai/api/v1".into(),
        }
    }
}
