use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub default_model: String,
    pub host: String,
    pub port: u16,
    pub threads: u32,
    pub n_gpu_layers: u32,
    pub n_ctx: u32,
    /// Extra browser origins allowed to call the daemon (e.g. "http://localhost:3000").
    /// The daemon's own origin and its loopback aliases are always allowed.
    #[serde(default)]
    pub allowed_origins: Vec<String>,
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
