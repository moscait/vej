//! Persistent worker-thread pool holding reusable llama.cpp contexts.

use anyhow::{Context, Result, anyhow};
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::llama_batch::LlamaBatch;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread::JoinHandle;

use super::EngineCore;

type WorkerJob =
    Box<dyn for<'a> FnOnce(&EngineCore, &mut LlamaContext<'a>, &mut LlamaBatch) + Send>;

struct QueueState<J> {
    jobs: VecDeque<J>,
    closed: bool,
}

type SharedQueue<J> = Arc<(Mutex<QueueState<J>>, Condvar)>;

struct WorkerGroup<J> {
    queue: SharedQueue<J>,
    handles: Vec<JoinHandle<()>>,
}

impl<J: Send + 'static> WorkerGroup<J> {
    fn start(
        worker_count: usize,
        name_prefix: &str,
        mut spawn_body: impl FnMut(
            usize,
            SharedQueue<J>,
            mpsc::SyncSender<Result<u32>>,
        ) -> Box<dyn FnOnce() + Send>,
    ) -> Result<(Self, u32)> {
        let count = worker_count.max(1);
        let queue: SharedQueue<J> = Arc::new((
            Mutex::new(QueueState {
                jobs: VecDeque::new(),
                closed: false,
            }),
            Condvar::new(),
        ));
        let mut group = Self {
            queue: Arc::clone(&queue),
            handles: Vec::with_capacity(count),
        };
        let mut init_rxs = Vec::with_capacity(count);

        for idx in 0..count {
            let (init_tx, init_rx) = mpsc::sync_channel(1);
            let body = spawn_body(idx, Arc::clone(&queue), init_tx);
            let handle = std::thread::Builder::new()
                .name(format!("{name_prefix}-{idx}"))
                .spawn(body)
                .with_context(|| format!("Failed to spawn inference worker {idx}"))?;
            group.handles.push(handle);
            init_rxs.push(init_rx);
        }

        let ctx_size = collect_worker_inits(init_rxs)?;
        Ok((group, ctx_size))
    }

    fn submit<T: Send + 'static>(
        &self,
        make_job: impl FnOnce(mpsc::SyncSender<Result<T>>) -> J,
    ) -> Result<T> {
        let (tx, rx) = mpsc::sync_channel(1);
        push_job(&self.queue, make_job(tx))?;
        rx.recv()
            .map_err(|_| anyhow!("Inference worker terminated unexpectedly"))?
    }
}

impl<J> Drop for WorkerGroup<J> {
    fn drop(&mut self) {
        let (lock, cv) = &*self.queue;
        if let Ok(mut state) = lock.lock() {
            state.closed = true;
            cv.notify_all();
        }
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}

/// Owns a fixed pool of worker threads, each holding an initialized `LlamaContext` and
/// `LlamaBatch` across requests so inference skips per-request Metal/CPU context setup.
///
/// Dropping the pool signals every worker to exit and joins all threads synchronously,
/// guaranteeing every `LlamaContext` is destroyed before the owning `DecisionEngine` releases
/// its `Arc<LlamaModel>`.
pub(crate) struct ContextPool {
    group: WorkerGroup<WorkerJob>,
    ctx_size: u32,
}

impl ContextPool {
    /// Spawns `worker_count` (clamped to >= 1) threads and waits until each initializes its
    /// inference context, failing model load immediately if any worker fails context creation.
    pub(crate) fn start(core: Arc<EngineCore>, worker_count: usize) -> Result<Self> {
        let (group, ctx_size) =
            WorkerGroup::start(worker_count, "vej-ctx", |_idx, queue, init_tx| {
                let worker_core = Arc::clone(&core);
                Box::new(move || run_context_worker(worker_core, queue, init_tx))
            })?;
        Ok(Self { group, ctx_size })
    }

    pub(crate) fn ctx_size(&self) -> u32 {
        self.ctx_size
    }

    /// Runs `f` on an idle worker thread with exclusive mutable access to its reusable context
    /// and batch buffer.
    pub(crate) fn run<F, T>(&self, f: F) -> Result<T>
    where
        F: for<'a> FnOnce(&EngineCore, &mut LlamaContext<'a>, &mut LlamaBatch) -> Result<T>
            + Send
            + 'static,
        T: Send + 'static,
    {
        self.group.submit(|tx| {
            Box::new(move |core, ctx, batch| {
                let _ = tx.send(f(core, ctx, batch));
            })
        })
    }
}

fn collect_worker_inits(init_rxs: Vec<mpsc::Receiver<Result<u32>>>) -> Result<u32> {
    let mut min_ctx_size = 0u32;
    for rx in init_rxs {
        let size = rx
            .recv()
            .map_err(|_| anyhow!("Inference worker exited during context initialization"))??;
        min_ctx_size = if min_ctx_size == 0 {
            size
        } else {
            min_ctx_size.min(size)
        };
    }
    Ok(min_ctx_size)
}

