//! Who owns the work a `beans serve` process starts, so a shutdown can be bounded.
//!
//! A permit is one piece of work this process started and can therefore wait for. A boundary
//! declared unsupported is work whose end nobody here can observe: it latches
//! [`ShutdownFailure::MissingClosure`], and the drain refuses a final flush while any is latched.
//! There is no flush here; L1 owns a drain, and nothing else. The types are ungated so every build
//! of the core can name them.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShutdownFailure {
    /// This process starts work it does not own, so it cannot prove when the writing stops.
    MissingClosure(&'static str),
    /// Owned work was still running when the drain's deadline arrived.
    Deadline,
}

impl std::fmt::Display for ShutdownFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShutdownFailure::MissingClosure(boundary) => write!(formatter, "no closure for {boundary}"),
            ShutdownFailure::Deadline => formatter.write_str("the drain deadline arrived with owned work running"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase { Open, RootsClosed, Closed }

struct OwnerState {
    phase: Phase,
    live: usize,
    missing: Vec<&'static str>,
}

/// The shutdown owner. One instance per `beans serve` process.
pub(crate) struct ServeOwner {
    state: Mutex<OwnerState>,
    /// Signalled by the last [`WorkPermit`] drop, and waited on by [`ServeOwner::join_closed_until`].
    drained: Condvar,
}

impl ServeOwner {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(OwnerState { phase: Phase::Open, live: 0, missing: Vec::new() }),
            drained: Condvar::new(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, OwnerState> {
        // A panic unwinding through a permit drop poisons the lock; the state it guarded is still
        // sound, and refusing to read it would abort inside `Drop`.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records work this process starts and does not track. Called before that work starts.
    pub(crate) fn declare_unsupported(&self, boundary: &'static str) {
        let mut state = self.lock();
        if !state.missing.contains(&boundary) { state.missing.push(boundary); }
    }

    /// Takes a permit for owned work, or `None` once roots are closed: a root that cannot be waited
    /// for is not started under a claim that it can.
    pub(crate) fn root_work(self: &Arc<Self>, boundary: &'static str) -> Option<WorkPermit> {
        let mut state = self.lock();
        match state.phase {
            Phase::Open => {
                state.live += 1;
                Some(WorkPermit { owner: Arc::clone(self) })
            }
            _ => {
                tracing::warn!(boundary, "refusing owned work after roots closed");
                None
            }
        }
    }

    /// Refuses new roots.
    pub(crate) fn close_roots(&self) {
        let mut state = self.lock();
        if state.phase == Phase::Open { state.phase = Phase::RootsClosed; }
    }

    /// Waits until no owned work is live, or `deadline` passes. Call off the async runtime.
    /// A latched boundary does not shorten this wait: it refuses a final flush, and the work that
    /// is live is still waited for.
    pub(crate) fn join_closed_until(&self, deadline: Instant) -> Result<(), ShutdownFailure> {
        let mut state = self.lock();
        loop {
            if state.live == 0 {
                state.phase = Phase::Closed;
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() { return Err(ShutdownFailure::Deadline); }
            let (guard, _) = self.drained.wait_timeout(state, remaining).unwrap_or_else(PoisonError::into_inner);
            state = guard;
        }
    }

    /// Why a final flush would be refused, or `None` when nothing stands in the way. L1 never flushes.
    pub(crate) fn flush_refusal(&self) -> Option<ShutdownFailure> {
        let state = self.lock();
        state.missing.first().map(|&boundary| ShutdownFailure::MissingClosure(boundary))
            .or(if state.live == 0 { None } else { Some(ShutdownFailure::Deadline) })
    }

    /// Owned work still running.
    pub(crate) fn live(&self) -> usize { self.lock().live }
}

/// One piece of owned work. Held by the task that runs it; dropping it ends that ownership.
pub(crate) struct WorkPermit {
    owner: Arc<ServeOwner>,
}

impl Drop for WorkPermit {
    fn drop(&mut self) {
        let mut state = self.owner.lock();
        state.live = state.live.saturating_sub(1);
        self.owner.drained.notify_all();
    }
}

/// How long a shutdown waits for owned work before giving up on it.
pub(crate) const DRAIN_BUDGET: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests {
    use super::*;

    fn soon() -> Instant { Instant::now() + Duration::from_secs(5) }

    #[test]
    fn a_missing_boundary_stays_latched_and_is_reported_once() {
        let owner = ServeOwner::new();
        assert_eq!(owner.flush_refusal(), None);
        owner.declare_unsupported("sync::run");
        owner.declare_unsupported("sync::run");
        assert_eq!(owner.flush_refusal(), Some(ShutdownFailure::MissingClosure("sync::run")));
    }

    #[test]
    fn owned_work_is_joined_and_then_the_drain_reports_the_missing_boundary() {
        let owner = ServeOwner::new();
        owner.declare_unsupported("sync::run");
        let permit = owner.root_work("catalog::check_in_background");
        assert!(permit.is_some());
        assert_eq!(owner.live(), 1);
        owner.close_roots();
        // The latch stands, and the join still waits: only owned work and the deadline end it.
        assert_eq!(owner.flush_refusal(), Some(ShutdownFailure::MissingClosure("sync::run")));
        assert_eq!(owner.join_closed_until(Instant::now()), Err(ShutdownFailure::Deadline));
        drop(permit);
        assert_eq!(owner.live(), 0);
        assert_eq!(owner.join_closed_until(soon()), Ok(()));
        assert_eq!(owner.flush_refusal(), Some(ShutdownFailure::MissingClosure("sync::run")));
    }

    #[test]
    fn a_zero_deadline_gives_up_on_owned_work() {
        let owner = ServeOwner::new();
        let permit = owner.root_work("catalog::check_in_background").expect("a root is admitted while open");
        owner.close_roots();
        assert_eq!(owner.join_closed_until(Instant::now()), Err(ShutdownFailure::Deadline));
        assert_eq!(owner.flush_refusal(), Some(ShutdownFailure::Deadline));
        drop(permit);
        assert_eq!(owner.live(), 0);
    }

    #[test]
    fn a_root_started_after_roots_close_is_refused() {
        let owner = ServeOwner::new();
        owner.close_roots();
        assert!(owner.root_work("catalog::check_in_background").is_none());
        assert_eq!(owner.live(), 0);
    }
}