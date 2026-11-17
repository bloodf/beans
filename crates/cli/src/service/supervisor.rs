//! Windows service ownership: a PID is only a lookup key, never authorization to stop.

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use crate::config::{self, Config, Format};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Record {
    pub format: Format,
    pub pid: u32,
    pub created_at: u64,
    pub home: PathBuf,
}

pub(super) fn read_record(config: &Config) -> anyhow::Result<Option<Record>> {
    if !config.validate_home()? { return Ok(None); }
    config::read_json_strict(&config.home.join("service.pid"))
}

fn command_home(args: &[String]) -> Option<&str> {
    if args.get(1)?.as_str() != "service" || args.get(2)?.as_str() != "run" { return None; }
    let mut home = None;
    for pair in args[3..].chunks(2) {
        if pair.len() != 2 || pair[0] != "--env" { return None; }
        let (name, value) = pair[1].split_once('=')?;
        if name.eq_ignore_ascii_case("BEANS_HOME") {
            if home.is_some() { return None; }
            home = Some(value);
        }
    }
    home
}

/// Whether the process image `image` runs one of `programs`, or a program the Windows installer
/// moved aside to replace it. A running `beans.exe` cannot be overwritten, only renamed, so the
/// installer renames it `beans.exe.old-<32 hex digits>` in its folder and the supervisor runs on
/// from there until it is stopped.
pub(super) fn runs_program(image: &Path, programs: &[&Path]) -> bool {
    let name = |path: &Path| path.file_name().and_then(|name| name.to_str()).map(str::to_ascii_lowercase);
    programs.iter().any(|program| {
        image == *program
            || (image.parent() == program.parent()
                && name(image).zip(name(program)).is_some_and(|(image, program)| {
                    image.strip_prefix(&program).and_then(|rest| rest.strip_prefix(".old-"))
                        .is_some_and(|id| id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit()))
                }))
    })
}

pub(super) fn matches(record: &Record, created_at: u64, image: &Path, programs: &[&Path], home: &Path, args: &[String]) -> bool {
    identify(record, created_at, image, programs, home, args) == Identity::Supervisor
}

/// What the process a record's PID names now is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Identity {
    /// This home's supervisor, proven.
    Supervisor,
    /// Another process that was created later under the same PID (Windows reuses PIDs quickly,
    /// and a record can outlive a restart of the computer): the record is stale, nothing is
    /// running for it, and the other process is never touched.
    Reused,
    /// Created when the supervisor was, but not provably it: refused, and left untouched.
    Unproven,
}

pub(super) fn identify(record: &Record, created_at: u64, image: &Path, programs: &[&Path], home: &Path, args: &[String]) -> Identity {
    if record.pid == 0 {
        return Identity::Unproven;
    }
    if record.created_at != created_at {
        return Identity::Reused;
    }
    match runs_program(image, programs) && record.home == home && command_home(args).is_some_and(|value| Path::new(value) == home) {
        true => Identity::Supervisor,
        false => Identity::Unproven,
    }
}

#[cfg(windows)]
pub(super) struct Handle(pub windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for Handle {
    fn drop(&mut self) { unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0); } }
}

