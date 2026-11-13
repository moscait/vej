//! Model lifecycle: lazy loading, single-flight loads, drain-before-switch, and
//! Ollama-style keep-alive unloading so an idle daemon releases RAM/VRAM.

use anyhow::{Context, Result};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::sync::{Notify, OwnedRwLockReadGuard, RwLock};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::keep_alive::KeepAlive;

/// Loads models by id. `load` is blocking and always runs on the blocking pool.
pub trait ModelLoader: Send + Sync + 'static {
    type Model: Send + Sync + 'static;

    /// Map a user-supplied query (id, alias, or path) to the id models are tracked by.
    fn canonical_id(&self, query: &str) -> String;

    /// Preflight check that `model_id` exists and can be loaded before evicting the current model.
    fn check(&self, model_id: &str) -> Result<()>;

    fn load(&self, model_id: &str) -> Result<Self::Model>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedStatus {
    pub model_id: String,
    pub in_flight: usize,
    pub keep_alive: KeepAlive,
    /// Time until the idle model is released; `None` while in use or when kept forever.
    pub expires_in: Option<Duration>,
}

/// The resident model plus its usage bookkeeping.
struct Loaded<M> {
    model_id: String,
    model: Arc<M>,
    /// Leases hold read guards; a model switch takes the write lock to wait for them to finish.
    drain_gate: Arc<RwLock<()>>,
    /// Distinguishes this load from earlier loads of the same model id.
    generation: u64,
    keep_alive: KeepAlive,
    in_flight: usize,
    /// Set when the last in-flight request finishes; the keep-alive clock starts here.
    idle_since: Option<Instant>,
}

impl<M> Loaded<M> {
    fn expires_at(&self) -> Option<Instant> {
        self.keep_alive.deadline(self.idle_since?)
    }
}

struct Slot<M> {
    loaded: Option<Loaded<M>>,
    next_generation: u64,
}

/// State shared between the manager, its leases, and the idle reaper task.
/// The std mutex is only ever held for short, non-async bookkeeping.
struct Shared<M> {
    slot: Mutex<Slot<M>>,
    reaper_wakeup: Notify,
}

impl<M> Shared<M> {
    fn lock(&self) -> MutexGuard<'_, Slot<M>> {
        // Bookkeeping updates cannot leave the slot half-written, so a poisoned lock is safe to reuse.
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Ends one lease. Returns the model to drop (outside the lock) if it should unload now.
    fn release(&self, generation: u64) -> Option<Loaded<M>> {
        let evicted = {
            let mut slot = self.lock();
            let should_evict = match slot.loaded.as_mut() {
                Some(loaded) if loaded.generation == generation => {
                    loaded.in_flight -= 1;
                    if loaded.in_flight == 0 {
                        loaded.idle_since = Some(Instant::now());
                    }
                    loaded.in_flight == 0 && loaded.keep_alive.unloads_immediately()
                }
                // This lease belongs to a model that was already switched out.
                _ => false,
            };
            if should_evict {
                slot.loaded.take()
            } else {
                None
            }
        };
        self.reaper_wakeup.notify_one();
        evicted
    }

    fn next_expiry(&self) -> Option<Instant> {
        self.lock().loaded.as_ref()?.expires_at()
    }

    fn evict_if_expired(&self, now: Instant) -> Option<Loaded<M>> {
        let mut slot = self.lock();
        let is_expired = slot
            .loaded
            .as_ref()
            .and_then(Loaded::expires_at)
            .is_some_and(|deadline| deadline <= now);
        if is_expired { slot.loaded.take() } else { None }
    }
}

/// Sleeps until the idle model's keep-alive deadline, re-arming whenever usage changes.
async fn run_reaper<M: Send + Sync + 'static>(shared: Arc<Shared<M>>) {
    loop {
        match shared.next_expiry() {
            Some(deadline) => {
                tokio::select! {
                    () = tokio::time::sleep_until(deadline) => {
                        if let Some(evicted) = shared.evict_if_expired(Instant::now()) {
                            tracing::info!(model_id = %evicted.model_id, "Unloading idle model (keep_alive elapsed)");
                        }
                    }
                    () = shared.reaper_wakeup.notified() => {}
                }
            }
            None => shared.reaper_wakeup.notified().await,
        }
    }
}

