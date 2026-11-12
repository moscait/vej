//! Request-activity tracking for `--idle-exit`: with socket activation the daemon can
//! exit entirely when unused, and launchd/systemd restart it on the next connection.

use axum::{extract::Request, extract::State, middleware::Next, response::Response};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::Instant;

#[derive(Debug)]
struct ActivityState {
    in_flight: usize,
    last_activity: Instant,
}

#[derive(Debug)]
pub struct ActivityTracker {
    state: Mutex<ActivityState>,
    changed: Notify,
}

/// Marks one request as in flight until dropped.
pub struct ActivityGuard {
    tracker: Arc<ActivityTracker>,
}

impl ActivityTracker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(ActivityState {
                in_flight: 0,
                last_activity: Instant::now(),
            }),
            changed: Notify::new(),
        })
    }

    pub fn begin(self: &Arc<Self>) -> ActivityGuard {
        {
            let mut state = self.lock();
            state.in_flight += 1;
            state.last_activity = Instant::now();
        }
        self.changed.notify_one();
        ActivityGuard {
            tracker: Arc::clone(self),
        }
    }

    /// When the daemon last became idle, or `None` while requests are in flight.
    pub fn idle_since(&self) -> Option<Instant> {
        let state = self.lock();
        (state.in_flight == 0).then_some(state.last_activity)
    }

    fn lock(&self) -> MutexGuard<'_, ActivityState> {
        // Counter updates cannot be left half-done, so a poisoned lock is safe to reuse.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        {
            let mut state = self.tracker.lock();
            state.in_flight = state.in_flight.saturating_sub(1);
            state.last_activity = Instant::now();
        }
        self.tracker.changed.notify_one();
    }
}

/// Resolves once no request has been in flight for `idle`.
pub async fn wait_until_idle_for(tracker: &ActivityTracker, idle: Duration) {
    loop {
        match tracker.idle_since() {
            Some(since) => {
                let deadline = since + idle;
                if Instant::now() >= deadline {
                    return;
                }
                tokio::select! {
                    () = tokio::time::sleep_until(deadline) => {}
                    () = tracker.changed.notified() => {}
                }
            }
            None => tracker.changed.notified().await,
        }
    }
}

/// Read-only status endpoints polled by `vej service status` and supervisors. They don't
/// count as activity, or merely watching the daemon would keep it from idling out.
const STATUS_PATHS: [&str; 3] = ["/", "/health", "/v1/ps"];

fn is_status_probe(req: &Request) -> bool {
    req.method() == axum::http::Method::GET && STATUS_PATHS.contains(&req.uri().path())
}

/// Axum middleware recording each request (other than status probes) as activity.
pub async fn track_activity(
    State(tracker): State<Arc<ActivityTracker>>,
    req: Request,
    next: Next,
) -> Response {
    if is_status_probe(&req) {
        return next.run(req).await;
    }
    let _guard = tracker.begin();
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    async fn resolves_within(
        tracker: &Arc<ActivityTracker>,
        idle: Duration,
        limit: Duration,
    ) -> bool {
        tokio::time::timeout(limit, wait_until_idle_for(tracker, idle))
            .await
            .is_ok()
    }

    #[tokio::test(start_paused = true)]
    async fn startup_counts_as_activity() {
        let tracker = ActivityTracker::new();
        assert_eq!(tracker.idle_since(), Some(Instant::now()));
        assert!(!resolves_within(&tracker, Duration::from_secs(60), Duration::from_secs(59)).await);
    }

    #[tokio::test(start_paused = true)]
    async fn resolves_after_idle_period_with_no_requests() {
        let tracker = ActivityTracker::new();
        assert!(resolves_within(&tracker, Duration::from_secs(60), Duration::from_secs(61)).await);
    }

    #[tokio::test(start_paused = true)]
    async fn in_flight_request_prevents_idle_exit() {
        let tracker = ActivityTracker::new();
        let guard = tracker.begin();
        assert_eq!(tracker.idle_since(), None);

        assert!(
            !resolves_within(&tracker, Duration::from_secs(10), Duration::from_secs(300)).await
        );

        drop(guard);
        assert!(resolves_within(&tracker, Duration::from_secs(10), Duration::from_secs(11)).await);
    }

    #[tokio::test(start_paused = true)]
    async fn finished_request_restarts_the_idle_clock() {
        let tracker = ActivityTracker::new();
        let waiter = {
            let tracker = Arc::clone(&tracker);
            tokio::spawn(
                async move { wait_until_idle_for(&tracker, Duration::from_secs(60)).await },
            )
        };

        sleep(Duration::from_secs(50)).await;
        drop(tracker.begin());
        sleep(Duration::from_secs(50)).await;
        assert!(!waiter.is_finished(), "t=100s: last activity at t=50s");

        sleep(Duration::from_secs(11)).await;
        assert!(waiter.is_finished());
    }

    #[tokio::test(start_paused = true)]
    async fn overlapping_requests_keep_daemon_busy_until_the_last_finishes() {
        let tracker = ActivityTracker::new();
        let first = tracker.begin();
        let second = tracker.begin();
        drop(first);
        assert_eq!(tracker.idle_since(), None);
        drop(second);
        assert_eq!(tracker.idle_since(), Some(Instant::now()));
    }
}