fn run_context_worker(
    core: Arc<EngineCore>,
    queue: SharedQueue<WorkerJob>,
    init_tx: mpsc::SyncSender<Result<u32>>,
) {
    let mut ctx = match core.create_context() {
        Ok(ctx) => ctx,
        Err(err) => {
            let _ = init_tx.send(Err(err));
            return;
        }
    };
    let ctx_size = ctx.n_ctx();
    let mut batch = LlamaBatch::new(ctx_size as usize, 1);
    if init_tx.send(Ok(ctx_size)).is_err() {
        return;
    }

    drain_queue(&queue, |job| job(&core, &mut ctx, &mut batch));
}

fn push_job<J>(queue: &SharedQueue<J>, job: J) -> Result<()> {
    let (lock, cv) = &**queue;
    let mut state = lock
        .lock()
        .map_err(|_| anyhow!("Inference worker queue poisoned"))?;
    if state.closed {
        return Err(anyhow!("Inference worker pool is shut down"));
    }
    state.jobs.push_back(job);
    cv.notify_one();
    Ok(())
}

fn drain_queue<J>(queue: &SharedQueue<J>, mut run_job: impl FnMut(J)) {
    let (lock, cv) = &**queue;
    loop {
        let job = {
            let Ok(mut state) = lock.lock() else {
                return;
            };
            while state.jobs.is_empty() && !state.closed {
                state = match cv.wait(state) {
                    Ok(guard) => guard,
                    Err(_) => return,
                };
            }
            if let Some(job) = state.jobs.pop_front() {
                job
            } else if state.closed {
                return;
            } else {
                continue;
            }
        };
        run_job(job);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};

    type TestJob = Box<dyn FnOnce(&mut (usize, usize)) + Send>;

    fn start_test_group(
        count: usize,
        active: Arc<AtomicUsize>,
        init: impl Fn(usize) -> Result<(usize, usize)> + Send + Sync + 'static,
    ) -> Result<WorkerGroup<TestJob>> {
        let init = Arc::new(init);
        let (group, _) = WorkerGroup::start(count, "test-ctx", |idx, queue, init_tx| {
            let active_cnt = Arc::clone(&active);
            let init_fn = Arc::clone(&init);
            Box::new(move || {
                active_cnt.fetch_add(1, Ordering::SeqCst);
                match init_fn(idx) {
                    Ok(mut state) => {
                        if init_tx.send(Ok(state.0 as u32)).is_ok() {
                            drain_queue(&queue, |job: TestJob| job(&mut state));
                        }
                    }
                    Err(err) => {
                        let _ = init_tx.send(Err(err));
                    }
                }
                active_cnt.fetch_sub(1, Ordering::SeqCst);
            })
        })?;
        Ok(group)
    }

    #[test]
    fn workers_run_concurrently_reuse_state_and_join_on_drop() {
        let active = Arc::new(AtomicUsize::new(0));
        let group = Arc::new(
            start_test_group(2, Arc::clone(&active), |idx| Ok((idx, 0)))
                .expect("pool starts with two workers"),
        );
        assert_eq!(active.load(Ordering::SeqCst), 2);

        // A Barrier(2) inside simultaneous jobs proves both workers execute concurrently.
        let barrier = Arc::new(Barrier::new(2));
        let mut callers = Vec::new();
        for _ in 0..2 {
            let g = Arc::clone(&group);
            let b = Arc::clone(&barrier);
            callers.push(std::thread::spawn(move || {
                g.submit(move |tx| {
                    Box::new(move |state| {
                        state.1 += 1;
                        b.wait();
                        let _ = tx.send(Ok(*state));
                    })
                })
                .expect("barrier job succeeds")
            }));
        }
        let mut barrier_workers = [false, false];
        for c in callers {
            let (worker_id, count) = c.join().expect("caller thread succeeds");
            barrier_workers[worker_id] = true;
            assert_eq!(count, 1);
        }
        assert_eq!(barrier_workers, [true, true]);

        // Subsequent jobs reuse the already-initialized worker states monotonically.
        let mut latest_counts = [1usize, 1usize];
        for _ in 0..4 {
            let (worker_id, count) = group
                .submit(|tx| {
                    Box::new(move |state| {
                        state.1 += 1;
                        let _ = tx.send(Ok(*state));
                    })
                })
                .expect("reuse job succeeds");
            assert_eq!(count, latest_counts[worker_id] + 1);
            latest_counts[worker_id] = count;
        }
        assert_eq!(latest_counts.iter().sum::<usize>(), 6);

        drop(group);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn init_failure_joins_all_spawned_workers_before_returning() {
        let active = Arc::new(AtomicUsize::new(0));
        let err = start_test_group(3, Arc::clone(&active), |idx| {
            if idx == 1 {
                Err(anyhow!("simulated OOM on worker 1"))
            } else {
                Ok((idx, 0))
            }
        })
        .err()
        .expect("init failure propagates");
        assert!(format!("{err:#}").contains("simulated OOM"), "{err:#}");
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }
}
