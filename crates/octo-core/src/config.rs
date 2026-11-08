use directories::{BaseDirs, ProjectDirs};
use std::path::PathBuf;

/// Single source of truth for configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub claude_bin: PathBuf,
    pub agy_bin: PathBuf,
    /// OpenRouter API root; overridable so tests can point at a fake server.
    pub openrouter_base: String,
    /// Claude Code session storage (`<home>/.claude/projects`).
    pub claude_projects_dir: PathBuf,
    /// Antigravity CLI storage (`<home>/.gemini/antigravity-cli`).
    pub agy_dir: PathBuf,
    /// Address the local API is actually bound to; Claude's approval prompts call back here
    /// (`/mcp/<run token>`). `None`: no callback, `confirm` stays non-interactive.
    pub local_api: Option<std::net::SocketAddr>,
    /// How long an approval request waits for a decision before it is denied.
    pub approval_timeout: std::time::Duration,
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
        let home = BaseDirs::new()
            .map(|d| d.home_dir().to_path_buf())
            .unwrap_or_default();
        Self {
            data_dir,
            claude_projects_dir: home.join(".claude").join("projects"),
            agy_dir: home.join(".gemini").join("antigravity-cli"),
            claude_bin: "claude".into(),
            agy_bin: "agy".into(),
            openrouter_base: "https://openrouter.ai/api/v1".into(),
            local_api: None,
            approval_timeout: std::time::Duration::from_secs(600),
        }
    }
}
