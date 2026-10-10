//! `beans serve`: the local websocket, the work it starts, and how that work is stopped.
//!
//! The process starts roots this module does not own (sync, routines, plugins, the local API), so
//! it cannot prove when their writing stops. It says so before starting them
//! ([`declare_startup_gaps`]), and a drain never claims a final flush it cannot justify. What it does
//! guarantee is bounded: commands are stopped, running turns are cancelled, and the owned work is
//! joined or given up on at one deadline. How the process ends is unchanged: a signal still ends it
//! by that signal, and a parent that went away still exits 0.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::app::App;
use crate::serve_owner::ServeOwner;

/// Work started here and not owned. Latched before any of it starts, never cleared.
fn declare_startup_gaps(owner: &ServeOwner) {
    for boundary in [
        "runtime::resume_sent_jobs",
        "shell::close_stale_rows",
        "plugins::refresh_installed",
        "plugins::mcp_json::start",
        "marketplace::check_in_background",
        "login_shell::environment",
        "sync::run",
        "routines::run",
        "app::refresh_models_periodically",
        "ws::serve",
    ] {
        owner.declare_unsupported(boundary);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StopReason {
    /// The process that started this one went away.
    ParentGone,
    /// A Unix signal arrived; the process ends by that signal, as it always has.
    Signal(i32),
    /// The local websocket stopped on its own, including a port it could not bind.
    ServeEnded,
}

/// The first reason to stop. Later ones change nothing: a shutdown happens once.
struct Supervisor {
    first: Mutex<Option<StopReason>>,
    arrived: Condvar,
}

impl Supervisor {
    fn new() -> Arc<Self> {
        Arc::new(Self { first: Mutex::new(None), arrived: Condvar::new() })
    }

    fn lock(&self) -> MutexGuard<'_, Option<StopReason>> {
        self.first.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn notify(&self, reason: StopReason) {
        let mut first = self.lock();
        if first.is_none() {
            *first = Some(reason);
            self.arrived.notify_all();
        }
    }

    /// Blocks until a reason arrives. Call off the async runtime.
    fn wait_first(&self) -> StopReason {
        let mut first = self.lock();
        loop {
            if let Some(reason) = *first { return reason; }
            first = self.arrived.wait(first).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// One deadline for the whole drain, including the blocking session shutdown.
fn drain_budget() -> Duration {
    #[cfg(test)]
    if let Some(millis) = std::env::var("BEANS_TEST_DRAIN_MS").ok().and_then(|value| value.trim().parse::<u64>().ok())
    {
        return Duration::from_millis(millis);
    }
    crate::serve_owner::DRAIN_BUDGET
}

/// Stops the commands bots left running, cancels the turns in flight, and joins owned work.
async fn drain(owner: &Arc<ServeOwner>, app: &Arc<App>, reason: StopReason) {
    let deadline = Instant::now() + drain_budget();
    owner.close_roots();
    #[cfg(feature = "runner")]
    {
        let app = Arc::clone(app);
        let stopping = tokio::task::spawn_blocking(move || app.shell_sessions.shutdown(&app));
        let _ = tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), stopping).await;
    }
    for job in app.running_jobs.lock().unwrap().values() { job.cancel.cancel(); }
    let joined = Arc::clone(owner);
    let outcome = tokio::task::spawn_blocking(move || joined.join_closed_until(deadline)).await;
    match outcome {
        Ok(Ok(())) => tracing::info!(?reason, "owned work joined"),
        Ok(Err(failure)) => tracing::warn!(%failure, ?reason, "the drain ended before owned work closed"),
        Err(error) => tracing::error!(%error, ?reason, "the drain did not finish"),
    }
    if let Some(failure) = owner.flush_refusal() {
        tracing::warn!(%failure, owned_still_running = owner.live(), "beans serve did not track all of its work; no final flush");
    }
}

/// Ends the process the way it ended before this drain existed.
#[cfg(unix)]
fn terminate_by_signal(number: i32) -> ! {
    unsafe {
        libc::signal(number, libc::SIG_DFL);
        libc::raise(number);
    }
    // `raise` on a signal this process ignores would return; the old disposition is gone, so the
    // default applies and this is unreachable in practice.
    std::process::exit(128 + number)
}

#[cfg(unix)]
async fn watch_signals(supervisor: Arc<Supervisor>) {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut terminate), Ok(mut interrupt), Ok(mut hangup)) =
        (signal(SignalKind::terminate()), signal(SignalKind::interrupt()), signal(SignalKind::hangup()))
    else {
        return;
    };
    let number = tokio::select! {
        _ = terminate.recv() => libc::SIGTERM,
        _ = interrupt.recv() => libc::SIGINT,
        _ = hangup.recv() => libc::SIGHUP,
    };
    supervisor.notify(StopReason::Signal(number));
}

/// Notifies instead of exiting: the drain runs first, and this process still ends as it did.
async fn watch_parent(supervisor: Arc<Supervisor>, pid: u32) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if !process_alive(pid) {
            tracing::info!(pid, "parent exited; stopping");
            supervisor.notify(StopReason::ParentGone);
            return;
        }
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    let signaled = unsafe { libc::kill(pid as i32, 0) } == 0;
    signaled || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// A process handle is signaled once the process has exited.
#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return false;
        }
        let running = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
        CloseHandle(handle);
        running
    }
}

