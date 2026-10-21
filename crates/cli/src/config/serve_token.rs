//! Private, home-bound credentials for the local WebSocket, separate from update control.
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use super::{Config, FormatMarker};

const NAME: &str = "serve-token";

#[cfg(windows)]
#[path = "serve_token_windows.rs"]
mod windows;
#[cfg(windows)]
fn windows_child(parent: &File, name: &str, create: bool) -> std::io::Result<File> { windows::open(parent, name, create, false) }
const MAX_BYTES: u64 = 4096;

pub(crate) fn parse(bytes: &[u8]) -> anyhow::Result<String> {
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| anyhow::anyhow!("Invalid local WebSocket token"))?;
    let raw = crate::keys::unb64_32(text).map_err(|_| anyhow::anyhow!("Invalid local WebSocket token"))?;
    if text.len() != 43 || crate::keys::b64(&raw) != text {
        anyhow::bail!("Invalid local WebSocket token");
    }
    Ok(text.to_owned())
}

impl Config {
    pub fn serve_token_path(&self) -> std::path::PathBuf { self.home.join(NAME) }

    /// Read only. The ordinary reader never admits another user's token, even as root.
    pub fn read_serve_token(&self) -> anyhow::Result<Option<String>> { read(self, false) }

    /// Root update-control clients may read the explicitly selected home's owner's token.
    /// This does not change the independent update-control token's ownership policy.
    pub fn read_control_serve_token(&self) -> anyhow::Result<Option<String>> { read(self, true) }

    #[cfg(windows)]
    pub(super) fn create_private_serve_marker(&self, admission: &super::StorageAdmission) -> anyhow::Result<File> {
        let marker = windows::open(&admission.directory, "format.json", true, false)?;
        crate::update_control::privacy::secure_new(&marker)?;
        Ok(marker)
    }

    #[cfg(windows)]
    pub(super) fn create_private_serve_home(&self) -> anyhow::Result<File> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT};
        let home = std::path::absolute(&self.home)?;
        let parent = home.parent().ok_or_else(|| anyhow::anyhow!("CLI home requires a parent"))?;
        let mut components = parent.components();
        let Some(std::path::Component::Prefix(prefix)) = components.next() else { anyhow::bail!("Absolute Windows home required"); };
        if !matches!(components.next(), Some(std::path::Component::RootDir)) { anyhow::bail!("Absolute Windows home required"); }
        let mut root = std::path::PathBuf::from(prefix.as_os_str());
        root.push("\\");
        let mut directory = std::fs::OpenOptions::new().read(true).custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).open(root)?;
        for component in components {
            let std::path::Component::Normal(name) = component else { anyhow::bail!("Windows home traversal refused"); };
            directory = windows::open(&directory, name.to_str().ok_or_else(|| anyhow::anyhow!("Invalid Windows path"))?, false, true)?;
        }
        let name = home.file_name().and_then(|name| name.to_str()).ok_or_else(|| anyhow::anyhow!("Invalid Windows home"))?;
        if let Ok(existing) = windows::open(&directory, name, false, true) {
            crate::update_control::privacy::validate_object(&existing, false)?;
            if std::fs::read_dir(&home)?.next().transpose()?.is_some() { anyhow::bail!("Unmarked home is not empty"); }
            return Ok(existing);
        }
        let created = windows::open(&directory, name, true, true)?;
        crate::update_control::privacy::secure_new(&created)?;
        Ok(created)
    }

    pub(crate) fn provision_serve_token(&self) -> anyhow::Result<String> {
        if let Some(token) = self.read_serve_token()? { return Ok(token); }
        #[cfg(unix)]
        let _admission = super::StorageAdmission::acquire(&self.home)?;
        if !self.validate_home()? { anyhow::bail!("Local WebSocket token needs an admitted Beans home"); }
        let directory = Directory::open(&self.home, false)?;
        let token = crate::keys::b64(&crate::keys::random_32());
        let name = format!(".serve-token-{}", uuid::Uuid::new_v4());
        let mut file = directory.create(&name)?;
        let result = (|| -> anyhow::Result<bool> {
            file.write_all(token.as_bytes())?;
            file.sync_all()?;
            directory.verify()?;
            directory.publish(&name, &file)
        })();
        directory.remove(&name, &file, matches!(&result, Ok(true)));
        drop(file);
        result?;
        drop(directory);
        self.read_serve_token()?.ok_or_else(|| anyhow::anyhow!("Local WebSocket token publication failed"))
    }
}

