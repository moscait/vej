//! Linux `systemd --user` units (per-user, no root needed; logs go to journald).

use std::fmt::Write;
use std::path::{Path, PathBuf};

use super::spec::ServiceSpec;

pub const SERVICE_UNIT: &str = "vej.service";
pub const SOCKET_UNIT: &str = "vej.socket";

pub fn unit_dir(config_home: &Path) -> PathBuf {
    config_home.join("systemd/user")
}

pub fn render_service(spec: &ServiceSpec) -> String {
    let exec_start = std::iter::once(spec.program.display().to_string())
        .chain(spec.args.iter().cloned())
        .map(|arg| quote(&arg, Context::ExecStart))
        .collect::<Vec<_>>()
        .join(" ");

    let mut unit = format!(
        "[Unit]\nDescription=vej local System-1 decision engine\nDocumentation={}\n\n[Service]\n",
        env!("CARGO_PKG_REPOSITORY")
    );
    // Writing to a String cannot fail.
    let _ = writeln!(unit, "ExecStart={exec_start}");
    for (key, value) in &spec.environment {
        let _ = writeln!(
            unit,
            "Environment={}",
            quote(&format!("{key}={value}"), Context::Environment)
        );
    }
    unit.push_str("Restart=on-failure\nRestartSec=5\n");
    if spec.on_demand.is_none() {
        unit.push_str("\n[Install]\nWantedBy=default.target\n");
    }
    unit
}

/// The socket unit for on-demand services; `None` for always-on.
pub fn render_socket(spec: &ServiceSpec) -> Option<String> {
    spec.on_demand.map(|addr| {
        format!(
            "[Unit]\nDescription=vej on-demand listener\n\n[Socket]\nListenStream={addr}\n\n\
             [Install]\nWantedBy=sockets.target\n"
        )
    })
}

/// The installed `ExecStart` arguments minus the program itself.
pub fn installed_args(service_unit: &str) -> Option<Vec<String>> {
    let line = service_unit
        .lines()
        .find_map(|l| l.strip_prefix("ExecStart="))?;
    Some(split_exec_words(line).into_iter().skip(1).collect())
}

/// `systemctl show -p MainPID --value` prints 0 when the service is not running.
pub fn parse_main_pid(output: &str) -> Option<u32> {
    output.trim().parse().ok().filter(|&pid| pid != 0)
}

/// Runs `systemctl --user`.
#[cfg(unix)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub mod user {
    use super::{
        SERVICE_UNIT, SOCKET_UNIT, installed_args, parse_main_pid, render_service, render_socket,
        unit_dir,
    };
    use crate::service::{ServiceSpec, ServiceStatus, run};
    use anyhow::{Context, Result, bail};
    use std::path::PathBuf;

    fn units() -> Result<PathBuf> {
        Ok(unit_dir(
            &dirs::config_dir().context("cannot determine the config directory")?,
        ))
    }

    fn systemctl(args: &[&str]) -> Result<String> {
        let full: Vec<&str> = std::iter::once("--user")
            .chain(args.iter().copied())
            .collect();
        run("systemctl", &full)
    }

    /// `systemctl` output even when it exits non-zero (e.g. `is-active` on a stopped unit).
    fn systemctl_output(args: &[&str]) -> Result<String> {
        let out = std::process::Command::new("systemctl")
            .arg("--user")
            .args(args)
            .output()
            .context("failed to run systemctl")?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    pub fn log_file() -> Option<PathBuf> {
        None
    }

    pub fn install(spec: &ServiceSpec) -> Result<()> {
        let dir = units()?;
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("cannot create {}", dir.display()))?;
        std::fs::write(dir.join(SERVICE_UNIT), render_service(spec))
            .with_context(|| format!("cannot write {SERVICE_UNIT}"))?;
        let entry_unit = match render_socket(spec) {
            Some(socket) => {
                std::fs::write(dir.join(SOCKET_UNIT), socket)
                    .with_context(|| format!("cannot write {SOCKET_UNIT}"))?;
                SOCKET_UNIT
            }
            None => SERVICE_UNIT,
        };
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", "--now", entry_unit])?;
        Ok(())
    }

    pub fn uninstall() -> Result<bool> {
        let dir = units()?;
        let mut existed = false;
        for unit in [SOCKET_UNIT, SERVICE_UNIT] {
            let path = dir.join(unit);
            if path.exists() {
                // Disabling a unit that never started is fine; removal is what matters.
                let _ = systemctl(&["disable", "--now", unit]);
                std::fs::remove_file(&path)
                    .with_context(|| format!("cannot remove {}", path.display()))?;
                existed = true;
            }
        }
        if existed {
            systemctl(&["daemon-reload"])?;
        }
        Ok(existed)
    }

    pub fn start() -> Result<()> {
        let dir = units()?;
        if !dir.join(SERVICE_UNIT).exists() {
            bail!("vej service is not installed; run `vej service install`");
        }
        let unit = if dir.join(SOCKET_UNIT).exists() {
            SOCKET_UNIT
        } else {
            SERVICE_UNIT
        };
        systemctl(&["start", unit])?;
        Ok(())
    }

    pub fn stop() -> Result<()> {
        let dir = units()?;
        for unit in [SOCKET_UNIT, SERVICE_UNIT] {
            if dir.join(unit).exists() {
                systemctl(&["stop", unit])?;
            }
        }
        Ok(())
    }

    pub fn status() -> Result<ServiceStatus> {
        let dir = units()?;
        let definition = dir.join(SERVICE_UNIT);
        let logs = format!("journalctl --user -u {SERVICE_UNIT}");
        let Ok(unit) = std::fs::read_to_string(&definition) else {
            return Ok(ServiceStatus::not_installed(definition, logs));
        };
        let on_demand = dir.join(SOCKET_UNIT).exists();
        let watched = if on_demand { SOCKET_UNIT } else { SERVICE_UNIT };
        let loaded = systemctl_output(&["is-active", watched])? == "active";
        let running = systemctl_output(&["is-active", SERVICE_UNIT])? == "active";
        let pid = parse_main_pid(&systemctl_output(&[
            "show",
            "-p",
            "MainPID",
            "--value",
            SERVICE_UNIT,
        ])?);
        Ok(ServiceStatus {
            installed: true,
            loaded,
            running,
            pid,
            on_demand,
            args: installed_args(&unit).unwrap_or_default(),
            definition,
            logs,
        })
    }

    pub fn post_install_hint() -> Option<&'static str> {
        Some(
            "Runs while you are logged in. To keep it running after logout: loginctl enable-linger $USER",
        )
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Context {
    ExecStart,
    Environment,
}

