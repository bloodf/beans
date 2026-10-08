//! Local update control for a Runner. An operator's updater asks the running `beans serve` to
//! prepare (`update.prepare`): the lease closes admission at once, under the lock every admission
//! takes, and running work finishes undisturbed; `ready` turns true once nothing runs here. Turns,
//! rooms, routines, routine checks, commands left running, and relay `job` / `request` envelopes
//! enter through `Control::try_admit`; the sync loop stops its cursor before an envelope it cannot
//! admit, so the envelope stays on the relay until the lease ends. While held, sync still polls
//! Pause policy, running-job Stop, requests answering or controlling existing work, and completion
//! envelopes: `job_result` matching a pending `job_id` and `response` matching a pending `request_id`.
//! Already-admitted running waits can finish under the lease without advancing the held cursor or
//! admitting queued jobs. Unmatched completions and cancellations for jobs not running here stay
//! on the relay for the main pull. Policy replays by version; consumed envelopes replay as no-ops,
//! preserving deduplication. Cancelling the lease, its
//! expiry, or a restart lets work in again. Mutating calls need the token in the file
//! `BEANS_UPDATE_TOKEN_FILE` names; without that variable they are off. Account Pause and running
//! work are never touched.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::app::App;

#[cfg(any(windows, test))]
mod privacy;

/// What a refused admission says.
pub const UPDATING: &str = "This Runner is installing an update. Try again in a few minutes.";
/// Names the file holding the operator's update control token.
pub const TOKEN_FILE_ENV: &str = "BEANS_UPDATE_TOKEN_FILE";
const DEFAULT_LEASE: Duration = Duration::from_secs(600);
const MIN_LEASE: Duration = Duration::from_secs(30);
const MAX_LEASE: Duration = Duration::from_secs(3600);
const MIN_TOKEN_CHARS: usize = 32;
const MAX_TOKEN_FILE_BYTES: u64 = 4096;

#[derive(Default)]
pub struct Control {
    gate: Arc<Mutex<Gate>>,
}

#[derive(Default)]
struct Gate {
    lease: Option<Lease>,
    /// Admitted work that has not finished.
    active: usize,
}

struct Lease {
    id: String,
    expires_at: Instant,
}

impl Gate {
    /// The lease, once an expired one is dropped.
    fn lease(&mut self) -> Option<&Lease> {
        if self.lease.as_ref().is_some_and(|lease| lease.expires_at <= Instant::now()) {
            self.lease = None;
        }
        self.lease.as_ref()
    }
}

/// Work counted as running here. Dropping it ends the admission.
pub struct Admission(Arc<Mutex<Gate>>);

impl Drop for Admission {
    fn drop(&mut self) {
        let mut gate = self.0.lock();
        gate.active = gate.active.saturating_sub(1);
    }
}

impl Control {
    /// Admits new work unless an update lease holds.
    pub fn try_admit(&self) -> Option<Admission> {
        let mut gate = self.gate.lock();
        if gate.lease().is_some() {
            return None;
        }
        gate.active += 1;
        Some(Admission(self.gate.clone()))
    }

    /// Counts work that admitted work starts or answers: a turn a room or a remote wait runs
    /// here, a command a turn left running, an answer to a card a turn waits on. It is never
    /// refused, since its parent still counts and no lease reads as ready meanwhile.
    pub fn hold(&self) -> Admission {
        self.gate.lock().active += 1;
        Admission(self.gate.clone())
    }