fn read(config: &Config, control: bool) -> anyhow::Result<Option<String>> {
    let directory = match Directory::open(&config.home, control) {
        Ok(directory) => directory,
        Err(error) if error.downcast_ref::<std::io::Error>().is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {
            if !config.validate_home()? { return Ok(None); }
            return Err(error);
        }
        Err(error) => return Err(error),
    };
    let mut file = match directory.open_file(NAME) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    directory.validate_file(&file, true)?;
    let before = identity(&file)?;
    directory.verify()?;
    let mut bytes = Vec::new();
    (&mut file).take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES { anyhow::bail!("Local WebSocket token is too large"); }
    let current = directory.open_file(NAME)?;
    directory.validate_file(&current, true)?;
    if identity(&current)? != before || identity(&file)? != before {
        anyhow::bail!("Local WebSocket token changed during admission");
    }
    directory.verify()?;
    Ok(Some(parse(&bytes)?))
}

#[cfg(unix)]
fn identity(file: &File) -> std::io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata()?;
    Ok((m.dev(), m.ino()))
}

#[cfg(unix)]
struct Directory {
    admission: super::StorageAdmission,
    owner: u32,
    control: bool,
    marker: File,
    marker_bytes: Vec<u8>,
}

#[cfg(unix)]
impl Directory {
    fn open(home: &Path, control: bool) -> anyhow::Result<Self> {
        use std::os::unix::fs::MetadataExt;
        let (mut directory, normalized, mut ancestors) = super::StorageAdmission::platform_entry(home)?;
        for component in normalized.components() {
            match component {
                std::path::Component::Normal(name) => {
                    let next = super::StorageAdmission::open_component(&directory, name)?;
                    ancestors.push(directory);
                    directory = next;
                }
                std::path::Component::RootDir | std::path::Component::CurDir => {},
                _ => anyhow::bail!("Local WebSocket home traversal refused"),
            }
        }
        let admission = super::StorageAdmission { directory, home: home.into(), ancestors };
        let owner = admission.directory.metadata()?.uid();
        let control = control && unsafe { libc::geteuid() } == 0;
        let mut marker = admission.marker()?;
        let marker_bytes = super::StorageAdmission::read_marker(&mut marker)?;
        serde_json::from_slice::<FormatMarker>(&marker_bytes)?;
        let result = Self { admission, owner, control, marker, marker_bytes };
        result.validate_file(&result.admission.directory, false)?;
        result.validate_file(&result.marker, false)?;
        if result.control {
            for ancestor in &result.admission.ancestors { result.validate_file(ancestor, false)?; }
        }
        result.verify()?;
        Ok(result)
    }

    fn validate_file(&self, file: &File, private: bool) -> anyhow::Result<()> {
        use std::os::unix::fs::MetadataExt;
        let m = file.metadata()?;
        let owner = if self.control { self.owner } else { unsafe { libc::geteuid() } };
        if (m.uid() != 0 && m.uid() != owner) || m.mode() & (if private { 0o077 } else { 0o022 }) != 0
            || (private && (!m.is_file() || m.len() > MAX_BYTES)) {
            anyhow::bail!("Local WebSocket credential objects must have private ownership and permissions");
        }
        Ok(())
    }

    fn verify(&self) -> anyhow::Result<()> {
        self.admission.verify(&self.marker, &self.marker_bytes)?;
        self.validate_file(&self.admission.directory, false)?;
        if self.control {
            // Reopen the complete path and compare every retained ancestor, not only the leaf.
            let (mut entry, normalized, mut ancestors) = super::StorageAdmission::platform_entry(&self.admission.home)?;
            for component in normalized.components() {
                match component {
                    std::path::Component::Normal(name) => {
                        let next = super::StorageAdmission::open_component(&entry, name)?;
                        ancestors.push(entry);
                        entry = next;
                    }
                    std::path::Component::RootDir | std::path::Component::CurDir => {},
                    _ => anyhow::bail!("Local WebSocket home traversal is forbidden"),
                }
            }
            if ancestors.len() != self.admission.ancestors.len() { anyhow::bail!("Local WebSocket home ancestry changed"); }
            for (a, b) in ancestors.iter().zip(&self.admission.ancestors) {
                if identity(a)? != identity(b)? { anyhow::bail!("Local WebSocket home ancestry changed"); }
                self.validate_file(a, false)?;
            }
        }
        Ok(())
    }

