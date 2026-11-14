use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::keep_alive::KeepAlive;
use crate::models::{TemplateStyle, find_model_spec};

/// Concurrent inferences allowed by default; each allocates its own KV cache.
const DEFAULT_MAX_CONCURRENT: usize = 2;
/// A config file in the working directory used to be read; now it only triggers a warning.
const LOCAL_CONFIG_NAME: &str = "vej.toml";

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
    /// `[models.<name>]` overrides; see [`ModelConfig`].
    pub models: HashMap<String, ModelConfig>,
    /// Directory of the file this config was read from. Relative `[models.<name>] path`
    /// values resolve against it. `None` when running on defaults.
    #[serde(skip)]
    pub config_dir: Option<PathBuf>,
}

impl Config {
    /// `[models.<name>]` keys that aren't catalog models define custom names, which need
    /// `path`. Two keys must not name the same catalog model through different aliases.
    pub fn validate_model_overrides(&self) -> Result<()> {
        let mut seen: HashMap<String, &str> = HashMap::new();
        for (key, entry) in &self.models {
            if entry.is_empty() {
                continue;
            }
            let keys = entry.stray_keys();
            if !keys.is_empty() {
                return Err(anyhow!(
                    "[models.{key}] has unexpected keys ({}); only `path` and `template` are \
                     allowed. Model ids with dots must be quoted: [models.\"llama-3.2-1b\"]",
                    keys.join(", ")
                ));
            }
            let canonical = match find_model_spec(key) {
                Some(spec) => spec.id.to_string(),
                None => {
                    if entry.path.is_none() {
                        return Err(anyhow!(
                            "[models.{key}] is not a catalog model, so it needs \
                             `path = \"/path/to/model.gguf\"`"
                        ));
                    }
                    key.trim().to_lowercase()
                }
            };
            if let Some(other) = seen.insert(canonical.clone(), key) {
                return Err(anyhow!(
                    "[models.{key}] and [models.{other}] both configure '{canonical}'"
                ));
            }
        }
        Ok(())
    }
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

/// `[models.<name>]`: point a catalog model at another GGUF file, fix its chat template, or
/// define a new name for a custom file (`path` is then required). A relative `path` is
/// resolved against the config file's directory.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    pub path: Option<PathBuf>,
    /// `Llama3`, `ChatML`, `Gemma` or `Gemma4` (case-insensitive).
    #[serde(deserialize_with = "deserialize_template")]
    pub template: Option<TemplateStyle>,
    /// Anything else under the table. An unquoted dotted header such as
    /// `[models.llama-3.2-1b]` lands here as nested tables, so this is how that mistake is
    /// reported instead of silently configuring a model called `llama-3`.
    #[serde(flatten)]
    pub unknown: HashMap<String, toml::Value>,
}

impl ModelConfig {
    /// Unknown keys that carry a value. Empty nested tables (an old example file's
    /// `[models.llama-3.2-1b]` with every setting commented out) are ignored.
    fn stray_keys(&self) -> Vec<&str> {
        let mut keys: Vec<&str> = self
            .unknown
            .iter()
            .filter(|(_, value)| !matches!(value, toml::Value::Table(t) if t.is_empty()))
            .map(|(key, _)| key.as_str())
            .collect();
        keys.sort_unstable();
        keys
    }

    fn is_empty(&self) -> bool {
        self.path.is_none() && self.template.is_none() && self.stray_keys().is_empty()
    }
}

fn deserialize_template<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<TemplateStyle>, D::Error> {
    let name: Option<String> = Option::deserialize(deserializer)?;
    name.map(|n| n.parse::<TemplateStyle>().map_err(serde::de::Error::custom))
        .transpose()
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
            config_dir: None,
        }
    }
}

/// Where the configuration file comes from, in precedence order. The working directory is
/// never consulted: results would otherwise depend on where vej is started, and an untrusted
/// checkout could supply the host, tokens and models.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    /// `--config <PATH>`; the file must exist.
    Flag(PathBuf),
    /// `VEJ_CONFIG`; the file must exist.
    Env(PathBuf),
    /// `<user config dir>/vej/config.toml`; defaults apply when it is missing.
    UserDir(PathBuf),
    /// No user config directory could be determined; defaults apply.
    None,
}

