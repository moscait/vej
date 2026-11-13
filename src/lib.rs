mod access;
mod activation;
mod cli;
mod config;
pub mod engine;
mod idle;
mod keep_alive;
mod model_manager;
pub mod models;
mod remote;
mod server;
mod service;
#[cfg(test)]
mod test_support;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Commands, ConfigCommands, DecideArgs, ModelsCommands, ServeArgs};
use config::{Config, get_config_path, load_config, save_config};
use engine::{DecisionEngine, DecisionRequest, EngineLoader};
use model_manager::ModelManager;
use models::{RECOMMENDED_MODELS, find_model_spec, get_cache_dir, pull_model, resolve_model_path};
use remote::RemoteAuth;
use remote::tokens::TokenStore;
use std::path::Path;

pub async fn run() -> Result<()> {
    // Plain text when stderr is a log file (launchd) or journald rather than a terminal.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();

    let cli = Cli::parse();
    let config = load_config().unwrap_or_else(|e| {
        eprintln!("Warning: {e:#}. Falling back to default configuration.");
        Config::default()
    });

    match cli.command {
        Commands::Serve(args) => handle_serve(args, config).await,
        Commands::Decide(args) => handle_decide(args, config),
        Commands::Models(args) => handle_models(args).await,
        Commands::Config(args) => handle_config(args),
        Commands::Service(args) => service::handle(args, config).await,
        Commands::Remote(args) => remote::handle(args, &config),
    }
}

async fn handle_serve(args: ServeArgs, config: Config) -> Result<()> {
    let settings = args.resolve(&config);
    settings.validate()?;

    // Fail fast on a missing/undownloaded model even when loading lazily.
    resolve_model_path(&settings.model)
        .with_context(|| format!("Could not resolve model '{}'", settings.model))?;

    let loader = EngineLoader {
        n_gpu_layers: settings.n_gpu_layers,
        threads: settings.threads,
        n_ctx: settings.n_ctx,
        max_concurrent: settings.max_concurrent,
    };
    // Take the socket before loading anything so a busy port fails fast.
    let (listener, activated) = if settings.tcp {
        let (listener, source) = activation::listener(&settings.host, settings.port)?;
        if source.is_activated() {
            tracing::info!(?source, "Using the socket passed by the service manager");
        }
        (Some(listener), source.is_activated())
    } else {
        (None, false)
    };

    let remote = prepare_remote_access(&settings, listener.as_ref())?;

    let manager = ModelManager::new(loader, &settings.model, settings.keep_alive);
    // A socket-activated start may be a mere health probe; load on the first real request.
    if settings.preload && !activated {
        drop(manager.acquire(None, None).await?);
    }

    tracing::info!(
        keep_alive = %settings.keep_alive,
        max_concurrent = settings.max_concurrent,
        preload = settings.preload,
        "Model lifecycle configured"
    );
    let state = server::AppState::new(manager, settings.max_concurrent);
    let options = server::ServeOptions {
        host: settings.host.clone(),
        allowed_origins: settings.allowed_origins.clone(),
        idle_exit: settings.idle_exit,
        remote_auth: remote.auth,
        remote_policy: remote.policy,
        remote_socket: remote.socket,
    };
    server::run_server(state, listener, options).await
}

struct RemoteAccess {
    auth: std::sync::Arc<RemoteAuth>,
    policy: server::ClientPolicy,
    socket: Option<activation::OwnedUnixSocket>,
}

/// Validates remote access before any model loads: tokens must exist when remote clients
/// can connect, and the remote socket is bound now so conflicts fail fast.
fn prepare_remote_access(
    settings: &cli::ServeSettings,
    tcp: Option<&std::net::TcpListener>,
) -> Result<RemoteAccess> {
    let remote = &settings.remote;
    let public_tcp = match tcp {
        Some(listener) => {
            let addr = listener.local_addr()?;
            (!addr.ip().to_canonical().is_loopback()).then_some(addr)
        }
        None => None,
    };
    let auth = RemoteAuth::new(TokenStore::open(&remote.tokens_file), remote.require_token);
    if public_tcp.is_some() || remote.socket.is_some() {
        auth.ensure_usable()?;
    }
    if let Some(tcp_addr) = public_tcp {
        tracing::warn!(
            %tcp_addr,
            "Listening beyond loopback: remote peers get the restricted API and need a bearer \
             token. Traffic is plain HTTP; prefer OpenZiti or a TLS proxy on --remote-socket."
        );
    }
    if !remote.require_token {
        tracing::warn!(
            "Remote bearer tokens are optional (--remote-token-optional or [remote] require_token = false): vej does not authenticate remote callers itself"
        );
    }
    let socket = remote
        .socket
        .as_deref()
        .map(activation::bind_unix_socket)
        .transpose()?;
    Ok(RemoteAccess {
        auth,
        policy: server::ClientPolicy {
            allow_model_selection: remote.allow_model_selection,
            allow_keep_alive: remote.allow_keep_alive,
            expose_internal_errors: false,
        },
        socket,
    })
}

