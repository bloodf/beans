//! Where the CLI keeps things and how it is configured.

mod membership_preflight;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_PORT: u16 = 4874;
pub const DEV_PORT: u16 = 4875;
pub const FORMAT: &str = "beans-v2";
pub const SQLITE_APPLICATION_ID: u32 = 0x424E5332;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    #[serde(rename = "beans-v2")]
    BeansV2,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FormatMarker {
    format: Format,
}

/// Storage-only protocol-6 marker; ordinary account admission still uses FormatMarker.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StagedFormatMarker {
    format: Format,
    min_core_protocol: u32,
}
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SELF_UPDATE_UNAVAILABLE: &str = "CLI self-update is unavailable in Beans; use the signed Beans update mechanism.";

#[derive(Debug, Clone)]
pub struct Config {
    pub home: PathBuf,
    pub port: u16,
}

impl Config {
    pub fn load(home_override: Option<PathBuf>, port_override: Option<u16>) -> Self {
        let dev = std::env::var("BEANS_DEV").ok().is_some_and(|value| !value.is_empty() && value != "0");
        let home = home_override
            .or_else(|| std::env::var_os("BEANS_HOME").map(PathBuf::from))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
                .join(if dev { ".beans-dev-v2" } else { ".beans-v2" }));
        let port = port_override
            .or_else(|| std::env::var("BEANS_PORT").ok().and_then(|p| p.parse().ok()))
            .unwrap_or(if dev { DEV_PORT } else { DEFAULT_PORT });
        Config { home, port }
    }

    pub fn identity_path(&self) -> PathBuf {
        self.home.join("identity.json")
    }
    pub fn machine_path(&self) -> PathBuf {
        self.home.join("machine.json")
    }
    pub fn credentials_path(&self) -> PathBuf {
        self.home.join("credentials.json")
    }
    pub fn database_path(&self) -> PathBuf {
        self.home.join("beans.sqlite3")
    }
    pub fn settings_path(&self) -> PathBuf {
        self.home.join("settings.json")
    }
    /// A validated public model catalog cached from this Device's selected relay.
    pub fn catalog_path(&self) -> PathBuf {
        self.home.join("catalog.json")
    }

    /// A validated public marketplace index cached from this Device's selected relay.
    pub fn marketplace_path(&self) -> PathBuf {
        self.home.join("marketplace.json")
    }

    /// Attachment bytes by id, sent from here or fetched from the relay.
    pub fn files_dir(&self) -> PathBuf {
        self.home.join("files")
    }

    /// Installed plugins: `installed.json`, `secrets.json`, and a folder per plugin.
    pub fn plugins_dir(&self) -> PathBuf {
        self.home.join("plugins")
    }

    /// The MCP servers the user adds themselves, in the format Claude Desktop and Cursor share.
    pub fn mcp_path(&self) -> PathBuf {
        self.home.join("mcp.json")
    }

    fn membership_preflight(&self) -> anyhow::Result<()> {
        membership_preflight::preflight(&self.home)
    }

    /// The CLI's own updates: the last check, a restart into a new release under way, and a
    /// release that went back.
    pub fn update_path(&self) -> PathBuf {
        self.home.join("update.json")
    }

    /// Read-only admission, before permissions, recovery, or account normalization.
    pub fn validate_home(&self) -> anyhow::Result<bool> {
        match std::fs::symlink_metadata(&self.home) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
            Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() =>
                anyhow::bail!("Beans storage must be a private directory, not a symlink"),
            Ok(_) => {}
        }
        self.membership_preflight()?;
        let marker = self.home.join("format.json");
        if std::fs::symlink_metadata(&marker).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            anyhow::bail!("Beans v2 format marker must not be a symlink; data is untouched");
        }
        let marked = read_json_strict::<FormatMarker>(&marker)?;
        if marked.is_none() {
            if std::fs::read_dir(&self.home)?.next().transpose()?.is_some() {
                anyhow::bail!("Beans v2 migration required for {}; existing data is untouched", self.home.display());
            }
            return Ok(false);
        }
        for entry in std::fs::read_dir(&self.home)? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() {
                anyhow::bail!("Beans v2 storage contains a symlink; data is untouched");
            }
            let path = entry.path();
            if path.file_name().is_some_and(|name| name == "state.json") {
                anyhow::bail!("Beans v2 migration required for {}; existing data is untouched", self.home.display());
            }
            if path.extension().is_some_and(|extension| extension == "sqlite3") {
                validate_database(&path)?;
            }
        }
        let identity = read_json_strict::<crate::keys::IdentityFile>(&self.identity_path())?;
        let machine = read_json_strict::<crate::keys::MachineFile>(&self.machine_path())?;
        if identity.is_some() && machine.is_none() {
            anyhow::bail!("Beans identity is missing its machine record; existing data is untouched");
        }
        if let Some(identity) = identity {
            identity.identity()?;
        }
        if let Some(machine) = machine {
            machine.machine()?;
            machine.dek()?;
            crate::keys::unb64_32(&machine.identity_pubkey)?;
            crate::keys::unb64_32(&machine.content_pubkey)?;
        }
        read_json_strict::<crate::credentials::Credentials>(&self.credentials_path())?;
        read_json_strict::<Settings>(&self.settings_path())?;
        crate::plugins::validate_account_files(self)?;
        Ok(true)
    }

    pub fn ensure_home(&self) -> anyhow::Result<()> {
        let _admission = self.ensure_home_admission()?;
        Ok(())
    }

    /// Acquire before creation and return exclusion to the caller through its DB effects.
    pub fn ensure_home_admission(&self) -> anyhow::Result<Option<StorageAdmission>> {
        #[cfg(unix)]
        let mut admission = Some(StorageAdmission::acquire_creation(&self.home)?);
        #[cfg(not(unix))]
        let admission: Option<StorageAdmission> = None;
        let marked = self.validate_home()?;
        std::fs::create_dir_all(&self.home)?;
        #[cfg(unix)]
        if let Some(guard) = admission.as_mut() { guard.attach_created_home()?; }
        set_private(&self.home)?;
        if !marked {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)] {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(self.home.join("format.json"))?;
            file.write_all(&serde_json::to_vec(&FormatMarker { format: Format::BeansV2 })?)?;
            file.sync_all()?;
        }
        #[cfg(unix)]
        if let Some(guard) = admission.as_ref() {
            guard.directory.sync_all()?;
            for ancestor in &guard.ancestors { ancestor.sync_all()?; }
            guard.revalidate_legacy()?;
        }
        self.validate_home()?;
        Ok(admission)
    }

    /// Explicit transition of an empty marked scratch home, coordinated with store admission.
    pub fn upgrade_staged_storage(&self) -> anyhow::Result<()> {
        #[cfg(not(unix))]
        anyhow::bail!("Staged storage requires Unix descriptor admission");
        #[cfg(unix)] {
            use std::io::Write;
            use std::os::fd::AsRawFd;
            let admission = StorageAdmission::acquire(&self.home)?;
            self.membership_preflight()?;
            let mut marker = admission.marker()?;
            let original = StorageAdmission::read_marker(&mut marker)?;
            serde_json::from_slice::<FormatMarker>(&original)?;
            if !self.validate_home()? {
                anyhow::bail!("Staged storage requires an existing Beans v2 marker");
            }
            for entry in std::fs::read_dir(&self.home)? {
                if entry?.file_name() != "format.json" {
                    anyhow::bail!("Staged storage accepts only an empty marked scratch home");
                }
            }
            admission.verify(&marker, &original)?;
            let temporary = std::ffi::CString::new(format!(".format-{}", uuid::Uuid::new_v4()))?;
            let mut file = admission.open_child(&temporary, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)?;
            let result = (|| -> anyhow::Result<()> {
                file.write_all(br#"{"format":"beans-v2","min_core_protocol":6}"#)?;
                file.sync_all()?;
                admission.verify(&marker, &original)?;
                let target = std::ffi::CString::new("format.json")?;
                // Both names resolve beneath the locked, retained directory descriptor.
                if unsafe { libc::renameat(admission.directory.as_raw_fd(), temporary.as_ptr(), admission.directory.as_raw_fd(), target.as_ptr()) } != 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                admission.revalidate_staged()
            })();
            if result.is_err() {
                unsafe { libc::unlinkat(admission.directory.as_raw_fd(), temporary.as_ptr(), 0) };
            }
            result
        }
    }

    /// The production CLI invokes this before constructing App or loading account state.
    pub fn preload_staged_storage(&self) -> anyhow::Result<crate::local_store::LocalStore> {
        self.upgrade_staged_storage()?;
        crate::local_store::LocalStore::open_staged(&self.database_path())
    }

    /// Retain this guard through SQLite admission and setup; errors precede database mutation.
    pub fn staged_storage_admission(&self) -> anyhow::Result<StorageAdmission> {
        let admission = StorageAdmission::acquire(&self.home)?;
        admission.revalidate_staged()?;
        Ok(admission)
    }
}

