#![cfg(unix)]

use beans::config::{Config, StorageAdmission};

#[test]
fn admission_contention_preserves_errno_and_private_stage_diagnostics() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("existing");
    std::fs::create_dir(&home).unwrap();
    let held = StorageAdmission::acquire(&home).unwrap();
    let check = |error: anyhow::Error, stage: &str, lock: &str| {
        let source = error.downcast_ref::<std::io::Error>().expect("retain flock error");
        assert_eq!(source.kind(), std::io::ErrorKind::WouldBlock);
        assert!(matches!(source.raw_os_error(), Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN));
        let diagnostic = error.to_string();
        assert!(diagnostic.contains(&format!("stage={stage}")), "{diagnostic}");
        assert!(diagnostic.contains(&format!("lock={lock}")), "{diagnostic}");
        assert!(diagnostic.contains(&format!("errno={:?}", source.raw_os_error())), "{diagnostic}");
        assert!(!format!("{error:#}").contains(root.path().to_str().unwrap()));
    };
    check(StorageAdmission::acquire(&home).err().unwrap(), "home", "exclusive");
    check(StorageAdmission::acquire(&home.join("child")).err().unwrap(), "ancestor", "shared");
    let new = Config { home: root.path().join("new"), port: 0 };
    check(new.ensure_home().unwrap_err(), "creation-parent", "exclusive");
    assert!(!new.home.exists());
    assert!(std::fs::read_dir(&home).unwrap().next().is_none());
    drop(held);
    new.ensure_home().unwrap();
}