fn handle_decide(args: DecideArgs, config: Config) -> Result<()> {
    if args.choices.is_empty() {
        eprintln!(
            "Error: At least one candidate choice must be provided via -c or --choices (e.g. -c 'refund,support,billing')"
        );
        std::process::exit(1);
    }

    let model_query = args.model.as_deref().unwrap_or(&config.default_model);

    let model_path = resolve_model_path(model_query)
        .with_context(|| format!("Could not resolve model '{}'", model_query))?;

    let spec = find_model_spec(model_query);
    let id_str = spec.map(|s| s.id).unwrap_or(model_query).to_string();
    let tmpl = spec.map(|s| s.template);

    let n_gpu_layers = args.gpu_layers.unwrap_or(config.n_gpu_layers);
    let threads = args.threads.unwrap_or(config.threads);

    let engine = DecisionEngine::load(
        &model_path,
        id_str,
        tmpl,
        n_gpu_layers,
        threads,
        config.n_ctx,
    )?;

    use std::io::IsTerminal;
    let state = if let Some(s) = args.state {
        s
    } else if let Some(s) = args.state_pos {
        s
    } else if !std::io::stdin().is_terminal() {
        use std::io::Read;
        let mut buffer = String::new();
        std::io::stdin().read_to_string(&mut buffer)?;
        buffer.trim().to_string()
    } else {
        eprintln!(
            "Error: Input state must be provided either as a positional argument, via --state, or piped into stdin."
        );
        std::process::exit(1);
    };

    if state.trim().is_empty() {
        eprintln!("Error: Input state cannot be empty.");
        std::process::exit(1);
    }

    let req = DecisionRequest {
        model: Some(model_query.to_string()),
        state: serde_json::Value::String(state),
        choices: Some(args.choices),
        instruction: args.instruction,
        questions: None,
        temperature: args.temperature,
        keep_alive: None,
    };

    let res = engine.evaluate_unified(&req)?;

    match res {
        engine::UnifiedResponse::Flat(flat) => {
            if args.json {
                println!("{}", serde_json::to_string_pretty(&flat)?);
            } else {
                println!("\n⚡ Decision: \x1b[1;32m{}\x1b[0m", flat.decision);
                println!(
                    "🎯 Confidence: \x1b[1;36m{:.2}%\x1b[0m",
                    flat.confidence * 100.0
                );
                println!(
                    "⏱️  Latency: \x1b[1;33m{:.2} ms\x1b[0m ({} tokens evaluated)",
                    flat.latency_ms, flat.tokens_evaluated
                );
                println!("🤖 Model: {}", flat.model);
                println!("\n📊 Distribution:");
                let mut sorted_probs: Vec<_> = flat.probabilities.iter().collect();
                sorted_probs.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap());
                for (choice, prob) in sorted_probs {
                    let bar_len = (prob * 30.0).round() as usize;
                    let bar = "█".repeat(bar_len);
                    println!("  • {:<16} {:>6.2}% {}", choice, prob * 100.0, bar);
                }
            }
        }
        engine::UnifiedResponse::Jev(jev) => {
            println!("{}", serde_json::to_string_pretty(&jev)?);
        }
    }

    Ok(())
}

async fn handle_models(args: cli::ModelsArgs) -> Result<()> {
    match args.command {
        ModelsCommands::List => {
            println!("\n📦 Supported Models Catalog (US & Allied Nations):\n");
            println!(
                "{:<16} {:<24} {:<18} {:<10} {:<12}",
                "ID", "NAME", "VENDOR (COUNTRY)", "SIZE", "LOCAL STATUS"
            );
            println!("{}", "─".repeat(82));

            let cache_dir = get_cache_dir();
            for m in RECOMMENDED_MODELS {
                let local_path = cache_dir.join(m.filename);
                let status = if local_path.exists() {
                    "\x1b[1;32m✓ Downloaded\x1b[0m"
                } else {
                    "\x1b[0;33mAvailable\x1b[0m"
                };
                let vendor_country = format!("{} ({})", m.vendor, m.country);
                println!(
                    "{:<16} {:<24} {:<18} {:<10} {:<12}",
                    m.id,
                    m.name,
                    vendor_country,
                    format!("~{:.2} GB", m.size_gb),
                    status
                );
            }
            println!("\n💡 Tip: Run `vej models pull <id>` to download a model to local cache.");
            println!("Cache location: {:?}", cache_dir);
        }
        ModelsCommands::Pull { model } => {
            pull_model(&model).await?;
        }
    }
    Ok(())
}

fn handle_config(args: cli::ConfigArgs) -> Result<()> {
    match args.command {
        ConfigCommands::Init { force } => {
            let path = Path::new("vej.toml");
            if path.exists() && !force {
                eprintln!("'vej.toml' already exists. Use --force to overwrite.");
                return Ok(());
            }
            let config = Config::default();
            save_config(&config, path)?;
            println!("Created default configuration file: vej.toml");
        }
        ConfigCommands::Path => {
            println!("Config path: {:?}", get_config_path());
        }
    }
    Ok(())
}
