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

pub(super) fn matches(record: &Record, created_at: u64, image: &Path, expected_image: &Path, home: &Path, args: &[String]) -> bool {
    record.pid != 0 && record.created_at == created_at && image == expected_image
        && record.home == home && command_home(args).is_some_and(|value| Path::new(value) == home)
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

#[cfg(windows)]
pub(super) fn bound(config: &Config) -> anyhow::Result<Option<(Record, Handle)>> {
    let Some(record) = read_record(config)? else { return Ok(None); };
    anyhow::ensure!(record.pid != 0, "Invalid supervisor PID; data is untouched");
    let Some(handle) = Handle::open(record.pid)? else { return Ok(None); };
    let home = std::fs::canonicalize(&config.home)?;
    anyhow::ensure!(matches(&record, handle.created_at()?, &handle.image()?, &super::exe()?, &home, &handle.args()?),
        "PID does not identify this home's Beans supervisor; data is untouched");
    Ok(Some((record, handle)))
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
        assert!(matches(&record, 100, image, image, home, &args));
        assert!(!matches(&record, 101, image, image, home, &args), "reused PID");
        assert!(!matches(&record, 100, Path::new("other.exe"), image, home, &args));
        assert!(!matches(&record, 100, image, image, Path::new("other-home"), &args));
        let mut other = args.clone();
        other[4] = "BEANS_HOME=other-home".into();
        assert!(!matches(&record, 100, image, image, home, &other));
        other = args.clone();
        other[2] = "status".into();
        assert!(!matches(&record, 100, image, image, home, &other));
        other = args.clone();
        other.extend(["--env".into(), "beans_home=other-home".into()]);
        assert!(!matches(&record, 100, image, image, home, &other), "duplicate home evidence");
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
