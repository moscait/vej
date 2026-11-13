use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
use std::time::Duration;

use crate::config::Config;
use crate::keep_alive::KeepAlive;

#[derive(Parser, Debug)]
#[command(
    name = "vej",
    version,
    about = "High-performance, local System-1 AI decision engine competing with Jev"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start the low-latency HTTP daemon with Jev-compatible API
    Serve(ServeArgs),

    /// Make a fast single-pass typed decision directly from CLI
    Decide(DecideArgs),

    /// Manage local models (list, download presets)
    Models(ModelsArgs),

    /// Manage configuration
    Config(ConfigArgs),

    /// Run vej as a per-user background service (launchd on macOS, systemd --user on Linux)
    Service(ServiceArgs),

    /// Manage access for clients on other machines
    Remote(RemoteArgs),
}

#[derive(Args, Debug)]
pub struct RemoteArgs {
    #[command(subcommand)]
    pub command: RemoteCommands,
}

#[derive(Subcommand, Debug)]
pub enum RemoteCommands {
    /// Manage bearer tokens for remote clients
    Token(TokenArgs),
}

#[derive(Args, Debug)]
pub struct TokenArgs {
    #[command(subcommand)]
    pub command: TokenCommands,
}

#[derive(Subcommand, Debug)]
pub enum TokenCommands {
    /// Create a token for a client and print it (it is shown only once)
    Create {
        /// Client name, e.g. laptop or ci-runner
        name: String,
    },
    /// List client names that have tokens
    List,
    /// Revoke a client's token (takes effect immediately, no restart needed)
    Revoke {
        /// Client name
        name: String,
    },
}

#[derive(Args, Debug)]
pub struct ServiceArgs {
    #[command(subcommand)]
    pub command: ServiceCommands,
}

#[derive(Subcommand, Debug)]
pub enum ServiceCommands {
    /// Install vej as a background service and start it
    Install(InstallArgs),
    /// Stop and remove the background service
    Uninstall,
    /// Start the installed service
    Start,
    /// Stop the installed service (it stays installed and starts again at next login)
    Stop,
    /// Show whether the service is installed and running
    Status,
}

#[derive(Args, Debug)]
pub struct InstallArgs {
    /// Options passed to `vej serve` by the service
    #[command(flatten)]
    pub serve: ServeArgs,

    /// Let launchd/systemd own the port and start vej on the first connection.
    /// Combine with --idle-exit so nothing stays resident while unused.
    #[arg(long)]
    pub on_demand: bool,