#[cfg(windows)]
impl Handle {
    pub fn open(pid: u32) -> anyhow::Result<Option<Self>> {
        use windows_sys::Win32::Foundation::{ERROR_INVALID_PARAMETER, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE};
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, pid) };
        if handle.is_null() {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) { return Ok(None); }
            return Err(error.into());
        }
        let handle = Self(handle);
        if unsafe { WaitForSingleObject(handle.0, 0) } == WAIT_OBJECT_0 { return Ok(None); }
        Ok(Some(handle))
    }

    pub fn created_at(&self) -> anyhow::Result<u64> {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::GetProcessTimes;
        let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        if unsafe { GetProcessTimes(self.0, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }

    pub fn image(&self) -> anyhow::Result<PathBuf> {
        use std::os::windows::ffi::OsStringExt;
        use windows_sys::Win32::System::Threading::QueryFullProcessImageNameW;
        let mut path = vec![0; 32768];
        let mut size = path.len() as u32;
        if unsafe { QueryFullProcessImageNameW(self.0, 0, path.as_mut_ptr(), &mut size) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(std::fs::canonicalize(std::ffi::OsString::from_wide(&path[..size as usize]))?)
    }

    pub fn args(&self) -> anyhow::Result<Vec<String>> {
        use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation};
        use windows_sys::Win32::Foundation::{LocalFree, UNICODE_STRING};
        use windows_sys::Win32::UI::Shell::CommandLineToArgvW;
        let mut size = 0;
        unsafe { NtQueryInformationProcess(self.0, ProcessCommandLineInformation, std::ptr::null_mut(), 0, &mut size); }
        anyhow::ensure!(size as usize >= std::mem::size_of::<UNICODE_STRING>() && size <= 128 * 1024, "Cannot verify supervisor command line");
        // Native UNICODE_STRING must be aligned; its pointer must stay inside this buffer.
        let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        if unsafe { NtQueryInformationProcess(self.0, ProcessCommandLineInformation, buffer.as_mut_ptr().cast(), size, &mut size) } < 0 {
            anyhow::bail!("Cannot verify supervisor command line");
        }
        let text = unsafe { &*buffer.as_ptr().cast::<UNICODE_STRING>() };
        let start = buffer.as_ptr() as usize;
        let end = start + std::mem::size_of_val(buffer.as_slice());
        let pointer = text.Buffer as usize;
        anyhow::ensure!(text.Length % 2 == 0 && pointer % 2 == 0
            && pointer >= start + std::mem::size_of::<UNICODE_STRING>()
            && pointer.checked_add(text.Length as usize).is_some_and(|last| last <= end), "Invalid supervisor command line");
        let mut line = Vec::with_capacity(text.Length as usize / 2 + 1);
        line.extend_from_slice(unsafe { std::slice::from_raw_parts(text.Buffer, text.Length as usize / 2) });
        line.push(0);
        let mut count = 0;
        let argv = unsafe { CommandLineToArgvW(line.as_ptr(), &mut count) };
        anyhow::ensure!(!argv.is_null() && count > 0, "Cannot parse supervisor command line");
        let result = (|| {
            let mut args = Vec::with_capacity(count as usize);
            for pointer in unsafe { std::slice::from_raw_parts(argv, count as usize) } {
                let mut length = 0;
                while unsafe { *pointer.add(length) } != 0 { length += 1; }
                args.push(String::from_utf16(unsafe { std::slice::from_raw_parts(*pointer, length) })?);
            }
            Ok::<_, anyhow::Error>(args)
        })();
        unsafe { LocalFree(argv.cast()); }
        result
    }

    pub fn terminate(&self) -> anyhow::Result<()> {
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};
        if unsafe { TerminateProcess(self.0, 0) } == 0 { return Err(std::io::Error::last_os_error().into()); }
        anyhow::ensure!(unsafe { WaitForSingleObject(self.0, 5000) } == WAIT_OBJECT_0, "Supervisor did not stop");
        Ok(())
    }
}

/// The supervisor this home's record names, proven to be one: the same process creation, a
/// home-bound `service run` command line, and an image that is this `beans`, one of `installed`
/// (the programs service definitions name), or one the installer moved aside.
#[cfg(windows)]
pub(super) fn bound(config: &Config, installed: &[PathBuf]) -> anyhow::Result<Option<(Record, Handle)>> {
    let Some(record) = read_record(config)? else { return Ok(None); };
    anyhow::ensure!(record.pid != 0, "Invalid supervisor PID; data is untouched");
    let Some(handle) = Handle::open(record.pid)? else { return Ok(None); };
    let home = std::fs::canonicalize(&config.home)?;
    // The process image is canonical (a verbatim path); so must the programs be.
    let canonical = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let exe = canonical(&super::exe()?);
    let installed: Vec<PathBuf> = installed.iter().map(|path| canonical(path)).collect();
    let programs: Vec<&Path> = std::iter::once(exe.as_path()).chain(installed.iter().map(PathBuf::as_path)).collect();
    let created_at = handle.created_at()?;
    if record.created_at != created_at {
        return Ok(None);
    }
    match identify(&record, created_at, &handle.image()?, &programs, &home, &handle.args()?) {
        Identity::Supervisor => Ok(Some((record, handle))),
        Identity::Reused => Ok(None),
        Identity::Unproven => anyhow::bail!("PID does not identify this home's Beans supervisor; data is untouched"),
    }
}

