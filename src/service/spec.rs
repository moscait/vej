//! Platform-neutral description of the background service, built from `vej service install`.

use anyhow::{Result, bail};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use crate::cli::{InstallArgs, ServeArgs};
use crate::config::Config;

/// Variables carried from the installing shell into the service environment.
const PASSTHROUGH_ENV: [&str; 3] = ["VEJ_CONFIG", "VEJ_CACHE_DIR", "RUST_LOG"];
/// Variables holding paths, which must be absolute once the service runs from `/`.
const PATH_ENV: [&str; 2] = ["VEJ_CONFIG", "VEJ_CACHE_DIR"];
const DEFAULT_LOG_FILTER: &str = "info";

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceSpec {
    pub program: PathBuf,
    /// Arguments after the program, starting with `serve`.
    pub args: Vec<String>,
    pub environment: Vec<(String, String)>,
    /// Address the service manager listens on for socket activation; `None` = always-on.
    pub on_demand: Option<SocketAddr>,
    /// Where stdout/stderr go (launchd); `None` leaves it to the manager (journald).
    pub log_file: Option<PathBuf>,
}

/// Builds the service definition. `env` reads the installing shell's environment and
/// `config_file` is the `--config` flag given to `vej service install`, if any.
pub fn build_spec(
    install: &InstallArgs,
    config: &Config,
    program: PathBuf,
    log_file: Option<PathBuf>,
    env: &dyn Fn(&str) -> Option<String>,
    config_file: Option<&Path>,
) -> Result<ServiceSpec> {
    let serve = &install.serve;
    let settings = serve.resolve(config);
    if !settings.tcp {
        bail!(
            "--no-tcp can't be installed as a vej service: on-demand start and `vej service \
             status` need the TCP port. Run `vej serve --no-tcp` under your own supervisor \
             (zvej does this for OpenZiti)."
        );
    }
    if settings.idle_exit.is_some() && !install.on_demand {
        bail!(
            "--idle-exit needs --on-demand: without socket activation nothing restarts vej \
             after it exits"
        );
    }
    let on_demand = if install.on_demand {
        Some(activation_address(&settings.host, settings.port)?)
    } else {
        None
    };
    Ok(ServiceSpec {
        program,
        args: serve_args(serve, &settings.host, settings.port, config_file),
        environment: service_environment(env),
        on_demand,
        log_file,
    })
}

/// Recovers `(host, port)` from installed `serve` arguments.
pub fn endpoint_from_args(args: &[String]) -> Option<(String, u16)> {
    let value_of = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
    };
    Some((
        value_of("--host")?.clone(),
        value_of("--port")?.parse().ok()?,
    ))
}

/// `serve` arguments: host and port always (the socket and server must agree), other
/// options only when given explicitly, so the service otherwise follows the config file.
fn serve_args(serve: &ServeArgs, host: &str, port: u16, config_file: Option<&Path>) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "serve".into(),
        "--host".into(),
        host.into(),
        "--port".into(),
        port.to_string(),
    ];
    // The service runs from `/`, so the config path must be absolute too.
    push_option(
        &mut args,
        "--config",
        config_file.map(|p| absolute(&p.display().to_string())),
    );
    let model = serve.model.as_deref().map(|m| {
        // The service runs from `/`, so relative model paths must be resolved now.
        if Path::new(m).exists() {
            absolute(m)
        } else {
            m.to_string()
        }
    });
    push_option(&mut args, "--model", model);
    push_option(
        &mut args,
        "--keep-alive",
        serve.keep_alive.map(|k| k.to_string()),
    );
    push_option(
        &mut args,
        "--max-concurrent",
        serve.max_concurrent.map(|n| n.to_string()),
    );
    if serve.no_preload {
        args.push("--no-preload".into());
    }
    push_option(
        &mut args,
        "--idle-exit",
        serve.idle_exit.map(|k| k.to_string()),
    );
    push_option(
        &mut args,
        "--gpu-layers",
        serve.gpu_layers.map(|n| n.to_string()),
    );
    push_option(&mut args, "--threads", serve.threads.map(|n| n.to_string()));
    push_option(
        &mut args,
        "--remote-socket",
        serve
            .remote_socket
            .as_deref()
            .map(|p| absolute(&p.display().to_string())),
    );
    if serve.remote_token_optional {
        args.push("--remote-token-optional".into());
    }
    for origin in &serve.allow_origins {
        push_option(&mut args, "--allow-origin", Some(origin.clone()));
    }
    args
}