    /// Takes the lease, or extends the one held, and reports how far the drain is.
    pub(crate) fn prepare(&self, app: &Arc<App>, ttl: Duration) -> Value {
        let (id, extended) = {
            let mut gate = self.gate.lock();
            let current = gate.lease().map(|lease| lease.id.clone());
            let id = current.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            gate.lease = Some(Lease { id: id.clone(), expires_at: Instant::now() + ttl });
            (id, current.is_some())
        };
        if !extended {
            tracing::info!(seconds = ttl.as_secs(), "update lease taken; new work waits while running work finishes");
        }
        let expiring = app.clone();
        let lease = id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(ttl).await;
            expiring.update.expire(&expiring, &lease);
        });
        let mut status = self.status(app);
        status["lease_id"] = id.into();
        status
    }

    /// Wakes the sync loop for the envelopes the lease held back once lease `id` is over: due
    /// now, or already dropped lazily by a status read or an admission, which also ended it
    /// without a wake. A lease renewed past this timer's due time is still held and wakes nothing.
    fn expire(&self, app: &App, id: &str) {
        let ended = {
            let mut gate = self.gate.lock();
            let state = gate.lease.as_ref().filter(|lease| lease.id == id).map(|lease| lease.expires_at > Instant::now());
            if state == Some(true) {
                false
            } else {
                if state == Some(false) {
                    gate.lease = None;
                }
                true
            }
        };
        if ended {
            tracing::info!("update lease over; new work starts again");
            app.outbox_notify.notify_waiters();
        }
    }

    /// Ends the lease. True when one held.
    pub(crate) fn cancel(&self, app: &App) -> bool {
        let released = self.gate.lock().lease.take().is_some();
        if released {
            tracing::info!("update lease released; new work starts again");
        }
        app.outbox_notify.notify_waiters();
        released
    }

    pub fn status(&self, app: &App) -> Value {
        let (expires_in, active) = {
            let mut gate = self.gate.lock();
            (gate.lease().map(|lease| lease.expires_at.saturating_duration_since(Instant::now()).as_secs()), gate.active)
        };
        // Jobs this Device waits on from other Runners count too: a room here does not resume.
        let jobs = app.running_jobs.lock().unwrap().len();
        let idle = active == 0 && jobs == 0;
        json!({
            "version": crate::config::VERSION,
            "pid": std::process::id(),
            "control": std::env::var_os(TOKEN_FILE_ENV).is_some(),
            "prepared": expires_in.is_some(),
            "expires_in": expires_in,
            "ready": expires_in.is_some() && idle,
            "idle": idle,
            "active": active,
            "jobs": jobs,
        })
    }
}

/// `update.status`, `update.prepare { token, ttl? }`, and `update.cancel { token }`.
pub fn dispatch(app: &Arc<App>, method: &str, params: &Value) -> Result<Value, String> {
    match method {
        "update.status" => Ok(app.update.status(app)),
        "update.prepare" => {
            authorize(params)?;
            let ttl = match params.get("ttl").filter(|value| !value.is_null()) {
                None => DEFAULT_LEASE,
                Some(value) => Duration::from_secs(value.as_u64().ok_or("ttl must be whole seconds")?),
            };
            Ok(app.update.prepare(app, ttl.clamp(MIN_LEASE, MAX_LEASE)))
        }
        "update.cancel" => {
            authorize(params)?;
            Ok(json!({ "released": app.update.cancel(app) }))
        }
        other => Err(format!("Unknown method {other}")),
    }
}

fn authorize(params: &Value) -> Result<(), String> {
    authorize_with(std::env::var_os(TOKEN_FILE_ENV), params)
}

/// `path` is the file `TOKEN_FILE_ENV` names, if any; without one the control is off.
fn authorize_with(path: Option<std::ffi::OsString>, params: &Value) -> Result<(), String> {
    let path = path.ok_or("Update control is off: BEANS_UPDATE_TOKEN_FILE is not set.")?;
    let expected = read_token(Path::new(&path))?;
    let given = params["token"].as_str().ok_or("Update control needs its token.")?;
    // Equal-length digests compared without an early exit.
    let (given, expected) = (Sha256::digest(given.as_bytes()), Sha256::digest(expected.as_bytes()));
    if given.iter().zip(expected.iter()).fold(0u8, |diff, (a, b)| diff | (a ^ b)) != 0 {
        return Err("Update control token does not match.".into());
    }
    Ok(())
}