/// A directory-scoped cross-process admission lock, released by closing its descriptor.
pub struct StorageAdmission {
    directory: std::fs::File,
    home: PathBuf,
    ancestors: Vec<std::fs::File>,
}

impl StorageAdmission {
    pub fn acquire(home: &Path) -> anyhow::Result<Self> {
        #[cfg(not(unix))]
        anyhow::bail!("Descriptor storage admission is unsupported on this platform");
        #[cfg(unix)] {
            Self::acquire_path(home, false)
        }
    }

    #[cfg(unix)]
    fn acquire_creation(home: &Path) -> anyhow::Result<Self> {
        Self::acquire_path(home, true)
    }

    #[cfg(unix)]
    fn acquire_path(home: &Path, allow_missing: bool) -> anyhow::Result<Self> {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::ffi::OsStrExt;
        let mut directory = Self::open_directory(Path::new(if home.is_absolute() { "/" } else { "." }))?;
        let mut ancestors = Vec::new();
        let segments: Vec<_> = home.components().filter_map(|part| match part {
            std::path::Component::Normal(segment) => Some(Ok(segment)),
            std::path::Component::RootDir | std::path::Component::CurDir => None,
            _ => Some(Err(anyhow::anyhow!("Storage admission refuses parent traversal"))),
        }).collect::<anyhow::Result<_>>()?;
        for segment in segments {
            Self::lock(&directory, libc::LOCK_SH)?;
            let name = std::ffi::CString::new(segment.as_bytes())?;
            let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
            if fd < 0 {
                let error = std::io::Error::last_os_error();
                if allow_missing && error.kind() == std::io::ErrorKind::NotFound {
                    Self::lock(&directory, libc::LOCK_EX)?;
                    return Ok(Self { directory, home: home.into(), ancestors });
                }
                return Err(error.into());
            }
            ancestors.push(directory);
            directory = unsafe { std::fs::File::from_raw_fd(fd) };
        }
        Self::lock(&directory, libc::LOCK_EX)?;
        Ok(Self { directory, home: home.into(), ancestors })
    }

    #[cfg(unix)]
    fn lock(file: &std::fs::File, mode: i32) -> anyhow::Result<()> {
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(file.as_raw_fd(), mode | libc::LOCK_NB) } != 0 {
            anyhow::bail!("Storage admission is busy or uncertain");
        }
        Ok(())
    }

    #[cfg(unix)]
    fn attach_created_home(&mut self) -> anyhow::Result<()> {
        use std::os::unix::fs::MetadataExt;
        let directory = Self::open_directory(&self.home)?;
        if directory.metadata()?.dev() != self.directory.metadata()?.dev()
            || directory.metadata()?.ino() != self.directory.metadata()?.ino() {
            Self::lock(&directory, libc::LOCK_EX)?;
            let ancestor = std::mem::replace(&mut self.directory, directory);
            self.ancestors.push(ancestor);
        }
        Ok(())
    }

    pub(crate) fn revalidate_legacy(&self) -> anyhow::Result<()> {
        #[cfg(not(unix))]
        anyhow::bail!("Descriptor storage admission is unsupported on this platform");
        #[cfg(unix)] {
            let mut marker = self.marker()?;
            let bytes = Self::read_marker(&mut marker)?;
            serde_json::from_slice::<FormatMarker>(&bytes)?;
            marker.sync_all()?;
            self.directory.sync_all()?;
            self.verify(&marker, &bytes)?;
            if Self::read_marker(&mut marker)? != bytes { anyhow::bail!("Legacy marker changed after sync"); }
            Ok(())
        }
    }

    /// Bind borrowed admission to the actual requested database parent, not path spelling.
    pub(crate) fn validate_target(&self, home: &Path) -> anyhow::Result<()> {
        #[cfg(not(unix))]
        anyhow::bail!("Descriptor storage admission is unsupported on this platform");
        #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            let target = Self::open_directory(home)?.metadata()?;
            let retained = self.directory.metadata()?;
            if target.dev() != retained.dev() || target.ino() != retained.ino() {
                anyhow::bail!("Storage admission guard belongs to a different home");
            }
            Ok(())
        }
    }

    #[cfg(unix)]
    fn open_directory(home: &Path) -> anyhow::Result<std::fs::File> {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::ffi::OsStrExt;
        let mut options = std::fs::OpenOptions::new();
        use std::os::unix::fs::OpenOptionsExt;
        options.read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
        let mut directory = options.open(if home.is_absolute() { "/" } else { "." })?;
        for part in home.components() {
            let segment = match part {
                std::path::Component::Normal(segment) => segment,
                std::path::Component::RootDir | std::path::Component::CurDir => continue,
                _ => anyhow::bail!("Storage admission refuses parent traversal"),
            };
            let name = std::ffi::CString::new(segment.as_bytes())?;
            let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
            if fd < 0 { return Err(std::io::Error::last_os_error().into()); }
            directory = unsafe { std::fs::File::from_raw_fd(fd) };
        }
        Ok(directory)
    }

    #[cfg(unix)]
    fn open_child(&self, name: &std::ffi::CStr, flags: i32) -> anyhow::Result<std::fs::File> {
        use std::os::fd::{AsRawFd, FromRawFd};
        let fd = unsafe { libc::openat(self.directory.as_raw_fd(), name.as_ptr(), flags | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC, 0o600) };
        if fd < 0 { return Err(std::io::Error::last_os_error().into()); }
        Ok(unsafe { std::fs::File::from_raw_fd(fd) })
    }

    #[cfg(unix)]
    fn marker(&self) -> anyhow::Result<std::fs::File> {
        self.open_child(&std::ffi::CString::new("format.json")?, libc::O_RDONLY)
    }

    #[cfg(unix)]
    fn read_marker(file: &mut std::fs::File) -> anyhow::Result<Vec<u8>> {
        use std::io::{Read, Seek};
        if !file.metadata()?.is_file() || file.metadata()?.len() > 256 {
            anyhow::bail!("Storage marker must be a bounded regular file");
        }
        file.rewind()?;
        let mut bytes = Vec::with_capacity(257);
        file.take(257).read_to_end(&mut bytes)?;
        if bytes.len() > 256 { anyhow::bail!("Storage marker exceeds its bound"); }
        Ok(bytes)
    }

    #[cfg(unix)]
    fn verify(&self, marker: &std::fs::File, bytes: &[u8]) -> anyhow::Result<()> {
        use std::os::unix::fs::MetadataExt;
        let same = |a: std::fs::Metadata, b: std::fs::Metadata| a.dev() == b.dev() && a.ino() == b.ino();
        if !same(self.directory.metadata()?, Self::open_directory(&self.home)?.metadata()?) {
            anyhow::bail!("Storage directory changed during admission");
        }
        let mut current = self.marker()?;
        if !same(marker.metadata()?, current.metadata()?) || Self::read_marker(&mut current)? != bytes {
            anyhow::bail!("Storage marker changed during admission");
        }
        Ok(())
    }

    pub fn revalidate_staged(&self) -> anyhow::Result<()> {
        #[cfg(not(unix))]
        anyhow::bail!("Staged durability admission is unsupported on this platform");
        #[cfg(unix)] {
            self.revalidate_with_sync(|file| file.sync_all())
        }
    }

    #[cfg(unix)]
    fn revalidate_with_sync(&self, mut sync: impl FnMut(&std::fs::File) -> std::io::Result<()>) -> anyhow::Result<()> {
        self.validate_target(&self.home)?;
        // Every staged consumer, including borrowed Store admission, reaches this under
        // its retained guard before marker sync or SQLite effects.
        Config { home: self.home.clone(), port: 0 }.membership_preflight()?;
        let mut marker = self.marker()?;
        let bytes = Self::read_marker(&mut marker)?;
        let parsed: StagedFormatMarker = serde_json::from_slice(&bytes)?;
        if parsed.min_core_protocol != 6 { anyhow::bail!("Staged storage requires min_core_protocol 6"); }
        sync(&marker)?;
        sync(&self.directory)?;
        self.verify(&marker, &bytes)?;
        if Self::read_marker(&mut marker)? != bytes { anyhow::bail!("Storage marker content changed after sync"); }
        Ok(())
    }
}