    fn open_file(&self, name: &str) -> std::io::Result<File> {
        use std::os::fd::{AsRawFd, FromRawFd};
        let name = std::ffi::CString::new(name)?;
        let fd = unsafe { libc::openat(self.admission.directory.as_raw_fd(), name.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if fd < 0 { return Err(std::io::Error::last_os_error()); }
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    fn create(&self, name: &str) -> anyhow::Result<File> {
        self.admission.open_child(&std::ffi::CString::new(name)?, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)
    }

    fn publish(&self, name: &str, file: &File) -> anyhow::Result<bool> {
        use std::os::fd::AsRawFd;
        let dir = self.admission.directory.as_raw_fd();
        let name = std::ffi::CString::new(name)?;
        let target = c"serve-token";
        let current = self.open_file(name.to_str()?)?;
        if identity(&current)? != identity(file)? { anyhow::bail!("Token temporary changed before publication"); }
        if unsafe { libc::linkat(dir, name.as_ptr(), dir, target.as_ptr(), 0) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::AlreadyExists { return Err(error.into()); }
        }
        self.admission.directory.sync_all()?;
        Ok(false) // Unix publication leaves the temporary link to remove in either case.
    }

    fn remove(&self, name: &str, file: &File, _published: bool) {
        use std::os::fd::AsRawFd;
        if self.open_file(name).and_then(|current| Ok(identity(&current)? == identity(file)?)).unwrap_or(false) {
            if let Ok(name) = std::ffi::CString::new(name) {
                unsafe { libc::unlinkat(self.admission.directory.as_raw_fd(), name.as_ptr(), 0); }
            }
        }
    }
}

#[cfg(windows)]
fn identity(file: &File) -> std::io::Result<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 { return Err(std::io::Error::last_os_error()); }
    Ok((u64::from(info.dwVolumeSerialNumber), (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)))
}

#[cfg(windows)]
struct Directory {
    home: std::path::PathBuf,
    directories: Vec<(std::path::PathBuf, File)>,
    marker: File,
    marker_bytes: Vec<u8>,
}

#[cfg(windows)]
impl Directory {
    fn open(home: &Path, _control: bool) -> anyhow::Result<Self> {
        let home = std::path::absolute(home)?;
        let directories = Self::open_directories(&home)?;
        let mut marker = windows_child(&directories.last().expect("admitted home").1, "format.json", false)?;
        crate::update_control::privacy::validate_object(&marker, false)?;
        if !marker.metadata()?.is_file() || marker.metadata()?.len() > 256 { anyhow::bail!("Invalid Beans format marker"); }
        let mut marker_bytes = Vec::new();
        (&mut marker).take(257).read_to_end(&mut marker_bytes)?;
        serde_json::from_slice::<FormatMarker>(&marker_bytes)?;
        let result = Self { home, directories, marker, marker_bytes };
        result.verify()?;
        Ok(result)
    }
    fn open_file(&self, name: &str) -> std::io::Result<File> {
        windows_child(&self.directories.last().expect("admitted home").1, name, false)
    }
    fn validate_file(&self, file: &File, private: bool) -> anyhow::Result<()> {
        crate::update_control::privacy::validate_object(file, private)?;
        if private && (!file.metadata()?.is_file() || file.metadata()?.len() > MAX_BYTES) { anyhow::bail!("Invalid local WebSocket token file"); }
        Ok(())
    }
    fn verify(&self) -> anyhow::Result<()> {
        let current = Self::open_directories(&self.home)?;
        if current.len() != self.directories.len() { anyhow::bail!("Local WebSocket ancestry changed"); }
        for ((_, current), (_, retained)) in current.iter().zip(&self.directories) {
            if identity(current)? != identity(retained)? { anyhow::bail!("Local WebSocket home changed"); }
        }
        let mut marker = self.open_file("format.json")?;
        crate::update_control::privacy::validate_object(&marker, false)?;
        if identity(&marker)? != identity(&self.marker)? { anyhow::bail!("Beans format marker changed"); }
        let mut bytes = Vec::new();
        (&mut marker).take(257).read_to_end(&mut bytes)?;
        if bytes != self.marker_bytes { anyhow::bail!("Beans format marker changed"); }
        Ok(())
    }
    fn open_directories(home: &Path) -> anyhow::Result<Vec<(std::path::PathBuf, File)>> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT};
        let mut components = home.components();
        let Some(std::path::Component::Prefix(prefix)) = components.next() else { anyhow::bail!("Absolute Windows home required"); };
        if !matches!(components.next(), Some(std::path::Component::RootDir)) { anyhow::bail!("Absolute Windows home required"); }
        let mut path = std::path::PathBuf::from(prefix.as_os_str());
        path.push("\\");
        let root = std::fs::OpenOptions::new().read(true).custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).open(&path)?;
        let mut result = vec![(path.clone(), root)];
        for component in components {
            let std::path::Component::Normal(name) = component else { anyhow::bail!("Windows home traversal refused"); };
            let name = name.to_str().ok_or_else(|| anyhow::anyhow!("Invalid Windows home component"))?;
            let child = windows::open(&result.last().expect("volume root").1, name, false, true)?;
            path.push(name);
            result.push((path.clone(), child));
        }
        crate::update_control::privacy::validate_object(&result.last().expect("admitted home").1, false)?;
        Ok(result)
    }
    fn create(&self, name: &str) -> anyhow::Result<File> {
        let file = windows_child(&self.directories.last().expect("admitted home").1, name, true)?;
        crate::update_control::privacy::secure_new(&file)?;
        self.validate_file(&file, true)?;
        Ok(file)
    }
    fn publish(&self, _name: &str, file: &File) -> anyhow::Result<bool> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Wdk::Storage::FileSystem::{NtSetInformationFile, FileRenameInformation};
        use windows_sys::Win32::Storage::FileSystem::FILE_RENAME_INFO;
        use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
        let name: Vec<u16> = NAME.encode_utf16().collect();
        let offset = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
        let size = std::mem::size_of::<FILE_RENAME_INFO>().max(offset + name.len() * 2);
        let mut buffer = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
        let rename = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        unsafe {
            (*rename).Anonymous.ReplaceIfExists = false;
            (*rename).RootDirectory = self.directories.last().expect("admitted home").1.as_raw_handle();
            (*rename).FileNameLength = (name.len() * 2) as u32;
            std::ptr::copy_nonoverlapping(name.as_ptr(), std::ptr::addr_of_mut!((*rename).FileName).cast(), name.len());
        }
        let mut io: IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };
        let status = unsafe { NtSetInformationFile(file.as_raw_handle(), &mut io, rename.cast(), size as u32, FileRenameInformation) };
        if status == windows_sys::Win32::Foundation::STATUS_OBJECT_NAME_COLLISION { return Ok(false); }
        if status < 0 { anyhow::bail!("Local WebSocket token publication failed: {status:#x}"); }
        Ok(true)
    }
    fn remove(&self, _name: &str, file: &File, published: bool) {
        use std::os::windows::io::AsRawHandle;
        if !published {
            let mut io = unsafe { std::mem::zeroed() };
            let delete: u8 = 1;
            unsafe { windows_sys::Wdk::Storage::FileSystem::NtSetInformationFile(file.as_raw_handle(), &mut io, (&delete as *const u8).cast(), 1, windows_sys::Wdk::Storage::FileSystem::FileDispositionInformation); }
        }
    }
}

