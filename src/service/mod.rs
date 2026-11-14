//! `vej service`: run vej as a per-user background service — launchd on macOS,
//! `systemd --user` on Linux. Always-on services start at login; on-demand services let
//! the service manager own the port and start vej on the first connection.

pub mod launchd;
pub mod spec;
pub mod systemd;

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::cli::{InstallArgs, ServiceArgs, ServiceCommands};
use crate::config::Config;
use crate::model_resolver::ModelResolver;
pub use spec::ServiceSpec;
use spec::{build_spec, endpoint_from_args};

/// Name of the launchd socket entry, shared by the plist and the activation lookup.
pub const LAUNCHD_SOCKET_NAME: &str = "Listeners";
const STATUS_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(target_os = "macos")]
use launchd::agent as platform;
#[cfg(target_os = "linux")]
use systemd::user as platform;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    use super::{ServiceSpec, ServiceStatus};
    use anyhow::{Result, bail};
    use std::path::PathBuf;

    const UNSUPPORTED: &str = "`vej service` supports macOS (launchd) and Linux (systemd) so far; \
         on this platform run `vej serve` from your OS's startup mechanism instead";

    pub fn log_file() -> Option<PathBuf> {
        None
    }
    pub fn install(_: &ServiceSpec) -> Result<()> {
        bail!(UNSUPPORTED)
    }
    pub fn uninstall() -> Result<bool> {
        bail!(UNSUPPORTED)
    }
    pub fn start() -> Result<()> {
        bail!(UNSUPPORTED)
    }
    pub fn stop() -> Result<()> {
        bail!(UNSUPPORTED)
    }
    pub fn status() -> Result<ServiceStatus> {
        bail!(UNSUPPORTED)
    }
    pub fn post_install_hint() -> Option<&'static str> {
        None
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceStatus {
    pub installed: bool,
    /// Registered with the service manager (for on-demand: the socket is armed).
    pub loaded: bool,
    pub running: bool,
    pub pid: Option<u32>,
    pub on_demand: bool,
    pub args: Vec<String>,
    pub definition: PathBuf,
    /// Log file path, or the command that shows the logs.
    pub logs: String,
}

impl ServiceStatus {
    #[cfg_attr(not(unix), allow(dead_code))]
    fn not_installed(definition: PathBuf, logs: String) -> Self {
        Self {
            installed: false,
            loaded: false,
            running: false,
            pid: None,
            on_demand: false,
            args: Vec::new(),
            definition,
            logs,
        }
    }
}

/// `config_file` is the `--config` flag, carried into the installed service's arguments.
pub async fn handle(args: ServiceArgs, config: Config, config_file: Option<&Path>) -> Result<()> {
    match args.command {
        ServiceCommands::Install(install) => install_service(&install, &config, config_file),
        ServiceCommands::Uninstall => {
            if platform::uninstall()? {
                println!("✓ vej service stopped and removed");
            } else {
                println!("vej service is not installed");
            }
            Ok(())
        }
        ServiceCommands::Start => {
            platform::start()?;
            println!("✓ vej service started");
            Ok(())
        }
        ServiceCommands::Stop => {
            platform::stop()?;
            println!(
                "✓ vej service stopped (it starts again at next login, or run `vej service start`)"
            );
            Ok(())
        }
        ServiceCommands::Status => print_status().await,
    }
}

fn install_service(
    install: &InstallArgs,
    config: &Config,
    config_file: Option<&Path>,
) -> Result<()> {
    let settings = install.serve.resolve(config);
    ModelResolver::from_config(config)?
        .resolve(&settings.model)
        .with_context(|| format!("Could not resolve model '{}'", settings.model))?;
    let program = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .context("cannot locate the vej executable")?;
    let spec = build_spec(
        install,
        config,
        program,
        platform::log_file(),
        &|name| std::env::var(name).ok(),
        config_file,
    )?;

    let existing = platform::status()?;
    if existing.installed {
        if !install.force {
            bail!(
                "vej service is already installed ({}). Use --force to replace it, or run \
                 `vej service uninstall`.",
                existing.definition.display()
            );
        }
        platform::uninstall()?;
    } else {
        // A busy port would leave an always-on service crash-looping; fail now instead.
        std::net::TcpListener::bind((settings.host.as_str(), settings.port)).with_context(
            || {
                format!(
                    "{}:{} is already in use; pick another --port",
                    settings.host, settings.port
                )
            },
        )?;
    }
    platform::install(&spec)?;

    print_install_summary(
        &spec,
        &settings.host,
        settings.port,
        &settings.keep_alive.to_string(),
    );
    if spec.program.components().any(|c| c.as_os_str() == "target") {
        println!(
            "⚠️  The service runs {} from a build directory; reinstall after `cargo install` \
             or moving the binary.",
            spec.program.display()
        );
    }
    if let Some(hint) = platform::post_install_hint() {
        println!("💡 {hint}");
    }
    Ok(())
}

