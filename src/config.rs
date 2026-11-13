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
    /// Exit after this long with no requests (0 or -1 = never). Meant for socket-activated
    /// services, which restart vej on the next connection.
    pub idle_exit: KeepAlive,
    /// Access for clients on other machines (non-loopback peers or the remote socket).
    pub remote: RemoteConfig,
    pub models: HashMap<String, ModelConfig>,
}

/// `[remote]`: how clients on other machines may use vej. Loopback clients are unaffected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteConfig {
    /// Unix socket serving the remote API, for an OpenZiti sidecar or a TLS reverse proxy.
    pub socket: Option<PathBuf>,
    /// Bearer-token digests (default: `<config dir>/vej/remote_tokens`).
    pub tokens_file: Option<PathBuf>,
    /// Require a bearer token from remote clients. Disable only when another layer (such as
    /// OpenZiti identities) authenticates callers.
    pub require_token: bool,
    /// Let remote requests name a catalog model, which can trigger a model switch.
    pub allow_model_selection: bool,
    /// Let remote requests set `keep_alive` (pin a model in memory, or unload it).
    pub allow_keep_alive: bool,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            socket: None,
            tokens_file: None,
            require_token: true,
            allow_model_selection: false,
            allow_keep_alive: false,
        }
    }
}

impl RemoteConfig {
    pub fn tokens_path(&self) -> PathBuf {
        self.tokens_file.clone().unwrap_or_else(|| {
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from(".vej"))
                .join("vej")
                .join("remote_tokens")
        })
    }
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
            idle_exit: KeepAlive::Forever,
            remote: RemoteConfig::default(),
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
    fn remote_access_is_locked_down_by_default() {
        let remote = Config::default().remote;
        assert_eq!(remote.socket, None);
        assert!(remote.require_token);
        assert!(!remote.allow_model_selection);
        assert!(!remote.allow_keep_alive);
    }

    #[test]
    fn parses_remote_section() {
        let config: Config = toml::from_str(
            "port = 8080\n[remote]\nsocket = \"/run/vej/remote.sock\"\nrequire_token = false\n\
             allow_keep_alive = true\ntokens_file = \"/etc/vej/tokens\"",
        )
        .unwrap();
        assert_eq!(
            config.remote.socket.as_deref(),
            Some(Path::new("/run/vej/remote.sock"))
        );
        assert_eq!(
            config.remote.tokens_file.as_deref(),
            Some(Path::new("/etc/vej/tokens"))
        );
        assert!(!config.remote.require_token);
        assert!(config.remote.allow_keep_alive);
        assert!(!config.remote.allow_model_selection);
    }

    #[test]
    fn default_config_round_trips_through_toml() {
        let text = toml::to_string_pretty(&Config::default()).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.keep_alive, KeepAlive::default());
        assert_eq!(parsed.port, Config::default().port);
    }
}