pub struct ModelManager<L: ModelLoader> {
    loader: Arc<L>,
    /// The model to load when a request names none and nothing is resident: the startup
    /// model at first, then whichever model was most recently loaded, so a hot-switch
    /// survives keep-alive unloads.
    preferred_model: Mutex<String>,
    default_keep_alive: KeepAlive,
    shared: Arc<Shared<L::Model>>,
    /// Serializes loads and switches so concurrent requests trigger a single load.
    load_lock: tokio::sync::Mutex<()>,
    reaper: JoinHandle<()>,
}

impl<L: ModelLoader> Drop for ModelManager<L> {
    fn drop(&mut self) {
        self.reaper.abort();
    }
}

/// Grants use of the loaded model; the model cannot be unloaded while a lease is alive.
pub struct ModelLease<M> {
    // Field order matters: the model reference must drop before the drain guard is
    // released, so a waiting switch sees the last reference go away.
    model: Arc<M>,
    _drain_guard: OwnedRwLockReadGuard<()>,
    shared: Arc<Shared<M>>,
    model_id: String,
    generation: u64,
}

impl<M> ModelLease<M> {
    pub fn model(&self) -> &M {
        &self.model
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }
}

impl<M> Drop for ModelLease<M> {
    fn drop(&mut self) {
        if let Some(evicted) = self.shared.release(self.generation) {
            tracing::info!(model_id = %evicted.model_id, "Unloading model (keep_alive = 0)");
        }
    }
}

impl<L: ModelLoader> ModelManager<L> {
    /// Must be called within a Tokio runtime (it spawns the idle reaper).
    pub fn new(loader: L, default_model: &str, default_keep_alive: KeepAlive) -> Arc<Self> {
        let preferred_model = Mutex::new(loader.canonical_id(default_model));
        let shared = Arc::new(Shared {
            slot: Mutex::new(Slot {
                loaded: None,
                next_generation: 0,
            }),
            reaper_wakeup: Notify::new(),
        });
        let reaper = tokio::spawn(run_reaper(Arc::clone(&shared)));
        Arc::new(Self {
            loader: Arc::new(loader),
            preferred_model,
            default_keep_alive,
            shared,
            load_lock: tokio::sync::Mutex::new(()),
            reaper,
        })
    }

