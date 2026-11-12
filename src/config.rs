use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::keep_alive::KeepAlive;

/// Concurrent inferences allowed by default; each allocates its own KV cache.
const DEFAULT_MAX_CONCURRENT: usize = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub default_model: String,
    pub host: String,
    pub port: u16,
    pub threads: u32,
    pub n_gpu_layers: u32,
    pub n_ctx: u32,
    /// Extra browser origins allowed to call the daemon (e.g. "http://localhost:3000").
    /// The daemon's own origin and its loopback aliases are always allowed.
    pub allowed_origins: Vec<String>,
    /// How long an idle model stays loaded (e.g. "5m", "1h", 0 = unload immediately, -1 = never).
    pub keep_alive: KeepAlive,
    pub max_concurrent: usize,
    /// Load the default model at startup instead of on the first request.
    pub preload: bool,
    pub models: HashMap<String, ModelConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub path: Option<String>,
    pub template: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        let threads = std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(4);

        Self {
            default_model: "gemma-4-e2b".to_string(),
            host: "127.0.0.1".to_string(),
            port: 8080,
            threads,
            n_gpu_layers: 99,
            n_ctx: 2048,
            allowed_origins: Vec::new(),
            keep_alive: KeepAlive::default(),
            max_concurrent: DEFAULT_MAX_CONCURRENT,
            preload: true,
            models: HashMap::new(),
        }
    }
}

pub fn get_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("VEJ_CONFIG") {
        PathBuf::from(p)
    } else if Path::new("vej.toml").exists() {
        PathBuf::from("vej.toml")
    } else if let Some(config_dir) = dirs::config_dir() {
        config_dir.join("vej").join("config.toml")
    } else {
        PathBuf::from("vej.toml")
    }
}

pub fn load_config() -> Result<Config> {
    let path = get_config_path();
    if !path.exists() {
        return Ok(Config::default());
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config from {:?}", path))?;

    let config: Config = toml::from_str(&content)
        .with_context(|| format!("Failed to parse TOML config from {:?}", path))?;

    Ok(config)
}

pub fn save_config(config: &Config, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(config)?;
    std::fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keep_alive::KeepAlive;
    use std::time::Duration;

    #[test]
    fn config_files_written_before_lifecycle_settings_still_parse() {
        let legacy = r#"
            default_model = "llama-3.2-3b"
            host = "127.0.0.1"
            port = 9090
            threads = 8
            n_gpu_layers = 99
            n_ctx = 2048

            [models.llama-3.2-1b]
        "#;

        let config: Config = toml::from_str(legacy).unwrap();

        assert_eq!(config.default_model, "llama-3.2-3b");
        assert_eq!(config.port, 9090);
        assert_eq!(config.keep_alive, KeepAlive::default());
        assert_eq!(config.max_concurrent, Config::default().max_concurrent);
        assert!(config.preload);
    }

    #[test]
    fn partial_config_falls_back_to_defaults() {
        let config: Config = toml::from_str("port = 7070").unwrap();
        assert_eq!(config.port, 7070);
        assert_eq!(config.default_model, Config::default().default_model);
    }

    #[test]
    fn parses_lifecycle_settings() {
        let config: Config =
            toml::from_str("keep_alive = \"30m\"\nmax_concurrent = 4\npreload = false").unwrap();
        assert_eq!(config.keep_alive, KeepAlive::For(Duration::from_secs(1800)));
        assert_eq!(config.max_concurrent, 4);
        assert!(!config.preload);
    }

    #[test]
    fn default_config_round_trips_through_toml() {
        let text = toml::to_string_pretty(&Config::default()).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.keep_alive, KeepAlive::default());
        assert_eq!(parsed.port, Config::default().port);
    }
}
