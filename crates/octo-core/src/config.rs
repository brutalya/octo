use directories::ProjectDirs;
use std::path::PathBuf;

/// Single source of truth for configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub claude_bin: PathBuf,
    pub agy_bin: PathBuf,
}

impl Config {
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
        }
    }
}
