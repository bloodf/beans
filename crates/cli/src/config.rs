//! Where the CLI keeps things and how it is configured.

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
        let marked = self.validate_home()?;
        std::fs::create_dir_all(&self.home)?;
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