impl ConfigSource {
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Flag(p) | Self::Env(p) | Self::UserDir(p) => Some(p),
            Self::None => None,
        }
    }

    /// Explicit sources must exist; the user config may be absent.
    pub fn is_explicit(&self) -> bool {
        matches!(self, Self::Flag(_) | Self::Env(_))
    }

    /// The rule that chose this source, for `vej config path` and error messages.
    pub fn rule(&self) -> &'static str {
        match self {
            Self::Flag(_) => "--config",
            Self::Env(_) => "VEJ_CONFIG",
            Self::UserDir(_) => "user config directory",
            Self::None => "no user config directory",
        }
    }
}

/// Precedence: `--config` > `VEJ_CONFIG` > `<user config dir>/vej/config.toml`. A blank
/// `VEJ_CONFIG` counts as unset.
pub fn resolve_config_path(
    explicit: Option<&Path>,
    env_var: Option<&str>,
    user_config_dir: Option<&Path>,
) -> ConfigSource {
    if let Some(path) = explicit {
        return ConfigSource::Flag(path.to_path_buf());
    }
    if let Some(value) = env_var.map(str::trim).filter(|v| !v.is_empty()) {
        return ConfigSource::Env(PathBuf::from(value));
    }
    match user_config_dir {
        Some(dir) => ConfigSource::UserDir(dir.join("vej").join("config.toml")),
        None => ConfigSource::None,
    }
}

/// [`resolve_config_path`] against the process environment.
pub fn config_source(explicit: Option<&Path>) -> ConfigSource {
    let env = std::env::var("VEJ_CONFIG").ok();
    resolve_config_path(explicit, env.as_deref(), dirs::config_dir().as_deref())
}

/// A `./vej.toml` in `cwd` used to be read; warn when it would silently be ignored now.
pub fn ignored_local_config_warning(source: &ConfigSource, cwd: &Path) -> Option<String> {
    if source.is_explicit() || !cwd.join(LOCAL_CONFIG_NAME).is_file() {
        return None;
    }
    Some(format!(
        "found ./{LOCAL_CONFIG_NAME} but it is no longer read; use --config ./{LOCAL_CONFIG_NAME} \
         (or `vej config init` to create a user config)"
    ))
}

