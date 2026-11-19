//! The ONNX Runtime a Runner downloads when none ships beside it (a CLI installed on its own, a
//! development build): the same official wheel the apps ship (`crates/cli/onnxruntime.json`,
//! placed by `scripts/onnxruntime.ts`), pinned by its size and SHA-256 and those of the library
//! inside it. It is fetched only as part of a model preview the user approved, like the model's
//! own files, and the wheel is unpacked in memory: only the library is kept.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

/// The pinned runtimes, one per platform, as the build scripts read them.
const MANIFEST: &str = include_str!("../../onnxruntime.json");

#[derive(Deserialize)]
struct Manifest {
    version: String,
    license: String,
    targets: BTreeMap<String, Pinned>,
}

/// One platform's runtime: the wheel to download and the library to keep from it.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Pinned {
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
    /// The library's path inside the wheel.
    pub member: String,
    pub library_bytes: u64,
    pub library_sha256: String,
}

fn manifest() -> &'static Manifest {
    static MANIFEST_PARSED: OnceLock<Manifest> = OnceLock::new();
    MANIFEST_PARSED.get_or_init(|| serde_json::from_str(MANIFEST).expect("crates/cli/onnxruntime.json is valid"))
}

/// The runtime's version and license, as previews show it.
pub fn version() -> &'static str {
    &manifest().version
}

pub fn license() -> &'static str {
    &manifest().license
}

/// The manifest's name for `os` and `arch` as Rust names them (`std::env::consts`).
pub fn target_for(os: &str, arch: &str) -> Option<String> {
    let os = match os {
        "macos" => "darwin",
        "linux" => "linux",
        "windows" => "windows",
        _ => return None,
    };
    let arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => return None,
    };
    Some(format!("{os}-{arch}"))
}

/// `target`'s pinned runtime, if one is pinned.
pub fn pinned_for(target: &str) -> Option<Pinned> {
    manifest().targets.get(target).cloned()
}

/// The runtime this Runner can download and load: none for a build without local models, or a
/// static (musl) build, which cannot load a shared library at all.
pub fn pinned() -> Option<Pinned> {
    if !cfg!(feature = "embedding-local") || cfg!(target_env = "musl") {
        return None;
    }
    pinned_for(&target_for(std::env::consts::OS, std::env::consts::ARCH)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pin_is_an_official_wheel_with_its_library_pinned_too() {
        assert!(!version().is_empty() && !license().is_empty());
        let targets = &manifest().targets;
        assert_eq!(targets.keys().map(String::as_str).collect::<Vec<_>>(), ["darwin-arm64", "linux-arm64", "linux-x64", "windows-arm64", "windows-x64"]);
        let hex = |text: &str| text.len() == 64 && text.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
        for (target, pin) in targets {
            assert!(pin.url.starts_with("https://files.pythonhosted.org/packages/"), "{target}");
            assert!(pin.url.contains(&format!("onnxruntime-{}-", version())), "{target}");
            assert!(hex(&pin.sha256) && hex(&pin.library_sha256), "{target}");
            assert!(pin.bytes > 1_000_000 && pin.library_bytes > 1_000_000, "{target}");
            assert!(pin.library_bytes <= 256 << 20, "{target}: under the runtime limit a load checks");
            assert!(pin.member.starts_with("onnxruntime/capi/"), "{target}");
        }
    }

    #[test]
    fn platforms_map_to_the_manifest_names() {
        assert_eq!(target_for("macos", "aarch64").as_deref(), Some("darwin-arm64"));
        assert_eq!(target_for("linux", "x86_64").as_deref(), Some("linux-x64"));
        assert_eq!(target_for("windows", "aarch64").as_deref(), Some("windows-arm64"));
        assert_eq!(target_for("freebsd", "x86_64"), None);
        assert_eq!(target_for("linux", "riscv64"), None);
        assert!(pinned_for("darwin-x64").is_none(), "no official wheel for Intel Macs");
        assert!(pinned_for("toString").is_none());
        // This build: a runtime to download exactly when it can load one.
        let here = target_for(std::env::consts::OS, std::env::consts::ARCH).and_then(|t| pinned_for(&t));
        let loads = cfg!(feature = "embedding-local") && !cfg!(target_env = "musl");
        assert_eq!(pinned(), if loads { here } else { None });
    }
}