fn push_option(args: &mut Vec<String>, flag: &str, value: Option<String>) {
    if let Some(value) = value {
        args.push(flag.to_string());
        args.push(value);
    }
}

fn service_environment(env: &dyn Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    let mut vars: Vec<(String, String)> = PASSTHROUGH_ENV
        .iter()
        .filter_map(|&name| {
            let value = env(name)?;
            let value = if PATH_ENV.contains(&name) {
                absolute(&value)
            } else {
                value
            };
            Some((name.to_string(), value))
        })
        .collect();
    if env("RUST_LOG").is_none() {
        vars.push(("RUST_LOG".to_string(), DEFAULT_LOG_FILTER.to_string()));
    }
    vars
}

fn absolute(path: &str) -> String {
    std::path::absolute(Path::new(path))
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.to_string())
}

fn activation_address(host: &str, port: u16) -> Result<SocketAddr> {
    let ip = if host.eq_ignore_ascii_case("localhost") {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    } else {
        match host.trim_matches(['[', ']']).parse::<IpAddr>() {
            Ok(ip) => ip,
            Err(_) => {
                bail!("--on-demand needs an IP address or 'localhost' as --host (got '{host}')")
            }
        }
    };
    Ok(SocketAddr::new(ip, port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands, ServiceArgs, ServiceCommands};
    use clap::Parser;
    use std::collections::HashMap;

    fn install_args(argv: &[&str]) -> InstallArgs {
        let cli = Cli::try_parse_from(["vej", "service", "install"].iter().chain(argv)).unwrap();
        match cli.command {
            Commands::Service(ServiceArgs {
                command: ServiceCommands::Install(args),
            }) => args,
            other => panic!("expected service install, got {other:?}"),
        }
    }

    fn spec_with_env(argv: &[&str], env: &[(&str, &str)]) -> Result<ServiceSpec> {
        let env: HashMap<String, String> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        build_spec(
            &install_args(argv),
            &Config::default(),
            PathBuf::from("/opt/vej/bin/vej"),
            Some(PathBuf::from("/Users/me/Library/Logs/vej/vej.log")),
            &|name| env.get(name).cloned(),
            None,
        )
    }

    fn spec(argv: &[&str]) -> ServiceSpec {
        spec_with_env(argv, &[]).unwrap()
    }

    #[test]
    fn always_pins_host_and_port_so_socket_and_server_agree() {
        let s = spec(&[]);
        assert_eq!(s.args, ["serve", "--host", "127.0.0.1", "--port", "8080"]);
        assert_eq!(s.on_demand, None);
        assert_eq!(s.program, PathBuf::from("/opt/vej/bin/vej"));
    }

    #[test]
    fn passes_through_only_explicitly_given_serve_options() {
        let s = spec(&[
            "-m",
            "llama-3.2-1b",
            "--port",
            "47831",
            "--keep-alive",
            "10m",
            "--max-concurrent",
            "1",
            "--no-preload",
            "--allow-origin",
            "http://localhost:3000",
            "--gpu-layers",
            "0",
            "--threads",
            "4",
        ]);
        assert_eq!(
            s.args,
            [
                "serve",
                "--host",
                "127.0.0.1",
                "--port",
                "47831",
                "--model",
                "llama-3.2-1b",
                "--keep-alive",
                "10m",
                "--max-concurrent",
                "1",
                "--no-preload",
                "--gpu-layers",
                "0",
                "--threads",
                "4",
                "--allow-origin",
                "http://localhost:3000",
            ]
        );
    }

    #[test]
    fn on_demand_listens_where_the_server_expects_and_carries_idle_exit() {
        let s = spec(&["--on-demand", "--idle-exit", "30m", "--port", "9000"]);
        assert_eq!(s.on_demand, Some("127.0.0.1:9000".parse().unwrap()));
        assert!(s.args.windows(2).any(|w| w == ["--idle-exit", "30m"]));

        let localhost = spec(&["--on-demand", "--host", "localhost"]);
        assert_eq!(localhost.on_demand, Some("127.0.0.1:8080".parse().unwrap()));
    }

    #[test]
    fn idle_exit_without_on_demand_is_rejected() {
        let err = spec_with_env(&["--idle-exit", "30m"], &[]).unwrap_err();
        assert!(err.to_string().contains("--on-demand"), "{err}");
    }

    #[test]
    fn on_demand_requires_an_ip_host() {
        let err = spec_with_env(&["--on-demand", "--host", "vej.example"], &[]).unwrap_err();
        assert!(err.to_string().contains("IP address"), "{err}");
    }

    #[test]
    fn carries_vej_environment_with_absolute_paths_and_defaults_logging() {
        let s =
            spec_with_env(&[], &[("VEJ_CACHE_DIR", "/Volumes/models"), ("HOME", "/x")]).unwrap();
        assert_eq!(
            s.environment,
            [
                ("VEJ_CACHE_DIR".to_string(), "/Volumes/models".to_string()),
                ("RUST_LOG".to_string(), "info".to_string()),
            ]
        );

        let relative = spec_with_env(&[], &[("VEJ_CONFIG", "vej.toml")]).unwrap();
        let (_, config_path) = &relative.environment[0];
        assert!(Path::new(config_path).is_absolute(), "{config_path}");

        let custom_log = spec_with_env(&[], &[("RUST_LOG", "debug")]).unwrap();
        assert_eq!(
            custom_log.environment,
            [("RUST_LOG".to_string(), "debug".to_string())]
        );
    }

    #[test]
    fn existing_relative_model_paths_become_absolute() {
        let s = spec(&["-m", "Cargo.toml"]);
        let model = &s.args[s.args.iter().position(|a| a == "--model").unwrap() + 1];
        assert!(Path::new(model).is_absolute(), "{model}");
    }

    #[test]
    fn passes_remote_socket_through_as_an_absolute_path() {
        let s = spec(&["--remote-socket", "run/remote.sock"]);
        let idx = s
            .args
            .iter()
            .position(|a| a == "--remote-socket")
            .expect("flag passed");
        assert!(
            Path::new(&s.args[idx + 1]).is_absolute(),
            "{}",
            s.args[idx + 1]
        );
        assert!(s.args[idx + 1].ends_with("run/remote.sock"));
    }

    #[test]
    fn passes_remote_token_optional_through() {
        let s = spec(&["--remote-socket", "/tmp/r.sock", "--remote-token-optional"]);
        assert!(s.args.iter().any(|a| a == "--remote-token-optional"));
        assert!(
            !spec(&[])
                .args
                .iter()
                .any(|a| a == "--remote-token-optional")
        );
    }

    #[test]
    fn refuses_socket_only_mode() {
        let err = spec_with_env(&["--no-tcp", "--remote-socket", "/tmp/r.sock"], &[]).unwrap_err();
        assert!(err.to_string().contains("--no-tcp"), "{err}");
    }

    #[test]
    fn config_flag_is_passed_to_the_service_as_an_absolute_path() {
        let s = build_spec(
            &install_args(&[]),
            &Config::default(),
            PathBuf::from("/opt/vej/bin/vej"),
            None,
            &|_| None,
            Some(Path::new("vej.toml")),
        )
        .unwrap();
        let idx = s
            .args
            .iter()
            .position(|a| a == "--config")
            .expect("--config passed through");
        assert!(
            Path::new(&s.args[idx + 1]).is_absolute(),
            "{}",
            s.args[idx + 1]
        );
        assert!(s.args[idx + 1].ends_with("vej.toml"));
        assert!(!spec(&[]).args.iter().any(|a| a == "--config"));
    }

    #[test]
    fn recovers_endpoint_from_installed_arguments() {
        let args: Vec<String> = [
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            "47831",
            "--no-preload",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            endpoint_from_args(&args),
            Some(("127.0.0.1".to_string(), 47831))
        );
        assert_eq!(endpoint_from_args(&["serve".to_string()]), None);
    }
}