    /// Replace an existing installation
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct ServeArgs {
    /// Model to load (e.g. llama-3.2-1b, smollm2-1.7b, gemma-4-e2b, or custom .gguf path)
    #[arg(short, long)]
    pub model: Option<String>,

    /// Host interface to bind to (default: config `host`, 127.0.0.1)
    #[arg(long)]
    pub host: Option<String>,

    /// Port to listen on (default: config `port`, 8080)
    #[arg(short, long)]
    pub port: Option<u16>,

    /// How long an idle model stays loaded: e.g. 5m, 1h, 0 (unload after each request), -1 (never)
    #[arg(long, value_parser = parse_keep_alive, allow_hyphen_values = true)]
    pub keep_alive: Option<KeepAlive>,

    /// Maximum concurrent inferences (each allocates its own KV cache)
    #[arg(long)]
    pub max_concurrent: Option<usize>,

    /// Don't load the model at startup; load it on the first request instead
    #[arg(long)]
    pub no_preload: bool,

    /// Exit after this long with no requests, e.g. 30m (0 or -1 = never). Only useful with
    /// socket activation (`vej service install --on-demand`), which restarts vej on demand.
    #[arg(long, value_parser = parse_keep_alive, allow_hyphen_values = true)]
    pub idle_exit: Option<KeepAlive>,

    /// Also serve the remote API on this Unix socket (for an OpenZiti sidecar or TLS proxy)
    #[arg(long, value_name = "PATH")]
    pub remote_socket: Option<PathBuf>,

    /// Serve only the remote socket: open no TCP port (needs a remote socket). The local
    /// admin API and playground are then unavailable; stop vej with SIGTERM.
    #[arg(long)]
    pub no_tcp: bool,

    /// Don't require a bearer token on the remote API, for when another layer (e.g. OpenZiti)
    /// already authenticates callers. A token that is presented must still be valid.
    #[arg(long)]
    pub remote_token_optional: bool,

    /// Extra browser origin allowed to call the API (repeatable), e.g. http://localhost:3000.
    /// The daemon's own origin is always allowed.
    #[arg(long = "allow-origin", value_name = "ORIGIN")]
    pub allow_origins: Vec<String>,

    /// Number of GPU layers to offload (default: 99 for full Metal/CUDA acceleration)
    #[arg(long)]
    pub gpu_layers: Option<u32>,

    /// Number of CPU threads
    #[arg(short, long)]
    pub threads: Option<u32>,
}

/// Effective `serve` settings after applying CLI flags over the config file.
#[derive(Debug, Clone, PartialEq)]
pub struct ServeSettings {
    pub model: String,
    pub host: String,
    pub port: u16,
    /// Whether to listen on `host:port` at all (false with `--no-tcp`).
    pub tcp: bool,
    pub allowed_origins: Vec<String>,
    pub n_gpu_layers: u32,
    pub threads: u32,
    pub n_ctx: u32,
    pub keep_alive: KeepAlive,
    pub max_concurrent: usize,
    pub preload: bool,
    /// `Some` when the daemon should exit after this long without requests.
    pub idle_exit: Option<Duration>,
    pub remote: RemoteSettings,
}

/// Effective `[remote]` settings.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteSettings {
    pub socket: Option<PathBuf>,
    pub tokens_file: PathBuf,
    pub require_token: bool,
    pub allow_model_selection: bool,
    pub allow_keep_alive: bool,
}

impl ServeArgs {
    pub fn resolve(&self, config: &Config) -> ServeSettings {
        let allowed_origins = config
            .allowed_origins
            .iter()
            .chain(&self.allow_origins)
            .cloned()
            .collect();
        ServeSettings {
            model: self
                .model
                .clone()
                .unwrap_or_else(|| config.default_model.clone()),
            host: self.host.clone().unwrap_or_else(|| config.host.clone()),
            port: self.port.unwrap_or(config.port),
            tcp: !self.no_tcp,
            allowed_origins,
            n_gpu_layers: self.gpu_layers.unwrap_or(config.n_gpu_layers),
            threads: self.threads.unwrap_or(config.threads),
            n_ctx: config.n_ctx,
            keep_alive: self.keep_alive.unwrap_or(config.keep_alive),
            max_concurrent: self.max_concurrent.unwrap_or(config.max_concurrent),
            preload: config.preload && !self.no_preload,
            idle_exit: match self.idle_exit.unwrap_or(config.idle_exit) {
                KeepAlive::For(d) if !d.is_zero() => Some(d),
                KeepAlive::For(_) | KeepAlive::Forever => None,
            },
            remote: RemoteSettings {
                socket: self
                    .remote_socket
                    .clone()
                    .or_else(|| config.remote.socket.clone()),
                tokens_file: config.remote.tokens_path(),
                require_token: config.remote.require_token && !self.remote_token_optional,
                allow_model_selection: config.remote.allow_model_selection,
                allow_keep_alive: config.remote.allow_keep_alive,
            },
        }
    }
}

impl ServeSettings {
    /// Rejects combinations that would leave vej unreachable.
    pub fn validate(&self) -> anyhow::Result<()> {
        if !self.tcp && self.remote.socket.is_none() {
            anyhow::bail!(
                "--no-tcp needs a remote socket: pass --remote-socket <PATH> or set \
                 `socket` under [remote]"
            );
        }
        Ok(())
    }
}

fn parse_keep_alive(value: &str) -> Result<KeepAlive, String> {
    KeepAlive::parse(value).map_err(|e| e.to_string())
}

#[derive(Args, Debug)]
pub struct DecideArgs {
    /// Input state / text to classify (positional argument)
    #[arg(value_name = "STATE")]
    pub state_pos: Option<String>,

    /// Input state / text to classify (named option, e.g. --state "...")
    #[arg(short, long)]
    pub state: Option<String>,

