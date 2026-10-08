//! Read-only discovery beneath one explicitly selected absolute directory.
//! Linux uses openat2 at every open; other platforms and unsupported kernels fail closed.
//! Returned paths are labels, not authority to reopen a file. No body or resource is loaded.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::frontmatter::parse_frontmatter;
use super::skills::SkillDiagnostic;

/// Hard bounds per call, including failed opens and non-skill directory entries.
pub const MAX_ENTRIES: usize = 4096;
pub const MAX_FILES: usize = 256;
pub const MAX_FILE_BYTES: u64 = 64 * 1024;
/// Root is depth zero; directories at depth four are inspected but not descended from.
pub const MAX_DEPTH: usize = 4;

/// Source metadata only: no invocation policy, grant, host identity or executable body.
/// Fields use Beans' flat scalar parser and are not lossless YAML or raw provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSkill {
    /// Relative to the opened root; never use this label as a secure-open capability.
    pub path: PathBuf,
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SkillDiscovery {
    pub skills: Vec<DiscoveredSkill>,
    pub diagnostics: Vec<SkillDiagnostic>,
}

impl SkillDiscovery {
    fn warn(&mut self, path: &Path, message: impl Into<String>) {
        self.diagnostics.push(SkillDiagnostic { path: path.to_path_buf(), message: message.into() });
    }
}

/// Scan only `root`, never a home directory or automatically discovered host root.
/// Every SKILL.md within the bounds is independent; duplicates and nested skills remain
/// distinct. Missing names/descriptions are diagnosed, not inferred from a host convention.
/// No ignore files, sidecars, hooks, resources, scripts or network are consulted.
/// Linux denies symlinks (including ancestors), magic links and child mount crossings.
/// Concurrent replacement cannot redirect an open outside the opened root object; this
/// is not a coherent filesystem snapshot and does not detect in-place content edits.
pub fn discover_skills(root: &Path) -> SkillDiscovery {
    let mut result = SkillDiscovery::default();
    #[cfg(target_os = "linux")]
    match secure::open_root(root) {
        Ok(root_handle) => {
            let mut scanner = secure::Scanner { root: root_handle, entries: 0, files: 0, result: &mut result };
            scanner.walk(Path::new(""), 0);
            result.skills.sort_by(|a, b| a.path.cmp(&b.path));
        }
        Err(error) => result.warn(root, format!("secure root open failed (no fallback): {error}")),
    }
    #[cfg(not(target_os = "linux"))]
    result.warn(root, "secure skill discovery unsupported on this platform; no files opened");
    result
}

#[cfg(target_os = "linux")]
mod secure {
    use super::*;
    use std::ffi::{CStr, CString, OsStr, OsString};
    use std::fs::File;
    use std::io::{self, BufRead, BufReader, Read};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::path::Component;

    // Linux open_how ABI, with no optional extension fields. Unknown syscall/flags fail closed.
    #[repr(C)]
    struct OpenHow { flags: u64, mode: u64, resolve: u64 }
    const NO_XDEV: u64 = 0x01;
    const NO_MAGICLINKS: u64 = 0x02;
    const NO_SYMLINKS: u64 = 0x04;
    const BENEATH: u64 = 0x08;

