//! Bounded preparation work; only the caller mutates the resulting snapshot.
use crate::model::Result;
use std::{
    any::Any,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::sync_channel,
    },
    thread,
};

/// Bound router concurrency while using all cores on a local workstation.
pub(crate) fn configured_workers(variable: &str) -> Result<usize> {
    match std::env::var(variable) {
        Ok(value) => value
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=16).contains(n))
            .ok_or_else(|| format!("{variable} must be between 1 and 16")),
        Err(std::env::VarError::NotPresent) => {
            Ok(thread::available_parallelism().map_or(1, |n| n.get().min(8)))
        }
        Err(e) => Err(e.to_string()),
    }
}

fn panic_error(stage: &str, payload: Box<dyn Any + Send>) -> String {
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic payload");
    format!("Preparation {stage} panicked: {message}")
}

/// Consume completed work on the calling thread, retaining its original index.
/// Completion order is unspecified. At most `workers` items are dispatched but
/// not yet consumed, including queued jobs, active work, and buffered results.
/// Errors stop dispatch and join all workers; already-running work must finish.
/// In particular, network work must provide its own request timeout.
/// Work and consumer panics are returned as errors when unwinding is enabled.
pub(crate) fn run_bounded<T: Send>(
    count: usize,
    workers: usize,
    work: impl Fn(usize) -> Result<T> + Sync,
    mut consume: impl FnMut(usize, T) -> Result<()>,
) -> Result<()> {
    if count == 0 {
        return Ok(());
    }
    if workers == 0 {
        return Err("Preparation requires at least one worker".into());
    }
    let workers = workers.min(count);
    let cancelled = AtomicBool::new(false);
    let mut jobs = Vec::with_capacity(workers);
    let (results_tx, results_rx) = sync_channel(workers);

    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers);
        let mut error = None;
        for worker in 0..workers {
            let (jobs_tx, jobs_rx) = sync_channel(1);
            jobs.push(jobs_tx);
            let results_tx = results_tx.clone();
            let work = &work;
            let cancelled = &cancelled;
            match thread::Builder::new().spawn_scoped(scope, move || loop {
                if cancelled.load(Ordering::Acquire) {
                    break;
                }
                let job = jobs_rx.recv();
                let Ok(index) = job else { break };
                if cancelled.load(Ordering::Acquire) {
                    break;
                }
                let result = catch_unwind(AssertUnwindSafe(|| work(index)))
                    .unwrap_or_else(|panic| Err(panic_error("worker", panic)));
                if result.is_err() {
                    cancelled.store(true, Ordering::Release);
                }
                if results_tx.send((worker, index, result)).is_err() {
                    break;
                }
            }) {
                Ok(handle) => handles.push(handle),
                Err(e) => {
                    error = Some(format!("Cannot start preparation worker: {e}"));
                    break;
                }
            }
        }
        // Closing every worker sender must disconnect the result receiver.
        drop(results_tx);
        if error.is_none() {
            let result = (|| -> Result<()> {
                for (index, jobs_tx) in jobs.iter().enumerate() {
                    if jobs_tx.send(index).is_err() && !cancelled.load(Ordering::Acquire) {
                        return Err("Preparation workers stopped unexpectedly".into());
                    }
                }
                let mut next = workers;
                for _ in 0..count {
                    let (worker, index, result) = results_rx
                        .recv()
                        .map_err(|_| "Preparation workers stopped unexpectedly".to_string())?;
                    let value = result?;
                    catch_unwind(AssertUnwindSafe(|| consume(index, value)))
                        .unwrap_or_else(|panic| Err(panic_error("consumer", panic)))?;
                    if next < count && !cancelled.load(Ordering::Acquire) {
                        if jobs[worker].send(next).is_err() && !cancelled.load(Ordering::Acquire) {
                            return Err("Preparation workers stopped unexpectedly".into());
                        }
                        next += 1;
                    }
                }
                Ok(())
            })();
            error = result.err();
        }
        cancelled.store(true, Ordering::Release);
        // Wake receivers and release buffered values before joining. A worker
        // finishing after cancellation must never block sending its result.
        drop(jobs);
        drop(results_rx);
        for handle in handles {
            if let Err(panic) = handle.join() {
                if error.is_none() {
                    error = Some(panic_error("worker", panic));
                }
            }
        }
        match error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicUsize, Barrier};

    #[test]
    fn bounded_parallel_work_keeps_indices_and_consumes_on_caller() {
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let barrier = Barrier::new(3);
        let caller = thread::current().id();
        let mut values = vec![None; 12];
        run_bounded(
            12,
            3,
            |index| {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(current, Ordering::SeqCst);
                barrier.wait();
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(index * index)
            },
            |index, value| {
                assert_eq!(thread::current().id(), caller);
                assert!(values[index].replace(value).is_none());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(peak.load(Ordering::SeqCst), 3);
        assert_eq!(values, (0..12).map(|i| Some(i * i)).collect::<Vec<_>>());
    }

    #[test]
    fn consumer_can_release_slower_work_without_ordering_deadlock() {
        let release = Barrier::new(2);
        let mut order = Vec::new();
        run_bounded(
            2,
            2,
            |index| {
                if index == 0 {
                    release.wait();
                }
                Ok(index)
            },
            |index, value| {
                assert_eq!(index, value);
                order.push(index);
                if index == 1 {
                    release.wait();
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(order, vec![1, 0]);
    }

    #[test]
    fn work_error_cancels_undispatched_jobs() {
        let started = AtomicUsize::new(0);
        let result = run_bounded(
            100,
            1,
            |_| -> Result<()> {
                started.fetch_add(1, Ordering::SeqCst);
                Err("provider refused request".into())
            },
            |_, _| panic!("failed work must not be consumed"),
        );
        assert_eq!(result, Err("provider refused request".into()));
        assert_eq!(started.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn consumer_failure_bounds_work_and_releases_all_results() {
        struct Value<'a>(&'a AtomicUsize);
        impl Drop for Value<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let started = AtomicUsize::new(0);
        let live = AtomicUsize::new(0);
        let result = run_bounded(
            100,
            3,
            |_| {
                started.fetch_add(1, Ordering::SeqCst);
                live.fetch_add(1, Ordering::SeqCst);
                Ok(Value(&live))
            },
            |_, _| Err("snapshot rejected result".into()),
        );
        assert_eq!(result, Err("snapshot rejected result".into()));
        assert!(started.load(Ordering::SeqCst) <= 3);
        assert_eq!(live.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn work_and_consumer_panics_return_errors() {
        let started = AtomicUsize::new(0);
        let result = run_bounded(
            10,
            1,
            |_| -> Result<()> {
                started.fetch_add(1, Ordering::SeqCst);
                panic!("worker exploded");
            },
            |_, _| Ok(()),
        );
        assert!(result.unwrap_err().contains("worker exploded"));
        assert_eq!(started.load(Ordering::SeqCst), 1);
        let result = run_bounded(10, 2, Ok, |_, _| panic!("consumer exploded"));
        assert!(result.unwrap_err().contains("consumer exploded"));
    }

    #[test]
    fn empty_work_needs_no_workers_but_nonempty_work_does() {
        run_bounded::<()>(0, 0, |_| panic!("no work"), |_, _| panic!("no result")).unwrap();
        assert!(run_bounded(1, 0, |_| Ok(()), |_, _| Ok(())).is_err());
    }
}
