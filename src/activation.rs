//! Listening sockets handed over by a service manager (socket activation), so launchd or
//! systemd can own the port and start vej only when a client connects.

use anyhow::{Context, Result};
use std::net::TcpListener;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenerSource {
    Bound,
    Systemd,
    Launchd,
}

impl ListenerSource {
    pub fn is_activated(self) -> bool {
        self != Self::Bound
    }
}

/// The socket a service manager passed in, or a freshly bound one.
pub fn listener(host: &str, port: u16) -> Result<(TcpListener, ListenerSource)> {
    if let Some(inherited) = inherited_listener()? {
        return Ok(inherited);
    }
    let listener = TcpListener::bind((host, port))
        .with_context(|| format!("failed to listen on {host}:{port} (is the port in use?)"))?;
    Ok((listener, ListenerSource::Bound))
}

fn inherited_listener() -> Result<Option<(TcpListener, ListenerSource)>> {
    let systemd = listenfd::ListenFd::from_env()
        .take_tcp_listener(0)
        .context("invalid socket passed by systemd (LISTEN_FDS)")?;
    if let Some(listener) = systemd {
        return Ok(Some((listener, ListenerSource::Systemd)));
    }
    #[cfg(target_os = "macos")]
    if let Some(listener) = launchd::take_listener(crate::service::LAUNCHD_SOCKET_NAME)? {
        return Ok(Some((listener, ListenerSource::Launchd)));
    }
    Ok(None)
}

#[cfg(target_os = "macos")]
mod launchd {
    use anyhow::{Context, Result, bail};
    use std::ffi::CString;
    use std::net::TcpListener;
    use std::os::fd::FromRawFd;
    use std::os::raw::{c_char, c_int};

    unsafe extern "C" {
        /// From liblaunch (part of libSystem): returns the descriptors for a `Sockets`
        /// entry in the job's plist. The array is malloc'd and owned by the caller.
        fn launch_activate_socket(
            name: *const c_char,
            fds: *mut *mut c_int,
            count: *mut libc::size_t,
        ) -> c_int;
    }

    /// Takes the named socket if launchd started us with one; `None` otherwise.
    pub fn take_listener(name: &str) -> Result<Option<TcpListener>> {
        let name = CString::new(name).context("socket name contains NUL")?;
        let mut fds: *mut c_int = std::ptr::null_mut();
        let mut count: libc::size_t = 0;
        // SAFETY: `name` is a valid NUL-terminated string that outlives the call, and both
        // out-pointers point to live, writable locals.
        let rc = unsafe { launch_activate_socket(name.as_ptr(), &mut fds, &mut count) };
        match rc {
            0 => {}
            // ESRCH: not managed by launchd. ENOENT: the job has no such socket (always-on mode).
            libc::ESRCH | libc::ENOENT => return Ok(None),
            other => bail!(
                "launchd socket activation failed: {}",
                std::io::Error::from_raw_os_error(other)
            ),
        }

        let descriptors: Vec<c_int> = if fds.is_null() || count == 0 {
            Vec::new()
        } else {
            // SAFETY: on success `fds` points to `count` initialized descriptors.
            unsafe { std::slice::from_raw_parts(fds, count) }.to_vec()
        };
        // SAFETY: launchd allocated the array with malloc; it is freed exactly once here and
        // not used afterwards (free(NULL) is a no-op).
        unsafe { libc::free(fds.cast()) };

        let mut listeners = descriptors.into_iter().map(|fd| {
            // SAFETY: each descriptor is an open listening socket that launchd handed to this
            // process; wrapping it transfers sole ownership so it is closed exactly once.
            unsafe { TcpListener::from_raw_fd(fd) }
        });
        let first = listeners.next();
        let extra = listeners.count();
        if extra > 0 {
            tracing::warn!(
                extra,
                "launchd passed more than one socket; using the first"
            );
        }
        Ok(first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_its_own_socket_outside_a_service_manager() {
        let (listener, source) = listener("127.0.0.1", 0).unwrap();
        assert_eq!(source, ListenerSource::Bound);
        assert!(!source.is_activated());
        assert_ne!(listener.local_addr().unwrap().port(), 0);
    }

    #[test]
    fn reports_a_busy_port_clearly() {
        let (taken, _) = listener("127.0.0.1", 0).unwrap();
        let port = taken.local_addr().unwrap().port();
        let err = listener("127.0.0.1", port).unwrap_err();
        assert!(err.to_string().contains("in use"), "{err}");
    }

    #[test]
    fn activated_sources_are_flagged() {
        assert!(ListenerSource::Systemd.is_activated());
        assert!(ListenerSource::Launchd.is_activated());
    }
}