fn print_install_summary(spec: &ServiceSpec, host: &str, port: u16, keep_alive: &str) {
    let url = format!("http://{host}:{port}");
    match spec.on_demand {
        None => println!("✓ vej service installed (always-on, starts at login): {url}"),
        Some(_) => println!(
            "✓ vej service installed (on-demand): {url} — vej starts on the first connection"
        ),
    }
    let idle_exit = spec
        .args
        .iter()
        .position(|a| a == "--idle-exit")
        .and_then(|i| spec.args.get(i + 1));
    match idle_exit {
        Some(idle) => {
            println!("  idle models unload after {keep_alive}; vej exits after {idle} idle")
        }
        None => println!("  idle models unload after {keep_alive}"),
    }
    if let Ok(status) = platform::status() {
        println!("  definition: {}", status.definition.display());
        println!("  logs:       {}", status.logs);
    }
    println!("  manage:     vej service status | stop | start | uninstall");
}

async fn print_status() -> Result<()> {
    let status = platform::status()?;
    if !status.installed {
        println!("vej service: not installed (run `vej service install`)");
        return Ok(());
    }
    let mode = if status.on_demand {
        "on-demand"
    } else {
        "always-on"
    };
    println!("vej service: installed ({mode})");
    println!("  definition: {}", status.definition.display());
    let process = match (status.loaded, status.running, status.pid) {
        (_, true, Some(pid)) => format!("running (pid {pid})"),
        (_, true, None) => "running".to_string(),
        (true, false, _) if status.on_demand => "idle — starts on the next connection".to_string(),
        (true, false, _) => "not running (exited; `vej service start` to restart)".to_string(),
        (false, false, _) => "stopped (`vej service start`)".to_string(),
    };
    println!("  process:    {process}");
    println!("  logs:       {}", status.logs);

    let Some((host, port)) = endpoint_from_args(&status.args) else {
        return Ok(());
    };
    println!("  endpoint:   http://{host}:{port}");
    // Only probe a running daemon: connecting to an idle on-demand socket would start vej.
    if status.running {
        println!("  model:      {}", describe_loaded_model(&host, port).await);
    }
    Ok(())
}

async fn describe_loaded_model(host: &str, port: u16) -> String {
    let probe_host = match host {
        "0.0.0.0" | "::" | "[::]" => "127.0.0.1",
        other => other,
    };
    let url = format!("http://{probe_host}:{port}/v1/ps");
    let response = reqwest::Client::new()
        .get(&url)
        .timeout(STATUS_PROBE_TIMEOUT)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status);
    let body: serde_json::Value = match response {
        Ok(res) => match res.bytes().await.map(|b| serde_json::from_slice(&b)) {
            Ok(Ok(body)) => body,
            Ok(Err(e)) => return format!("unknown (invalid /v1/ps response: {e})"),
            Err(e) => return format!("unknown (could not read /v1/ps: {e})"),
        },
        Err(e) => return format!("unknown (could not reach {url}: {e})"),
    };
    let Some(model) = body["models"].get(0) else {
        return "none loaded".to_string();
    };
    let name = model["model"].as_str().unwrap_or("?");
    match model["expires_in_ms"].as_u64() {
        Some(ms) => format!("{name} loaded, unloads in {}s if idle", ms / 1000),
        None => format!("{name} loaded"),
    }
}

/// Runs a command, returning stdout or an error with its output.
#[cfg_attr(not(unix), allow(dead_code))]
fn run(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to run {program}"))?;
    if !out.status.success() {
        let detail = [out.stderr.as_slice(), out.stdout.as_slice()]
            .iter()
            .map(|b| String::from_utf8_lossy(b).trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" / ");
        bail!("`{program} {}` failed: {detail}", args.join(" "));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg_attr(not(unix), allow(dead_code))]
fn succeeds(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|out| out.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_returns_stdout_and_reports_failures_with_output() {
        assert_eq!(run("echo", &["hello"]).unwrap().trim(), "hello");
        let err = run("ls", &["/definitely/not/here"])
            .unwrap_err()
            .to_string();
        assert!(err.contains("`ls /definitely/not/here` failed"), "{err}");
        assert!(succeeds("true", &[]));
        assert!(!succeeds("false", &[]));
    }

    #[tokio::test]
    async fn model_probe_reports_unreachable_daemons() {
        let (unused, _) = crate::activation::listener("127.0.0.1", 0).unwrap();
        let port = unused.local_addr().unwrap().port();
        drop(unused);
        assert!(
            describe_loaded_model("127.0.0.1", port)
                .await
                .starts_with("unknown")
        );
    }
}
