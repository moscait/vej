//! macOS LaunchAgent definition (per-user, no admin rights needed).

use anyhow::{Context, Result};
use plist::{Dictionary, Value};
use std::path::{Path, PathBuf};

use super::spec::ServiceSpec;

pub const LABEL: &str = "com.moscait.vej";

pub fn plist_path(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

/// Renders the LaunchAgent. Always-on agents start at login and restart after crashes
/// (but not after a clean `/v1/shutdown`); on-demand agents let launchd own the socket.
pub fn render_plist(spec: &ServiceSpec) -> Result<String> {
    let mut agent = Dictionary::new();
    agent.insert("Label".into(), LABEL.into());
    let program_arguments = std::iter::once(spec.program.display().to_string())
        .chain(spec.args.iter().cloned())
        .map(Value::String)
        .collect();
    agent.insert("ProgramArguments".into(), Value::Array(program_arguments));

    if !spec.environment.is_empty() {
        let env: Dictionary = spec
            .environment
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        agent.insert("EnvironmentVariables".into(), Value::Dictionary(env));
    }
    if let Some(log) = &spec.log_file {
        let log = log.display().to_string();
        agent.insert("StandardOutPath".into(), log.clone().into());
        agent.insert("StandardErrorPath".into(), log.into());
    }
    // Without a ProcessType launchd throttles CPU and I/O; inference latency matters here.
    agent.insert("ProcessType".into(), "Interactive".into());

    match spec.on_demand {
        None => {
            agent.insert("RunAtLoad".into(), true.into());
            let mut keep_alive = Dictionary::new();
            keep_alive.insert("SuccessfulExit".into(), false.into());
            agent.insert("KeepAlive".into(), Value::Dictionary(keep_alive));
        }
        Some(addr) => {
            let mut listener = Dictionary::new();
            listener.insert("SockNodeName".into(), addr.ip().to_string().into());
            listener.insert("SockServiceName".into(), addr.port().to_string().into());
            listener.insert("SockType".into(), "stream".into());
            let family = if addr.is_ipv4() { "IPv4" } else { "IPv6" };
            listener.insert("SockFamily".into(), family.into());
            let mut sockets = Dictionary::new();
            sockets.insert(
                super::LAUNCHD_SOCKET_NAME.into(),
                Value::Dictionary(listener),
            );
            agent.insert("Sockets".into(), Value::Dictionary(sockets));
        }
    }

    let mut xml = Vec::new();
    Value::Dictionary(agent)
        .to_writer_xml(&mut xml)
        .context("failed to render LaunchAgent plist")?;
    String::from_utf8(xml).context("LaunchAgent plist is not UTF-8")
}

/// The installed `ProgramArguments` minus the program itself.
pub fn installed_args(plist_xml: &[u8]) -> Result<Vec<String>> {
    let agent = Value::from_reader_xml(plist_xml).context("invalid LaunchAgent plist")?;
    let args = agent
        .as_dictionary()
        .and_then(|d| d.get("ProgramArguments"))
        .and_then(Value::as_array)
        .context("LaunchAgent plist has no ProgramArguments")?;
    Ok(args
        .iter()
        .skip(1)
        .filter_map(Value::as_string)
        .map(str::to_string)
        .collect())
}

/// Whether the installed plist uses socket activation.
pub fn is_on_demand(plist_xml: &[u8]) -> bool {
    Value::from_reader_xml(plist_xml)
        .ok()
        .and_then(|v| v.into_dictionary())
        .is_some_and(|d| d.contains_key("Sockets"))
}

/// Reads `(running, pid)` from the job's top-level `launchctl print` fields. Nested
/// sections (e.g. sockets) carry their own `state =` lines and are ignored.
pub fn parse_print(output: &str) -> (bool, Option<u32>) {
    let field = |name: &str| {
        output.lines().find_map(|line| {
            line.strip_prefix('\t')
                .filter(|rest| !rest.starts_with('\t'))
                .and_then(|rest| rest.strip_prefix(name))
                .and_then(|rest| rest.strip_prefix(" = "))
        })
    };
    let running = field("state") == Some("running");
    let pid = field("pid").and_then(|p| p.trim().parse().ok());
    (running, pid)
}

/// Runs `launchctl` against the per-user GUI domain.
#[cfg(unix)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub mod agent {
    use super::{LABEL, installed_args, is_on_demand, parse_print, plist_path, render_plist};
    use crate::service::{ServiceSpec, ServiceStatus, run, succeeds};
    use anyhow::{Context, Result, bail};
    use std::path::PathBuf;
    use std::time::Duration;

    const UNLOAD_TIMEOUT: Duration = Duration::from_secs(15);
    const UNLOAD_POLL_INTERVAL: Duration = Duration::from_millis(100);

    fn home() -> Result<PathBuf> {
        dirs::home_dir().context("cannot determine the home directory")
    }

    pub fn log_file() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join("Library/Logs/vej/vej.log"))
    }

    fn domain() -> String {
        // SAFETY: getuid has no preconditions and cannot fail.
        format!("gui/{}", unsafe { libc::getuid() })
    }

    fn target() -> String {
        format!("{}/{LABEL}", domain())
    }

    fn is_loaded() -> bool {
        succeeds("launchctl", &["print", &target()])
    }

    /// `launchctl bootout` returns before launchd has finished removing the job; a
    /// bootstrap issued too early fails with "5: Input/output error".
    fn bootout_and_wait() -> Result<()> {
        run("launchctl", &["bootout", &target()])?;
        let deadline = std::time::Instant::now() + UNLOAD_TIMEOUT;
        while is_loaded() {
            if std::time::Instant::now() >= deadline {
                bail!("launchd did not unload {LABEL} within {UNLOAD_TIMEOUT:?}");
            }
            std::thread::sleep(UNLOAD_POLL_INTERVAL);
        }
        Ok(())
    }

    pub fn install(spec: &ServiceSpec) -> Result<()> {
        let path = plist_path(&home()?);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        if let Some(dir) = spec.log_file.as_ref().and_then(|l| l.parent()) {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        std::fs::write(&path, render_plist(spec)?)
            .with_context(|| format!("cannot write {}", path.display()))?;
        run(
            "launchctl",
            &["bootstrap", &domain(), &path.display().to_string()],
        )?;
        Ok(())
    }

    pub fn uninstall() -> Result<bool> {
        let path = plist_path(&home()?);
        if is_loaded() {
            bootout_and_wait()?;
        }
        let existed = path.exists();
        if existed {
            std::fs::remove_file(&path)
                .with_context(|| format!("cannot remove {}", path.display()))?;
        }
        Ok(existed)
    }

    pub fn start() -> Result<()> {
        let path = plist_path(&home()?);
        if !path.exists() {
            bail!("vej service is not installed; run `vej service install`");
        }
        if is_loaded() {
            run("launchctl", &["kickstart", &target()])?;
        } else {
            run(
                "launchctl",
                &["bootstrap", &domain(), &path.display().to_string()],
            )?;
        }
        Ok(())
    }

    pub fn stop() -> Result<()> {
        if is_loaded() {
            bootout_and_wait()?;
        }
        Ok(())
    }

    pub fn status() -> Result<ServiceStatus> {
        let definition = plist_path(&home()?);
        let logs = log_file().map_or_else(String::new, |l| l.display().to_string());
        let Ok(xml) = std::fs::read(&definition) else {
            return Ok(ServiceStatus::not_installed(definition, logs));
        };
        let printed = std::process::Command::new("launchctl")
            .args(["print", &target()])
            .output()
            .context("failed to run launchctl")?;
        let loaded = printed.status.success();
        let (running, pid) = if loaded {
            parse_print(&String::from_utf8_lossy(&printed.stdout))
        } else {
            (false, None)
        };
        Ok(ServiceStatus {
            installed: true,
            loaded,
            running,
            pid,
            on_demand: is_on_demand(&xml),
            args: installed_args(&xml)?,
            definition,
            logs,
        })
    }

    pub fn post_install_hint() -> Option<&'static str> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plist::Value;

    fn spec(on_demand: Option<&str>) -> ServiceSpec {
        ServiceSpec {
            program: PathBuf::from("/Applications/Tools & Co/vej"),
            args: ["serve", "--host", "127.0.0.1", "--port", "8080"]
                .map(String::from)
                .to_vec(),
            environment: vec![("RUST_LOG".into(), "info".into())],
            on_demand: on_demand.map(|a| a.parse().unwrap()),
            log_file: Some(PathBuf::from("/Users/me/Library/Logs/vej/vej.log")),
        }
    }

    fn parsed(spec: &ServiceSpec) -> plist::Dictionary {
        let xml = render_plist(spec).unwrap();
        Value::from_reader_xml(xml.as_bytes())
            .unwrap()
            .into_dictionary()
            .expect("top level is a dict")
    }

    fn strings(value: &Value) -> Vec<&str> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_string().unwrap())
            .collect()
    }

    #[test]
    fn always_on_agent_starts_at_login_and_restarts_only_after_crashes() {
        let d = parsed(&spec(None));

        assert_eq!(d["Label"].as_string(), Some(LABEL));
        assert_eq!(
            strings(&d["ProgramArguments"]),
            [
                "/Applications/Tools & Co/vej",
                "serve",
                "--host",
                "127.0.0.1",
                "--port",
                "8080"
            ]
        );
        assert_eq!(d["RunAtLoad"].as_boolean(), Some(true));
        let keep_alive = d["KeepAlive"].as_dictionary().unwrap();
        assert_eq!(keep_alive["SuccessfulExit"].as_boolean(), Some(false));
        assert!(!d.contains_key("Sockets"));
    }

    #[test]
    fn agent_logs_to_file_with_environment_and_unthrottled_priority() {
        let d = parsed(&spec(None));
        assert_eq!(
            d["StandardOutPath"].as_string(),
            Some("/Users/me/Library/Logs/vej/vej.log")
        );
        assert_eq!(
            d["StandardErrorPath"].as_string(),
            Some("/Users/me/Library/Logs/vej/vej.log")
        );
        let env = d["EnvironmentVariables"].as_dictionary().unwrap();
        assert_eq!(env["RUST_LOG"].as_string(), Some("info"));
        assert_eq!(d["ProcessType"].as_string(), Some("Interactive"));
    }

    #[test]
    fn on_demand_agent_lets_launchd_own_the_socket() {
        let d = parsed(&spec(Some("127.0.0.1:47831")));

        let listener = d["Sockets"].as_dictionary().unwrap()["Listeners"]
            .as_dictionary()
            .unwrap();
        assert_eq!(listener["SockNodeName"].as_string(), Some("127.0.0.1"));
        assert_eq!(listener["SockServiceName"].as_string(), Some("47831"));
        assert_eq!(listener["SockType"].as_string(), Some("stream"));
        assert_eq!(listener["SockFamily"].as_string(), Some("IPv4"));
        assert!(!d.contains_key("RunAtLoad"));
        assert!(!d.contains_key("KeepAlive"));
    }

    #[test]
    fn on_demand_ipv6_uses_ipv6_family() {
        let d = parsed(&spec(Some("[::1]:8080")));
        let listener = d["Sockets"].as_dictionary().unwrap()["Listeners"]
            .as_dictionary()
            .unwrap();
        assert_eq!(listener["SockNodeName"].as_string(), Some("::1"));
        assert_eq!(listener["SockFamily"].as_string(), Some("IPv6"));
    }

    #[test]
    fn installed_args_round_trip() {
        let xml = render_plist(&spec(None)).unwrap();
        assert_eq!(
            installed_args(xml.as_bytes()).unwrap(),
            ["serve", "--host", "127.0.0.1", "--port", "8080"]
        );
    }

    #[test]
    fn reads_running_state_and_pid_from_top_level_fields_only() {
        let running = "gui/501/com.moscait.vej = {\n\tactive count = 1\n\tstate = running\n\n\
                       \tprogram = /opt/vej\n\tpid = 4242\n\tsockets = {\n\t\tstate = active\n\t}\n}\n";
        assert_eq!(parse_print(running), (true, Some(4242)));

        let waiting = "gui/501/com.moscait.vej = {\n\tstate = waiting\n\tsockets = {\n\t\tstate = active\n\t}\n}\n";
        assert_eq!(parse_print(waiting), (false, None));
    }

    #[test]
    fn detects_on_demand_plists() {
        let on_demand = render_plist(&spec(Some("127.0.0.1:8080"))).unwrap();
        let always_on = render_plist(&spec(None)).unwrap();
        assert!(is_on_demand(on_demand.as_bytes()));
        assert!(!is_on_demand(always_on.as_bytes()));
    }

    #[test]
    fn plist_lives_in_user_launch_agents() {
        assert_eq!(
            plist_path(Path::new("/Users/me")),
            PathBuf::from("/Users/me/Library/LaunchAgents/com.moscait.vej.plist")
        );
    }
}
