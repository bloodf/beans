//! Explicit, bounded, inactive managed copies. Linux only; no path-based fallback.
//! Source labels are provenance, not capabilities. Caller authorizes the selected root.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

pub const MAX_ENTRIES: usize = 1024;
pub const MAX_FILES: usize = 256;
pub const MAX_DEPTH: usize = 4;
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LibraryFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Exact bounded source inventory. Digests describe bytes, not authenticated provenance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourcePreview {
    pub metadata: BTreeMap<String, String>,
    pub license: Option<String>,
    pub findings: Vec<String>,
    pub directories: Vec<String>,
    pub files: Vec<LibraryFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LibraryManifest {
    pub version: u32,
    pub bundle_id: String,
    /// Caller supplied host label; not authenticated by this local primitive.
    pub source_host: String,
    pub source_path: PathBuf,
    pub imported_unix_seconds: u64,
    pub metadata: BTreeMap<String, String>,
    /// Declared source value only; absent means unknown, not permission to redistribute.
    pub license: Option<String>,
    pub findings: Vec<String>,
    pub directories: Vec<String>,
    pub files: Vec<LibraryFile>,
}

#[derive(Debug)]
pub struct ImportOutcome {
    pub manifest: LibraryManifest,
    /// Rename committed successfully; a root fsync failure cannot undo publication.
    pub durability_warning: Option<String>,
}

#[derive(Debug)]
pub struct UninstallOutcome {
    /// The exact bundle is no longer visible. Hidden tomb cleanup may need operator repair.
    pub cleanup_warning: Option<String>,
    pub durability_warning: Option<String>,
}

/// Opens an existing private Runner-owned directory, never creates or claims a source root.
pub struct SkillLibrary {
    #[cfg(target_os = "linux")]
    root: std::fs::File,
}

impl SkillLibrary {
    pub fn open(owned_root: &Path) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let root = linux::open_root(owned_root)?;
            linux::private(&root)?;
            Ok(Self { root })
        }
        #[cfg(not(target_os = "linux"))]
        { let _ = owned_root; Err(unsupported()) }
    }

    /// Caller explicitly authorizes reading this selected directory and all bounded resources.
    /// No scanner output is used as authority. No body, hooks or resources are executed.
    pub fn import_authorized(&self, selected_source: &Path, source_host: &str) -> io::Result<ImportOutcome> {
        #[cfg(target_os = "linux")]
        { linux::import(&self.root, selected_source, source_host, None, || Ok(())) }
        #[cfg(not(target_os = "linux"))]
        { let _ = (selected_source, source_host); Err(unsupported()) }
    }

    pub fn uninstall(&self, managed_id: &str) -> io::Result<UninstallOutcome> {
        #[cfg(target_os = "linux")]
        { linux::uninstall(&self.root, managed_id, || Ok(())) }
        #[cfg(not(target_os = "linux"))]
        { let _ = managed_id; Err(unsupported()) }
    }

    /// Reads bounded resources without creating staging or publishing a managed copy.
    pub fn preview_source(&self, selected_source: &Path) -> io::Result<SourcePreview> {
        #[cfg(target_os = "linux")]
        { linux::preview(&self.root, selected_source) }
        #[cfg(not(target_os = "linux"))]
        { let _ = selected_source; Err(unsupported()) }
    }

    /// Validates an existing managed copy and recomputes metadata from its current bytes.
    /// Returned relative inventory labels are not resource-open capabilities.
    pub fn validated_inventory(&self, managed_id: &str) -> io::Result<SourcePreview> {
        #[cfg(target_os = "linux")]
        { linux::validated_inventory(&self.root, managed_id) }
        #[cfg(not(target_os = "linux"))]
        { let _ = managed_id; Err(unsupported()) }
    }

    /// Compares actual staged bytes with approval before acquiring publication authority.
    /// The returned guard remains owned here through publication and durability reporting.
    pub fn import_approved<G>(&self, selected_source: &Path, source_host: &str, expected: &SourcePreview,
        admit_publication: impl FnOnce() -> io::Result<G>) -> io::Result<ImportOutcome> {
        #[cfg(target_os = "linux")]
        { linux::import(&self.root, selected_source, source_host, Some(expected), admit_publication) }
        #[cfg(not(target_os = "linux"))]
        { let _ = (selected_source, source_host, expected, admit_publication); Err(unsupported()) }
    }

    /// The returned guard remains owned here through validated tomb publication and cleanup.
    pub fn uninstall_guarded<G>(&self, managed_id: &str,
        admit_removal: impl FnOnce() -> io::Result<G>) -> io::Result<UninstallOutcome> {
        #[cfg(target_os = "linux")]
        { linux::uninstall(&self.root, managed_id, admit_removal) }
        #[cfg(not(target_os = "linux"))]
        { let _ = (managed_id, admit_removal); Err(unsupported()) }
    }
}