#[cfg(not(any(unix, windows)))]
compile_error!("Local WebSocket private token admission requires Unix or Windows");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_token_reuses_private_bytes_and_refuses_corruption_without_repair() {
        let home = std::env::temp_dir().join(format!("beans-token-{}", uuid::Uuid::new_v4()));
        let config = Config { home: home.clone(), port: 0 };
        config.ensure_home().unwrap();
        let token = config.provision_serve_token().unwrap();
        assert_eq!(config.provision_serve_token().unwrap(), token);
        assert_eq!(config.read_serve_token().unwrap(), Some(token));
        let path = config.serve_token_path();
        std::fs::write(&path, b"malformed").unwrap();
        assert!(config.provision_serve_token().is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"malformed");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "MIXED-UID UNVERIFIED until run as root with private TMPDIR and actual update handshake acceptance"]
    fn root_control_reader_accepts_only_selected_home_owner_or_root() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } != 0 {
            panic!("MIXED-UID UNVERIFIED: run this isolated acceptance as root; do not count same-UID coverage as PASS");
        }
        let home = std::env::temp_dir().join(format!("beans-root-token-{}", uuid::Uuid::new_v4()));
        let config = Config { home: home.clone(), port: 0 };
        config.ensure_home().unwrap();
        let token = config.provision_serve_token().unwrap();
        let owner = 65534;
        for path in [&home, &home.join("format.json"), &config.serve_token_path()] {
            use std::os::unix::ffi::OsStrExt;
            let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::chown(path.as_ptr(), owner, u32::MAX) }, 0);
        }
        assert!(config.read_serve_token().is_err());
        // Fixture must have private root/U ancestry; a public temporary ancestor is refused.
        let parent = home.parent().unwrap();
        if std::fs::metadata(parent).unwrap().permissions().mode() & 0o022 != 0 {
            std::fs::remove_dir_all(home).unwrap();
            panic!("MIXED-UID UNVERIFIED: TMPDIR must be an isolated root-owned private directory");
        }
        assert_eq!(config.read_control_serve_token().unwrap(), Some(token));
        let path = config.serve_token_path();
        use std::os::unix::ffi::OsStrExt;
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::chown(name.as_ptr(), 65533, u32::MAX) }, 0);
        assert!(config.read_control_serve_token().is_err());
        std::fs::remove_dir_all(home).unwrap();
    }
}

#[cfg(windows)]
pub(super) fn validate_admission(admission: &super::StorageAdmission, home: &Path) -> anyhow::Result<()> {
    let directory = Directory::open(home, false)?;
    let current = &directory.directories.last().expect("admitted home").1;
    if identity(current)? != identity(&admission.directory)? { anyhow::bail!("Windows storage home changed during admission"); }
    directory.verify()
}