    /// Comma-separated list of candidate choices (e.g. "refund,support,billing")
    #[arg(short, long, value_delimiter = ',')]
    pub choices: Vec<String>,

    /// Optional guidance instruction or classification task definition
    #[arg(short, long)]
    pub instruction: Option<String>,

    /// Model to use for the decision
    #[arg(short, long)]
    pub model: Option<String>,

    /// Softmax temperature
    #[arg(short, long, default_value_t = 1.0)]
    pub temperature: f32,

    /// Number of GPU layers to offload
    #[arg(long)]
    pub gpu_layers: Option<u32>,

    /// Number of CPU threads
    #[arg(long)]
    pub threads: Option<u32>,

    /// Output full JSON response
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ModelsArgs {
    #[command(subcommand)]
    pub command: ModelsCommands,
}

#[derive(Subcommand, Debug)]
pub enum ModelsCommands {
    /// List all supported models, availability, origin, and cache status
    List,

    /// Download a model preset from Hugging Face into the local cache
    Pull {
        /// Model name or alias (e.g. llama-3.2-1b, smollm2-1.7b, gemma-4-e2b)
        model: String,
    },
}

#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommands,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// Initialize a default vej.toml in the current directory
    Init {
        /// Overwrite if existing
        #[arg(short, long)]
        force: bool,
    },
    /// Print current config path
    Path,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::keep_alive::KeepAlive;
    use std::time::Duration;

    fn serve_args(argv: &[&str]) -> ServeArgs {
        let cli = Cli::try_parse_from(["vej", "serve"].iter().chain(argv)).unwrap();
        match cli.command {
            Commands::Serve(args) => args,
            other => panic!("expected serve, got {other:?}"),
        }
    }

    fn config() -> Config {
        Config {
            host: "0.0.0.0".into(),
            port: 9000,
            keep_alive: KeepAlive::For(Duration::from_secs(60)),
            max_concurrent: 3,
            preload: true,
            ..Config::default()
        }
    }

    #[test]
    fn serve_uses_config_values_when_flags_are_omitted() {
        let settings = serve_args(&[]).resolve(&config());
        assert_eq!(settings.host, "0.0.0.0");
        assert_eq!(settings.port, 9000);
        assert_eq!(settings.keep_alive, KeepAlive::For(Duration::from_secs(60)));
        assert_eq!(settings.max_concurrent, 3);
        assert!(settings.preload);
        assert_eq!(settings.model, Config::default().default_model);
    }

    #[test]
    fn serve_flags_override_config() {
        let args = serve_args(&[
            "--host",
            "127.0.0.1",
            "--port",
            "8181",
            "--keep-alive",
            "-1",
            "--max-concurrent",
            "1",
            "--no-preload",
            "-m",
            "llama-3.2-3b",
        ]);
        let settings = args.resolve(&config());
        assert_eq!(settings.host, "127.0.0.1");
        assert_eq!(settings.port, 8181);
        assert_eq!(settings.keep_alive, KeepAlive::Forever);
        assert_eq!(settings.max_concurrent, 1);
        assert!(!settings.preload);
        assert_eq!(settings.model, "llama-3.2-3b");
    }

    #[test]
    fn allowed_origins_combine_config_and_flags() {
        let config = Config {
            allowed_origins: vec!["http://localhost:3000".into()],
            ..Config::default()
        };
        let args = serve_args(&["--allow-origin", "http://wordstudy.local:9000"]);
        assert_eq!(
            args.resolve(&config).allowed_origins,
            ["http://localhost:3000", "http://wordstudy.local:9000"]
        );
    }

    #[test]
    fn idle_exit_is_disabled_by_default_and_by_zero_or_negative() {
        assert_eq!(serve_args(&[]).resolve(&Config::default()).idle_exit, None);
        assert_eq!(
            serve_args(&["--idle-exit", "0"])
                .resolve(&config())
                .idle_exit,
            None
        );
        assert_eq!(
            serve_args(&["--idle-exit", "-1"])
                .resolve(&config())
                .idle_exit,
            None
        );
        assert_eq!(
            serve_args(&["--idle-exit", "30m"])
                .resolve(&config())
                .idle_exit,
            Some(Duration::from_secs(1800))
        );
        let from_config = Config {
            idle_exit: KeepAlive::For(Duration::from_secs(600)),
            ..Config::default()
        };
        assert_eq!(
            serve_args(&[]).resolve(&from_config).idle_exit,
            Some(Duration::from_secs(600))
        );
    }