#[cfg(not(target_os = "linux"))]
fn unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "managed skill library requires Linux openat2; no fallback")
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::collections::BTreeSet;
    use std::ffi::{CStr, CString};
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::Component;
    use sha2::{Digest, Sha256};

    fn invalid(message: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }
    fn name(path: &Path) -> io::Result<CString> {
        CString::new(path.as_os_str().as_bytes()).map_err(|_| invalid("NUL in path"))
    }
    #[repr(C)]
    struct OpenHow { flags: u64, mode: u64, resolve: u64 }

    // Same kernel resolution boundary as skill_discovery; its helpers remain private.
    fn open_at(parent: &File, path: &Path, flags: i32, mode: u64, no_mounts: bool) -> io::Result<File> {
        let path = name(path)?;
        let how = OpenHow {
            flags: (flags | libc::O_CLOEXEC | libc::O_NOFOLLOW | if flags & libc::O_PATH == 0 { libc::O_NONBLOCK } else { 0 }) as u64,
            mode,
            resolve: 0x08 | 0x04 | 0x02 | if no_mounts { 0x01 } else { 0 },
        };
        // SAFETY: valid descriptor, terminated path and Linux open_how ABI.
        let fd = unsafe { libc::syscall(libc::SYS_openat2, parent.as_raw_fd(), path.as_ptr(), &how, std::mem::size_of::<OpenHow>()) };
        if fd < 0 { return Err(io::Error::last_os_error()); }
        // SAFETY: fresh descriptor ownership transfers to File.
        Ok(unsafe { File::from_raw_fd(fd as i32) })
    }
    fn resource(parent: &File, path: &Path) -> io::Result<File> {
        let object = open_at(parent, path, libc::O_PATH, 0, true)?;
        let m = object.metadata()?;
        if !m.is_file() && !m.is_dir() { return Err(invalid("special resource rejected")); }
        let opened = open_at(parent, path, libc::O_RDONLY | if m.is_dir() { libc::O_DIRECTORY } else { 0 }, 0, true)?;
        if identity(&object)? != identity(&opened)? { return Err(invalid("resource replaced during open")); }
        Ok(opened)
    }
    pub(super) fn open_root(path: &Path) -> io::Result<File> {
        if !path.is_absolute() || path.components().any(|c| !matches!(c, Component::RootDir | Component::Normal(_))) {
            return Err(invalid("root must be absolute without traversal"));
        }
        let anchor = File::open("/")?;
        let relative = path.strip_prefix("/").map_err(|_| invalid("invalid absolute root"))?;
        open_at(&anchor, if relative.as_os_str().is_empty() { Path::new(".") } else { relative }, libc::O_RDONLY | libc::O_DIRECTORY, 0, false)
    }
    pub(super) fn private(file: &File) -> io::Result<()> {
        let m = file.metadata()?;
        // SAFETY: geteuid has no memory preconditions.
        if !m.is_dir() || m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o777 != 0o700 {
            return Err(invalid("library directory must be owned by effective user with mode 0700"));
        }
        Ok(())
    }
    fn identity(file: &File) -> io::Result<(u64, u64)> {
        let m = file.metadata()?; Ok((m.dev(), m.ino()))
    }
    // Walk descriptors, not caller labels, so aliases and ancestor replacement cannot
    // authorize overlapping source/library trees. This is not same-user confinement.
    fn has_ancestor(child: &File, ancestor: &File) -> io::Result<bool> {
        let target = identity(ancestor)?;
        let mut current = child.try_clone()?;
        for _ in 0..4096 {
            let here = identity(&current)?;
            if here == target { return Ok(true); }
            let parent = name(Path::new(".."))?;
            // SAFETY: pinned directory; '..' is used only to compare ancestors, never read resources.
            let fd = unsafe { libc::openat(current.as_raw_fd(), parent.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
            if fd < 0 { return Err(io::Error::last_os_error()); }
            // SAFETY: newly owned descriptor.
            let next = unsafe { File::from_raw_fd(fd) };
            if identity(&next)? == here { return Ok(false); }
            current = next;
        }
        Err(invalid("ancestor comparison limit reached"))
    }
    struct Lock(File);
    impl Drop for Lock {
        fn drop(&mut self) {
            // SAFETY: descriptor remains live through unlock.
            unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN); }
        }
    }
    fn lock(root: &File) -> io::Result<Lock> {
        // Independent open-file description: concurrent operations on one instance also serialize.
        let file = open_at(root, Path::new("."), libc::O_RDONLY | libc::O_DIRECTORY, 0, true)?;
        // SAFETY: live owned directory descriptor.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 { return Err(io::Error::last_os_error()); }
        Ok(Lock(file))
    }
    fn mkdir(parent: &File, label: &str) -> io::Result<File> {
        let c = name(Path::new(label))?;
        // SAFETY: pinned parent and single generated/validated component.
        if unsafe { libc::mkdirat(parent.as_raw_fd(), c.as_ptr(), 0o700) } != 0 { return Err(io::Error::last_os_error()); }
        open_at(parent, Path::new(label), libc::O_RDONLY | libc::O_DIRECTORY, 0, true)
    }
    fn write_new(parent: &File, label: &str, bytes: &[u8]) -> io::Result<()> {
        let mut file = open_at(parent, Path::new(label), libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL, 0o600, true)?;
        file.write_all(bytes)?;
        file.sync_all()
    }
    fn rename(parent: &File, from: &str, to: &str) -> io::Result<()> {
        let from = name(Path::new(from))?; let to = name(Path::new(to))?;
        // SAFETY: live shared parent, valid names; no replacement even on ID collision.
        if unsafe { libc::syscall(libc::SYS_renameat2, parent.as_raw_fd(), from.as_ptr(), parent.as_raw_fd(), to.as_ptr(), libc::RENAME_NOREPLACE) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    struct Directory(*mut libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) { unsafe { libc::closedir(self.0); } }
    }
    fn entries(dir: &File, count: &mut usize) -> io::Result<Vec<String>> {
        let fd = open_at(dir, Path::new("."), libc::O_RDONLY | libc::O_DIRECTORY, 0, true)?.into_raw_fd();
        // SAFETY: fdopendir assumes ownership on success only.
        let pointer = unsafe { libc::fdopendir(fd) };
        if pointer.is_null() {
            let error = io::Error::last_os_error(); unsafe { libc::close(fd); } return Err(error);
        }
        let directory = Directory(pointer);
        let mut labels = Vec::new();
        loop {
            // SAFETY: Linux thread-local errno; readdir result lives until next call.
            unsafe { *libc::__errno_location() = 0; }
            let entry = unsafe { libc::readdir(directory.0) };
            if entry.is_null() {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(0) { return Err(error); }
                break;
            }
            let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if bytes == b"." || bytes == b".." { continue; }
            *count += 1;
            if *count > MAX_ENTRIES + 2 { return Err(invalid("entry limit exceeded")); }
            let label = std::str::from_utf8(bytes).map_err(|_| invalid("non-UTF-8 resource name"))?;
            labels.push(label.to_string());
        }
        labels.sort();
        Ok(labels)
    }
    fn read_file(mut file: File, limit: u64) -> io::Result<Vec<u8>> {
        let m = file.metadata()?;
        if !m.is_file() || m.len() > limit { return Err(invalid("nonregular or oversized file")); }
        let mut bytes = Vec::with_capacity(m.len() as usize);
        (&mut file).take(limit + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit { return Err(invalid("file grew beyond byte limit")); }
        Ok(bytes)
    }
    fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
    #[derive(Default)]
    struct Inventory { entries: usize, bytes: u64, directories: Vec<String>, files: Vec<LibraryFile> }

    fn walk(source_root: &File, directory: &File, destination: Option<&File>, library: &File, relative: &str, depth: usize, inventory: &mut Inventory, metadata: &mut Option<(BTreeMap<String, String>, Vec<String>)>) -> io::Result<()> {
        // Child descriptors identify expected objects, never authorize detached subtrees.
        let current = open_at(source_root, Path::new(if relative.is_empty() { "." } else { relative }), libc::O_RDONLY | libc::O_DIRECTORY, 0, true)?;
        if identity(&current)? != identity(directory)? { return Err(invalid("source directory replaced or detached")); }
        if identity(&current)? == identity(library)? { return Err(invalid("resource tree overlaps library root")); }
        let labels = entries(&current, &mut inventory.entries)?;
        if inventory.entries > MAX_ENTRIES { return Err(invalid("entry limit exceeded")); }
        for label in labels {
            let path = if relative.is_empty() { label.clone() } else { format!("{relative}/{label}") };
            let file = resource(source_root, Path::new(&path))?;
            let m = file.metadata()?;
            if m.is_dir() {
                if depth == MAX_DEPTH { return Err(invalid("directory depth limit exceeded")); }
                inventory.directories.push(path.clone());
                let output = destination.map(|d| mkdir(d, &label)).transpose()?;
                walk(source_root, &file, output.as_ref(), library, &path, depth + 1, inventory, metadata)?;
            } else {
                if inventory.files.len() == MAX_FILES { return Err(invalid("file count limit exceeded")); }
                let bytes = read_file(file, MAX_FILE_BYTES)?;
                inventory.bytes += bytes.len() as u64;
                if inventory.bytes > MAX_TOTAL_BYTES { return Err(invalid("aggregate byte limit exceeded")); }
                if path == "SKILL.md" { *metadata = Some(skill_metadata(&bytes)); }
                inventory.files.push(LibraryFile { path, bytes: bytes.len() as u64, sha256: digest(&bytes) });
                if let Some(d) = destination { write_new(d, &label, &bytes)?; }
            }
        }
        if let Some(d) = destination { d.sync_all()?; }
        Ok(())
    }
    fn skill_metadata(bytes: &[u8]) -> (BTreeMap<String, String>, Vec<String>) {
        let mut findings = vec!["copied body and resources are inactive; no host policy or execution".into()];
        let Ok(text) = std::str::from_utf8(bytes) else { return (BTreeMap::new(), vec!["SKILL.md is not UTF-8; metadata unavailable".into()]); };
        let mut lines = text.split_inclusive('\n');
        if !matches!(lines.next(), Some("---\n" | "---\r\n")) {
            return (BTreeMap::new(), vec!["missing standalone frontmatter; metadata unavailable".into()]);
        }
        let mut frontmatter = String::from("---\n");
        let mut closed = false;
        for line in lines {
            frontmatter.push_str(line);
            if line.trim_end_matches(['\r', '\n']) == "---" { closed = true; break; }
        }
        if !closed { return (BTreeMap::new(), vec!["unterminated frontmatter; metadata unavailable".into()]); }
        let parsed = super::super::frontmatter::parse_frontmatter(&frontmatter);
        for key in parsed.fields.keys() {
            if !matches!(key.as_str(), "name" | "description" | "license" | "compatibility") {
                findings.push(format!("unsupported field {key:?}; retained as metadata only"));
            }
        }
        if frontmatter.lines().any(|l| l.starts_with([' ', '\t']) || l.split_once(':').is_some_and(|(_, v)| v.trim().starts_with(['[', '{', '&', '*', '!', '|', '>']))) {
            findings.push("structured YAML unsupported; flat metadata is not lossless".into());
        }
        for key in ["name", "description"] {
            if parsed.get(key).is_none() { findings.push(format!("missing {key}; no inferred fallback")); }
        }
        (parsed.fields, findings)
    }
    fn valid_id(id: &str) -> bool { id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) }
    fn valid_relative(path: &str) -> bool {
        !path.is_empty() && path.split('/').all(|s| !s.is_empty() && s != "." && s != ".." && !s.contains('\0'))
            && Path::new(path).components().all(|c| matches!(c, Component::Normal(_)))
    }
    // Used only for private staging/tombs whose contents were created or validated here.
    fn remove_tree(dir: &File, count: &mut usize, depth: usize) -> io::Result<()> {
        if depth > MAX_DEPTH + 2 { return Err(invalid("cleanup depth exceeded")); }
        for label in entries(dir, count)? {
            let file = resource(dir, Path::new(&label))?;
            let is_dir = file.metadata()?.is_dir();
            if is_dir { remove_tree(&file, count, depth + 1)?; }
            else if !file.metadata()?.is_file() { return Err(invalid("unexpected cleanup object")); }
            unlink(dir, &label, is_dir)?;
        }
        Ok(())
    }
    fn unlink(parent: &File, label: &str, directory: bool) -> io::Result<()> {
        let label = name(Path::new(label))?;
        // SAFETY: anchored parent and validated single component; never follows a link.
        if unsafe { libc::unlinkat(parent.as_raw_fd(), label.as_ptr(), if directory { libc::AT_REMOVEDIR } else { 0 }) } != 0 { return Err(io::Error::last_os_error()); }
        Ok(())
    }
    struct Stage<'a> { root: &'a File, name: String, directory: File, committed: bool }
    impl Drop for Stage<'_> {
        fn drop(&mut self) {
            if !self.committed {
                let _ = remove_tree(&self.directory, &mut 0, 0);
                let _ = unlink(self.root, &self.name, true);
            }
        }
    }
    fn source_preview(inventory: Inventory, metadata: Option<(BTreeMap<String, String>, Vec<String>)>) -> io::Result<SourcePreview> {
        let (metadata, findings) = metadata.ok_or_else(|| invalid("selected root requires regular SKILL.md"))?;
        let preview = SourcePreview {
            license: metadata.get("license").filter(|s| !s.trim().is_empty()).cloned(), metadata, findings,
            directories: inventory.directories, files: inventory.files,
        };
        if serde_json::to_vec(&preview).map_err(io::Error::other)?.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(invalid("preview byte limit exceeded"));
        }
        Ok(preview)
    }

    pub(super) fn preview(root: &File, selected: &Path) -> io::Result<SourcePreview> {
        let _lock = lock(root)?;
        private(root)?;
        if selected.as_os_str().len() > 4096 { return Err(invalid("source provenance exceeds bounds")); }
        let source = open_root(selected)?;
        if has_ancestor(&source, root)? || has_ancestor(root, &source)? { return Err(invalid("source and library roots must be disjoint")); }
        let mut inventory = Inventory::default(); let mut metadata = None;
        walk(&source, &source, None, root, "", 0, &mut inventory, &mut metadata)?;
        source_preview(inventory, metadata)
    }

    pub(super) fn import<G>(root: &File, selected: &Path, host: &str, expected: Option<&SourcePreview>,
        admit_publication: impl FnOnce() -> io::Result<G>) -> io::Result<ImportOutcome> {
        let _lock = lock(root)?;
        private(root)?;
        if host.is_empty() || host.len() > 1024 || selected.as_os_str().len() > 4096 { return Err(invalid("source provenance exceeds bounds or host is empty")); }
        let source = open_root(selected)?;
        if has_ancestor(&source, root)? || has_ancestor(root, &source)? { return Err(invalid("source and library roots must be disjoint")); }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let stage_name = format!(".staging-{}", uuid::Uuid::new_v4().simple());
        let mut stage = Stage { root, directory: mkdir(root, &stage_name)?, name: stage_name, committed: false };
        let content = mkdir(&stage.directory, "content")?;
        let mut inventory = Inventory::default(); let mut metadata = None;
        walk(&source, &source, Some(&content), root, "", 0, &mut inventory, &mut metadata)?;
        let actual = source_preview(inventory, metadata)?;
        if expected.is_some_and(|expected| expected != &actual) { return Err(invalid("selected source changed since approval")); }
        let manifest = LibraryManifest {
            version: 1, bundle_id: id.clone(), source_host: host.into(), source_path: selected.into(),
            imported_unix_seconds: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|_| invalid("clock predates Unix epoch"))?.as_secs(),
            license: actual.license, metadata: actual.metadata, findings: actual.findings,
            directories: actual.directories, files: actual.files,
        };
        let bytes = serde_json::to_vec(&manifest).map_err(io::Error::other)?;
        if bytes.len() as u64 > MAX_MANIFEST_BYTES { return Err(invalid("manifest byte limit exceeded")); }
        write_new(&stage.directory, "manifest.json", &bytes)?;
        stage.directory.sync_all()?;
        let _authority = admit_publication()?;
        rename(root, &stage.name, &id)?;
        stage.committed = true;
        Ok(ImportOutcome { manifest, durability_warning: root.sync_all().err().map(|e| e.to_string()) })
    }
    fn validate(manifest: &LibraryManifest, id: &str) -> io::Result<()> {
        if manifest.version != 1 || manifest.bundle_id != id || manifest.files.is_empty() || manifest.files.len() > MAX_FILES || manifest.files.len() + manifest.directories.len() > MAX_ENTRIES {
            return Err(invalid("invalid managed manifest identity or bounds"));
        }
        let mut paths = BTreeSet::new(); let mut total = 0u64;
        for d in &manifest.directories {
            if !valid_relative(d) || d.split('/').count() > MAX_DEPTH || !paths.insert(d.as_str()) { return Err(invalid("invalid manifest directory")); }
        }
        for f in &manifest.files {
            if !valid_relative(&f.path) || f.path.split('/').count() > MAX_DEPTH + 1 || !paths.insert(f.path.as_str()) || f.bytes > MAX_FILE_BYTES || f.sha256.len() != 64 || !f.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) { return Err(invalid("invalid manifest file")); }
            total = total.checked_add(f.bytes).ok_or_else(|| invalid("manifest byte overflow"))?;
        }
        if total > MAX_TOTAL_BYTES || !manifest.files.iter().any(|f| f.path == "SKILL.md") { return Err(invalid("invalid manifest content bounds")); }
        Ok(())
    }
    // Caller retains the root lock through validation and any subsequent publication.
    fn validated_copy(root: &File, id: &str) -> io::Result<(File, Inventory, Option<(BTreeMap<String, String>, Vec<String>)>)> {
        let bundle = open_at(root, Path::new(id), libc::O_RDONLY | libc::O_DIRECTORY, 0, true)?;
        private(&bundle)?;
        let labels = entries(&bundle, &mut 0)?;
        if labels != ["content", "manifest.json"] { return Err(invalid("unrecognized bundle contents")); }
        let bytes = read_file(open_at(&bundle, Path::new("manifest.json"), libc::O_RDONLY, 0, true)?, MAX_MANIFEST_BYTES)?;
        let manifest: LibraryManifest = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        validate(&manifest, id)?;
        let content = open_at(&bundle, Path::new("content"), libc::O_RDONLY | libc::O_DIRECTORY, 0, true)?;
        let mut actual = Inventory::default();
        let mut metadata = None;
        walk(&content, &content, None, root, "", 0, &mut actual, &mut metadata)?;
        if actual.files != manifest.files || actual.directories != manifest.directories { return Err(invalid("managed copy differs from manifest; nothing removed")); }
        Ok((bundle, actual, metadata))
    }
    pub(super) fn validated_inventory(root: &File, id: &str) -> io::Result<SourcePreview> {
        if !valid_id(id) { return Err(invalid("not an exact managed bundle ID")); }
        let _lock = lock(root)?;
        private(root)?;
        let (_, inventory, metadata) = validated_copy(root, id)?;
        source_preview(inventory, metadata)
    }
    pub(super) fn uninstall<G>(root: &File, id: &str, admit_removal: impl FnOnce() -> io::Result<G>) -> io::Result<UninstallOutcome> {
        if !valid_id(id) { return Err(invalid("not an exact managed bundle ID")); }
        let _lock = lock(root)?;
        private(root)?;
        let (bundle, _, _) = validated_copy(root, id)?;
        let tomb = format!(".removed-{}", uuid::Uuid::new_v4().simple());
        let _authority = admit_removal()?;
        rename(root, id, &tomb)?;
        let durability_warning = root.sync_all().err().map(|e| e.to_string());
        let cleanup = remove_tree(&bundle, &mut 0, 0).and_then(|()| unlink(root, &tomb, true)).and_then(|()| root.sync_all());
        Ok(UninstallOutcome { cleanup_warning: cleanup.err().map(|e| e.to_string()), durability_warning })
    }

    #[cfg(test)]
    #[test]
    fn managed_library_boundary_fixture() {
        use std::fs;
        use std::os::unix::fs::{symlink, PermissionsExt};
        let base = std::env::temp_dir().join(format!("beans-library-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&base).unwrap();
        let base = base.canonicalize().unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
        let _cleanup = Cleanup(base.clone());
        let source = base.join("selected"); let owned = base.join("library"); let outside = base.join("outside");
        for p in [&source, &owned, &outside] { fs::create_dir(p).unwrap(); fs::set_permissions(p, fs::Permissions::from_mode(0o700)).unwrap(); }
        let skill = b"---\nname: fixture\ndescription: bounded\nhooks: [never]\n---\nnever execute\n";
        fs::write(source.join("SKILL.md"), skill).unwrap();
        fs::create_dir(source.join("resources")).unwrap(); fs::write(source.join("resources/data"), b"abc").unwrap();
        fs::write(outside.join("secret"), b"outside sentinel").unwrap();
        fs::write(owned.join("unrelated"), b"keep").unwrap();
        let library = SkillLibrary::open(&owned).unwrap();
        let approved = library.preview_source(&source).unwrap();
        assert_eq!(fs::read_dir(&owned).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>(), vec![std::ffi::OsString::from("unrelated")]);
        fs::write(source.join("resources/data"), b"changed after preview").unwrap();
        let mut admitted = false;
        assert!(library.import_approved(&source, "fixture", &approved, || { admitted = true; Ok(()) }).is_err());
        assert!(!admitted, "changed staged bytes must reject before publication admission");
        fs::write(source.join("resources/data"), b"abc").unwrap();
        assert!(library.import_approved::<()>(&source, "fixture", &approved, || Err(invalid("authority revoked"))).is_err());
        assert_eq!(fs::read_dir(&owned).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>(), vec![std::ffi::OsString::from("unrelated")]);
        struct Guard { root: PathBuf, removed_id: Option<String> }
        impl Drop for Guard {
            fn drop(&mut self) {
                if let Some(id) = &self.removed_id {
                    assert!(!self.root.join(id).exists(), "removal guard dropped before tomb rename");
                } else {
                    assert!(fs::read_dir(&self.root).unwrap().any(|e| valid_id(&e.unwrap().file_name().to_string_lossy())), "import guard dropped before visible rename");
                }
            }
        }
        let guarded = library.import_approved(&source, "fixture", &approved, || Ok(Guard { root: owned.clone(), removed_id: None })).unwrap();
        let guarded_id = guarded.manifest.bundle_id;
        assert_eq!(fs::read(owned.join(&guarded_id).join("content/resources/data")).unwrap(), b"abc");
        assert!(library.uninstall_guarded::<()>(&guarded_id, || Err(invalid("authority revoked"))).is_err());
        assert_eq!(fs::read(owned.join(&guarded_id).join("content/resources/data")).unwrap(), b"abc");
        library.uninstall_guarded(&guarded_id, || Ok(Guard { root: owned.clone(), removed_id: Some(guarded_id.clone()) })).unwrap();
        assert!(!owned.join(&guarded_id).exists());
        assert_eq!(fs::read(source.join("resources/data")).unwrap(), b"abc");
        let result = library.import_authorized(&source, "synthetic-host").unwrap();
        let manifest = result.manifest;
        assert!(valid_id(&manifest.bundle_id)); assert_eq!(manifest.version, 1);
        assert_eq!(manifest.source_path, source); assert_eq!(manifest.source_host, "synthetic-host");
        assert_eq!(manifest.license, None);
        assert!(manifest.findings.iter().any(|f| f.contains("unsupported field")));
        assert_eq!(manifest.files.iter().find(|f| f.path == "resources/data").unwrap().sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(fs::read(source.join("SKILL.md")).unwrap(), skill);
        assert_eq!(fs::read(owned.join(&manifest.bundle_id).join("content/resources/data")).unwrap(), b"abc");
        let validated = library.validated_inventory(&manifest.bundle_id).unwrap();
        assert_eq!(validated, approved);
        assert!(library.validated_inventory("../selected").is_err());
        assert!(library.validated_inventory(&"0".repeat(32)).is_err());
        let manifest_path = owned.join(&manifest.bundle_id).join("manifest.json");
        let manifest_bytes = fs::read(&manifest_path).unwrap();
        let mut forged_metadata = manifest.clone();
        forged_metadata.metadata.insert("name".into(), "forged name".into());
        forged_metadata.metadata.insert("description".into(), "forged description".into());
        forged_metadata.license = Some("forged license".into());
        forged_metadata.findings = vec!["forged findings".into()];
        let forged_bytes = serde_json::to_vec(&forged_metadata).unwrap();
        fs::write(&manifest_path, &forged_bytes).unwrap();
        assert_eq!(library.validated_inventory(&manifest.bundle_id).unwrap(), approved);
        assert_eq!(fs::read(&manifest_path).unwrap(), forged_bytes, "validation must not repair or rewrite the manifest");
        assert_eq!(fs::read(owned.join(&manifest.bundle_id).join("content/SKILL.md")).unwrap(), skill);
        fs::write(&manifest_path, b"{}").unwrap();
        assert!(library.validated_inventory(&manifest.bundle_id).is_err());
        assert_eq!(fs::read(&manifest_path).unwrap(), b"{}");
        fs::write(&manifest_path, &manifest_bytes).unwrap();
        let mut wrong_identity = manifest.clone();
        wrong_identity.bundle_id = "0".repeat(32);
        fs::write(&manifest_path, serde_json::to_vec(&wrong_identity).unwrap()).unwrap();
        assert!(library.validated_inventory(&manifest.bundle_id).is_err());
        fs::write(&manifest_path, &manifest_bytes).unwrap();
        let skill_path = owned.join(&manifest.bundle_id).join("content/SKILL.md");
        fs::write(&skill_path, b"---\nname: changed\ndescription: changed\n---\n").unwrap();
        assert!(library.validated_inventory(&manifest.bundle_id).is_err());
        fs::write(&skill_path, skill).unwrap();
        assert_eq!(library.validated_inventory(&manifest.bundle_id).unwrap(), approved);
        assert!(library.uninstall("../selected").is_err());
        fs::write(owned.join(&manifest.bundle_id).join("content/resources/data"), b"changed").unwrap();
        assert!(library.validated_inventory(&manifest.bundle_id).is_err());
        assert_eq!(fs::read(owned.join(&manifest.bundle_id).join("content/resources/data")).unwrap(), b"changed");
        assert_eq!(fs::read(&manifest_path).unwrap(), manifest_bytes);
        assert!(library.uninstall(&manifest.bundle_id).is_err());
        assert!(owned.join(&manifest.bundle_id).exists());
        fs::write(owned.join(&manifest.bundle_id).join("content/resources/data"), b"abc").unwrap();
        assert_eq!(library.validated_inventory(&manifest.bundle_id).unwrap(), approved);
        let removed = library.uninstall(&manifest.bundle_id).unwrap(); assert!(removed.cleanup_warning.is_none());
        assert!(!owned.join(&manifest.bundle_id).exists()); assert_eq!(fs::read(owned.join("unrelated")).unwrap(), b"keep");
        assert_eq!(fs::read(source.join("resources/data")).unwrap(), b"abc");
        symlink(&outside, source.join("escape")).unwrap();
        assert!(library.import_authorized(&source, "fixture").is_err()); fs::remove_file(source.join("escape")).unwrap();
        let fifo = name(&source.join("special-fifo")).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(library.import_authorized(&source, "fixture").is_err());
        fs::remove_file(source.join("special-fifo")).unwrap();
        // Sorted SKILL.md copies before this rejection: production rollback leaves no visible partial.
        fs::write(source.join("zz-too-large"), vec![0u8; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert!(library.import_authorized(&source, "fixture").is_err()); fs::remove_file(source.join("zz-too-large")).unwrap();
        assert_eq!(fs::read_dir(&owned).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>(), vec![std::ffi::OsString::from("unrelated")]);
        assert!(library.import_authorized(&owned, "fixture").is_err());
        assert!(library.import_authorized(&source.join("../outside"), "fixture").is_err());
        // Deterministic ancestor replacement after descriptor acquisition exercises real open authority.
        let pinned = open_root(&source).unwrap();
        // Detach a pinned child, then create a file only after it is outside the selected root.
        // Exercise the production recursive copier with its expected child identity retained.
        let child = open_at(&pinned, Path::new("resources"), libc::O_RDONLY | libc::O_DIRECTORY, 0, true).unwrap();
        fs::rename(source.join("resources"), outside.join("detached-resources")).unwrap();
        fs::write(outside.join("detached-resources/NEWoutsidefile"), b"must not import").unwrap();
        let output = mkdir(&library.root, ".fixture-detached-output").unwrap();
        let mut detached_inventory = Inventory::default();
        assert!(walk(&pinned, &child, Some(&output), &library.root, "resources", 1, &mut detached_inventory, &mut None).is_err());
        assert!(!owned.join(".fixture-detached-output/NEWoutsidefile").exists());
        assert_eq!(fs::read(outside.join("detached-resources/NEWoutsidefile")).unwrap(), b"must not import");
        unlink(&library.root, ".fixture-detached-output", true).unwrap();
        fs::rename(outside.join("detached-resources"), source.join("resources")).unwrap();
        fs::remove_file(source.join("resources/NEWoutsidefile")).unwrap();
        fs::rename(&source, base.join("moved")).unwrap(); symlink(&outside, &source).unwrap();
        assert!(library.import_authorized(&source, "fixture").is_err());
        assert_eq!(read_file(open_at(&pinned, Path::new("resources/data"), libc::O_RDONLY, 0, true).unwrap(), MAX_FILE_BYTES).unwrap(), b"abc");
        fs::rename(base.join("moved/resources"), base.join("moved/resources-old")).unwrap();
        symlink(&outside, base.join("moved/resources")).unwrap();
        assert!(open_at(&pinned, Path::new("resources/secret"), libc::O_RDONLY, 0, true).is_err());
        assert_eq!(fs::read(outside.join("secret")).unwrap(), b"outside sentinel");
        // Existing destinations remain unchanged at the actual no-replace publication boundary.
        fs::create_dir(owned.join("collision-a")).unwrap(); fs::create_dir(owned.join("collision-b")).unwrap();
        assert!(rename(&library.root, "collision-a", "collision-b").is_err());
        assert!(owned.join("collision-a").exists() && owned.join("collision-b").exists());
    }
}