#[cfg(windows)]
pub(super) fn own_process_tree() -> anyhow::Result<Handle> {
    use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject, JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let job = Handle(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) });
    anyhow::ensure!(!job.0.is_null(), "Cannot create supervisor process job");
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if unsafe { SetInformationJobObject(job.0, JobObjectExtendedLimitInformation, (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(), std::mem::size_of_val(&limits) as u32) } == 0
        || unsafe { AssignProcessToJobObject(job.0, GetCurrentProcess()) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(job)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_same_created_supervisor_for_the_same_home_matches() {
        let home = Path::new("account");
        let image = Path::new("beans.exe");
        let record = Record { format: Format::BeansV2, pid: 42, created_at: 100, home: home.into() };
        let args: Vec<String> = ["beans.exe", "service", "run", "--env", "BEANS_HOME=account"].map(String::from).into();
        assert!(matches(&record, 100, image, &[image], home, &args));
        assert!(!matches(&record, 101, image, &[image], home, &args), "reused PID");
        assert!(!matches(&record, 100, Path::new("other.exe"), &[image], home, &args));
        assert!(!matches(&record, 100, image, &[image], Path::new("other-home"), &args));
        let mut other = args.clone();
        other[4] = "BEANS_HOME=other-home".into();
        assert!(!matches(&record, 100, image, &[image], home, &other));
        other = args.clone();
        other[2] = "status".into();
        assert!(!matches(&record, 100, image, &[image], home, &other));
        other = args.clone();
        other.extend(["--env".into(), "beans_home=other-home".into()]);
        assert!(!matches(&record, 100, image, &[image], home, &other), "duplicate home evidence");
    }

    #[test]
    fn a_record_whose_pid_was_reused_is_stale_not_an_error() {
        let home = Path::new("account");
        let image = Path::new("beans.exe");
        let record = Record { format: Format::BeansV2, pid: 42, created_at: 100, home: home.into() };
        let args: Vec<String> = ["beans.exe", "service", "run", "--env", "BEANS_HOME=account"].map(String::from).into();
        assert_eq!(identify(&record, 100, image, &[image], home, &args), Identity::Supervisor);
        // After a restart, PID 42 is some other program, created later.
        let other: Vec<String> = ["svchost.exe", "-k", "netsvcs"].map(String::from).into();
        assert_eq!(identify(&record, 555, Path::new(r"C:\Windows\System32\svchost.exe"), &[image], home, &other), Identity::Reused);
        assert_eq!(identify(&record, 555, image, &[image], home, &args), Identity::Reused, "even another beans, created later");
        // Same creation, but not provably this home's supervisor: refused, untouched.
        assert_eq!(identify(&record, 100, Path::new("other.exe"), &[image], home, &args), Identity::Unproven);
        assert_eq!(identify(&record, 100, image, &[image], Path::new("elsewhere"), &args), Identity::Unproven);
        assert_eq!(identify(&Record { pid: 0, ..record }, 100, image, &[image], home, &args), Identity::Unproven);
    }

    #[test]
    fn the_home_is_read_only_from_a_well_formed_service_run_command_line() {
        let line = |args: &[&str]| args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        assert_eq!(command_home(&line(&["beans.exe", "service", "run", "--env", "BEANS_HOME=C:\\h"])), Some("C:\\h"));
        assert_eq!(command_home(&line(&["beans.exe", "service", "run", "--env", "BEANS_PORT=1", "--env", "beans_home=C:\\h=x"])), Some("C:\\h=x"), "names ignore case; values keep =");
        assert_eq!(command_home(&line(&["beans.exe", "service", "run", "--env", "BEANS_HOME="])), Some(""));
        for args in [
            vec!["beans.exe", "service", "run"],
            vec!["beans.exe", "service", "run", "--env"],
            vec!["beans.exe", "service", "run", "--env", "BEANS_HOME"],
            vec!["beans.exe", "service", "run", "--other", "BEANS_HOME=x"],
            vec!["beans.exe", "service", "status", "--env", "BEANS_HOME=x"],
            vec!["beans.exe", "run", "service", "--env", "BEANS_HOME=x"],
            vec!["beans.exe", "service", "run", "--env", "BEANS_HOME=a", "--env", "BEANS_HOME=a"],
            vec!["beans.exe"],
            vec![],
        ] {
            assert_eq!(command_home(&line(&args)), None, "{args:?}");
        }
    }

    #[test]
    fn a_supervisor_the_installer_moved_aside_is_still_recognized() {
        let program = Path::new(r"C:\Users\me\.local\bin\beans.exe");
        let moved = Path::new(r"C:\Users\me\.local\bin\beans.exe.old-0123456789abcdef0123456789ABCDEF");
        let record = Record { format: Format::BeansV2, pid: 42, created_at: 100, home: "account".into() };
        let args: Vec<String> = ["beans.exe", "service", "run", "--env", "BEANS_HOME=account"].map(String::from).into();
        assert!(matches(&record, 100, moved, &[program], Path::new("account"), &args));
        assert!(runs_program(Path::new(r"C:\Users\me\.local\bin\BEANS.EXE.old-0123456789abcdef0123456789abcdef"), &[program]), "Windows names ignore case");
        // Only a sibling renamed the installer's way, and only with the other proofs.
        for image in [
            r"C:\Users\me\Downloads\beans.exe.old-0123456789abcdef0123456789abcdef",
            r"C:\Users\me\.local\bin\beans.exe.old-",
            r"C:\Users\me\.local\bin\beans.exe.old-0123",
            r"C:\Users\me\.local\bin\beans.exe.old-0123456789abcdef0123456789abcdeg",
            r"C:\Users\me\.local\bin\beans.exe.bak",
            r"C:\Users\me\.local\bin\other.exe",
            r"C:\Users\me\.local\bin\beans.exe.old-0123456789abcdef0123456789abcdef\x.exe",
        ] {
            assert!(!runs_program(Path::new(image), &[program]), "{image}");
        }
        assert!(!matches(&record, 101, moved, &[program], Path::new("account"), &args), "a reused PID is still refused");
        // The program a Run key names counts alongside this one.
        let elsewhere = Path::new(r"D:\tools\beans.exe");
        assert!(matches(&record, 100, elsewhere, &[program, elsewhere], Path::new("account"), &args));
        assert!(!matches(&record, 100, elsewhere, &[program], Path::new("account"), &args));
    }

    #[test]
    fn invalid_home_is_rejected_before_reading_or_removing_its_pid() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("service.pid");
        std::fs::write(&path, b"42").unwrap();
        let config = Config { home: home.path().into(), port: 0 };
        assert!(read_record(&config).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"42");
        assert!(!home.path().join("format.json").exists());
    }
}