/// Persistent settings the app can change at runtime.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub relay_url: Option<String>,
    /// An explicit marketplace feed override; otherwise use the selected relay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketplace_url: Option<String>,
    /// Retained self-update preference; Beans does not enable the unadapted updater.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_update: Option<bool>,
}

impl Settings {
    pub fn load(config: &Config) -> Self {
        read_json(&config.settings_path()).unwrap_or_default()
    }

    pub fn save(&self, config: &Config) -> anyhow::Result<()> {
        write_json_private(&config.settings_path(), self)
    }

    /// `BEANS_RELAY_URL` wins over the saved setting.
    pub fn effective_relay_url(&self) -> Option<String> {
        std::env::var("BEANS_RELAY_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.relay_url.clone())
            .map(|url| url.trim().trim_end_matches('/').to_string())
            .filter(|url| !url.is_empty())
    }
}

/// The relay the launching app ships with (`BEANS_DEFAULT_RELAY_URL`), used when nothing else
/// names one.
pub fn default_relay_url() -> Option<String> {
    std::env::var("BEANS_DEFAULT_RELAY_URL")
        .ok()
        .map(|url| url.trim().trim_end_matches('/').to_string())
        .filter(|url| !url.is_empty())
}

/// The relay port `bun run dev` and `bun run relay` listen on.
pub const DEV_RELAY_PORT: u16 = 8787;

/// In dev (`BEANS_DEV=1`, set by the dev loop), a Device with no relay configured uses the
/// relay the dev loop runs on this machine, addressed by this computer's LAN IP so a phone on the
/// same network can reach it through the pairing code.
pub fn dev_relay_url() -> Option<String> {
    if std::env::var("BEANS_DEV").ok().filter(|v| !v.is_empty() && v != "0").is_none() {
        return None;
    }
    let host = lan_ip().map(|ip| ip.to_string()).unwrap_or_else(|| "127.0.0.1".into());
    Some(format!("http://{host}:{DEV_RELAY_PORT}"))
}

/// This machine's address on the local network: a private IPv4 on a real interface (Wi-Fi or
/// Ethernet), never a VPN tunnel, which a default route would pick. Falls back to the source
/// address of a route to a public host.
pub fn lan_ip() -> Option<std::net::IpAddr> {
    let mut candidates: Vec<(u8, std::net::Ipv4Addr)> = Vec::new();
    for iface in if_addrs::get_if_addrs().unwrap_or_default() {
        let std::net::IpAddr::V4(ip) = iface.ip() else { continue };
        if ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() {
            continue;
        }
        let name = iface.name.to_lowercase();
        if ["utun", "tun", "tap", "bridge", "docker", "vmnet", "awdl", "llw", "ipsec", "ppp", "wg", "zt"].iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let rank = if ip.is_private() { 0 } else { 1 };
        candidates.push((rank, ip));
    }
    candidates.sort();
    if let Some((_, ip)) = candidates.first() {
        return Some(std::net::IpAddr::V4(*ip));
    }
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("1.1.1.1:80").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    if ip.is_loopback() || ip.is_unspecified() {
        None
    } else {
        Some(ip)
    }
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Missing is distinct from unreadable, malformed, or unversioned account data.
pub fn read_json_strict<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<Option<T>> {
    use anyhow::Context;
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    serde_json::from_slice(&bytes).map(Some)
        .with_context(|| format!("Beans v2 account record invalid at {}; migration required, data untouched", path.display()))
}

/// Check the SQLite header without opening SQLite or touching WAL/SHM files.
pub fn validate_database(path: &Path) -> anyhow::Result<()> {
    use std::io::Read;
    let mut header = [0; 72];
    let mut file = std::fs::File::open(path)?;
    if file.read_exact(&mut header).is_err()
        || &header[..16] != b"SQLite format 3\0"
        || u32::from_be_bytes(header[68..72].try_into().expect("SQLite application id")) != SQLITE_APPLICATION_ID {
        anyhow::bail!("Beans v2 migration required for {}; database is untouched", path.display());
    }
    Ok(())
}

/// Writes atomically with mode 0600.
pub fn write_json_private<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    write_private(path, &serde_json::to_vec_pretty(value)?)
}

/// Writes `bytes` atomically with mode 0600.
pub fn write_private(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp-{}-{n}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    set_private(&tmp)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(unix)]
pub fn set_private(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(path)?;
    let mode = if metadata.is_dir() { 0o700 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
pub fn set_private(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

pub fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn now_unix() -> i64 {
    now_secs() as i64
}

#[cfg(test)]
mod format_tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn staged_production_preload_refuses_uncertain_admission() {
        use std::os::unix::fs::symlink;
        let home = tempfile::tempdir().unwrap();
        let config = Config { home: home.path().into(), port: 0 };
        let fresh_home = home.path().join("fresh-parent");
        let fresh = Config { home: fresh_home, port: 0 };
        assert!(!fresh.home.exists());
        let precreation = StorageAdmission::acquire_creation(&fresh.home).unwrap();
        // Actual App and direct store cannot reach their first mkdir under exclusion.
        assert!(crate::app::App::load(fresh.clone()).is_err());
        assert!(crate::local_store::LocalStore::open(&fresh.database_path()).is_err());
        assert!(!fresh.home.exists());
        drop(precreation);
        let admission = fresh.ensure_home_admission().unwrap();
        let legacy = std::fs::read(fresh.home.join("format.json")).unwrap();
        assert!(fresh.upgrade_staged_storage().is_err());
        assert_eq!(std::fs::read(fresh.home.join("format.json")).unwrap(), legacy);
        let store = crate::local_store::LocalStore::open_with_admission(&fresh.database_path(), admission.as_ref(), false).unwrap();
        store.recover_memory_queue().unwrap();
        store.load_state().unwrap();
        assert!(fresh.upgrade_staged_storage().is_err());
        drop(store);
        drop(admission);
        let app = crate::app::App::load(fresh.clone()).unwrap();
        assert!(fresh.database_path().exists());
        assert_eq!(std::fs::read(fresh.home.join("format.json")).unwrap(), legacy);
        drop(app);
        std::fs::remove_dir_all(&fresh.home).unwrap();
        config.ensure_home().unwrap();
        let target = Config { home: home.path().join("binding-target"), port: 0 };
        target.ensure_home().unwrap();
        let target_marker = std::fs::read(target.home.join("format.json")).unwrap();
        let ordinary_guard = config.ensure_home_admission().unwrap();
        assert!(crate::local_store::LocalStore::open_with_admission(&target.database_path(), ordinary_guard.as_ref(), false).is_err());
        assert!(crate::local_store::LocalStore::open_with_admission(&target.database_path(), None, false).is_err());
        assert_eq!(std::fs::read(target.home.join("format.json")).unwrap(), target_marker);
        assert!(!target.database_path().exists());
        drop(ordinary_guard);
        std::fs::remove_file(target.home.join("format.json")).unwrap();
        std::fs::remove_dir(&target.home).unwrap();
        let marker_path = home.path().join("format.json");
        let original = std::fs::read(&marker_path).unwrap();
        let registry_path = config.home.join("memberships.json");
        let registry = br#"{"registry_version":1,"active_membership_id":null,"memberships":[]}"#;
        write_private(&registry_path, registry).unwrap();
        assert!(config.upgrade_staged_storage().is_err());
        assert!(crate::app::App::load(config.clone()).is_err());
        assert!(crate::local_store::LocalStore::open(&config.database_path()).is_err());
        assert_eq!(std::fs::read(&marker_path).unwrap(), original);
        assert_eq!(std::fs::read(&registry_path).unwrap(), registry);
        assert!(!config.database_path().exists());
        std::fs::remove_file(&registry_path).unwrap();
        std::fs::write(home.path().join("account"), b"keep").unwrap();
        assert!(config.preload_staged_storage().is_err());
        assert_eq!(std::fs::read(&marker_path).unwrap(), original);
        assert!(!config.database_path().exists());
        std::fs::remove_file(home.path().join("account")).unwrap();
        let lock = StorageAdmission::acquire(home.path()).unwrap();
        assert!(config.preload_staged_storage().is_err());
        assert!(!config.database_path().exists());
        drop(lock);
        config.upgrade_staged_storage().unwrap();
        let staged_marker = std::fs::read(&marker_path).unwrap();
        let borrowed = config.staged_storage_admission().unwrap();
        write_private(&registry_path, registry).unwrap();
        assert!(borrowed.revalidate_staged().is_err());
        assert!(crate::local_store::LocalStore::open_with_admission(&config.database_path(), Some(&borrowed), true).is_err());
        assert_eq!(std::fs::read(&marker_path).unwrap(), staged_marker);
        assert_eq!(std::fs::read(&registry_path).unwrap(), registry);
        assert!(!config.database_path().exists());
        assert!(!config.home.join("beans.sqlite3-wal").exists());
        assert!(!config.home.join("beans.sqlite3-shm").exists());
        std::fs::remove_file(&registry_path).unwrap();
        for artifact in ["memberships", "membership-migration.json"] {
            let path = config.home.join(artifact);
            if artifact == "memberships" { std::fs::create_dir(&path).unwrap(); set_private(&path).unwrap(); }
            else { write_private(&path, b"{}").unwrap(); }
            assert!(crate::local_store::LocalStore::open_with_admission(&config.database_path(), Some(&borrowed), true).is_err());
            assert_eq!(std::fs::read(&marker_path).unwrap(), staged_marker);
            assert!(!config.database_path().exists());
            assert!(!config.home.join("beans.sqlite3-wal").exists());
            assert!(!config.home.join("beans.sqlite3-shm").exists());
            if artifact == "memberships" { std::fs::remove_dir(&path).unwrap(); } else { std::fs::remove_file(&path).unwrap(); }
        }
        drop(borrowed);
        write_private(&registry_path, registry).unwrap();
        assert!(config.staged_storage_admission().is_err());
        assert!(crate::local_store::LocalStore::open_staged(&config.database_path()).is_err());
        assert_eq!(std::fs::read(&registry_path).unwrap(), registry);
        assert_eq!(std::fs::read(&marker_path).unwrap(), staged_marker);
        assert!(!config.database_path().exists());
        std::fs::remove_file(&registry_path).unwrap();
        let store = crate::local_store::LocalStore::open_staged(&config.database_path()).unwrap();
        target.ensure_home().unwrap();
        let target_marker = std::fs::read(target.home.join("format.json")).unwrap();
        let target_store = crate::local_store::LocalStore::open(&target.database_path()).unwrap();
        drop(target_store);
        let target_database = std::fs::read(target.database_path()).unwrap();
        let staged_guard = config.staged_storage_admission().unwrap();
        assert!(crate::local_store::LocalStore::open_with_admission(&target.database_path(), Some(&staged_guard), true).is_err());
        assert!(crate::local_store::LocalStore::open_with_admission(&target.database_path(), None, false).is_err());
        assert_eq!(std::fs::read(target.home.join("format.json")).unwrap(), target_marker);
        assert_eq!(std::fs::read(target.database_path()).unwrap(), target_database);
        drop(staged_guard);
        std::fs::remove_dir_all(&target.home).unwrap();
        let connection = store.connection.into_inner().unwrap();
        let mode: u32 = connection.pragma_query_value(None, "synchronous", |row| row.get(0)).unwrap();
        assert_eq!(mode, 2);
        connection.execute_batch("CREATE TABLE scratch(value TEXT); INSERT INTO scratch VALUES ('keep');").unwrap();
        drop(connection);
        let database = std::fs::read(config.database_path()).unwrap();
        write_private(&registry_path, registry).unwrap();
        assert!(crate::local_store::LocalStore::open_staged(&config.database_path()).is_err());
        assert_eq!(std::fs::read(config.database_path()).unwrap(), database);
        assert_eq!(std::fs::read(&marker_path).unwrap(), staged_marker);
        assert_eq!(std::fs::read(&registry_path).unwrap(), registry);
        std::fs::remove_file(&registry_path).unwrap();
        assert!(crate::local_store::LocalStore::open(&config.database_path()).is_err());
        let admission = config.staged_storage_admission().unwrap();
        for fail_at in [1, 2] {
            let mut calls = 0;
            assert!(admission.revalidate_with_sync(|file| {
                calls += 1;
                if calls == fail_at { return Err(std::io::Error::other("uncertain durability")); }
                file.sync_all()
            }).is_err());
            assert_eq!(std::fs::read(config.database_path()).unwrap(), database);
        }
        let mut marker = admission.marker().unwrap();
        let bytes = StorageAdmission::read_marker(&mut marker).unwrap();
        let moved = home.path().with_extension("displaced");
        std::fs::rename(home.path(), &moved).unwrap();
        std::fs::create_dir(home.path()).unwrap();
        assert!(admission.verify(&marker, &bytes).is_err());
        std::fs::remove_dir(home.path()).unwrap();
        std::fs::rename(&moved, home.path()).unwrap();
        std::fs::rename(&marker_path, home.path().join("retained-marker")).unwrap();
        std::fs::write(&marker_path, &bytes).unwrap();
        assert!(admission.verify(&marker, &bytes).is_err());
        assert_eq!(std::fs::read(config.database_path()).unwrap(), database);
        std::fs::remove_file(&marker_path).unwrap();
        symlink(home.path().join("retained-marker"), &marker_path).unwrap();
        assert!(admission.revalidate_staged().is_err());
        drop(admission);
        assert!(crate::local_store::LocalStore::open_staged(&config.database_path()).is_err());
        assert_eq!(std::fs::read(config.database_path()).unwrap(), database);
        std::fs::remove_file(&marker_path).unwrap();
        std::fs::rename(home.path().join("retained-marker"), &marker_path).unwrap();
        let admission = config.staged_storage_admission().unwrap();
        let mut marker = admission.marker().unwrap();
        let bytes = StorageAdmission::read_marker(&mut marker).unwrap();
        std::fs::write(&marker_path, br#"{"format":"beans-v2","min_core_protocol":7}"#).unwrap();
        marker.sync_all().unwrap();
        admission.directory.sync_all().unwrap();
        assert!(admission.verify(&marker, &bytes).is_err());
        drop(admission);
        for invalid in [vec![b' '; 257], b"{".to_vec(), br#"{"format":"beans-v2"}"#.to_vec()] {
            std::fs::write(&marker_path, invalid).unwrap();
            assert!(crate::local_store::LocalStore::open_staged(&config.database_path()).is_err());
            assert_eq!(std::fs::read(config.database_path()).unwrap(), database);
        }
        std::fs::remove_file(&marker_path).unwrap();
        assert!(crate::local_store::LocalStore::open_staged(&config.database_path()).is_err());
        assert_eq!(std::fs::read(config.database_path()).unwrap(), database);
    }


    #[test]
    fn old_account_is_rejected_without_touching_bytes_or_permissions() {
        let home = tempfile::tempdir().unwrap();
        let old = home.path().join("state.json");
        std::fs::write(&old, b"existing-account").unwrap();
        let mode = std::fs::metadata(home.path()).unwrap().permissions();
        let config = Config { home: home.path().into(), port: 0 };
        assert!(config.ensure_home().is_err());
        assert_eq!(std::fs::read(old).unwrap(), b"existing-account");
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(home.path()).unwrap().permissions().mode(), mode.mode());
        }

        // A marked registry root must not reach account/store initialization.
        let root = tempfile::tempdir().unwrap();
        let root_config = Config { home: root.path().into(), port: 0 };
        root_config.ensure_home().unwrap();
        let registry = root.path().join("memberships.json");
        let valid = br#"{"registry_version":1,"active_membership_id":null,"memberships":[]}"#;
        for bytes in [
            valid.as_slice(),
            br#"{"registry_version":1,"registry_version":1,"active_membership_id":null,"memberships":[]}"#.as_slice(),
            br#"{"registry_version":2,"active_membership_id":null,"memberships":[]}"#.as_slice(),
            br#"{"registry_version":1,"active_membership_id":"../../escape","memberships":[]}"#.as_slice(),
            br#"{"registry_version":1,"active_membership_id":null,"memberships":[],"future":true}"#.as_slice(),
            b"{broken".as_slice(),
        ] {
            write_private(&registry, bytes).unwrap();
            let before = std::fs::read(root.path().join("format.json")).unwrap();
            assert!(root_config.validate_home().is_err());
            assert!(crate::app::App::load(root_config.clone()).is_err());
            assert!(crate::local_store::LocalStore::open(&root_config.database_path()).is_err());
            assert_eq!(std::fs::read(&registry).unwrap(), bytes);
            assert_eq!(std::fs::read(root.path().join("format.json")).unwrap(), before);
            for name in ["beans.sqlite3", "beans.sqlite3-wal", "beans.sqlite3-shm", "identity.json", "machine.json"] {
                assert!(!root.path().join(name).exists());
            }
        }
        std::fs::remove_file(&registry).unwrap();
        write_private(&root.path().join("membership-migration.json"), b"{}").unwrap();
        assert!(root_config.validate_home().is_err());
        assert!(!root_config.database_path().exists());
        std::fs::remove_file(root.path().join("membership-migration.json")).unwrap();
        assert!(root_config.validate_home().unwrap());
        assert!(!root_config.database_path().exists());
    }

    #[test]
    fn fresh_format_rejects_malformed_records_and_direct_old_database() {
        let home = tempfile::tempdir().unwrap();
        let config = Config { home: home.path().into(), port: 0 };
        config.ensure_home().unwrap();
        let database = config.database_path();
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection.execute_batch("CREATE TABLE old_account (id TEXT); INSERT INTO old_account VALUES ('keep');").unwrap();
        drop(connection);
        let bytes = std::fs::read(&database).unwrap();
        assert!(crate::local_store::LocalStore::open(&database).is_err());
        assert_eq!(std::fs::read(&database).unwrap(), bytes);
        assert!(!database.with_extension("sqlite3-wal").exists());
        std::fs::remove_file(&database).unwrap();
        std::fs::write(config.identity_path(), b"{invalid-account").unwrap();
        assert!(crate::app::App::load(config.clone()).is_err());
        assert_eq!(std::fs::read(config.identity_path()).unwrap(), b"{invalid-account");
        assert!(!database.exists());
    }

    #[test]
    fn partial_identity_is_rejected_before_mutation_but_machine_only_is_valid() {
        let home = tempfile::tempdir().unwrap();
        let config = Config { home: home.path().into(), port: 0 };
        config.ensure_home().unwrap();
        let identity = crate::keys::IdentityFile::new(&crate::keys::Identity::from_master([17; 32]));
        write_json_private(&config.identity_path(), &identity).unwrap();
        let bytes = std::fs::read(config.identity_path()).unwrap();
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let permissions = std::fs::metadata(home.path()).unwrap().permissions();
        let error = crate::app::App::load(config.clone()).err().expect("partial identity must fail admission");
        assert!(error.to_string().contains("missing its machine record"));
        assert_eq!(std::fs::read(config.identity_path()).unwrap(), bytes);
        assert!(!config.database_path().exists());
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(home.path()).unwrap().permissions().mode(), permissions.mode());
        }
        std::fs::remove_file(config.identity_path()).unwrap();
        let app = crate::app::App::load(config.clone()).unwrap();
        crate::identity::create(&app, None).unwrap();
        let machine = std::fs::read(config.machine_path()).unwrap();
        std::fs::remove_file(config.identity_path()).unwrap();
        let paired = crate::app::App::load(config.clone()).unwrap();
        assert!(paired.has_identity());
        assert!(!paired.is_identity_device());
        assert_eq!(std::fs::read(config.machine_path()).unwrap(), machine);
    }