/// The one Serve entry point. Returns only when the local websocket ends on its own; every other
/// end is the process ending, by signal or with status 0, as before.
pub async fn run(app: Arc<App>, parent_pid: Option<u32>, ready_stdout: bool) -> anyhow::Result<()> {
    let owner = ServeOwner::new();
    declare_startup_gaps(&owner);
    let supervisor = Supervisor::new();

    #[cfg(unix)]
    tokio::spawn(watch_signals(Arc::clone(&supervisor)));
    if let Some(pid) = parent_pid {
        tokio::spawn(watch_parent(Arc::clone(&supervisor), pid));
    }

    crate::runtime::resume_sent_jobs(&app);
    // A command a Beans that quit left waiting went with it; its row says so now.
    {
        let app = Arc::clone(&app);
        tokio::task::spawn_blocking(move || crate::shell::close_stale_rows(&app));
    }
    // Installed plugins follow the last verified marketplace index.
    crate::plugins::refresh_installed(&app, &crate::marketplace::current(&app).plugins);
    crate::plugins::mcp_json::start(&app);
    crate::marketplace::check_in_background(&app);
    // The only work this process owns: one catalog check, joinable and bounded.
    let catalog = owner.root_work("catalog::check_in_background");
    crate::catalog::check_in_background_owned(&app, catalog);
    // Bots' commands and plugin servers start with it; read it while the rest starts.
    tokio::spawn(beans_agent::login_shell::environment());
    tokio::spawn(crate::sync::run(Arc::clone(&app)));
    tokio::spawn(crate::routines::run(Arc::clone(&app)));
    #[cfg(feature = "provider-auth")]
    tokio::spawn(crate::app::refresh_models_periodically(Arc::clone(&app)));

    let serving = Arc::clone(&app);
    let listener = {
        let supervisor = Arc::clone(&supervisor);
        tokio::spawn(async move {
            // Whether it ended by stopping or by failing to bind, the websocket ending is the stop
            // reason; what it returned is still what the caller reports.
            let outcome = crate::ws::serve(serving, ready_stdout).await;
            supervisor.notify(StopReason::ServeEnded);
            outcome
        })
    };
    let reason = {
        let supervisor = Arc::clone(&supervisor);
        tokio::task::spawn_blocking(move || supervisor.wait_first()).await?
    };

    drain(&owner, &app, reason).await;
    match reason {
        StopReason::ServeEnded => match listener.await {
            Ok(result) => result,
            Err(error) => Err(anyhow::anyhow!("the local websocket ended: {error}")),
        },
        StopReason::ParentGone => std::process::exit(0),
        #[cfg(unix)]
        StopReason::Signal(number) => terminate_by_signal(number),
        #[cfg(not(unix))]
        StopReason::Signal(_) => Ok(()),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::{BufRead, Read, Write};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::Receiver;
    use std::sync::{Arc, Condvar, Mutex, PoisonError};
    use std::time::{Duration, Instant};

    /// The real Serve entry, in this process, for the parent test to drive.
    #[test]
    #[ignore]
    fn serve_child_process() {
        let home = std::env::var("BEANS_TEST_HOME").expect("BEANS_TEST_HOME");
        let port: u16 = std::env::var("BEANS_TEST_PORT").ok().and_then(|value| value.trim().parse().ok()).unwrap_or(0);
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let app = crate::app::App::load(crate::config::Config { home: PathBuf::from(home), port }).unwrap();
            // What `main` does with it: a Serve that cannot start reports and leaves nonzero.
            if let Err(error) = super::run(app, None, true).await {
                eprintln!("{error:#}");
                std::process::exit(1);
            }
        });
    }

    /// The catalog body the child will accept: this build's catalog, dated ahead.
    fn catalog_body() -> String {
        let mut body: serde_json::Value = serde_json::from_str(beans_models::BUNDLED).unwrap();
        body["updated"] = serde_json::Value::from("2999-01-01T00:00:00Z");
        body.to_string()
    }

    /// Serves the catalog once, and only when the parent opens the gate.
    fn catalog_server() -> (String, Receiver<()>, std::sync::Arc<Gate>) {
        let gate = std::sync::Arc::new(Gate::default());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/models/v1.json", listener.local_addr().unwrap());
        let (arrived_tx, arrived) = std::sync::mpsc::channel();
        let held = std::sync::Arc::clone(&gate);
        let body = catalog_body();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 2048];
            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                match socket.read(&mut chunk) {
                    Ok(0) | Err(_) => return,
                    Ok(count) => request.extend_from_slice(&chunk[..count]),
                }
            }
            let _ = arrived_tx.send(());
            held.wait();
            let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = socket.write_all(header.as_bytes()).and_then(|()| socket.write_all(body.as_bytes()));
        });
        (url, arrived, gate)
    }

    #[derive(Default)]
    struct Gate {
        open: Mutex<bool>,
        changed: Condvar,
    }

    impl Gate {
        fn wait(&self) {
            let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
            while !*open { open = self.changed.wait(open).unwrap_or_else(PoisonError::into_inner); }
        }

        fn open(&self) {
            *self.open.lock().unwrap_or_else(PoisonError::into_inner) = true;
            self.changed.notify_all();
        }
    }

    fn scratch() -> PathBuf {
        std::env::temp_dir().join(format!("beans-serve-drain-{}", uuid::Uuid::new_v4()))
    }

    fn child_command(home: &Path, models_url: &str, drain_ms: u64, port: u16) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--ignored", "--exact", "serve::tests::serve_child_process", "--nocapture", "--test-threads", "1"])
            .env_clear()
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("BEANS_TEST_HOME", home)
            .env("BEANS_TEST_DRAIN_MS", drain_ms.to_string())
            .env("BEANS_TEST_PORT", port.to_string())
            .env("BEANS_MODELS_URL", models_url)
            .env("RUST_LOG", "off")
            .stdin(Stdio::null())
            .stdout(Stdio::piped());
        command
    }

    /// A child Serve is this test's to clean up: it is killed and reaped when the test ends early, so
    /// a failed assertion never leaves a real `beans serve` running.
    struct ChildGuard(Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }

    impl std::ops::Deref for ChildGuard {
        type Target = Child;
        fn deref(&self) -> &Child { &self.0 }
    }

    impl std::ops::DerefMut for ChildGuard {
        fn deref_mut(&mut self) -> &mut Child { &mut self.0 }
    }

    fn spawn_guarded(command: &mut Command) -> ChildGuard {
        ChildGuard(command.spawn().expect("the real Serve child starts"))
    }

    fn spawn_child(home: &Path, models_url: &str, drain_ms: u64) -> ChildGuard {
        spawn_guarded(child_command(home, models_url, drain_ms, 0).stderr(Stdio::null()))
    }

    /// Waits for the child's own ready record, the same one `beans serve --ready-stdout` prints.
    fn wait_ready(child: &mut ChildGuard) {
        let stdout = child.stdout.take().expect("the child prints its ready record");
        let (ready_tx, ready) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if line.contains("\"ready\"") {
                    let _ = ready_tx.send(());
                    break;
                }
            }
        });
        ready.recv_timeout(Duration::from_secs(60)).expect("beans serve reports ready");
    }

    fn signal_child(child: &ChildGuard) {
        unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    }

    fn wait_for_exit(child: &mut ChildGuard) -> std::process::ExitStatus {
        wait_for_exit_within(child, Duration::from_secs(60))
            .expect("beans serve ends on its own")
    }

    fn wait_for_exit_within(child: &mut ChildGuard, within: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + within;
        loop {
            if let Some(status) = child.try_wait().unwrap() { return Some(status); }
            if Instant::now() >= deadline { return None; }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn assert_died_by_sigterm(status: std::process::ExitStatus) {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(libc::SIGTERM), "the process still ends by its own signal: {status:?}");
    }

    /// A Serve that cannot bind its port says so and leaves, instead of waiting for a stop reason
    /// that never comes. The bound error is what `main` reported before, still reported.
    #[test]
    fn a_port_already_taken_ends_the_process_with_that_error() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = taken.local_addr().unwrap().port();
        let home = scratch();
        let mut child = spawn_guarded(child_command(&home, "", 700, port).stderr(Stdio::piped()));
        let status = wait_for_exit_within(&mut child, Duration::from_secs(30))
            .expect("a Serve that cannot bind reports and leaves");
        assert!(!status.success(), "a Serve that cannot bind leaves nonzero: {status:?}");
        let mut reported = String::new();
        child.stderr.take().unwrap().read_to_string(&mut reported).unwrap();
        assert!(reported.contains("cannot bind"), "the bind failure is reported: {reported}");
        drop(taken);
        std::fs::remove_dir_all(home).ok();
    }

    /// The drain waits for the catalog check this process owns, then ends by the original signal.
    /// This runs with the startup latch standing, so it is also the regression that a latched
    /// boundary must not shorten the wait for owned work.
    #[test]
    fn drain_joins_owned_work_before_the_process_ends() {
        let (url, arrived, gate) = catalog_server();
        let home = scratch();
        let mut child = spawn_child(&home, &url, 10_000);
        wait_ready(&mut child);
        assert!(arrived.recv_timeout(Duration::from_secs(30)).is_ok(), "the catalog check started");

        signal_child(&child);
        std::thread::sleep(Duration::from_millis(250));
        assert!(child.try_wait().unwrap().is_none(), "beans serve waits for work it owns");

        gate.open();
        assert_died_by_sigterm(wait_for_exit(&mut child));
        let cached: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(home.join("catalog.json")).expect("the owned catalog check finished"),
        ).unwrap();
        assert_eq!(cached["source"].as_str(), Some(url.as_str()));
        assert!(cached["checked_at"].as_i64().unwrap_or_default() > 0);
        std::fs::remove_dir_all(home).ok();
    }

    /// Work that outlasts the budget ends the drain, and the process still dies by its signal.
    #[test]
    fn the_drain_deadline_is_bounded() {
        let (url, arrived, gate) = catalog_server();
        let home = scratch();
        let mut child = spawn_child(&home, &url, 700);
        wait_ready(&mut child);
        assert!(arrived.recv_timeout(Duration::from_secs(30)).is_ok(), "the catalog check started");

        let sent = Instant::now();
        signal_child(&child);
        assert_died_by_sigterm(wait_for_exit(&mut child));
        assert!(sent.elapsed() >= Duration::from_millis(600), "the drain waited for owned work: {:?}", sent.elapsed());
        assert!(!home.join("catalog.json").exists(), "the held check never finished, so it cached nothing");
        gate.open();
        std::fs::remove_dir_all(home).ok();
    }
}