    #[test]
    fn service_install_accepts_serve_options() {
        let cli = Cli::try_parse_from([
            "vej",
            "service",
            "install",
            "--on-demand",
            "--idle-exit",
            "30m",
            "-m",
            "llama-3.2-1b",
            "--port",
            "47831",
        ])
        .unwrap();
        let Commands::Service(ServiceArgs {
            command: ServiceCommands::Install(install),
        }) = cli.command
        else {
            panic!("expected service install");
        };
        assert!(install.on_demand);
        assert!(!install.force);
        assert_eq!(install.serve.model.as_deref(), Some("llama-3.2-1b"));
        assert_eq!(install.serve.port, Some(47831));
    }

    #[test]
    fn remote_settings_combine_flag_and_config() {
        let mut config = config();
        config.remote.tokens_file = Some("/etc/vej/tokens".into());
        config.remote.allow_model_selection = true;

        let settings = serve_args(&["--remote-socket", "/tmp/vej-remote.sock"]).resolve(&config);

        assert_eq!(
            settings.remote.socket.as_deref(),
            Some(std::path::Path::new("/tmp/vej-remote.sock"))
        );
        assert_eq!(
            settings.remote.tokens_file,
            std::path::PathBuf::from("/etc/vej/tokens")
        );
        assert!(settings.remote.require_token);
        assert!(settings.remote.allow_model_selection);
        assert!(!settings.remote.allow_keep_alive);
    }

    #[test]
    fn tcp_listener_and_required_tokens_are_the_defaults() {
        let settings = serve_args(&[]).resolve(&config());
        assert!(settings.tcp);
        assert!(settings.remote.require_token);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn socket_only_mode_with_optional_remote_tokens() {
        let settings = serve_args(&[
            "--no-tcp",
            "--remote-socket",
            "/tmp/vej-remote.sock",
            "--remote-token-optional",
        ])
        .resolve(&config());

        assert!(!settings.tcp);
        assert!(!settings.remote.require_token);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn no_tcp_needs_a_remote_socket_from_flag_or_config() {
        let err = serve_args(&["--no-tcp"])
            .resolve(&config())
            .validate()
            .unwrap_err();
        assert!(err.to_string().contains("--remote-socket"), "{err}");

        let mut config = config();
        config.remote.socket = Some("/run/vej/remote.sock".into());
        assert!(
            serve_args(&["--no-tcp"])
                .resolve(&config)
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn config_can_also_make_remote_tokens_optional() {
        let mut config = config();
        config.remote.require_token = false;
        assert!(!serve_args(&[]).resolve(&config).remote.require_token);
    }

    #[test]
    fn remote_tokens_file_defaults_to_the_vej_config_dir() {
        let settings = serve_args(&[]).resolve(&Config::default());
        assert!(
            settings.remote.tokens_file.ends_with("vej/remote_tokens"),
            "{:?}",
            settings.remote.tokens_file
        );
        assert_eq!(settings.remote.socket, None);
    }

    #[test]
    fn parses_remote_token_commands() {
        let cli = Cli::try_parse_from(["vej", "remote", "token", "create", "laptop"]).unwrap();
        let Commands::Remote(RemoteArgs {
            command:
                RemoteCommands::Token(TokenArgs {
                    command: TokenCommands::Create { name },
                }),
        }) = cli.command
        else {
            panic!("expected remote token create");
        };
        assert_eq!(name, "laptop");
        assert!(Cli::try_parse_from(["vej", "remote", "token", "list"]).is_ok());
        assert!(Cli::try_parse_from(["vej", "remote", "token", "revoke", "laptop"]).is_ok());
    }

    #[test]
    fn serve_rejects_malformed_keep_alive() {
        assert!(Cli::try_parse_from(["vej", "serve", "--keep-alive", "soon"]).is_err());
    }
}
