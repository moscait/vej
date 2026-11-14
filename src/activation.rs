//! Listening sockets handed over by a service manager (socket activation), so launchd or
//! systemd can own the port and start vej only when a client connects.

use anyhow::{Context, Result};
use std::net::TcpListener;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenerSource {
    Bound,
    Systemd,
    /// Only constructed on macOS; other targets still match on it.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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

/// A Unix socket this process created; the socket file is removed when dropped.
#[cfg(unix)]
pub struct OwnedUnixSocket {
    listener: Option<std::os::unix::net::UnixListener>,
    path: std::path::PathBuf,
}

#[cfg(unix)]
impl OwnedUnixSocket {
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// The listener; the file stays owned (and is removed when `self` drops).
    pub fn take_listener(&mut self) -> Option<std::os::unix::net::UnixListener> {
        self.listener.take()
    }
}

#[cfg(unix)]
impl Drop for OwnedUnixSocket {
    fn drop(&mut self) {
        use std::os::unix::fs::FileTypeExt;
        let is_socket =
            std::fs::symlink_metadata(&self.path).is_ok_and(|m| m.file_type().is_socket());
        if is_socket {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Binds an owner-only (0600) Unix socket at `path`. A stale socket left by a crash is
/// replaced; a live socket or any other kind of file is never touched.
#[cfg(unix)]
pub fn bind_unix_socket(path: &std::path::Path) -> Result<OwnedUnixSocket> {
    use anyhow::bail;
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use std::os::unix::net::UnixStream;

    match std::fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_socket() => {
            bail!(
                "{} exists and is not a socket; refusing to replace it",
                path.display()
            );
        }
        Ok(_) if UnixStream::connect(path).is_ok() => {
            bail!("{} is already in use by another process", path.display());
        }
        Ok(_) => std::fs::remove_file(path)
            .with_context(|| format!("cannot remove stale socket {}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("cannot inspect {}", path.display())),
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        prepare_socket_directory(dir)?;
    }
    let listener =
        bind_owner_only(path).with_context(|| format!("failed to listen on {}", path.display()))?;
    let socket = OwnedUnixSocket {
        listener: Some(listener),
        path: path.to_path_buf(),
    };
    // Only this user may connect (e.g. the OpenZiti sidecar running as the same user).
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("cannot restrict permissions on {}", path.display()))?;
    Ok(socket)
}

/// Creates a missing socket directory as owner-only (0700) and warns when an existing one
/// lets other users write to it (they could delete or replace the socket file).
#[cfg(unix)]
fn prepare_socket_directory(dir: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("cannot create {}", dir.display()))?;
    let mode = std::fs::metadata(dir)
        .with_context(|| format!("cannot inspect {}", dir.display()))?
        .permissions()
        .mode();
    if let Some(reason) = shared_directory_reason(mode) {
        tracing::warn!(
            dir = %dir.display(),
            mode = format!("{:o}", mode & 0o7777),
            "The remote socket's directory is {reason}: other users could delete or replace \
             the socket. Prefer a private (0700) directory."
        );
    }
    Ok(())
}

/// Why a directory with this mode is unsafe for the remote socket, if it is.
#[cfg(unix)]
fn shared_directory_reason(mode: u32) -> Option<&'static str> {
    if mode & 0o002 != 0 {
        Some("world-writable")
    } else if mode & 0o020 != 0 {
        Some("group-writable")
    } else {
        None
    }
}

/// Serializes every temporary umask change in vej, so concurrent binds can't save and
/// restore each other's masks out of order.
#[cfg(unix)]
static UMASK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs `f` with the process umask set to `mask`, restoring the previous mask afterwards.
#[cfg(unix)]
fn with_umask<T>(mask: libc::mode_t, f: impl FnOnce() -> T) -> T {
    // The guarded data is `()`, so a poisoned lock leaves nothing inconsistent.
    let _guard = UMASK_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // SAFETY: umask only swaps the process file-mode creation mask and cannot fail.
    let previous = unsafe { libc::umask(mask) };
    let result = f();
    // SAFETY: as above; restores the mask that was in place before.
    unsafe { libc::umask(previous) };
    result
}

/// Binds with a 0o077 umask so the socket file never exists with group or world access,
/// even before the explicit chmod. The umask is process-wide, so it is restored right away;
/// 0o077 (not 0o177) keeps anything created concurrently usable by its owner (directories
/// get 0700, not an unusable 0600).
#[cfg(unix)]
fn bind_owner_only(path: &std::path::Path) -> std::io::Result<std::os::unix::net::UnixListener> {
    const OWNER_ONLY_UMASK: libc::mode_t = 0o077;
    with_umask(OWNER_ONLY_UMASK, || {
        std::os::unix::net::UnixListener::bind(path)
    })
}

/// Placeholder so callers compile on platforms without Unix domain sockets.
#[cfg(not(unix))]
pub struct OwnedUnixSocket(std::convert::Infallible);

#[cfg(not(unix))]
impl OwnedUnixSocket {
    /// A value of this type can't exist on this platform, so code holding one is unreachable.
    pub fn unreachable(self) -> ! {
        match self.0 {}
    }
}

#[cfg(not(unix))]
pub fn bind_unix_socket(_path: &std::path::Path) -> Result<OwnedUnixSocket> {
    anyhow::bail!("--remote-socket needs Unix domain sockets (macOS or Linux)")
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

    #[cfg(unix)]
    mod unix_socket {
        use super::super::bind_unix_socket;
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};
        use std::path::PathBuf;

        fn temp_socket(name: &str) -> PathBuf {
            // macOS limits socket paths to ~104 bytes, so keep this short.
            let dir = std::env::temp_dir().join(format!("vs{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(name);
            let _ = std::fs::remove_file(&path);
            path
        }

        #[test]
        fn binds_an_owner_only_socket_and_removes_it_on_drop() {
            let path = temp_socket("a.sock");
            let socket = bind_unix_socket(&path).unwrap();
            let meta = std::fs::metadata(&path).unwrap();
            assert!(meta.file_type().is_socket());
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
            drop(socket);
            assert!(!path.exists(), "socket file removed on drop");
        }

        #[test]
        fn replaces_a_stale_socket_left_by_a_crash() {
            let path = temp_socket("b.sock");
            let stale = std::os::unix::net::UnixListener::bind(&path).unwrap();
            drop(stale); // closing doesn't unlink: the file is left behind
            assert!(path.exists());
            assert!(bind_unix_socket(&path).is_ok());
        }

        #[test]
        fn refuses_to_replace_a_live_socket_or_a_regular_file() {
            let live = temp_socket("c.sock");
            let _owner = bind_unix_socket(&live).unwrap();
            let err = bind_unix_socket(&live).err().expect("live socket");
            assert!(err.to_string().contains("in use"), "{err}");

            let file = temp_socket("d.sock");
            std::fs::write(&file, "not a socket").unwrap();
            let err = bind_unix_socket(&file).err().expect("regular file");
            assert!(err.to_string().contains("not a socket"), "{err}");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), "not a socket");
        }

        #[test]
        fn creates_missing_parent_directories_owner_only() {
            let path = temp_socket("nested").join("e.sock");
            let _socket = bind_unix_socket(&path).unwrap();
            assert!(path.exists());
            let dir_mode = std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(dir_mode & 0o777, 0o700);
        }

        #[test]
        fn binds_owner_only_even_in_a_world_writable_directory() {
            let dir = temp_socket("shared");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
            let path = dir.join("f.sock");

            let _socket = bind_unix_socket(&path).unwrap();

            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        /// Reads the process umask. umask can only be read by setting it, so this holds the
        /// same lock as `bind_owner_only` and never sees a concurrent bind's temporary mask.
        fn current_umask() -> libc::mode_t {
            let _guard = super::super::UMASK_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // SAFETY: umask only swaps the process file-mode creation mask and cannot fail;
            // the original mask is put straight back.
            unsafe {
                let mask = libc::umask(0o077);
                libc::umask(mask);
                mask
            }
        }

        #[test]
        fn restores_the_process_umask_after_binding() {
            let before = current_umask();
            let _socket = bind_unix_socket(&temp_socket("g.sock")).unwrap();
            assert_eq!(current_umask(), before);
        }
    }

    #[cfg(unix)]
    #[test]
    fn flags_parent_directories_other_users_can_write() {
        use super::shared_directory_reason;
        assert_eq!(shared_directory_reason(0o700), None);
        assert_eq!(shared_directory_reason(0o755), None);
        assert_eq!(shared_directory_reason(0o750), None);
        assert_eq!(shared_directory_reason(0o775), Some("group-writable"));
        assert_eq!(shared_directory_reason(0o777), Some("world-writable"));
        assert_eq!(shared_directory_reason(0o1777), Some("world-writable"));
        assert_eq!(shared_directory_reason(0o40757), Some("world-writable"));
    }

    #[test]
    fn activated_sources_are_flagged() {
        assert!(ListenerSource::Systemd.is_activated());
        assert!(ListenerSource::Launchd.is_activated());
    }
}