    #[test]
    fn plugin_corruption_is_rejected_before_permissions_or_database_writes() {
        for name in ["installed.json", "secrets.json"] {
            let cases: &[&[u8]] = if name == "installed.json" {
                &[b"{broken", b"[]", b"{}", b"{\"future\":[]}", b"{\"plugins\":null}", b"{\"plugins\":[],\"future\":{\"KEEP\":\"retained\"}}"]
            } else {
                &[b"{broken", b"[]", b"{\"plugin\":null}"]
            };
            for &bytes in cases {
                let home = tempfile::tempdir().unwrap();
                let config = Config { home: home.path().into(), port: 0 };
                config.ensure_home().unwrap();
                std::fs::create_dir(config.plugins_dir()).unwrap();
                let path = config.plugins_dir().join(name);
                std::fs::write(&path, bytes).unwrap();
                #[cfg(unix)] {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
                }
                let permissions = std::fs::metadata(home.path()).unwrap().permissions();
                assert!(crate::plugins::Store::load(&config).is_err());
                assert!(crate::app::App::load(config.clone()).is_err());
                assert_eq!(std::fs::read(&path).unwrap(), bytes);
                assert!(!config.database_path().exists());
                #[cfg(unix)] {
                    use std::os::unix::fs::PermissionsExt;
                    assert_eq!(std::fs::metadata(home.path()).unwrap().permissions().mode(), permissions.mode());
                }
            }
            let home = tempfile::tempdir().unwrap();
            let config = Config { home: home.path().into(), port: 0 };
            config.ensure_home().unwrap();
            std::fs::create_dir_all(config.plugins_dir().join(name)).unwrap();
            assert!(config.ensure_home().is_err(), "unreadable record must not default");
            assert!(config.plugins_dir().join(name).is_dir());
        }
    }
}