    /// The model that serves requests naming none: the resident one, or the one that
    /// will be loaded next.
    pub fn default_model(&self) -> String {
        self.preferred_model
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Lease a model, loading it if needed. `requested = None` uses whatever is loaded,
    /// falling back to [`Self::default_model`]. `keep_alive = None` applies the server default.
    pub async fn acquire(
        &self,
        requested: Option<&str>,
        keep_alive: Option<KeepAlive>,
    ) -> Result<ModelLease<L::Model>> {
        let target = requested.map(|q| self.loader.canonical_id(q));
        let keep_alive = keep_alive.unwrap_or(self.default_keep_alive);
        if let Some(lease) = self.try_lease(target.as_deref(), keep_alive) {
            return Ok(lease);
        }

        let _load_guard = self.load_lock.lock().await;
        // Another task may have loaded the model while we waited for the lock.
        if let Some(lease) = self.try_lease(target.as_deref(), keep_alive) {
            return Ok(lease);
        }

        let model_id = target.unwrap_or_else(|| self.default_model());
        self.check_blocking(&model_id).await?;
        self.evict_and_drain().await;
        let model = self.load_blocking(&model_id).await?;
        Ok(self.install(model_id, model, keep_alive))
    }

    /// Unload the current model now, or as soon as in-flight requests finish.
    /// Returns the id of the model being unloaded.
    pub fn unload(&self) -> Option<String> {
        let (model_id, evicted) = {
            let mut slot = self.shared.lock();
            let loaded = slot.loaded.as_mut()?;
            let model_id = loaded.model_id.clone();
            if loaded.in_flight > 0 {
                loaded.keep_alive = KeepAlive::For(Duration::ZERO);
                (model_id, None)
            } else {
                (model_id, slot.loaded.take())
            }
        };
        if evicted.is_some() {
            tracing::info!(model_id = %model_id, "Unloading model (requested)");
        }
        drop(evicted);
        self.shared.reaper_wakeup.notify_one();
        Some(model_id)
    }

    pub fn status(&self) -> Option<LoadedStatus> {
        let slot = self.shared.lock();
        let loaded = slot.loaded.as_ref()?;
        Some(LoadedStatus {
            model_id: loaded.model_id.clone(),
            in_flight: loaded.in_flight,
            keep_alive: loaded.keep_alive,
            expires_in: loaded
                .expires_at()
                .map(|deadline| deadline.saturating_duration_since(Instant::now())),
        })
    }

    fn try_lease(
        &self,
        target: Option<&str>,
        keep_alive: KeepAlive,
    ) -> Option<ModelLease<L::Model>> {
        let mut slot = self.shared.lock();
        let loaded = slot.loaded.as_mut()?;
        if target.is_some_and(|t| t != loaded.model_id) {
            return None;
        }
        let drain_guard = Arc::clone(&loaded.drain_gate).try_read_owned().ok()?;
        loaded.in_flight += 1;
        loaded.idle_since = None;
        loaded.keep_alive = keep_alive;
        Some(ModelLease {
            model: Arc::clone(&loaded.model),
            _drain_guard: drain_guard,
            shared: Arc::clone(&self.shared),
            model_id: loaded.model_id.clone(),
            generation: loaded.generation,
        })
    }

    /// Removes the resident model and waits until no request is still using it, so two
    /// models never occupy memory at once.
    async fn evict_and_drain(&self) {
        let previous = self.shared.lock().loaded.take();
        if let Some(previous) = previous {
            tracing::info!(model_id = %previous.model_id, "Unloading model to switch");
            let _drained = Arc::clone(&previous.drain_gate).write_owned().await;
            drop(previous);
        }
    }

    async fn check_blocking(&self, model_id: &str) -> Result<()> {
        let loader = Arc::clone(&self.loader);
        let id = model_id.to_string();
        tokio::task::spawn_blocking(move || loader.check(&id))
            .await
            .context("model check task panicked")?
    }

    async fn load_blocking(&self, model_id: &str) -> Result<L::Model> {
        let loader = Arc::clone(&self.loader);
        let id = model_id.to_string();
        tokio::task::spawn_blocking(move || loader.load(&id))
            .await
            .context("model load task panicked")?
    }

    fn install(
        &self,
        model_id: String,
        model: L::Model,
        keep_alive: KeepAlive,
    ) -> ModelLease<L::Model> {
        let drain_gate = Arc::new(RwLock::new(()));
        let drain_guard = Arc::clone(&drain_gate)
            .try_read_owned()
            .expect("a freshly created lock is uncontended");
        let model = Arc::new(model);
        *self
            .preferred_model
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = model_id.clone();
        let mut slot = self.shared.lock();
        let generation = slot.next_generation;
        slot.next_generation += 1;
        slot.loaded = Some(Loaded {
            model_id: model_id.clone(),
            model: Arc::clone(&model),
            drain_gate,
            generation,
            keep_alive,
            in_flight: 1,
            idle_since: None,
        });
        drop(slot);
        self.shared.reaper_wakeup.notify_one();
        ModelLease {
            model,
            _drain_guard: drain_guard,
            shared: Arc::clone(&self.shared),
            model_id,
            generation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{FakeLoader, Probe};
    use tokio::time::sleep;

    fn secs(s: u64) -> KeepAlive {
        KeepAlive::For(Duration::from_secs(s))
    }

    fn manager_with(loader: FakeLoader, keep: KeepAlive) -> Arc<ModelManager<FakeLoader>> {
        ModelManager::new(loader, "model-a", keep)
    }

    fn manager(keep: KeepAlive) -> (Arc<ModelManager<FakeLoader>>, Probe) {
        let (loader, probe) = FakeLoader::new();
        (manager_with(loader, keep), probe)
    }

    #[tokio::test]
    async fn starts_with_nothing_loaded() {
        let (m, probe) = manager(secs(60));
        assert_eq!(m.status(), None);
        assert!(probe.events().is_empty());
        assert_eq!(m.default_model(), "model-a");
    }

    #[tokio::test]
    async fn lazily_loads_default_model_on_first_acquire() {
        let (m, probe) = manager(secs(60));

        let lease = m.acquire(None, None).await.unwrap();

        assert_eq!(lease.model_id(), "model-a");
        assert_eq!(probe.loads(), ["model-a"]);
    }

    #[tokio::test]
    async fn reuses_loaded_model_across_requests() {
        let (m, probe) = manager(secs(60));

        drop(m.acquire(None, None).await.unwrap());
        drop(m.acquire(Some("model-a"), None).await.unwrap());

        assert_eq!(probe.loads(), ["model-a"]);
    }

    #[tokio::test]
    async fn serves_currently_loaded_model_when_request_names_none() {
        let (m, probe) = manager(secs(60));

        drop(m.acquire(Some("model-b"), None).await.unwrap());
        let lease = m.acquire(None, None).await.unwrap();

        assert_eq!(lease.model_id(), "model-b");
        assert_eq!(probe.loads(), ["model-b"]);
    }

    #[tokio::test]
    async fn canonicalizes_requested_model_ids() {
        let (m, probe) = manager(secs(60));

        let lease = m.acquire(Some("A-ALIAS"), None).await.unwrap();
        assert_eq!(lease.model_id(), "model-a");
        drop(lease);
        drop(m.acquire(Some("model-a"), None).await.unwrap());

        assert_eq!(probe.loads(), ["model-a"]);
    }

    #[tokio::test]
    async fn concurrent_acquires_share_a_single_load() {
        let (loader, probe) = FakeLoader::new();
        let m = manager_with(loader.with_load_delay(Duration::from_millis(50)), secs(60));

        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let m = Arc::clone(&m);
            tasks.spawn(async move {
                m.acquire(None, None)
                    .await
                    .map(|l| l.model_id().to_string())
            });
        }
        while let Some(result) = tasks.join_next().await {
            assert_eq!(result.unwrap().unwrap(), "model-a");
        }

        assert_eq!(probe.loads(), ["model-a"]);
    }

    #[tokio::test]
    async fn switching_models_drains_in_flight_requests_before_loading() {
        let (m, probe) = manager(secs(60));
        let lease_a = m.acquire(Some("model-a"), None).await.unwrap();

        let switcher = {
            let m = Arc::clone(&m);
            tokio::spawn(async move {
                m.acquire(Some("model-b"), None)
                    .await
                    .map(|l| l.model_id().to_string())
            })
        };
        sleep(Duration::from_millis(50)).await;
        assert!(
            !switcher.is_finished(),
            "switch must wait for in-flight request"
        );
        assert_eq!(probe.events(), ["load model-a"]);

        drop(lease_a);

        assert_eq!(switcher.await.unwrap().unwrap(), "model-b");
        assert_eq!(
            probe.events(),
            ["load model-a", "drop model-a", "load model-b"]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn unloads_idle_model_after_keep_alive() {
        let (m, probe) = manager(secs(10));
        drop(m.acquire(None, None).await.unwrap());

        sleep(Duration::from_secs(9)).await;
        assert!(m.status().is_some());

        sleep(Duration::from_secs(2)).await;
        assert_eq!(m.status(), None);
        assert_eq!(probe.drops(), ["model-a"]);
    }

    #[tokio::test(start_paused = true)]
    async fn never_unloads_while_a_request_is_in_flight() {
        let (m, probe) = manager(secs(1));
        let lease = m.acquire(None, None).await.unwrap();

        sleep(Duration::from_secs(60)).await;
        assert!(m.status().is_some());
        assert!(probe.drops().is_empty());

        drop(lease);
        sleep(Duration::from_secs(2)).await;
        assert_eq!(m.status(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn each_request_extends_the_idle_deadline() {
        let (m, _probe) = manager(secs(10));
        drop(m.acquire(None, None).await.unwrap());
        sleep(Duration::from_secs(8)).await;
        drop(m.acquire(None, None).await.unwrap());

        sleep(Duration::from_secs(8)).await;
        assert!(m.status().is_some(), "t=16s: deadline was pushed to t=18s");

        sleep(Duration::from_secs(3)).await;
        assert_eq!(m.status(), None);
    }

    #[tokio::test]
    async fn keep_alive_zero_unloads_when_last_request_finishes() {
        let (m, probe) = manager(secs(60));
        let first = m
            .acquire(None, Some(KeepAlive::For(Duration::ZERO)))
            .await
            .unwrap();
        let second = m
            .acquire(None, Some(KeepAlive::For(Duration::ZERO)))
            .await
            .unwrap();

        drop(first);
        assert!(m.status().is_some(), "still serving the second request");

        drop(second);
        assert_eq!(m.status(), None);
        assert_eq!(probe.drops(), ["model-a"]);
    }

    #[tokio::test(start_paused = true)]
    async fn forever_keep_alive_never_unloads() {
        let (m, _probe) = manager(secs(10));
        drop(m.acquire(None, Some(KeepAlive::Forever)).await.unwrap());

        sleep(Duration::from_secs(7 * 24 * 3600)).await;

        let status = m.status().unwrap();
        assert_eq!(status.keep_alive, KeepAlive::Forever);
        assert_eq!(status.expires_in, None);
    }

    #[tokio::test(start_paused = true)]
    async fn most_recent_request_keep_alive_wins() {
        let (m, _probe) = manager(secs(10));
        drop(m.acquire(None, Some(KeepAlive::Forever)).await.unwrap());
        sleep(Duration::from_secs(3600)).await;
        assert!(m.status().is_some());

        drop(m.acquire(None, None).await.unwrap());
        sleep(Duration::from_secs(11)).await;
        assert_eq!(
            m.status(),
            None,
            "omitted keep_alive falls back to the default"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn status_reports_in_flight_requests_and_expiry() {
        let (m, _probe) = manager(secs(10));
        let lease = m.acquire(None, None).await.unwrap();

        let busy = m.status().unwrap();
        assert_eq!(busy.model_id, "model-a");
        assert_eq!(busy.in_flight, 1);
        assert_eq!(busy.expires_in, None);

        drop(lease);
        sleep(Duration::from_secs(4)).await;
        let idle = m.status().unwrap();
        assert_eq!(idle.in_flight, 0);
        assert_eq!(idle.keep_alive, secs(10));
        assert_eq!(idle.expires_in, Some(Duration::from_secs(6)));
    }

    #[tokio::test]
    async fn failed_load_is_reported_and_can_be_retried() {
        let (loader, probe) = FakeLoader::new();
        let m = manager_with(loader.failing_first(1), secs(60));

        let err = m.acquire(None, None).await.err().expect("first load fails");
        assert!(
            err.to_string().contains("simulated load failure"),
            "{err:#}"
        );
        assert_eq!(m.status(), None);

        let lease = m.acquire(None, None).await.unwrap();
        assert_eq!(lease.model_id(), "model-a");
        assert_eq!(probe.events(), ["load-failed model-a", "load model-a"]);
    }

    #[tokio::test]
    async fn explicit_unload_releases_an_idle_model() {
        let (m, probe) = manager(secs(60));
        drop(m.acquire(None, None).await.unwrap());

        assert_eq!(m.unload().as_deref(), Some("model-a"));

        assert_eq!(m.status(), None);
        assert_eq!(probe.drops(), ["model-a"]);
    }

    #[tokio::test]
    async fn explicit_unload_waits_for_in_flight_request() {
        let (m, probe) = manager(secs(60));
        let lease = m.acquire(None, None).await.unwrap();

        assert_eq!(m.unload().as_deref(), Some("model-a"));
        assert!(m.status().is_some());
        assert!(probe.drops().is_empty());

        drop(lease);
        assert_eq!(m.status(), None);
        assert_eq!(probe.drops(), ["model-a"]);
    }

    #[tokio::test]
    async fn unload_with_nothing_loaded_is_a_no_op() {
        let (m, _probe) = manager(secs(60));
        assert_eq!(m.unload(), None);
    }

    #[tokio::test]
    async fn model_reloads_after_being_unloaded() {
        let (m, probe) = manager(secs(60));
        drop(m.acquire(None, None).await.unwrap());
        m.unload();

        drop(m.acquire(None, None).await.unwrap());

        assert_eq!(
            probe.events(),
            ["load model-a", "drop model-a", "load model-a"]
        );
    }

    #[tokio::test]
    async fn switched_model_stays_active_across_idle_unloads() {
        // A hot-switch (e.g. picking a model in a client's settings) must survive the
        // keep-alive unload; reloading the startup model instead would silently revert it.
        let (m, probe) = manager(secs(60));
        drop(m.acquire(Some("model-b"), None).await.unwrap());
        m.unload();

        assert_eq!(m.default_model(), "model-b", "reported while unloaded");
        let lease = m.acquire(None, None).await.unwrap();

        assert_eq!(lease.model_id(), "model-b");
        assert_eq!(probe.loads(), ["model-b", "model-b"]);
    }

    #[tokio::test]
    async fn failed_switch_keeps_the_previous_model_preferred() {
        let (loader, _probe) = FakeLoader::new();
        let m = manager_with(loader.failing_first(1), secs(60));

        assert!(m.acquire(Some("model-b"), None).await.is_err());

        assert_eq!(m.default_model(), "model-a");
    }

    #[tokio::test]
    async fn failed_check_keeps_resident_model_loaded() {
        let (loader, probe) = FakeLoader::new();
        let m = manager_with(loader.with_failing_checks(["missing"]), secs(60));
        drop(m.acquire(None, None).await.unwrap());

        assert!(m.acquire(Some("missing"), None).await.is_err());

        assert_eq!(probe.drops(), Vec::<String>::new());
        assert_eq!(m.status().unwrap().model_id, "model-a");
        assert_eq!(m.default_model(), "model-a");
    }
}