/// The operator's token from `path`: a small regular file (never a symlink) that root or this
/// user owns and that its group cannot write nor other users read, holding at least 32
/// characters. The checks run on the opened file, so the path cannot change under them. Errors
/// never include the file's contents.
pub fn read_token(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let unreadable = || "Cannot read the update control token file.";
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A symlink is refused, and a FIFO cannot block the open.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path).map_err(|_| unreadable())?;
    let metadata = file.metadata().map_err(|_| unreadable())?;
    if !metadata.is_file() || metadata.len() > MAX_TOKEN_FILE_BYTES {
        return Err("The update control token file must be a small regular file.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // SAFETY: `geteuid` has no preconditions.
        if metadata.uid() != 0 && metadata.uid() != unsafe { libc::geteuid() } {
            return Err("The update control token file must belong to root or the user running Beans.".into());
        }
        if metadata.permissions().mode() & 0o027 != 0 {
            return Err("The update control token file must not be writable by its group or accessible to other users.".into());
        }
    }
    #[cfg(windows)]
    let _parent = privacy::validate_token(&file).map_err(|_| "The update control token and its directory must have private Windows owner/DACL access.")?;
    let mut text = String::new();
    file.take(MAX_TOKEN_FILE_BYTES + 1).read_to_string(&mut text).map_err(|_| unreadable())?;
    if text.len() as u64 > MAX_TOKEN_FILE_BYTES {
        return Err("The update control token file must be a small regular file.".into());
    }
    let token = text.trim();
    if token.chars().count() < MIN_TOKEN_CHARS {
        return Err(format!("The update control token must be at least {MIN_TOKEN_CHARS} characters."));
    }
    Ok(token.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_app() -> (Arc<App>, std::path::PathBuf) {
        let home = std::env::temp_dir().join(format!("beans-update-{}", uuid::Uuid::new_v4()));
        (App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap(), home)
    }

    #[tokio::test]
    async fn a_lease_closes_admission_at_once_and_is_ready_only_after_running_work_ends() {
        let (app, home) = scratch_app();
        let running = app.update.try_admit().expect("no lease yet");
        let prepared = app.update.prepare(&app, MIN_LEASE);
        assert_eq!((prepared["prepared"].as_bool(), prepared["ready"].as_bool()), (Some(true), Some(false)));
        assert!(app.update.try_admit().is_none(), "new work waits while the lease holds");
        let continuation = app.update.hold();
        drop(running);
        assert_eq!(app.update.status(&app)["ready"], false, "work admitted work started still counts");
        drop(continuation);
        assert_eq!(app.update.status(&app)["ready"], true);
        let extended = app.update.prepare(&app, MIN_LEASE);
        assert_eq!(extended["lease_id"], prepared["lease_id"], "preparing again extends the same lease");
        assert!(app.update.cancel(&app));
        assert!(app.update.try_admit().is_some(), "cancelling lets work in again");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn no_admission_succeeds_once_prepare_has_returned() {
        let control = Arc::new(Control::default());
        let prepared = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let violations = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let (control, prepared, violations) = (control.clone(), prepared.clone(), violations.clone());
                std::thread::spawn(move || {
                    for _ in 0..20_000 {
                        // Read first: an admission taken before the lease may land after the flag.
                        let closed = prepared.load(std::sync::atomic::Ordering::SeqCst);
                        if control.try_admit().is_some() && closed {
                            violations.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                })
            })
            .collect();
        std::thread::sleep(Duration::from_millis(1));
        control.gate.lock().lease = Some(Lease { id: "lease".into(), expires_at: Instant::now() + MIN_LEASE });
        prepared.store(true, std::sync::atomic::Ordering::SeqCst);
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(violations.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(control.gate.lock().active, 0, "every dropped admission is uncounted");
    }

    #[tokio::test]
    async fn expiry_wakes_the_sync_loop_even_when_status_dropped_the_lease_first() {
        let (app, home) = scratch_app();
        let woken = app.outbox_notify.notified();
        tokio::pin!(woken);
        woken.as_mut().enable();
        app.update.prepare(&app, Duration::from_millis(60));
        // Reading status after the due time drops the lease lazily, before the timer task runs.
        while app.update.status(&app)["prepared"] == true {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::timeout(Duration::from_secs(5), woken).await.expect("expiry notifies the held sync loop");
        assert!(app.update.try_admit().is_some());
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn renewing_a_lease_outlives_the_wake_of_its_earlier_timer() {
        let (app, home) = scratch_app();
        let first = app.update.prepare(&app, Duration::from_millis(50));
        let renewed = app.update.prepare(&app, MIN_LEASE);
        assert_eq!(first["lease_id"], renewed["lease_id"]);
        let woken = app.outbox_notify.notified();
        tokio::pin!(woken);
        woken.as_mut().enable();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(app.update.try_admit().is_none(), "the earlier timer must not end the renewed lease");
        assert!(tokio::time::timeout(Duration::from_millis(10), woken).await.is_err(), "no wake while the lease holds");
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn cancel_wakes_the_sync_loop_and_reports_whether_a_lease_held() {
        let (app, home) = scratch_app();
        assert!(!app.update.cancel(&app), "nothing to release");
        app.update.prepare(&app, MIN_LEASE);
        let woken = app.outbox_notify.notified();
        tokio::pin!(woken);
        woken.as_mut().enable();
        assert!(app.update.cancel(&app));
        tokio::time::timeout(Duration::from_secs(5), woken).await.expect("cancel notifies the held sync loop");
        assert!(!app.update.cancel(&app), "a second cancel finds no lease");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn control_is_off_without_a_token_file_and_needs_the_matching_token() {
        let params = json!({ "token": "x".repeat(MIN_TOKEN_CHARS) });
        assert!(authorize_with(None, &params).unwrap_err().contains("is off"));
        #[cfg(unix)]
        {
            let file = token_file(&"x".repeat(MIN_TOKEN_CHARS), 0o600);
            assert!(authorize_with(Some(file.path.clone().into()), &params).is_ok());
            let wrong = json!({ "token": "y".repeat(MIN_TOKEN_CHARS) });
            assert!(authorize_with(Some(file.path.clone().into()), &wrong).unwrap_err().contains("does not match"));
            assert!(authorize_with(Some(file.path.clone().into()), &json!({})).unwrap_err().contains("needs its token"));
        }
    }

    #[cfg(unix)]
    struct TokenFile {
        path: std::path::PathBuf,
    }

    #[cfg(unix)]
    impl Drop for TokenFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[cfg(unix)]
    fn token_file(contents: &str, mode: u32) -> TokenFile {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("beans-token-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, contents).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        TokenFile { path }
    }

    #[cfg(unix)]
    #[test]
    fn token_file_must_be_private_regular_small_and_long_enough() {
        let token = "t".repeat(MIN_TOKEN_CHARS);
        assert_eq!(read_token(&token_file(&format!("{token}\n"), 0o600).path).unwrap(), token);
        assert_eq!(read_token(&token_file(&token, 0o640).path).unwrap(), token, "a group may read");
        assert!(read_token(&token_file(&token, 0o604).path).is_err(), "others may not read");
        assert!(read_token(&token_file(&token, 0o660).path).is_err(), "the group may not write");
        assert!(read_token(&token_file("short", 0o600).path).unwrap_err().contains("at least"));
        let big = token_file(&"t".repeat(MAX_TOKEN_FILE_BYTES as usize + 1), 0o600);
        assert!(read_token(&big.path).unwrap_err().contains("small regular file"));
        let error = read_token(&token_file("secret-that-is-too-short", 0o600).path).unwrap_err();
        assert!(!error.contains("secret-that"), "errors never carry the file's contents");
    }

    #[cfg(unix)]
    #[test]
    fn token_file_that_is_a_symlink_or_a_directory_is_refused() {
        let real = token_file(&"t".repeat(MIN_TOKEN_CHARS), 0o600);
        let link = std::env::temp_dir().join(format!("beans-token-link-{}", uuid::Uuid::new_v4()));
        std::os::unix::fs::symlink(&real.path, &link).unwrap();
        let refused = read_token(&link);
        let _ = std::fs::remove_file(&link);
        assert!(refused.is_err(), "a symlink to a good file is not followed");
        assert!(read_token(&std::env::temp_dir()).is_err());
    }
}
