//! The CLI installers move an installed service onto the `beans` they install by running
//! `beans service refresh`. These tests run `install-cli.sh`'s own function against stand-in
//! `beans` programs, and check the real `beans` keeps the contract it relies on. Nothing here runs
//! a service command.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// `install-cli.sh`'s `say` and `refresh_service`, as shipped.
fn installer_functions() -> String {
    let script = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/public/install-cli.sh")).unwrap();
    let function = |name: &str| {
        let start = script.find(&format!("\n{name}() {{\n")).unwrap_or_else(|| panic!("install-cli.sh has no {name}()"));
        let end = script[start + 1..].find("\n}\n").unwrap() + start + 1;
        script[start..end + 3].to_string()
    };
    function("say") + &function("refresh_service")
}

/// A stand-in `beans` in its own folder: `service refresh --help` exits `help_exit`, and
/// `service refresh` prints `output` and exits `exit`. Anything else fails the test.
fn stand_in(folder: &Path, help_exit: i32, output: &str, exit: i32) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = folder.join(format!("stand-in-{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("output"), output).unwrap();
    let script = format!(
        "#!/bin/sh\nhere=$(dirname \"$0\")\ncase \"$*\" in\n  'service refresh --help') exit {help_exit} ;;\n  'service refresh') cat \"$here/output\"; exit {exit} ;;\n  *) echo \"unexpected: $*\" >&2; exit 99 ;;\nesac\n"
    );
    let beans = dir.join("beans");
    std::fs::write(&beans, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&beans, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

/// Runs the installer's hook under `set -eu`, as the installer does, and returns its stdout and
/// stderr; the installer must carry on whatever happened.
fn run_hook(install_dir: &Path, environment: &[(&str, &str)]) -> (String, String) {
    let script = format!("set -eu\n{}\ninstall_dir=\"$1\"\nrefresh_service\necho '[installer continues]'\n", installer_functions());
    let output = Command::new("sh").args(["-c", &script, "sh"]).arg(install_dir).envs(environment.iter().copied()).output().unwrap();
    let (stdout, stderr) = (String::from_utf8(output.stdout).unwrap(), String::from_utf8(output.stderr).unwrap());
    assert!(output.status.success() && stdout.ends_with("[installer continues]\n"), "the installer stopped: {stdout}{stderr}");
    (stdout.trim_end_matches("[installer continues]\n").to_string(), stderr)
}

#[test]
fn a_release_without_refresh_is_left_alone() {
    let folder = tempfile::tempdir().unwrap();
    let old = stand_in(folder.path(), 2, "never run", 0);
    assert_eq!(run_hook(&old, &[]), (String::new(), String::new()));
}

#[test]
fn no_service_says_nothing() {
    let folder = tempfile::tempdir().unwrap();
    let beans = stand_in(folder.path(), 0, "No service is installed; nothing to refresh.\n", 0);
    assert_eq!(run_hook(&beans, &[]), (String::new(), String::new()));
}

#[test]
fn what_refresh_did_is_shown() {
    let folder = tempfile::tempdir().unwrap();
    let said = "Stopped an old beans serve (pid 41, version 0.1.0) left holding port 4874.\nThe service is up to date with this beans.\n";
    let beans = stand_in(folder.path(), 0, said, 0);
    assert_eq!(run_hook(&beans, &[]).0, said);
    // What a cleanup without a service says is shown too, though it starts like "no service".
    let cleaned = "Stopped an old beans serve (pid 41, version 0.1.0) left holding port 4874.\nNo service is installed; cleaned up what was left.\n";
    let beans = stand_in(folder.path(), 0, cleaned, 0);
    assert_eq!(run_hook(&beans, &[]).0, cleaned);
}

#[test]
fn a_failed_refresh_warns_and_the_installer_carries_on() {
    let folder = tempfile::tempdir().unwrap();
    let beans = stand_in(folder.path(), 0, "Error: launchctl failed: the service was put back as it was\n", 1);
    let (stdout, stderr) = run_hook(&beans, &[]);
    assert_eq!(stdout, "");
    assert!(stderr.contains("could not move onto this version") && stderr.contains("put back as it was") && stderr.contains("beans service status"), "{stderr}");
}

#[test]
fn colour_and_narrow_terminals_change_nothing() {
    let folder = tempfile::tempdir().unwrap();
    let beans = stand_in(folder.path(), 0, "The service is up to date with this beans.\n", 0);
    for environment in [[("FORCE_COLOR", "1"), ("COLUMNS", "20")], [("CLICOLOR_FORCE", "1"), ("TERM", "xterm-256color")]] {
        assert_eq!(run_hook(&beans, &environment).0, "The service is up to date with this beans.\n");
    }
}

#[test]
fn a_folder_with_spaces_and_quotes_is_quoted() {
    let folder = tempfile::tempdir().unwrap();
    let odd = folder.path().join("my \"bin\" $HOME 'x'");
    std::fs::create_dir_all(&odd).unwrap();
    let beans = stand_in(&odd, 0, "The service is up to date with this beans.\n", 0);
    assert_eq!(run_hook(&beans, &[]).0, "The service is up to date with this beans.\n");
}

/// The real `beans`: `service refresh --help` succeeds whatever the terminal, and a subcommand it
/// does not have fails, which is how the installers tell releases apart.
#[test]
fn beans_keeps_the_contract_the_installers_rely_on() {
    let beans = env!("CARGO_BIN_EXE_beans");
    for environment in [vec![], vec![("FORCE_COLOR", "1"), ("CLICOLOR_FORCE", "1"), ("COLUMNS", "20")]] {
        let status = Command::new(beans).args(["service", "refresh", "--help"]).envs(environment.clone()).output().unwrap().status;
        assert!(status.success(), "{environment:?}");
        let status = Command::new(beans).args(["service", "no-such-command", "--help"]).envs(environment.clone()).output().unwrap().status;
        assert!(!status.success(), "{environment:?}");
    }
}