    fn open_beneath(parent: &File, path: &Path, directory: bool, no_mounts: bool) -> io::Result<File> {
        let name = CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))?;
        let how = OpenHow {
            flags: (libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK | if directory { libc::O_DIRECTORY } else { 0 }) as u64,
            mode: 0,
            resolve: BENEATH | NO_SYMLINKS | NO_MAGICLINKS | if no_mounts { NO_XDEV } else { 0 },
        };
        // SAFETY: live parent descriptor, NUL-terminated path, correctly sized open_how.
        let fd = unsafe { libc::syscall(libc::SYS_openat2, parent.as_raw_fd(), name.as_ptr(), &how, std::mem::size_of::<OpenHow>()) };
        if fd < 0 { return Err(io::Error::last_os_error()); }
        // SAFETY: successful syscall transfers one fresh descriptor to File.
        Ok(unsafe { File::from_raw_fd(fd as libc::c_int) })
    }

    pub(super) fn open_root(root: &Path) -> io::Result<File> {
        if !root.is_absolute() || root.components().any(|c| !matches!(c, Component::RootDir | Component::Normal(_))) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "root must be absolute without parent traversal"));
        }
        // '/' has no ancestors or symlink components. All caller-selected components are
        // resolved in one kernel operation, not canonicalized and subsequently reopened.
        let anchor = File::open("/")?;
        let relative = root.strip_prefix("/").map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid root"))?;
        open_beneath(&anchor, if relative.as_os_str().is_empty() { Path::new(".") } else { relative }, true, false)
    }

    struct Directory(*mut libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) {
            // SAFETY: pointer owned by this guard, closed exactly once.
            unsafe { libc::closedir(self.0); }
        }
    }

    pub(super) struct Scanner<'a> {
        pub root: File,
        pub entries: usize,
        pub files: usize,
        pub result: &'a mut SkillDiscovery,
    }

    impl Scanner<'_> {
        pub fn walk(&mut self, relative: &Path, depth: usize) {
            let directory = match open_beneath(&self.root, if relative.as_os_str().is_empty() { Path::new(".") } else { relative }, true, true) {
                Ok(file) => file,
                Err(error) => { self.result.warn(relative, format!("secure directory open failed: {error}")); return; }
            };
            use std::os::fd::IntoRawFd;
            let fd = directory.into_raw_fd();
            // SAFETY: fd is exclusively owned; fdopendir takes ownership only on success.
            let pointer = unsafe { libc::fdopendir(fd) };
            if pointer.is_null() {
                let error = io::Error::last_os_error();
                // SAFETY: failed fdopendir left ownership with us.
                unsafe { libc::close(fd); }
                self.result.warn(relative, format!("directory enumeration failed: {error}"));
                return;
            }
            let directory = Directory(pointer);
            loop {
                // SAFETY: Linux errno is thread local; readdir's pointer is valid until next call.
                unsafe { *libc::__errno_location() = 0; }
                let entry = unsafe { libc::readdir(directory.0) };
                if entry.is_null() {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() != Some(0) { self.result.warn(relative, format!("directory enumeration failed: {error}")); }
                    break;
                }
                // SAFETY: d_name is NUL terminated by readdir; copy before recursing.
                let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
                if name == b"." || name == b".." { continue; }
                if self.entries == MAX_ENTRIES {
                    self.result.warn(relative, format!("entry limit {MAX_ENTRIES} reached; discovery incomplete"));
                    return;
                }
                self.entries += 1;
                let name = OsString::from_vec(name.to_vec());
                let path = relative.join(&name);
                // d_type is only a hint. Every actual open is resolved again from root.
                let kind = unsafe { (*entry).d_type };
                if kind == libc::DT_LNK {
                    self.result.warn(&path, "symlink rejected");
                    continue;
                }
                if name == OsStr::new("SKILL.md") {
                    if self.files == MAX_FILES {
                        self.result.warn(&path, format!("file limit {MAX_FILES} reached; discovery incomplete"));
                        return;
                    }
                    self.files += 1;
                    self.metadata(&path);
                } else if kind == libc::DT_DIR || kind == libc::DT_UNKNOWN {
                    // Unknown types must be securely opened before classification.
                    match open_beneath(&self.root, &path, true, true) {
                        Ok(_) if depth < MAX_DEPTH => self.walk(&path, depth + 1),
                        Ok(_) => self.result.warn(&path, format!("depth limit {MAX_DEPTH} reached; subtree omitted")),
                        Err(error) if kind == libc::DT_UNKNOWN && error.raw_os_error() == Some(libc::ENOTDIR) => {
                            self.result.warn(&path, "non-SKILL resource or sidecar not inspected");
                        }
                        Err(error) => self.result.warn(&path, format!("secure directory open failed: {error}")),
                    }
                } else {
                    self.result.warn(&path, "non-SKILL resource or sidecar not inspected");
                }
                if self.entries == MAX_ENTRIES || self.files == MAX_FILES {
                    self.result.warn(relative, "entry or file limit reached; discovery incomplete");
                    return;
                }
            }
        }

        fn metadata(&mut self, path: &Path) {
            let result = (|| -> io::Result<String> {
                let file = open_beneath(&self.root, path, false, true)?;
                let metadata = file.metadata()?;
                if !metadata.is_file() { return Err(io::Error::new(io::ErrorKind::InvalidData, "not a regular file")); }
                if metadata.len() > MAX_FILE_BYTES { return Err(io::Error::new(io::ErrorKind::InvalidData, format!("file exceeds {MAX_FILE_BYTES} bytes"))); }
                let mut reader = BufReader::new(file.take(MAX_FILE_BYTES));
                let mut text = String::new();
                let mut line = String::new();
                reader.read_line(&mut line)?;
                if !matches!(line.as_str(), "---\n" | "---\r\n") {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "frontmatter must start with a standalone --- line"));
                }
                text.push_str(&line);
                loop {
                    line.clear();
                    let count = reader.read_line(&mut line)?;
                    if text.len() + count > MAX_FILE_BYTES as usize { return Err(io::Error::new(io::ErrorKind::InvalidData, "frontmatter byte limit reached")); }
                    if count == 0 { return Err(io::Error::new(io::ErrorKind::InvalidData, "unterminated frontmatter")); }
                    text.push_str(&line);
                    if matches!(line.trim_end_matches(['\r', '\n']), "---") { break; }
                }
                Ok(text)
            })();
            match result {
                Ok(text) => {
                    // Reuse the existing parser only on the bounded frontmatter, never body.
                    let parsed = parse_frontmatter(&text);
                    for key in parsed.fields.keys() {
                        if !matches!(key.as_str(), "name" | "description" | "license" | "compatibility") {
                            self.result.warn(path, format!("unsupported field {key:?}; source metadata only, no policy or execution"));
                        }
                    }
                    // Scalar parsing flattens nested constructs; warn rather than claim YAML.
                    if text.lines().any(|line| line.starts_with([' ', '\t']) || line.split_once(':').is_some_and(|(_, value)| {
                        let value = value.trim();
                        value.starts_with(['[', '{', '&', '*', '!']) || matches!(value, "|" | ">")
                    })) {
                        self.result.warn(path, "structured YAML or block scalar is unsupported; flat parser is not lossless");
                    }
                    for key in ["name", "description"] {
                        if parsed.get(key).is_none() { self.result.warn(path, format!("missing {key}; no host-specific fallback applied")); }
                    }
                    self.result.skills.push(DiscoveredSkill { path: path.to_path_buf(), fields: parsed.fields });
                }
                Err(error) => self.result.warn(path, format!("metadata read rejected: {error}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_discovery_security_and_metadata() {
        #[cfg(not(target_os = "linux"))]
        {
            let result = discover_skills(Path::new("/explicit-synthetic-root"));
            assert!(result.skills.is_empty());
            assert!(result.diagnostics.iter().any(|d| d.message.contains("unsupported")));
        }
        #[cfg(target_os = "linux")]
        {
            use std::fs;
            use std::os::unix::fs::{symlink, PermissionsExt};
            let base = std::env::temp_dir().join(format!("beans-discovery-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&base).unwrap();
            // Resolve only the synthetic fixture location (e.g. a symlinked system temp).
            // The production scanner itself never canonicalizes caller-selected paths.
            let base = base.canonicalize().unwrap();
            struct Cleanup(PathBuf);
            impl Drop for Cleanup { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
            let _cleanup = Cleanup(base.clone());
            let root = base.join("selected");
            let outside = base.join("outside");
            fs::create_dir(&root).unwrap();
            fs::create_dir(&outside).unwrap();
            let valid = "---\nname: valid\ndescription: \"Read metadata: only\"\n---\n!`never execute`";
            fs::write(root.join("SKILL.md"), valid).unwrap();
            fs::write(outside.join("SKILL.md"), "---\nname: outside\ndescription: forbidden\n---\n").unwrap();
            fs::create_dir(root.join("unsupported")).unwrap();
            fs::write(root.join("unsupported/SKILL.md"), "---\nname: unsupported\ndescription: sample\nhooks:\n  command: forbidden\npaths: [src]\n---\n").unwrap();
            fs::create_dir(root.join("oversize")).unwrap();
            fs::write(root.join("oversize/SKILL.md"), vec![b'x'; MAX_FILE_BYTES as usize + 1]).unwrap();
            fs::create_dir(root.join("unreadable")).unwrap();
            fs::write(root.join("unreadable/SKILL.md"), valid).unwrap();
            fs::set_permissions(root.join("unreadable/SKILL.md"), fs::Permissions::from_mode(0)).unwrap();
            // A directory named SKILL.md is always rejected even under privileged test users.
            fs::create_dir(root.join("not-file")).unwrap();
            fs::create_dir(root.join("not-file/SKILL.md")).unwrap();
            symlink(&outside, root.join("link")).unwrap();
            symlink(outside.join("SKILL.md"), root.join("file-link")).unwrap();
            let deep = root.join("a/b/c/d/e");
            fs::create_dir_all(&deep).unwrap();
            fs::write(deep.join("SKILL.md"), valid).unwrap();
            let result = discover_skills(&root);
            // Kernel support is a required check prerequisite, not a silently skipped test.
            assert!(result.skills.iter().any(|s| s.path == Path::new("SKILL.md") && s.fields.get("description").map(String::as_str) == Some("Read metadata: only")), "{:?}", result.diagnostics);
            assert!(!result.skills.iter().any(|s| s.fields.get("name").map(String::as_str) == Some("outside") || s.path == Path::new("a/b/c/d/e/SKILL.md")));
            for fragment in ["unsupported field", "not lossless", "symlink rejected", "exceeds", "depth limit", "not a regular file"] {
                assert!(result.diagnostics.iter().any(|d| d.message.contains(fragment)), "missing {fragment}: {:?}", result.diagnostics);
            }
            // Root may legitimately read mode-zero files; unprivileged runs must diagnose them.
            if unsafe { libc::geteuid() } != 0 {
                assert!(result.diagnostics.iter().any(|d| d.path == Path::new("unreadable/SKILL.md") && d.message.contains("rejected")));
            }
            let alias = base.join("alias");
            symlink(&root, &alias).unwrap();
            assert!(discover_skills(&alias.join("unsupported")).skills.is_empty());
            assert!(discover_skills(&root.join("../outside")).skills.is_empty());
            let counted = base.join("counted");
            fs::create_dir(&counted).unwrap();
            fs::write(counted.join("SKILL.md"), valid).unwrap();
            for (entries, files) in [(MAX_ENTRIES - 1, 0), (0, MAX_FILES - 1)] {
                let mut limited = SkillDiscovery::default();
                let mut scanner = secure::Scanner { root: secure::open_root(&counted).unwrap(), entries, files, result: &mut limited };
                scanner.walk(Path::new(""), 0);
                assert!(scanner.entries <= MAX_ENTRIES && scanner.files <= MAX_FILES);
                assert!(limited.diagnostics.iter().any(|d| d.message.contains("limit reached")));
            }
            // Deterministic replacement race: an opened root remains its original object.
            let handle = secure::open_root(&root).unwrap();
            let moved = base.join("moved");
            fs::rename(&root, &moved).unwrap();
            symlink(&outside, &root).unwrap();
            fs::rename(moved.join("unsupported"), base.join("detached")).unwrap();
            symlink(&outside, moved.join("unsupported")).unwrap();
            let mut anchored = SkillDiscovery::default();
            secure::Scanner { root: handle, entries: 0, files: 0, result: &mut anchored }.walk(Path::new(""), 0);
            assert!(anchored.skills.iter().any(|s| s.path == Path::new("SKILL.md") && s.fields.get("name").map(String::as_str) == Some("valid")));
            assert!(!anchored.skills.iter().any(|s| s.fields.get("name").map(String::as_str) == Some("outside")));
            assert!(discover_skills(&root).skills.is_empty());
            fs::remove_file(moved.join("SKILL.md")).unwrap();
            symlink(outside.join("SKILL.md"), moved.join("SKILL.md")).unwrap();
            let replaced = discover_skills(&moved);
            assert!(!replaced.skills.iter().any(|s| s.path == Path::new("SKILL.md") || s.fields.get("name").map(String::as_str) == Some("outside")));
        }
    }
}
