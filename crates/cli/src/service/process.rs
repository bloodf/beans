//! The processes around the service: asking what answers on its port, proving which process that
//! is before stopping it, stopping it, bounding the commands that ask the service manager, and
//! keeping two service commands (the app's launch and an installer, say) from interleaving.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use super::plan;

/// Runs `command` to its end and returns its output, or `None` when it could not start or had
/// not finished within `limit`, when it is ended. Queries of a service manager that hangs (a
/// session bus that never answers) must not hold up the app's launch.
pub(crate) fn output_within(command: &mut Command, limit: Duration) -> Option<std::process::Output> {
    use std::io::Read;
    let mut child = command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().ok()?;
    // Read both pipes while waiting, so a full pipe cannot stall the command.
    let mut stdout = child.stdout.take()?;
    let mut stderr = child.stderr.take()?;
    let out = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let err = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    Some(std::process::Output { status, stdout: out.join().ok()?, stderr: err.join().ok()? })
}

/// The most a `hello` reply may be; a real one is a few hundred bytes.
const HELLO_LIMIT: usize = 64 << 10;

/// The `beans serve` answering on `port`, as its `hello` describes it; `None` when nothing
/// answers there as one within four seconds, however slowly or strangely it answers.
pub(crate) fn probe(port: u16) -> Option<plan::Holder> {
    probe_within(port, Duration::from_secs(4))
}

pub(crate) fn probe_within(port: u16, limit: Duration) -> Option<plan::Holder> {
    let (sender, receiver) = std::sync::mpsc::channel();
    // Timeouts bound each read, not a peer that trickles a byte at a time; the thread does.
    std::thread::spawn(move || {
        let _ = sender.send(ask(port, limit));
    });
    receiver.recv_timeout(limit).ok().flatten()
}