/// Quotes one systemd value: `%` specifiers are escaped everywhere and `$` variable
/// expansion in `ExecStart`; values with whitespace or quotes are double-quoted.
fn quote(value: &str, context: Context) -> String {
    let mut escaped = value.replace('%', "%%");
    if context == Context::ExecStart {
        escaped = escaped.replace('$', "$$");
    }
    let needs_quotes = escaped.is_empty()
        || escaped
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | ';'));
    if !needs_quotes {
        return escaped;
    }
    format!("\"{}\"", escaped.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Inverse of [`quote`] for a whole `ExecStart` line.
fn split_exec_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut in_quotes = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' if in_quotes => current.extend(chars.next()),
            '"' => {
                in_quotes = !in_quotes;
                in_word = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            c => {
                current.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(current);
    }
    words
        .into_iter()
        .map(|w| w.replace("%%", "%").replace("$$", "$"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(on_demand: Option<&str>) -> ServiceSpec {
        ServiceSpec {
            program: PathBuf::from("/home/me/.cargo/bin/vej"),
            args: ["serve", "--host", "127.0.0.1", "--port", "8080"]
                .map(String::from)
                .to_vec(),
            environment: vec![
                ("VEJ_CACHE_DIR".into(), "/home/me/My Models".into()),
                ("RUST_LOG".into(), "info".into()),
            ],
            on_demand: on_demand.map(|a| a.parse().unwrap()),
            log_file: None,
        }
    }

    fn line_starting<'a>(unit: &'a str, prefix: &str) -> Vec<&'a str> {
        unit.lines().filter(|l| l.starts_with(prefix)).collect()
    }

    #[test]
    fn always_on_service_starts_with_the_user_session_and_restarts_on_failure() {
        let unit = render_service(&spec(None));

        assert_eq!(
            line_starting(&unit, "ExecStart="),
            ["ExecStart=/home/me/.cargo/bin/vej serve --host 127.0.0.1 --port 8080"]
        );
        assert_eq!(line_starting(&unit, "Restart="), ["Restart=on-failure"]);
        assert_eq!(
            line_starting(&unit, "WantedBy="),
            ["WantedBy=default.target"]
        );
        assert_eq!(
            line_starting(&unit, "Environment="),
            [
                "Environment=\"VEJ_CACHE_DIR=/home/me/My Models\"",
                "Environment=RUST_LOG=info"
            ]
        );
        assert_eq!(render_socket(&spec(None)), None);
    }

    #[test]
    fn on_demand_service_is_started_by_its_socket() {
        let s = spec(Some("127.0.0.1:47831"));
        let unit = render_service(&s);
        let socket = render_socket(&s).expect("on-demand has a socket unit");

        assert!(
            line_starting(&unit, "WantedBy=").is_empty(),
            "activated by the socket only"
        );
        assert_eq!(
            line_starting(&socket, "ListenStream="),
            ["ListenStream=127.0.0.1:47831"]
        );
        assert_eq!(
            line_starting(&socket, "WantedBy="),
            ["WantedBy=sockets.target"]
        );

        let ipv6 = render_socket(&spec(Some("[::1]:8080"))).unwrap();
        assert_eq!(
            line_starting(&ipv6, "ListenStream="),
            ["ListenStream=[::1]:8080"]
        );
    }

    #[test]
    fn exec_start_quotes_and_escapes_special_characters() {
        let mut s = spec(None);
        s.program = PathBuf::from("/opt/my tools/vej");
        s.args = ["serve", "--model", "/m/100%\"best\" $HOME.gguf"]
            .map(String::from)
            .to_vec();

        let unit = render_service(&s);

        assert_eq!(
            line_starting(&unit, "ExecStart="),
            [r#"ExecStart="/opt/my tools/vej" serve --model "/m/100%%\"best\" $$HOME.gguf""#]
        );
        assert_eq!(
            installed_args(&unit).unwrap(),
            ["serve", "--model", "/m/100%\"best\" $HOME.gguf"]
        );
    }

    #[test]
    fn installed_args_round_trip() {
        let unit = render_service(&spec(None));
        assert_eq!(
            installed_args(&unit).unwrap(),
            ["serve", "--host", "127.0.0.1", "--port", "8080"]
        );
        assert_eq!(installed_args("[Service]\n"), None);
    }

    #[test]
    fn main_pid_zero_means_not_running() {
        assert_eq!(parse_main_pid("4242\n"), Some(4242));
        assert_eq!(parse_main_pid("0\n"), None);
        assert_eq!(parse_main_pid(""), None);
    }

    #[test]
    fn units_live_in_the_user_config_dir() {
        assert_eq!(
            unit_dir(Path::new("/home/me/.config")),
            PathBuf::from("/home/me/.config/systemd/user")
        );
    }
}
