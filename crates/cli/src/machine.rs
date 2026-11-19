//! What this computer offers a local model: memory, free disk where the Runner keeps models,
//! cores, and whether ONNX Runtime can run here. Read from the system each time, with no
//! dependency; a fact the system does not give is unknown (`None`), never guessed.

use std::path::Path;

use crate::schedule_models::recommend::Machine;
use crate::schedule_models::runner::RuntimeSupply;

/// This computer, for models kept under `home`, with runtimes from `supply`.
pub(crate) fn this(home: &Path, supply: &RuntimeSupply) -> Machine {
    Machine {
        memory_bytes: memory_bytes(),
        free_disk_bytes: free_disk_bytes(home),
        cores: std::thread::available_parallelism().map_or(0, |n| n.get()),
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        onnx_runtime: onnx_runtime(supply),
        runtime_download_bytes: supply.download_bytes(),
    }
}

/// ONNX and image models need a binary that can load a shared library (a static musl build
/// cannot), and a runtime: beside the binary, or pinned for this platform to download.
fn onnx_runtime(supply: &RuntimeSupply) -> bool {
    cfg!(feature = "embedding-local") && !cfg!(target_env = "musl") && (supply.beside.is_some() || supply.download.is_some())
}

#[cfg(target_os = "linux")]
fn memory_bytes() -> Option<u64> {
    meminfo_total(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

/// `MemTotal` from `/proc/meminfo`, which counts in kibibytes.
#[cfg(any(target_os = "linux", test))]
fn meminfo_total(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

#[cfg(target_os = "macos")]
fn memory_bytes() -> Option<u64> {
    let mut bytes: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    let name = c"hw.memsize";
    // SAFETY: hw.memsize is a 64-bit integer; `size` says how much `bytes` holds.
    let status = unsafe { libc::sysctlbyname(name.as_ptr(), (&mut bytes as *mut u64).cast(), &mut size, std::ptr::null_mut(), 0) };
    (status == 0 && size == std::mem::size_of::<u64>() && bytes > 0).then_some(bytes)
}

#[cfg(windows)]
fn memory_bytes() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: a zeroed MEMORYSTATUSEX with its length set is what the call fills in.
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0 && status.ullTotalPhys > 0).then_some(status.ullTotalPhys)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn memory_bytes() -> Option<u64> {
    None
}

/// Free space for an unprivileged user on the disk holding `path`, or its nearest existing
/// parent (the Runner's model folder may not exist yet).
fn free_disk_bytes(path: &Path) -> Option<u64> {
    let existing = path.ancestors().find(|p| p.exists())?;
    free_at(existing)
}

#[cfg(unix)]
fn free_at(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: a zeroed statvfs is what the call fills in, for a NUL-terminated path.
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut stats) } != 0 {
        return None;
    }
    #[allow(clippy::unnecessary_cast)] // The field types differ between systems.
    (stats.f_bavail as u64).checked_mul(stats.f_frsize as u64)
}

#[cfg(windows)]
fn free_at(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut available: u64 = 0;
    // SAFETY: a NUL-terminated wide path; only the caller's available bytes are asked for.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(available)
}

#[cfg(not(any(unix, windows)))]
fn free_at(_: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_computer_is_read_and_unknowns_stay_unknown() {
        let home = tempfile::tempdir().unwrap();
        let machine = this(&home.path().join("not").join("made").join("yet"), &RuntimeSupply::default());
        assert!(machine.cores >= 1);
        assert_eq!((machine.os.as_str(), machine.arch.as_str()), (std::env::consts::OS, std::env::consts::ARCH));
        if cfg!(any(target_os = "linux", target_os = "macos", windows)) {
            assert!(machine.memory_bytes.is_some_and(|m| m > 64 << 20), "{machine:?}");
            assert!(machine.free_disk_bytes.is_some(), "a missing folder reads its nearest parent");
        }
        assert!(free_disk_bytes(Path::new("")).is_none() || cfg!(windows), "no path, no disk");
        assert!(!machine.onnx_runtime, "no runtime here, none to download");
        assert_eq!(machine.runtime_download_bytes, 0);
    }

    #[test]
    fn a_runtime_beside_the_binary_needs_no_download() {
        let home = tempfile::tempdir().unwrap();
        let pin = crate::embeddings::runtime_download::pinned_for("linux-x64").unwrap();
        let loads = cfg!(feature = "embedding-local") && !cfg!(target_env = "musl");
        let download = RuntimeSupply { beside: None, download: Some(pin.clone()) };
        let downloading = this(home.path(), &download);
        assert_eq!((downloading.onnx_runtime, downloading.runtime_download_bytes), (loads, pin.library_bytes));
        let beside = RuntimeSupply { beside: Some(home.path().join("libonnxruntime.so")), download: Some(pin) };
        let shipped = this(home.path(), &beside);
        assert_eq!((shipped.onnx_runtime, shipped.runtime_download_bytes), (loads, 0));
    }

    #[test]
    fn meminfo_is_read_in_kibibytes_and_nonsense_is_unknown() {
        assert_eq!(meminfo_total("MemTotal:       16314140 kB\nMemFree: 1 kB\n"), Some(16314140 * 1024));
        assert_eq!(meminfo_total("MemFree: 1 kB\n"), None);
        assert_eq!(meminfo_total("MemTotal: lots kB\n"), None);
        assert_eq!(meminfo_total(&format!("MemTotal: {} kB\n", u64::MAX)), None, "no overflow");
        assert_eq!(meminfo_total(""), None);
    }
}
