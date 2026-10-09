//! Read-only recognition of membership roots. No membership or migration is executable here.
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::Path;

use anyhow::{bail, ensure, Context};
use serde::{Deserialize, Deserializer};

const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    registry_version: u64,
    #[serde(deserialize_with = "required_option")]
    active_membership_id: Option<String>,
    #[serde(deserialize_with = "entries")]
    memberships: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    membership_id: String,
    endpoint: String,
    #[serde(deserialize_with = "required_option")]
    identity_pubkey: Option<String>,
    state: State,
    incarnation: u64,
    display_name: String,
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum State { Joining, Ready, Removing }

fn required_option<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)
}

fn entries<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Entry>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = Vec<Entry>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("at most 256 membership entries")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut result = Vec::new();
            while let Some(entry) = seq.next_element()? {
                if result.len() == 256 {
                    return Err(serde::de::Error::custom("too many memberships"));
                }
                result.push(entry);
            }
            Ok(result)
        }
    }
    d.deserialize_seq(Visitor)
}

fn metadata(path: &Path, directory: bool) -> anyhow::Result<Option<std::fs::Metadata>> {
    let m = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(!m.file_type().is_symlink() && if directory { m.is_dir() } else { m.is_file() },
        "Membership storage has an invalid file type; data is untouched");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(m.uid() == unsafe { libc::geteuid() } && m.mode() & 0o077 == 0,
            "Membership storage must be privately owned; data is untouched");
    }
    Ok(Some(m))
}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let m = metadata(path, false)?.context("Membership record is missing")?;
    ensure!(m.len() <= MAX_BYTES, "Membership record exceeds its size bound");
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file: File = options.open(path)?;
    let opened = file.metadata()?;
    ensure!(opened.is_file() && opened.len() <= MAX_BYTES, "Membership record is invalid");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(m.dev() == opened.dev() && m.ino() == opened.ino(), "Membership record changed during admission");
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_BYTES, "Membership record exceeds its size bound");
    Ok(serde_json::from_slice(&bytes)?)
}

pub(crate) fn preflight(home: &Path) -> anyhow::Result<()> {
    let registry_path = home.join("memberships.json");
    // Presence checks never interpret malformed metadata or a symlink as absence.
    let registry = match std::fs::symlink_metadata(&registry_path) {
        Ok(_) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.into()),
    };
    for name in ["membership-migration.json", "memberships"] {
        match std::fs::symlink_metadata(home.join(name)) {
            Ok(_) if !registry => bail!("Membership layout requires recovery; migration is disabled"),
            Ok(_) => {},
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(e.into()),
        }
    }
    if !registry { return Ok(()); }
    metadata(home, true)?.context("Membership root is missing")?;
    let registry: Registry = read(&registry_path)?;
    ensure!(registry.registry_version == 1, "Unsupported membership registry version");
    let mut ids = HashSet::new();
    let mut keys = HashSet::new();
    let mut active_found = registry.active_membership_id.is_none();
    let member_root = home.join("memberships");
    metadata(&member_root, true)?;
    for entry in registry.memberships {
        ensure!(entry.membership_id.len() == 32 && entry.membership_id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "Invalid membership ID");
        ensure!(ids.insert(entry.membership_id.clone()), "Duplicate membership ID");
        ensure!(entry.incarnation > 0 && entry.display_name.len() <= 256 && entry.endpoint.len() <= 4096, "Invalid membership fields");
        let url = reqwest::Url::parse(&entry.endpoint)?;
        ensure!(matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
            && url.username().is_empty() && url.password().is_none() && url.query().is_none() && url.fragment().is_none(), "Invalid membership endpoint");
        if let Some(key) = &entry.identity_pubkey {
            crate::keys::unb64_32(key)?;
            ensure!(keys.insert(key.clone()), "Duplicate membership identity");
        }
        ensure!(entry.state != State::Ready || entry.identity_pubkey.is_some(), "Ready membership requires an identity");
        if registry.active_membership_id.as_ref() == Some(&entry.membership_id) {
            ensure!(entry.state == State::Ready, "Active membership is not ready");
            active_found = true;
        }
        let member = member_root.join(&entry.membership_id);
        let exists = metadata(&member, true)?.is_some();
        ensure!(exists || entry.state != State::Ready, "Ready membership namespace is missing");
        if exists {
            for child in std::fs::read_dir(&member)? {
                ensure!(!child?.file_type()?.is_symlink(), "Membership namespace contains a symlink");
            }
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Marker { format: crate::config::Format }
            let marker: Marker = read(&member.join("format.json"))?;
            ensure!(marker.format == crate::config::Format::BeansV2, "Invalid membership format");
            if entry.state == State::Ready {
                let machine: crate::keys::MachineFile = read(&member.join("machine.json"))?;
                machine.machine()?;
                machine.dek()?;
                crate::keys::unb64_32(&machine.content_pubkey)?;
                ensure!(Some(&machine.identity_pubkey) == entry.identity_pubkey.as_ref(), "Membership machine identity mismatch");
                ensure!(machine.relay_url.as_deref() == Some(entry.endpoint.as_str()), "Membership machine endpoint mismatch");
                let identity_path = member.join("identity.json");
                if metadata(&identity_path, false)?.is_some() {
                    let identity: crate::keys::IdentityFile = read(&identity_path)?;
                    ensure!(identity.identity()?.pubkey() == machine.identity_pubkey, "Membership identity binding mismatch");
                }
            }
        }
    }
    ensure!(active_found, "Active membership is missing");
    bail!("Membership root cannot be opened as an account; migration is disabled")
}