pub fn load_config(source: &ConfigSource) -> Result<Config> {
    let Some(path) = source.path() else {
        return Ok(Config::default());
    };
    if !path.exists() {
        if source.is_explicit() {
            return Err(anyhow!(
                "config file {:?} (from {}) does not exist",
                path,
                source.rule()
            ));
        }
        return Ok(Config::default());
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config from {:?}", path))?;
    let mut config: Config = toml::from_str(&content)
        .with_context(|| format!("Failed to parse TOML config from {:?}", path))?;
    config.config_dir = std::path::absolute(path)
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    config
        .validate_model_overrides()
        .map_err(|e| anyhow!("{e} (in {:?})", path))?;
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

    use crate::models::TemplateStyle;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn scratch(test: &str) -> PathBuf {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("vej-config-{test}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn config_flag_beats_env_var_beats_user_dir() {
        let flag = Path::new("/flag/vej.toml");
        let user = Path::new("/home/me/.config");

        let source = resolve_config_path(Some(flag), Some("/env/vej.toml"), Some(user));
        assert_eq!(source, ConfigSource::Flag(flag.to_path_buf()));

        let source = resolve_config_path(None, Some("/env/vej.toml"), Some(user));
        assert_eq!(source, ConfigSource::Env(PathBuf::from("/env/vej.toml")));

        let source = resolve_config_path(None, None, Some(user));
        assert_eq!(
            source,
            ConfigSource::UserDir(user.join("vej").join("config.toml"))
        );

        assert_eq!(resolve_config_path(None, None, None), ConfigSource::None);
    }

    #[test]
    fn empty_env_var_counts_as_unset() {
        let user = Path::new("/home/me/.config");
        let source = resolve_config_path(None, Some("  "), Some(user));
        assert!(matches!(source, ConfigSource::UserDir(_)), "{source:?}");
    }

    #[test]
    fn explicit_config_that_does_not_exist_is_an_error() {
        let dir = scratch("explicit-missing");
        let missing = dir.join("nope.toml");
        for source in [
            ConfigSource::Flag(missing.clone()),
            ConfigSource::Env(missing.clone()),
        ] {
            let err = load_config(&source).unwrap_err().to_string();
            assert!(err.contains("nope.toml"), "{err}");
            assert!(err.contains(source.rule()), "{err}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_user_config_and_no_user_dir_fall_back_to_defaults() {
        let dir = scratch("user-missing");
        let config = load_config(&ConfigSource::UserDir(dir.join("config.toml"))).unwrap();
        assert_eq!(config.port, Config::default().port);
        assert_eq!(config.config_dir, None);
        assert_eq!(load_config(&ConfigSource::None).unwrap().port, 8080);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn comment_only_config_parses_as_defaults_and_records_its_directory() {
        let dir = scratch("comment-only");
        let path = dir.join("vej-config.toml");
        std::fs::write(&path, "# managed by wordstudy\n").unwrap();
        let config = load_config(&ConfigSource::Env(path)).unwrap();
        assert_eq!(config.default_model, Config::default().default_model);
        assert_eq!(config.config_dir.as_deref(), Some(dir.as_path()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_errors_are_reported_with_the_path() {
        let dir = scratch("parse-error");
        let path = dir.join("vej.toml");
        std::fs::write(&path, "port = \"eighty\"\n").unwrap();
        let err = load_config(&ConfigSource::Flag(path))
            .unwrap_err()
            .to_string();
        assert!(err.contains("vej.toml"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn template_names_are_case_insensitive_and_unknown_ones_fail() {
        let config: Config = toml::from_str(
            "[models.\"llama-3.2-1b\"]\ntemplate = \"chatml\"\n\
             [models.\"gemma-4-e2b\"]\ntemplate = \"GEMMA4\"",
        )
        .unwrap();
        assert_eq!(
            config.models["llama-3.2-1b"].template,
            Some(TemplateStyle::ChatML)
        );
        assert_eq!(
            config.models["gemma-4-e2b"].template,
            Some(TemplateStyle::Gemma4)
        );
        config.validate_model_overrides().unwrap();

        let err = toml::from_str::<Config>("[models.x]\ntemplate = \"mistral\"")
            .unwrap_err()
            .to_string();
        assert!(err.contains("mistral"), "{err}");
        assert!(err.contains("Llama3"), "{err}");
    }

    #[test]
    fn unquoted_dotted_model_keys_are_rejected_with_a_hint() {
        // TOML reads `[models.llama-3.2-1b]` as models -> "llama-3" -> "2-1b".
        let config: Config =
            toml::from_str("[models.llama-3.2-1b]\ntemplate = \"ChatML\"").unwrap();
        let err = config.validate_model_overrides().unwrap_err().to_string();
        assert!(err.contains("[models.llama-3]"), "{err}");
        assert!(err.contains("2-1b"), "{err}");
        assert!(err.contains("[models.\"llama-3.2-1b\"]"), "{err}");

        // An empty table (the old example file's commented-out sections) is harmless.
        let empty: Config = toml::from_str("[models.llama-3.2-1b]\n").unwrap();
        empty.validate_model_overrides().unwrap();
    }

    #[test]
    fn custom_model_without_a_path_fails_at_load() {
        let dir = scratch("custom-nopath");
        let path = dir.join("vej.toml");
        std::fs::write(&path, "[models.mistral]\ntemplate = \"ChatML\"\n").unwrap();
        let err = load_config(&ConfigSource::Flag(path))
            .unwrap_err()
            .to_string();
        assert!(err.contains("[models.mistral]"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn warns_about_an_ignored_local_vej_toml_only_without_explicit_config() {
        let dir = scratch("local-warning");
        assert_eq!(
            ignored_local_config_warning(&ConfigSource::None, &dir),
            None
        );

        std::fs::write(dir.join("vej.toml"), "port = 1\n").unwrap();
        let warning = ignored_local_config_warning(&ConfigSource::None, &dir).unwrap();
        assert!(warning.contains("--config ./vej.toml"), "{warning}");
        assert!(
            ignored_local_config_warning(&ConfigSource::UserDir(dir.join("c.toml")), &dir)
                .is_some()
        );
        assert_eq!(
            ignored_local_config_warning(&ConfigSource::Flag(dir.join("c.toml")), &dir),
            None
        );
        assert_eq!(
            ignored_local_config_warning(&ConfigSource::Env(dir.join("c.toml")), &dir),
            None
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