fn ask(port: u16, limit: Duration) -> Option<plan::Holder> {
    use tokio_tungstenite::tungstenite::{self, protocol::WebSocketConfig, Message};
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let stream = std::net::TcpStream::connect_timeout(&address, Duration::from_millis(500)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
    let config = WebSocketConfig::default().max_message_size(Some(HELLO_LIMIT)).max_frame_size(Some(HELLO_LIMIT));
    let (mut socket, _) = tungstenite::client::client_with_config(format!("ws://127.0.0.1:{port}/ws"), stream, Some(config)).ok()?;
    socket.send(Message::Text(r#"{"id":1,"method":"hello","params":{}}"#.into())).ok()?;
    let deadline = Instant::now() + limit;
    // Events share the socket; the reply is the message with this id.
    while Instant::now() < deadline {
        let Message::Text(text) = socket.read().ok()? else { continue };
        let Ok(reply) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        if reply["id"] == 1 {
            let _ = socket.close(None);
            return Some(holder_from_hello(port, &reply["result"], |path| Path::new(path).is_file()));
        }
    }
    None
}

/// A [`plan::Holder`] from a `hello` result.
pub(crate) fn holder_from_hello(port: u16, hello: &serde_json::Value, exists: impl Fn(&str) -> bool) -> plan::Holder {
    let executable = hello["executable"].as_str().map(str::to_string);
    plan::Holder {
        port,
        pid: hello["pid"].as_u64().and_then(|pid| u32::try_from(pid).ok()).filter(|pid| *pid != 0),
        version: hello["version"].as_str().map(str::to_string),
        executable_exists: executable.as_deref().is_some_and(&exists),
        executable,
    }
}

/// The process that ran this one, which clearing the port never stops.
pub(crate) fn parent_pid() -> Option<u32> {
    #[cfg(unix)]
    {
        Some(unsafe { libc::getppid() } as u32)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// The file name in `path`, whichever separator it uses.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// A path without what Linux appends to a deleted binary's, or Windows prefixes a verbatim one.
fn plain(path: &str) -> &str {
    let path = path.strip_suffix(" (deleted)").unwrap_or(path);
    path.strip_prefix(r"\\?\").unwrap_or(path)
}

/// Whether a binary of this name can be a `beans`: `beans`, `beans.exe`, one the Windows installer
/// moved aside (`beans.exe.old-…`), or a test build (`beans-<hash>`).
pub(crate) fn beans_named(path: &str) -> bool {
    let name = file_name(plain(path)).to_ascii_lowercase();
    name == "beans" || name == "beans.exe" || name.starts_with("beans.exe.old-") || name.starts_with("beans-")
}

/// Whether `actual`, the binary the operating system says a process runs, is `reported`, the one
/// that process said it runs: the same path, allowing for a binary deleted since (Linux), and for
/// case and verbatim prefixes (Windows).
pub(crate) fn same_executable(actual: &str, reported: &str) -> bool {
    let (actual, reported) = (plain(actual), plain(reported));
    actual == reported || (cfg!(windows) && actual.eq_ignore_ascii_case(reported))
}

/// The binary the operating system says process `pid` runs; `None` when it cannot say (the process
/// is gone or belongs to another user, or there is no `/proc`).
fn running_executable(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/exe")).ok().map(|path| path.to_string_lossy().into_owned())
    }
    #[cfg(target_os = "macos")]
    {
        let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let length = unsafe { libc::proc_pidpath(i32::try_from(pid).ok()?, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
        (length > 0).then(|| String::from_utf8_lossy(&buffer[..length as usize]).into_owned())
    }
    #[cfg(windows)]
    {
        super::supervisor::Handle::open(pid).ok()??.image().ok().map(|path| path.to_string_lossy().into_owned())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = pid;
        None
    }
}

/// Whether process `pid` is the `beans` that answered: the operating system says it runs a binary
/// named like one, and the binary it named, when it named one. A reused PID, or an answer naming
/// another process, is not stopped.
pub(crate) fn is_the_holder(pid: u32, reported: Option<&str>, actual: Option<String>) -> bool {
    let Some(actual) = actual else { return false };
    pid > 1 && beans_named(&actual) && reported.map_or(true, |reported| same_executable(&actual, reported) || canonical_match(&actual, reported))
}

/// macOS reports a process's real path; the binary may have named itself through a symlink.
fn canonical_match(actual: &str, reported: &str) -> bool {
    std::fs::canonicalize(reported).is_ok_and(|reported| same_executable(actual, &reported.to_string_lossy()))
}

/// Stops a leftover `beans serve`: asked first, so it drains (`beans serve` stops its work within
/// ten seconds of SIGTERM), and ended after fifteen.
#[cfg(unix)]
pub(crate) fn stop_process(pid: u32) -> anyhow::Result<()> {
    let pid = libc::pid_t::try_from(pid)?;
    anyhow::ensure!(pid > 1, "Refusing to stop process {pid}");
    let gone = || {
        // A process that ended but is not yet reaped is gone for this purpose.
        #[cfg(target_os = "linux")]
        if std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| stat.rsplit_once(')').is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))) {
            return true;
        }
        let signalled = unsafe { libc::kill(pid, 0) };
        signalled != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    };
    let wait = |seconds: u64| {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < deadline {
            if gone() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        gone()
    };
    for (signal, seconds) in [(libc::SIGTERM, 15), (libc::SIGKILL, 5)] {
        if unsafe { libc::kill(pid, signal) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(());
            }
            anyhow::bail!("Cannot stop the old beans serve (pid {pid}): {error}");
        }
        if wait(seconds) {
            return Ok(());
        }
    }
    anyhow::bail!("The old beans serve (pid {pid}) did not stop")
}

#[cfg(windows)]
pub(crate) fn stop_process(pid: u32) -> anyhow::Result<()> {
    anyhow::ensure!(pid > 4, "Refusing to stop process {pid}");
    // Windows has no signal to ask a windowless process to drain; its job ends with it.
    match super::supervisor::Handle::open(pid)? {
        Some(handle) => handle.terminate(),
        None => Ok(()),
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn stop_process(pid: u32) -> anyhow::Result<()> {
    anyhow::bail!("Cannot stop process {pid} on this system")
}

/// [`plan::Action::ClearPort`], the same everywhere: each port's `beans serve`, if it is a
/// leftover and provably the process that answered, stops; if something starts it again at once,
/// that is said (or, `strict`, refused).
pub(crate) fn clear_port(ports: &[u16], strict: bool) -> anyhow::Result<Vec<plan::Note>> {
    let mut notes = Vec::new();
    for &port in ports {
        let Some(holder) = probe(port) else { continue };
        match plan::clearing(&holder, crate::config::VERSION, strict, std::process::id(), parent_pid()) {
            plan::Clearing::Stop { pid, note } => {
                if !is_the_holder(pid, holder.executable.as_deref(), running_executable(pid)) {
                    notes.push(plan::Note::OldUnidentified { port, version: holder.version });
                    continue;
                }
                stop_process(pid)?;
                notes.push(note);
                std::thread::sleep(Duration::from_secs(1));
                if let Some(again) = probe(port).filter(|again| plan::classify(again, crate::config::VERSION) == plan::Holding::Old) {
                    anyhow::ensure!(!strict, "An old beans serve started again on port {port} (pid {}) as soon as it was stopped: whatever runs it (an old Beans app, or another service) restarts it. Quit or remove that first.", again.pid.map_or("unknown".into(), |pid| pid.to_string()));
                    notes.push(plan::Note::OldProcess { port, pid: again.pid, version: again.version });
                }
            }
            plan::Clearing::Leave(note) => notes.push(note),
            plan::Clearing::Refuse(refusal) => return Err(refusal.into()),
        }
    }
    Ok(notes)
}

/// Held while a service command changes anything, so the app's launch and an installer running
/// `refresh` at once, or an `install` beside them, take turns instead of interleaving their steps.
pub(crate) struct ServiceLock {
    _file: std::fs::File,
}

/// Where the lock may live, in order: this user's cache folder, outside any Beans home since
/// commands for different homes share the one service; then, where that folder cannot be written
/// (a read-only home), a file of this user's own in the temporary folder.
pub(crate) fn lock_paths() -> Vec<std::path::PathBuf> {
    #[cfg(unix)]
    let own = format!("beans-service-{}.lock", unsafe { libc::getuid() });
    #[cfg(not(unix))]
    let own = String::from("beans-service.lock");
    dirs::cache_dir().map(|cache| cache.join("beans").join("service.lock")).into_iter().chain([std::env::temp_dir().join(own)]).collect()
}

/// Why the lock was not taken.
#[derive(Debug)]
pub(crate) enum LockError {
    /// Another command holds it.
    Busy,
    /// It could not be made here.
    Unusable(std::io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Busy => write!(f, "Another beans service command is still running; try again when it finishes."),
            LockError::Unusable(error) => write!(f, "Cannot take the beans service lock: {error}"),
        }
    }
}

impl std::error::Error for LockError {}

/// Takes the service lock at `path`, waiting up to `wait` for another command to finish.
pub(crate) fn lock_service(path: &Path, wait: Duration) -> Result<ServiceLock, LockError> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(LockError::Unusable)?;
    }
    let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path).map_err(LockError::Unusable)?;
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(ServiceLock { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Err(std::fs::TryLockError::WouldBlock) => return Err(LockError::Busy),
            Err(std::fs::TryLockError::Error(error)) => return Err(LockError::Unusable(error)),
        }
    }
}

/// The lock at the first of `paths` that can hold it. A busy lock is never stepped around: only a
/// place where the lock cannot be made at all passes it on to the next.
pub(crate) fn lock_first(paths: &[std::path::PathBuf], wait: Duration) -> Result<ServiceLock, LockError> {
    let mut last = LockError::Unusable(std::io::Error::other("no place for the lock"));
    for path in paths {
        match lock_service(path, wait) {
            Err(LockError::Unusable(error)) => last = LockError::Unusable(error),
            taken_or_busy => return taken_or_busy,
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::{self, Message};

    // MARK: Bounded commands

    #[cfg(unix)]
    #[test]
    fn a_command_that_hangs_is_ended_at_its_limit() {
        let started = Instant::now();
        assert!(output_within(Command::new("sleep").arg("30"), Duration::from_millis(300)).is_none());
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    }

    #[cfg(unix)]
    #[test]
    fn a_command_that_finishes_gives_its_whole_output_even_when_large() {
        // More than a pipe holds, on both streams, so reading while waiting matters.
        let output = output_within(Command::new("sh").args(["-c", "head -c 300000 /dev/zero | tr '\\0' a; head -c 300000 /dev/zero | tr '\\0' b >&2; exit 3"]), Duration::from_secs(20)).unwrap();
        assert_eq!((output.stdout.len(), output.stderr.len(), output.status.code()), (300_000, 300_000, Some(3)));
        assert!(output_within(&mut Command::new("/nonexistent/beans-query"), Duration::from_secs(1)).is_none());
    }

    // MARK: The probe

    /// A listener on a free port that treats each connection with `serve`.
    fn server(serve: impl Fn(std::net::TcpStream) + Send + Sync + 'static) -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let serve = std::sync::Arc::new(serve);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let serve = serve.clone();
                std::thread::spawn(move || serve(stream));
            }
        });
        port
    }

    /// A websocket server that answers the first message with `reply(request id)`, after `before`.
    fn websocket(before: Vec<String>, reply: impl Fn(&serde_json::Value) -> String + Send + Sync + 'static) -> u16 {
        server(move |stream| {
            let Ok(mut socket) = tungstenite::accept(stream) else { return };
            while let Ok(message) = socket.read() {
                let Message::Text(text) = message else { continue };
                let request: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                for frame in &before {
                    let _ = socket.send(Message::Text(frame.clone().into()));
                }
                let _ = socket.send(Message::Text(reply(&request["id"]).into()));
            }
        })
    }

    #[test]
    fn the_reply_is_found_among_events() {
        let events = vec![r#"{"event":"relay","data":{}}"#.to_string(), "not json".to_string(), r#"{"id":7,"result":{}}"#.to_string()];
        let port = websocket(events, |id| serde_json::json!({ "id": id, "result": { "version": "0.1.0", "pid": 99, "executable": "/x/beans" } }).to_string());
        let holder = probe(port).unwrap();
        assert_eq!((holder.port, holder.pid, holder.version.as_deref(), holder.executable.as_deref()), (port, Some(99), Some("0.1.0"), Some("/x/beans")));
    }

    #[test]
    fn an_error_reply_is_a_holder_that_does_not_say_what_it_is() {
        let port = websocket(Vec::new(), |id| serde_json::json!({ "id": id, "error": "no" }).to_string());
        let holder = probe(port).unwrap();
        assert_eq!(plan::classify(&holder, "0.1.0"), plan::Holding::Unknown, "never stopped");
    }

    #[test]
    fn a_peer_that_accepts_and_says_nothing_is_given_up_on() {
        let port = server(|stream| {
            std::thread::sleep(Duration::from_secs(20));
            drop(stream);
        });
        let started = Instant::now();
        assert_eq!(probe_within(port, Duration::from_secs(2)), None);
        assert!(started.elapsed() < Duration::from_secs(4), "{:?}", started.elapsed());
    }

    #[test]
    fn a_peer_that_trickles_its_answer_is_given_up_on() {
        let port = server(|mut stream| {
            use std::io::Write;
            // A byte every half second never trips a read timeout.
            for byte in b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n".iter().cycle().take(200) {
                if stream.write_all(&[*byte]).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
        let started = Instant::now();
        assert_eq!(probe_within(port, Duration::from_secs(2)), None);
        assert!(started.elapsed() < Duration::from_secs(4), "{:?}", started.elapsed());
    }

    #[test]
    fn a_reply_too_large_for_a_hello_is_refused() {
        let port = websocket(Vec::new(), |id| serde_json::json!({ "id": id, "result": { "version": "x".repeat(HELLO_LIMIT * 2) } }).to_string());
        assert_eq!(probe(port), None);
    }

    #[test]
    fn a_reply_that_never_comes_is_given_up_on() {
        let port = server(|stream| {
            let Ok(mut socket) = tungstenite::accept(stream) else { return };
            // Events forever, never the reply.
            while socket.send(Message::Text(r#"{"event":"tick"}"#.into())).is_ok() {
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let started = Instant::now();
        assert_eq!(probe_within(port, Duration::from_secs(2)), None);
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn what_is_not_a_websocket_is_nothing() {
        let port = server(|mut stream| {
            use std::io::Write;
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok");
        });
        assert_eq!(probe(port), None);
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert_eq!(probe(closed), None);
    }

    #[test]
    fn hello_is_read_as_a_holder() {
        let hello = serde_json::json!({ "version": "0.1.10", "pid": 4242, "executable": "/old/beans", "has_identity": true });
        assert_eq!(holder_from_hello(4874, &hello, |path| path == "/old/beans"), plan::Holder {
            port: 4874, pid: Some(4242), version: Some("0.1.10".into()), executable: Some("/old/beans".into()), executable_exists: true,
        });
        assert!(!holder_from_hello(4874, &hello, |_| false).executable_exists, "its binary was deleted");
        let older = holder_from_hello(4874, &serde_json::json!({ "version": "0.1.9" }), |_| true);
        assert_eq!((older.pid, older.executable, older.executable_exists), (None, None, false));
        for odd in [
            serde_json::json!(null), serde_json::json!({ "pid": -1 }), serde_json::json!({ "pid": 0 }), serde_json::json!({ "pid": 1u64 << 40 }),
            serde_json::json!({ "pid": "42" }), serde_json::json!({ "pid": 4.5 }), serde_json::json!({ "version": 1 }), serde_json::json!([1, 2]),
        ] {
            let holder = holder_from_hello(1, &odd, |_| true);
            assert_eq!((holder.pid, holder.version), (None, None), "{odd}");
        }
    }

    // MARK: Proving the process

    #[test]
    fn only_a_binary_named_like_beans_can_be_stopped() {
        for name in ["/example/user/.local/bin/beans", r"C:\Users\me\.local\bin\beans.exe", r"C:\bin\BEANS.EXE", r"C:\bin\beans.exe.old-0123456789abcdef0123456789abcdef",
            "/x/target/debug/deps/beans-60717c7e82695169", "/example/user/.local/bin/beans (deleted)", r"\\?\C:\bin\beans.exe"] {
            assert!(beans_named(name), "{name}");
        }
        for name in ["/sbin/init", "/usr/lib/systemd/systemd", "/bin/sh", r"C:\Windows\explorer.exe", "/x/beansprout", "/beans/launchd", "", "/"] {
            assert!(!beans_named(name), "{name}");
        }
    }

    #[test]
    fn the_process_must_run_the_binary_it_named() {
        assert!(same_executable("/example/user/.local/bin/beans", "/example/user/.local/bin/beans"));
        // Replaced since it started: Linux says so on both sides.
        assert!(same_executable("/example/user/.local/bin/beans (deleted)", "/example/user/.local/bin/beans (deleted)"));
        assert!(same_executable("/example/user/.local/bin/beans (deleted)", "/example/user/.local/bin/beans"));
        assert!(same_executable(r"\\?\C:\bin\beans.exe", r"C:\bin\beans.exe"));
        assert!(!same_executable("/opt/beans", "/example/user/.local/bin/beans"));
        assert_eq!(same_executable(r"C:\BIN\BEANS.EXE", r"c:\bin\beans.exe"), cfg!(windows), "Windows names ignore case");
    }

    #[test]
    fn a_reused_or_misnamed_pid_is_never_the_holder() {
        let beans = Some("/example/user/.local/bin/beans".to_string());
        assert!(is_the_holder(4242, Some("/example/user/.local/bin/beans"), beans.clone()));
        assert!(is_the_holder(4242, None, beans.clone()), "a beans from before hello named its binary");
        // The PID now belongs to another program.
        assert!(!is_the_holder(4242, Some("/example/user/.local/bin/beans"), Some("/usr/bin/python3".into())));
        // Another beans than the one that answered.
        assert!(!is_the_holder(4242, Some("/example/user/.local/bin/beans"), Some("/opt/other/beans".into())));
        // Gone, another user's, or no way to tell.
        assert!(!is_the_holder(4242, Some("/example/user/.local/bin/beans"), None));
        // An answer naming init or the idle process, even for a binary called beans.
        for pid in [0, 1] {
            assert!(!is_the_holder(pid, Some("/beans"), Some("/beans".into())));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_process_is_proved_by_its_own_binary() {
        let me = std::process::id();
        let actual = running_executable(me).unwrap();
        assert!(is_the_holder(me, Some(&std::env::current_exe().unwrap().to_string_lossy()), Some(actual)));
        assert_eq!(running_executable(u32::MAX - 1), None);
    }

    #[cfg(unix)]
    #[test]
    fn stopping_refuses_init_and_what_is_not_a_pid() {
        assert!(stop_process(0).is_err());
        assert!(stop_process(1).is_err());
        assert!(stop_process(u32::MAX).is_err(), "not a PID");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_process_that_ended_but_is_not_yet_reaped_is_already_stopped() {
        // As a leftover whose parent is slow to reap it.
        let mut child = Command::new("true").spawn().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let started = Instant::now();
        stop_process(child.id()).unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        child.wait().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_process_that_ignores_sigterm_is_ended() {
        use std::os::unix::process::ExitStatusExt;
        let mut child = Command::new("sh").args(["-c", "trap '' TERM; sleep 60"]).spawn().unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let pid = child.id();
        let reaper = std::thread::spawn(move || child.wait().unwrap());
        let started = Instant::now();
        stop_process(pid).unwrap();
        assert!(started.elapsed() >= Duration::from_secs(14), "it had its chance to drain: {:?}", started.elapsed());
        assert_eq!(reaper.join().unwrap().signal(), Some(libc::SIGKILL));
    }

    // MARK: The lock

    #[test]
    fn a_lock_that_cannot_be_made_falls_back_but_a_busy_one_never_does() {
        let folder = tempfile::tempdir().unwrap();
        // A file where the lock's folder would go: the cache folder cannot be written.
        let blocked = folder.path().join("blocked");
        std::fs::write(&blocked, b"").unwrap();
        let unusable = blocked.join("beans/service.lock");
        let fallback = folder.path().join("beans-service.lock");
        let taken = lock_first(&[unusable.clone(), fallback.clone()], Duration::from_secs(1)).unwrap();
        assert!(fallback.exists());
        // Held at the fallback: another command waits for it there and does not go past it.
        assert!(matches!(lock_first(&[unusable.clone(), fallback.clone()], Duration::from_millis(100)), Err(LockError::Busy)));
        drop(taken);
        let usable = folder.path().join("cache/beans/service.lock");
        let held = lock_first(&[usable.clone(), fallback.clone()], Duration::from_secs(1)).unwrap();
        assert!(matches!(lock_first(&[usable, fallback.clone()], Duration::from_millis(100)), Err(LockError::Busy)), "busy is not passed on to the fallback");
        drop(held);
        assert!(matches!(lock_first(&[unusable], Duration::from_millis(100)), Err(LockError::Unusable(_))));
        assert!(matches!(lock_first(&[], Duration::from_millis(100)), Err(LockError::Unusable(_))));
    }

    #[test]
    fn service_commands_take_turns() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("nested/service.lock");
        let first = lock_service(&path, Duration::from_secs(1)).unwrap();
        let error = lock_service(&path, Duration::from_millis(200)).err().unwrap();
        assert!(error.to_string().contains("still running"), "{error}");
        let waiting = {
            let path = path.clone();
            std::thread::spawn(move || lock_service(&path, Duration::from_secs(10)).map(|_| Instant::now()))
        };
        std::thread::sleep(Duration::from_millis(300));
        let released = Instant::now();
        drop(first);
        assert!(waiting.join().unwrap().unwrap() >= released, "the second waited for the first");
        // Released, the file stays behind but holds nothing.
        assert!(path.exists());
        drop(lock_service(&path, Duration::from_millis(10)).unwrap());
    }
}
