//! `beans service`: runs `beans serve` in the background on a computer without the app, from
//! login on and again whenever it stops, the way the apps keep their own CLI running. macOS gets a
//! launchd agent, Linux a systemd user unit (with lingering, so it runs without a login), and
//! Windows a supervisor, `beans service run`, that the user's Run key starts at sign-in. Each
//! starts `beans serve` with `BEANS_SERVICE=1`, and with the `BEANS_HOME`, `BEANS_PORT`, and
//! `BEANS_RELAY_URL` of the shell that installed it.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Config;

/// The launchd label.
#[cfg(target_os = "macos")]
const NAME: &str = "app.beans.serve";
/// The systemd user unit.
#[cfg(target_os = "linux")]
const UNIT: &str = "beans.service";
/// A log larger than this starts over when `beans serve` starts under the service.
const LOG_LIMIT: u64 = 20 << 20;

#[cfg(any(windows, test))]
mod supervisor;
mod plan;
mod process;

use process::{clear_port, output_within, probe};

/// Whether `beans service` started this process.
pub fn supervised() -> bool {
    std::env::var("BEANS_SERVICE").is_ok_and(|value| value == "1")
}

/// How the service stands, in words for `beans service status`.
pub struct Status {
    pub installed: bool,
    pub running: Option<u32>,
    /// Where `beans serve` writes its log, or how to read it.
    pub log: String,
    /// What is wrong with it, or what a command changed, in sentences.
    pub notes: Vec<String>,
}

/// The binary the service runs: this one, through any symlink to it. On Windows the canonical
/// path is a verbatim one (`\\?\C:\…`), which the Run key's sign-in start and other tools may
/// not take; it is written plainly whenever the plain form means the same file.
fn exe() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    if replaced_while_running(&exe.to_string_lossy()) {
        anyhow::bail!("This beans was replaced while it ran ({}); run the new one.", exe.display());
    }
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    #[cfg(windows)]
    let exe = match exe.to_str() {
        Some(path) => PathBuf::from(plain_windows_path(path)),
        None => exe,
    };
    Ok(exe)
}

/// Whether Linux says this binary's file was deleted or replaced since it started: it then names
/// it `… (deleted)`, which is no path to run.
fn replaced_while_running(path: &str) -> bool {
    cfg!(target_os = "linux") && path.ends_with(" (deleted)")
}

/// Why `program` is somewhere that goes away, so no service may run it: macOS's App
/// Translocation (an app opened where it was downloaded runs from a randomized read-only copy
/// until it is moved), a mounted disk image or removable volume, an AppImage's mount, or the
/// temporary folder `temp`, where an archive opened without extracting it runs from.
fn transient_location(program: &str, temp: &str) -> Option<&'static str> {
    let fold = |path: &str| path.replace('\\', "/").to_lowercase();
    let (program_folded, temp) = (fold(program), fold(temp));
    let temp = temp.trim_end_matches('/');
    if program_folded.contains("/apptranslocation/") {
        Some("a temporary copy macOS runs an app from until it is moved to Applications")
    } else if cfg!(target_os = "macos") && program.starts_with("/Volumes/") {
        Some("a disk image or a volume that can be ejected")
    } else if program_folded.contains("/.mount_") {
        Some("an AppImage's temporary mount")
    } else if !temp.is_empty() && program_folded.starts_with(&format!("{temp}/")) {
        Some("the temporary folder")
    } else {
        None
    }
}

/// Refuses to make a service of a `beans` running from somewhere that goes away.
fn permanent(program: &str) -> anyhow::Result<()> {
    match transient_location(program, &std::env::temp_dir().to_string_lossy()) {
        Some(reason) => anyhow::bail!("This beans runs from {program}, {reason}, which does not last. Move beans to a lasting folder and run beans service install from there."),
        None => Ok(()),
    }
}

/// `home` as a path that means the same folder wherever the service starts: launchd starts jobs
/// in `/`, systemd in the home folder, a sign-in wherever Explorer is, so a relative `BEANS_HOME`
/// is made absolute from `cwd`, where the installing command ran. `.` components go; `..` stays,
/// since a symlink before it decides what it means.
fn absolute_home(home: &Path, cwd: &Path) -> PathBuf {
    let joined = if home.is_absolute() { home.to_path_buf() } else { cwd.join(home) };
    joined.components().filter(|component| *component != std::path::Component::CurDir).collect()
}

/// `path` without its verbatim prefix, when Win32's ordinary path rules read the result as the
/// same file: `\\?\C:\dir\beans.exe` is `C:\dir\beans.exe`, and `\\?\UNC\server\share\…` is
/// `\\server\share\…`. It stays verbatim when it is too long for them, or has a component they
/// would change or read differently: `.` or `..`, a trailing dot or space, a reserved device name
/// (`CON`, `NUL`, `COM1`, …), or a character they forbid.
#[cfg(any(windows, test))]
fn plain_windows_path(path: &str) -> String {
    let (plain, rest) = if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        (format!(r"\\{rest}"), rest)
    } else if let Some(rest) = path.strip_prefix(r"\\?\").filter(|rest| {
        let bytes = rest.as_bytes();
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\'
    }) {
        (rest.to_string(), &rest[3..])
    } else {
        return path.to_string();
    };
    let reserved = |name: &str| {
        let stem = name.split('.').next().unwrap_or(name).trim_end().to_ascii_uppercase();
        matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
            || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit() && stem.as_bytes()[3] != b'0')
    };
    let ordinary = plain.chars().count() < 260
        && rest.split('\\').all(|part| {
            !part.is_empty() && part != "." && part != ".." && !part.ends_with('.') && !part.ends_with(' ') && !reserved(part)
                && !part.chars().any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '|' | '?' | '*') || (c as u32) < 32)
        });
    if ordinary { plain } else { path.to_string() }
}

/// A path as the text a service definition holds. A lossy conversion would point the service at
/// another binary or another home, so a path that is not Unicode stops the install instead.
fn text<'a>(path: &'a Path, what: &str) -> anyhow::Result<&'a str> {
    path.to_str().ok_or_else(|| anyhow::anyhow!("The path of {what} ({}) is not valid Unicode, so beans service cannot name it in a service definition.", path.display()))
}

/// What `beans serve` needs to serve the same account as the command that installed it: a
/// folder or port other than the default, and a relay named in the environment.
fn passed_on(config: &Config) -> anyhow::Result<Vec<(String, String)>> {
    let default_home = dirs::home_dir().map(|home| home.join(".beans-v2"));
    environment(config, default_home.as_deref(), std::env::var("BEANS_RELAY_URL").ok(), &std::env::current_dir()?)
}

/// [`passed_on`] from its inputs. Values pass exactly as given; each platform's definition
/// escapes them for its own syntax.
fn environment(config: &Config, default_home: Option<&Path>, relay: Option<String>, cwd: &Path) -> anyhow::Result<Vec<(String, String)>> {
    let mut vars = Vec::new();
    let home = absolute_home(&config.home, cwd);
    if Some(home.as_path()) != default_home {
        vars.push(("BEANS_HOME".to_string(), text(&home, "BEANS_HOME")?.to_string()));
    }
    if config.port != crate::config::DEFAULT_PORT {
        vars.push(("BEANS_PORT".to_string(), config.port.to_string()));
    }
    if let Some(url) = relay.filter(|url| !url.trim().is_empty()) {
        vars.push(("BEANS_RELAY_URL".to_string(), url));
    }
    Ok(vars)
}

/// A `beans serve` started under the service keeps its log in bounds: launchd appends to the
/// same file forever, so one past [`LOG_LIMIT`] starts over.
pub fn trim_log() {
    #[cfg(unix)]
    if supervised() {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(2, &mut stat) } == 0 && (stat.st_mode & libc::S_IFMT) == libc::S_IFREG && stat.st_size as u64 > LOG_LIMIT {
            unsafe { libc::ftruncate(2, 0) };
        }
    }
}

// MARK: - Definitions
//
// The text each service manager reads, made from plain inputs so that every platform's is tested
// on any platform. Reading the environment, writing files, and the commands that install and
// start the service stay in the platform sections below.

/// A service definition cannot carry this value as it is.
#[derive(Debug, PartialEq, Eq)]
struct Unrepresentable {
    /// The variable, or the path, that holds it.
    what: String,
    character: char,
    /// Why the definition cannot hold it.
    reason: String,
}

impl std::fmt::Display for Unrepresentable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The value itself is not repeated: a relay URL can hold a token.
        write!(f, "{} holds U+{:04X}, which {}; beans service cannot pass it on unchanged.", self.what, self.character as u32, self.reason)
    }
}

impl std::error::Error for Unrepresentable {}

/// No environment holds a NUL, so none is passed on.
fn refuse_nul(what: &str, value: &str) -> Result<(), Unrepresentable> {
    match value.contains('\0') {
        true => Err(Unrepresentable { what: what.to_string(), character: '\0', reason: "no environment variable can hold".into() }),
        false => Ok(()),
    }
}

/// Whether XML 1.0, and so an XML property list, can hold `c` at all, even as a character
/// reference.
#[cfg(any(target_os = "macos", test))]
fn xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')
}

/// `text` as the content of a plist `<string>`. A carriage return is a character reference,
/// since XML reads a bare one, or one before a newline, as a newline.
///
/// The other control characters cannot be written in an XML plist at all. Until there is a
/// decided policy for them, such a value stops the install, and nothing is written.
/// The first character of `text` a plist cannot carry unchanged, if any.
#[cfg(test)]
fn plist_refused(text: &str) -> Option<char> {
    match text.starts_with('\u{feff}') {
        true => Some('\u{feff}'),
        false => text.chars().find(|c| !xml_char(*c)),
    }
}

#[cfg(any(target_os = "macos", test))]
fn plist_string(what: &str, text: &str) -> Result<String, Unrepresentable> {
    refuse_nul(what, text)?;
    // CoreFoundation reads a leading U+FEFF as a byte order mark and drops it, written
    // plainly or as a character reference.
    if text.starts_with('\u{feff}') {
        return Err(Unrepresentable { what: what.to_string(), character: '\u{feff}', reason: "macOS drops at the start of a property list string".into() });
    }
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\r' => escaped.push_str("&#13;"),
            c if xml_char(c) => escaped.push(c),
            character => return Err(Unrepresentable { what: what.to_string(), character, reason: "an XML property list cannot hold".into() }),
        }
    }
    Ok(escaped)
}

/// The launchd agent that runs `exe serve`, logging to `log`, with `BEANS_SERVICE=1` and
/// `environment`.
#[cfg(any(target_os = "macos", test))]
fn launchd_plist(label: &str, exe: &str, log: &str, environment: &[(String, String)]) -> Result<String, Unrepresentable> {
    let mut variables = String::from("\t\t<key>BEANS_SERVICE</key>\n\t\t<string>1</string>\n");
    for (name, value) in environment {
        variables.push_str(&format!("\t\t<key>{}</key>\n\t\t<string>{}</string>\n", plist_string(name, name)?, plist_string(name, value)?));
    }
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{exe}</string>
		<string>serve</string>
	</array>
	<key>EnvironmentVariables</key>
	<dict>
{variables}	</dict>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<true/>
	<key>StandardOutPath</key>
	<string>{log}</string>
	<key>StandardErrorPath</key>
	<string>{log}</string>
</dict>
</plist>
"#,
        label = plist_string("the launchd label", label)?,
        exe = plist_string("the path of beans", exe)?,
        log = plist_string("the log path", log)?,
    ))
}

/// The first systemd that keeps a newline in an `Environment=` value, and the first that keeps
/// the other control characters. Older ones log the assignment as invalid and start the service
/// without that variable at all. Read from systemd's `env_value_is_valid`: 237 (Ubuntu 18.04)
/// allows only a tab, 245 (Ubuntu 20.04) a tab and a newline, 247 (Debian 11) any. The releases
/// between 237 and 245, such as RHEL 8's 239, are taken to be as strict as 237.
#[cfg(any(target_os = "linux", test))]
const SYSTEMD_KEEPS_NEWLINES: u32 = 245;
#[cfg(any(target_os = "linux", test))]
const SYSTEMD_KEEPS_CONTROLS: u32 = 247;

/// Whether systemd reads `c` as invalid UTF-8, as every release from 237 to 257 does for the
/// noncharacters (`unichar_is_valid`): it then drops an `Environment=` assignment holding it,
/// and skips the whole line of any other setting.
#[cfg(any(target_os = "linux", test))]
fn systemd_noncharacter(c: char) -> bool {
    ('\u{FDD0}'..='\u{FDEF}').contains(&c) || (c as u32 & 0xFFFE) == 0xFFFE
}

/// The systemd release in `systemctl --version`'s first line, `systemd 255 (255.4-1ubuntu8)`.
#[cfg(any(target_os = "linux", test))]
fn systemd_release(version: &str) -> Option<u32> {
    let release = version.lines().next()?.strip_prefix("systemd ")?;
    let digits = release.find(|c: char| !c.is_ascii_digit()).unwrap_or(release.len());
    release[..digits].parse().ok()
}

/// `text` as one quoted word of a systemd unit setting. systemd reads C escapes inside the
/// quotes, so backslashes, quotes, and every control character are escaped, and a newline cannot
/// end the line. It expands `%` specifiers in every setting, so `%` is `%%`. `$` is literal in
/// `Environment=` and in the program's path, the only places these words go.
#[cfg(any(target_os = "linux", test))]
fn systemd_word(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for c in text.chars() {
        match c {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\t' => quoted.push_str("\\t"),
            '\r' => quoted.push_str("\\r"),
            c if c.is_ascii_control() => quoted.push_str(&format!("\\x{:02x}", c as u32)),
            '%' => quoted.push_str("%%"),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

/// One `Environment=` assignment, if `systemd`, the release that will read it (`None` when it
/// is not known), keeps `value` rather than dropping the variable.
#[cfg(any(target_os = "linux", test))]
fn systemd_assignment(name: &str, value: &str, systemd: Option<u32>) -> Result<String, Unrepresentable> {
    refuse_nul(name, value)?;
    if let Some(character) = value.chars().find(|c| systemd_noncharacter(*c)) {
        return Err(Unrepresentable { what: name.to_string(), character, reason: format!("systemd reads as invalid UTF-8 and would drop, starting beans serve without {name}") });
    }
    for c in value.chars().filter(|c| c.is_ascii_control() && *c != '\t') {
        let needed = if c == '\n' { SYSTEMD_KEEPS_NEWLINES } else { SYSTEMD_KEEPS_CONTROLS };
        if systemd.map_or(true, |release| release < needed) {
            let reading = systemd.map_or_else(|| "this computer's systemd".to_string(), |release| format!("systemd {release}"));
            return Err(Unrepresentable { what: name.to_string(), character: c, reason: format!("{reading} would drop, starting beans serve without {name} (systemd {needed} and later keep it)") });
        }
    }
    Ok(format!("Environment={}", systemd_word(&format!("{name}={value}"))))
}

/// The program's path in `ExecStart=`. Every systemd refuses a path holding a quote, a
/// backslash, a control character, or a noncharacter, and then cannot load the unit at all. The
/// path is not expanded for `$` (only the arguments are), so `$` stays as it is.
#[cfg(any(target_os = "linux", test))]
fn systemd_program(exe: &str) -> Result<String, Unrepresentable> {
    let what = "the path of beans";
    refuse_nul(what, exe)?;
    if let Some(character) = exe.chars().find(|c| matches!(c, '"' | '\'' | '\\') || c.is_ascii_control() || systemd_noncharacter(*c)) {
        return Err(Unrepresentable { what: what.to_string(), character, reason: "systemd refuses in the path of a program".into() });
    }
    Ok(systemd_word(exe))
}

/// The systemd user unit that runs `exe serve`, with `BEANS_SERVICE=1` and `environment`, for
/// the systemd release that will read it.
#[cfg(any(target_os = "linux", test))]
fn systemd_unit(exe: &str, environment: &[(String, String)], systemd: Option<u32>) -> Result<String, Unrepresentable> {
    let mut variables = String::from("Environment=BEANS_SERVICE=1\n");
    for (name, value) in environment {
        variables.push_str(&systemd_assignment(name, value, systemd)?);
        variables.push('\n');
    }
    Ok(format!(
        "[Unit]\nDescription=Beans: this computer's Runner (beans serve)\n\n[Service]\nExecStart={} serve\n{variables}Restart=always\nRestartSec=2\n\n[Install]\nWantedBy=default.target\n",
        systemd_program(exe)?
    ))
}

/// `arg` as one argument of a Windows command line, read back the same by the C runtime's
/// rules, Rust's `std::env::args`, and `CommandLineToArgvW`: quoted, with the backslashes before
/// a quote doubled and the quote escaped.
///
/// The Run key's command line also goes through environment expansion at sign-in, which has no
/// escape for `%`. Each `%` therefore stands outside the quotes, between a closing and an opening
/// one, which the argument rules drop: any `%NAME%` the expansion could see then has a quote at
/// each end of NAME, and no variable is named that way. The line never holds `""` inside quotes,
/// which the parsers read differently.
#[cfg(any(windows, test))]
fn windows_argument(what: &str, arg: &str) -> Result<String, Unrepresentable> {
    refuse_nul(what, arg)?;
    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => {
                backslashes += 1;
                continue;
            }
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
            }
            '%' => {
                quoted.push_str(&"\\".repeat(backslashes * 2));
                quoted.push_str("\"%\"");
            }
            c => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push(c);
            }
        }
        backslashes = 0;
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    Ok(quoted)
}

/// The arguments after the program that start the Windows supervisor, `beans service run`,
/// with `environment`: the same text whether sign-in reads it from the Run key or `beans service
/// install` starts the supervisor at once.
#[cfg(any(windows, test))]
fn windows_arguments(environment: &[(String, String)]) -> Result<String, Unrepresentable> {
    let mut line = String::from("service run");
    for (name, value) in environment {
        line.push_str(" --env ");
        line.push_str(&windows_argument(name, &format!("{name}={value}"))?);
    }
    Ok(line)
}

/// The Run key's command line: `exe` and [`windows_arguments`]. The program name is read up to
/// the next quote, without escapes, so it cannot hold a quote, nor two `%`, which the sign-in
/// expansion could read as a variable.
#[cfg(any(windows, test))]
fn windows_command_line(exe: &str, environment: &[(String, String)]) -> Result<String, Unrepresentable> {
    let what = "the path of beans";
    refuse_nul(what, exe)?;
    if exe.contains('"') {
        return Err(Unrepresentable { what: what.to_string(), character: '"', reason: "a Windows program name cannot hold".into() });
    }
    if exe.matches('%').count() > 1 {
        return Err(Unrepresentable { what: what.to_string(), character: '%', reason: "a Run key program name cannot hold twice without it reading as a variable at sign-in".into() });
    }
    Ok(format!("\"{exe}\" {}", windows_arguments(environment)?))
}

// MARK: - Shared

/// What `beans service uninstall` did.
pub struct Uninstalled {
    /// Whether a service, or anything left of one, was there to remove.
    pub removed: bool,
    pub notes: Vec<String>,
}

/// What `beans service refresh` did.
pub struct Refreshed {
    /// Whether the service was installed, so there was anything to refresh.
    pub installed: bool,
    /// Whether anything changed.
    pub changed: bool,
    pub notes: Vec<String>,
}

/// How long a read-only question to the service manager may take.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const QUERY_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Held while a service command changes anything; see [`process::lock_first`].
fn lock() -> anyhow::Result<process::ServiceLock> {
    Ok(process::lock_first(&process::lock_paths(), std::time::Duration::from_secs(90))?)
}

fn words(notes: &[plan::Note]) -> Vec<String> {
    notes.iter().map(ToString::to_string).collect()
}

/// `refresh`'s plan for `found`, from `program` (this `beans`), unless this `beans` runs from
/// somewhere that goes away.
fn refresh_plan(found: &plan::Found, program: &str, fallback: Option<plan::Definition>) -> plan::Plan {
    match transient_location(program, &std::env::temp_dir().to_string_lossy()) {
        Some(reason) => plan::refresh_from_transient(found, program, reason, crate::config::VERSION),
        None => plan::refresh(found, program, fallback, crate::config::VERSION),
    }
}

/// What `refresh` reports, from what it found, planned and did.
fn refreshed(found: &plan::Found, plan: &plan::Plan, done: Vec<plan::Note>) -> Refreshed {
    Refreshed {
        installed: found.stored != plan::Stored::Absent,
        changed: !plan.actions.is_empty(),
        notes: words(&[plan.notes.clone(), done].concat()),
    }
}

/// `status`'s answer from what was found, or from why it could not be read.
fn described(found: anyhow::Result<plan::Found>, log: String) -> Status {
    match (found, exe()) {
        (Ok(found), Ok(exe)) => Status {
            installed: found.stored != plan::Stored::Absent,
            running: found.running,
            notes: words(&plan::findings(&found, &exe.to_string_lossy(), crate::config::VERSION)),
            log,
        },
        (Err(error), _) | (_, Err(error)) => Status { installed: false, running: None, log, notes: vec![format!("Could not read the service: {error:#}")] },
    }
}

/// Writes `bytes` to `path` whole: a temporary file beside it, renamed over it, so the service
/// manager never reads half a definition and a failed write leaves the previous one.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn write_whole(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let folder = path.parent().ok_or_else(|| anyhow::anyhow!("{} has no folder", path.display()))?;
    std::fs::create_dir_all(folder)?;
    let mut file = tempfile::NamedTempFile::new_in(folder)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

/// The file `path` holds, as the platform's reader finds it; absent when there is none.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn stored_file(path: &Path, read: impl Fn(&str) -> plan::Stored) -> anyhow::Result<plan::Stored> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(match String::from_utf8(bytes) {
            Ok(text) => read(&text),
            Err(error) => plan::Stored::Edited { text: String::from_utf8_lossy(error.as_bytes()).into_owned() },
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(plan::Stored::Absent),
        Err(error) => Err(error.into()),
    }
}

/// The ports a service's `beans serve` may hold: the one its definition names, then this
/// command's.
fn service_ports(config: &Config, stored: &plan::Stored) -> Vec<u16> {
    let named = plan::served_environment_of(stored).iter().find(|(name, _)| name == "BEANS_PORT").and_then(|(_, port)| port.parse().ok());
    let mut ports: Vec<u16> = named.into_iter().chain([config.port]).collect();
    ports.dedup();
    ports
}

/// What every platform finds the same way, around what its service manager says: whether the
/// definition's program is on disk and, when it is another `beans` than this one, its version;
/// and the `beans serve` answering on the service's port.
fn found_with(config: &Config, stored: plan::Stored, active: bool, running: Option<u32>) -> plan::Found {
    let program = match &stored {
        plan::Stored::Beans { definition, .. } => Some(definition.program.clone()),
        _ => None,
    };
    let program_exists = program.as_deref().is_some_and(|program| Path::new(program).is_file());
    let this = exe().ok().map(|exe| exe.to_string_lossy().into_owned());
    let program_version = program.filter(|program| program_exists && Some(program) != this.as_ref()).and_then(|program| program_version(Path::new(&program)));
    let holder = service_ports(config, &stored).into_iter().find_map(probe);
    plan::Found { stored, active, running, program_exists, program_version, holder, overrides: Vec::new() }
}

/// The version `program --version` prints, `beans 0.1.10`; `None` if it does not answer so within
/// two seconds.
fn program_version(program: &Path) -> Option<String> {
    let output = output_within(Command::new(program).arg("--version"), std::time::Duration::from_secs(2))?;
    version_output(&String::from_utf8_lossy(&output.stdout))
}

/// The version in `beans --version`'s output.
fn version_output(output: &str) -> Option<String> {
    let version = output.lines().next()?.trim().strip_prefix("beans ")?.trim();
    plan::compare(version, version).map(|_| version.to_string())
}

// MARK: - macOS

#[cfg(target_os = "macos")]
fn plist_path() -> anyhow::Result<PathBuf> {
    Ok(dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home folder"))?.join("Library/LaunchAgents").join(format!("{NAME}.plist")))
}

#[cfg(target_os = "macos")]
fn log_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("Library/Logs/Beans/serve.log")
}

/// The launchd domains an agent can run in, in the order Beans tries them: the login session's,
/// then the user's own, which exists without a GUI login (a Mac reached only over SSH).
#[cfg(target_os = "macos")]
fn domains() -> [String; 2] {
    let uid = unsafe { libc::getuid() };
    [format!("gui/{uid}"), format!("user/{uid}")]
}

/// Tries `attempt` in each domain in turn and returns the one that worked, or every domain's error.
#[cfg(any(target_os = "macos", test))]
fn first_domain(domains: &[String], mut attempt: impl FnMut(&str) -> anyhow::Result<()>) -> anyhow::Result<String> {
    let mut errors = Vec::new();
    for domain in domains {
        match attempt(domain) {
            Ok(()) => return Ok(domain.clone()),
            Err(error) => errors.push(format!("{domain}: {error:#}")),
        }
    }
    anyhow::bail!("launchd would not start the service in any domain ({})", errors.join("; "))
}

/// launchd's job, if a domain has one loaded under the label: that domain and its process.
#[cfg(target_os = "macos")]
fn launchd_job() -> Option<(String, Option<u32>)> {
    domains().into_iter().find_map(|domain| {
        let output = output_within(Command::new("launchctl").args(["print", &format!("{domain}/{NAME}")]), QUERY_LIMIT)?;
        output.status.success().then(|| (domain, launchd_pid(&String::from_utf8_lossy(&output.stdout))))
    })
}

/// The job's own `pid = …` in `launchctl print`: one level into the job's braces, not a pid in a
/// nested block (a spawned endpoint's, say).
#[cfg(any(target_os = "macos", test))]
fn launchd_pid(print: &str) -> Option<u32> {
    let mut depth = 0usize;
    for line in print.lines() {
        let trimmed = line.trim();
        if depth == 1 {
            if let Some(pid) = trimmed.strip_prefix("pid = ") {
                return pid.trim().parse().ok().filter(|pid| *pid != 0);
            }
        }
        depth += trimmed.matches('{').count();
        depth = depth.saturating_sub(trimmed.matches('}').count());
    }
    None
}

#[cfg(target_os = "macos")]
fn found(config: &Config) -> anyhow::Result<plan::Found> {
    let log = log_path();
    let log = text(&log, "the log")?.to_string();
    let stored = stored_file(&plist_path()?, |text| plan::read_launchd(text, NAME, &log))?;
    let job = launchd_job();
    Ok(found_with(config, stored, job.is_some(), job.and_then(|(_, pid)| pid)))
}

#[cfg(target_os = "macos")]
struct Launchd {
    ports: Vec<u16>,
}

#[cfg(target_os = "macos")]
impl plan::Host for Launchd {
    fn clear_port(&mut self, strict: bool) -> anyhow::Result<Vec<plan::Note>> {
        clear_port(&self.ports, strict)
    }
    fn stop(&mut self) -> anyhow::Result<()> {
        // From whichever domain has it, until none does; a job that will not go is an error.
        for _ in 0..4 {
            let Some((domain, _)) = launchd_job() else { return Ok(()) };
            let result = run(Command::new("launchctl").args(["bootout", &format!("{domain}/{NAME}")]));
            // Already gone is stopped.
            if result.is_err() && launchd_job().is_some_and(|(still, _)| still == domain) {
                return result;
            }
        }
        match launchd_job() {
            Some((domain, _)) => anyhow::bail!("launchd keeps {domain}/{NAME} loaded after bootout"),
            None => Ok(()),
        }
    }
    fn remove(&mut self) -> anyhow::Result<()> {
        match std::fs::remove_file(plist_path()?) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        }
    }
    fn write(&mut self, definition: &plan::Definition) -> anyhow::Result<()> {
        let log = log_path();
        std::fs::create_dir_all(log.parent().unwrap())?;
        let plist = launchd_plist(NAME, &definition.program, text(&log, "the log")?, &definition.environment)?;
        write_whole(&plist_path()?, plist.as_bytes())
    }
    fn restore(&mut self, text: &str) -> anyhow::Result<()> {
        write_whole(&plist_path()?, text.as_bytes())
    }
    fn start(&mut self) -> anyhow::Result<()> {
        // `launchctl disable`, by hand or by another tool, outlives the job and makes bootstrap
        // fail with an unhelpful error until it is enabled again.
        let path = plist_path()?;
        first_domain(&domains(), |domain| {
            let _ = output_within(Command::new("launchctl").args(["enable", &format!("{domain}/{NAME}")]), QUERY_LIMIT);
            run(Command::new("launchctl").args(["bootstrap", domain]).arg(&path))
        })
        .map(|_| ())
    }
}

#[cfg(target_os = "macos")]
pub fn install(config: &Config) -> anyhow::Result<Status> {
    let _lock = lock()?;
    let exe = exe()?;
    // The whole definition is made before anything changes, so a value it cannot hold leaves the
    // service as it was.
    let wanted = plan::Definition { program: text(&exe, "beans")?.to_string(), environment: passed_on(config)? };
    permanent(&wanted.program)?;
    launchd_plist(NAME, &wanted.program, text(&log_path(), "the log")?, &wanted.environment)?;
    let found = found(config)?;
    let plan = plan::install(&found, wanted, crate::config::VERSION)?;
    let done = plan::apply(&plan, &found, &mut Launchd { ports: vec![config.port] })?;
    Ok(Status { notes: words(&[plan.notes, done].concat()), ..status(config) })
}

#[cfg(target_os = "macos")]
pub fn refresh(config: &Config) -> anyhow::Result<Refreshed> {
    let _lock = lock()?;
    let exe = exe()?;
    let found = found(config)?;
    let program = text(&exe, "beans")?;
    let fallback = passed_on(config).ok().map(|environment| plan::Definition { program: program.to_string(), environment });
    let plan = refresh_plan(&found, program, fallback);
    let done = plan::apply(&plan, &found, &mut Launchd { ports: service_ports(config, &found.stored) })?;
    Ok(refreshed(&found, &plan, done))
}

#[cfg(target_os = "macos")]
pub fn uninstall(config: &Config) -> anyhow::Result<Uninstalled> {
    let _lock = lock()?;
    let found = found(config)?;
    let plan = plan::uninstall(&found);
    plan::apply(&plan, &found, &mut Launchd { ports: Vec::new() })?;
    Ok(Uninstalled { removed: !plan.actions.is_empty(), notes: words(&plan.notes) })
}

#[cfg(target_os = "macos")]
pub fn status(config: &Config) -> Status {
    described(found(config), log_path().display().to_string())
}

// MARK: - Linux

#[cfg(target_os = "linux")]
fn unit_path() -> anyhow::Result<PathBuf> {
    Ok(dirs::config_dir().ok_or_else(|| anyhow::anyhow!("no configuration folder"))?.join("systemd/user").join(UNIT))
}

#[cfg(target_os = "linux")]
fn systemctl(args: &[&str]) -> Command {
    let mut command = Command::new("systemctl");
    command.arg("--user").args(args);
    command
}

/// What the systemd user manager says of the unit, from `systemctl --user show`.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Default, PartialEq, Eq)]
struct SystemdUnit {
    /// The file systemd loaded it from, empty when it has none.
    fragment: String,
    /// Drop-ins that change it (`beans.service.d/*.conf`, as `systemctl edit` writes).
    drop_ins: Vec<String>,
    load: String,
    active: String,
    enabled: String,
    pid: Option<u32>,
}

#[cfg(any(target_os = "linux", test))]
fn read_systemd_show(output: &str) -> SystemdUnit {
    let mut unit = SystemdUnit::default();
    for line in output.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        match key {
            "FragmentPath" => unit.fragment = value.to_string(),
            "DropInPaths" => unit.drop_ins = value.split_whitespace().map(str::to_string).collect(),
            "LoadState" => unit.load = value.to_string(),
            "ActiveState" => unit.active = value.to_string(),
            "UnitFileState" => unit.enabled = value.to_string(),
            "MainPID" => unit.pid = value.parse().ok().filter(|pid| *pid != 0),
            _ => {}
        }
    }
    unit
}

/// What the unit file and the user manager together say: a unit systemd loads from a file other
/// than Beans's (one a package or an administrator installed) is not Beans's to change.
/// What the unit file and the user manager together say, as (stored, active, running). A unit
/// systemd loads from another file (one a package or an administrator installed) is `Elsewhere`,
/// with its definition when `other` reads that file as a Beans one.
#[cfg(any(target_os = "linux", test))]
fn systemd_found(stored: plan::Stored, ours: &str, unit: &SystemdUnit, other: impl Fn(&str) -> Option<plan::Definition>) -> (plan::Stored, bool, Option<u32>) {
    let stored = match stored {
        plan::Stored::Absent if !unit.fragment.is_empty() && unit.fragment != ours => {
            plan::Stored::Elsewhere { path: unit.fragment.clone(), definition: other(&unit.fragment) }
        }
        stored => stored,
    };
    let active = matches!(unit.enabled.as_str(), "enabled" | "enabled-runtime" | "linked" | "linked-runtime")
        || matches!(unit.active.as_str(), "active" | "activating" | "reloading" | "deactivating");
    (stored, active, unit.pid)
}

#[cfg(target_os = "linux")]
fn systemd_show() -> SystemdUnit {
    let output = output_within(&mut systemctl(&["show", UNIT, "--property=FragmentPath,DropInPaths,LoadState,ActiveState,UnitFileState,MainPID"]), QUERY_LIMIT);
    output.filter(|o| o.status.success()).map(|o| read_systemd_show(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn found(config: &Config) -> anyhow::Result<plan::Found> {
    let path = unit_path()?;
    let stored = stored_file(&path, plan::read_systemd)?;
    let other = |path: &str| match std::fs::read_to_string(path).map(|text| plan::read_systemd(&text)) {
        Ok(plan::Stored::Beans { definition, .. }) => Some(definition),
        _ => None,
    };
    let unit = systemd_show();
    let (stored, active, running) = systemd_found(stored, text(&path, "the unit")?, &unit, other);
    Ok(plan::Found { overrides: unit.drop_ins, ..found_with(config, stored, active, running) })
}

#[cfg(target_os = "linux")]
struct Systemd {
    release: Option<u32>,
    ports: Vec<u16>,
}

#[cfg(target_os = "linux")]
impl plan::Host for Systemd {
    fn clear_port(&mut self, strict: bool) -> anyhow::Result<Vec<plan::Note>> {
        clear_port(&self.ports, strict)
    }
    fn stop(&mut self) -> anyhow::Result<()> {
        run(&mut systemctl(&["stop", UNIT]))?;
        // A unit whose file is gone cannot be disabled, and one that never failed has nothing to reset.
        let _ = systemctl(&["disable", UNIT]).output();
        let _ = systemctl(&["reset-failed", UNIT]).output();
        Ok(())
    }
    fn remove(&mut self) -> anyhow::Result<()> {
        match std::fs::remove_file(unit_path()?) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        // Without a user session there is nothing loaded to forget.
        let _ = systemctl(&["daemon-reload"]).output();
        Ok(())
    }
    fn write(&mut self, definition: &plan::Definition) -> anyhow::Result<()> {
        let unit = systemd_unit(&definition.program, &definition.environment, self.release)?;
        write_whole(&unit_path()?, unit.as_bytes())
    }
    fn restore(&mut self, text: &str) -> anyhow::Result<()> {
        write_whole(&unit_path()?, text.as_bytes())
    }
    fn start(&mut self) -> anyhow::Result<()> {
        run(&mut systemctl(&["daemon-reload"]))?;
        // Beans's unit takes precedence over one of the same name in /etc or /usr; one in a
        // place that outranks it would still win, and the service would not be Beans's.
        let ours = unit_path()?;
        let fragment = systemd_show().fragment;
        if ours.exists() && fragment != ours.to_string_lossy() {
            anyhow::bail!("systemd loads {UNIT} from {fragment}, which takes precedence over beans's {}", ours.display());
        }
        run(&mut systemctl(&["enable", UNIT]))?;
        run(&mut systemctl(&["restart", UNIT]))
    }
}

#[cfg(target_os = "linux")]
fn systemd_release_here() -> Option<u32> {
    output_within(Command::new("systemctl").arg("--version"), QUERY_LIMIT).and_then(|o| systemd_release(&String::from_utf8_lossy(&o.stdout)))
}

#[cfg(target_os = "linux")]
pub fn install(config: &Config) -> anyhow::Result<Status> {
    let _lock = lock()?;
    if !output_within(&mut systemctl(&["show-environment"]), QUERY_LIMIT).is_some_and(|o| o.status.success()) {
        anyhow::bail!(
            "This computer has no systemd user session (a container, or WSL without systemd), so beans service cannot keep beans serve running. Start `beans serve` from your own init system, or in tmux."
        );
    }
    let exe = exe()?;
    let release = systemd_release_here();
    let wanted = plan::Definition { program: text(&exe, "beans")?.to_string(), environment: passed_on(config)? };
    permanent(&wanted.program)?;
    systemd_unit(&wanted.program, &wanted.environment, release)?;
    let found = found(config)?;
    let plan = plan::install(&found, wanted, crate::config::VERSION)?;
    let done = plan::apply(&plan, &found, &mut Systemd { release, ports: vec![config.port] })?;
    // Without lingering, the user's services stop at logout and start only at the next login.
    let user = std::env::var("USER").unwrap_or_default();
    let lingers = || Command::new("loginctl").args(["show-user", &user, "-p", "Linger", "--value"]).output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "yes");
    if !user.is_empty() && !lingers() && Command::new("loginctl").args(["enable-linger", &user]).output().map_or(true, |o| !o.status.success()) {
        eprintln!("beans serve runs while you are logged in. To keep it running after you log out, and to start it at boot, run: sudo loginctl enable-linger {user}");
    }
    Ok(Status { notes: words(&[plan.notes, done].concat()), ..status(config) })
}

#[cfg(target_os = "linux")]
pub fn refresh(config: &Config) -> anyhow::Result<Refreshed> {
    let _lock = lock()?;
    let exe = exe()?;
    let found = found(config)?;
    let program = text(&exe, "beans")?;
    let fallback = passed_on(config).ok().map(|environment| plan::Definition { program: program.to_string(), environment });
    let plan = refresh_plan(&found, program, fallback);
    let done = plan::apply(&plan, &found, &mut Systemd { release: systemd_release_here(), ports: service_ports(config, &found.stored) })?;
    Ok(refreshed(&found, &plan, done))
}

#[cfg(target_os = "linux")]
pub fn uninstall(config: &Config) -> anyhow::Result<Uninstalled> {
    let _lock = lock()?;
    let found = found(config)?;
    let plan = plan::uninstall(&found);
    plan::apply(&plan, &found, &mut Systemd { release: None, ports: Vec::new() })?;
    Ok(Uninstalled { removed: !plan.actions.is_empty(), notes: words(&plan.notes) })
}

#[cfg(target_os = "linux")]
pub fn status(config: &Config) -> Status {
    described(found(config), format!("journalctl --user -u {UNIT}"))
}

// MARK: - Windows

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_VALUE: &str = "Beans";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const DETACHED_PROCESS: u32 = 0x0000_0008;

#[cfg(windows)]
fn pid_path(config: &Config) -> PathBuf {
    config.home.join("service.pid")
}

#[cfg(windows)]
fn log_path(config: &Config) -> PathBuf {
    config.home.join("serve.log")
}

/// The Run key's `Beans` value, read with the registry API so that no command-line tool's
/// parsing stands between it and Beans.
#[cfg(windows)]
fn run_key_value() -> anyhow::Result<Option<String>> {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (key, value) = (wide(RUN_KEY), wide(RUN_VALUE));
    let mut size = 0u32;
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS {
        // Present, but not a plain string: nothing Beans writes.
        return Ok(Some(String::new()));
    }
    let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut size) };
    anyhow::ensure!(status == ERROR_SUCCESS, "Cannot read the Run key's Beans value (error {status})");
    let length = buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len());
    Ok(Some(String::from_utf16_lossy(&buffer[..length])))
}

#[cfg(windows)]
fn reg(args: &[&str]) -> anyhow::Result<()> {
    let key = format!(r"HKCU\{RUN_KEY}");
    let mut command = Command::new("reg");
    command.arg(args[0]).arg(&key).args(&args[1..]);
    run(&mut command)
}

/// The home and port a definition's supervisor serves: its `BEANS_HOME` and `BEANS_PORT`, which
/// every Run key Beans writes names, or this command's.
#[cfg(windows)]
fn served_config(config: &Config, environment: &[(String, String)]) -> Config {
    let value = |name: &str| environment.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone());
    Config {
        home: value("BEANS_HOME").map(PathBuf::from).unwrap_or_else(|| config.home.clone()),
        port: value("BEANS_PORT").and_then(|port| port.parse().ok()).unwrap_or(config.port),
    }
}

/// The programs the Run key names, which a running supervisor may be (or have been moved aside from).
#[cfg(windows)]
fn installed_programs(stored: &plan::Stored) -> Vec<PathBuf> {
    match stored {
        plan::Stored::Beans { definition, .. } => vec![PathBuf::from(&definition.program)],
        _ => Vec::new(),
    }
}

#[cfg(windows)]
fn found(config: &Config) -> anyhow::Result<(plan::Found, Config)> {
    let stored = match run_key_value()? {
        None => plan::Stored::Absent,
        Some(line) => plan::read_run_key(&line),
    };
    let served = served_config(config, plan::served_environment_of(&stored));
    // A home that is gone has no supervisor; an incompatible one stops here, untouched.
    let running = supervisor::bound(&served, &installed_programs(&stored))?.map(|(record, _)| record.pid);
    Ok((found_with(config, stored, running.is_some(), running), served))
}

#[cfg(windows)]
struct RunKey {
    /// The home whose supervisor `stop` stops: the one the stored definition serves.
    served: Config,
    ports: Vec<u16>,
    installed: Vec<PathBuf>,
    /// What `start` starts: the definition last written or restored.
    definition: Option<plan::Definition>,
}

#[cfg(windows)]
impl plan::Host for RunKey {
    fn stop(&mut self) -> anyhow::Result<()> {
        stop(&self.served, &self.installed)
    }
    fn clear_port(&mut self, strict: bool) -> anyhow::Result<Vec<plan::Note>> {
        clear_port(&self.ports, strict)
    }
    fn remove(&mut self) -> anyhow::Result<()> {
        if run_key_value()?.is_some() {
            reg(&["delete", "/v", RUN_VALUE, "/f"])?;
        }
        Ok(())
    }
    fn write(&mut self, definition: &plan::Definition) -> anyhow::Result<()> {
        let line = windows_command_line(&definition.program, &definition.environment)?;
        reg(&["add", "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &line, "/f"])?;
        self.definition = Some(definition.clone());
        Ok(())
    }
    fn restore(&mut self, text: &str) -> anyhow::Result<()> {
        reg(&["add", "/v", RUN_VALUE, "/t", "REG_SZ", "/d", text, "/f"])?;
        self.definition = match plan::read_run_key(text) {
            plan::Stored::Beans { definition, .. } => Some(definition),
            _ => None,
        };
        Ok(())
    }
    fn start(&mut self) -> anyhow::Result<()> {
        use std::os::windows::process::CommandExt;
        let definition = self.definition.clone().ok_or_else(|| anyhow::anyhow!("No supervisor to start"))?;
        // Started now with the Run key's own arguments, so it runs as a sign-in would start it.
        let mut command = Command::new(&definition.program);
        command.raw_arg(windows_arguments(&definition.environment)?).creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
        command.spawn()?;
        // The supervisor writes its pid once it runs.
        let config = served_config(&self.served, &definition.environment);
        let installed = vec![PathBuf::from(&definition.program)];
        for _ in 0..50 {
            if supervisor::bound(&config, &installed).ok().flatten().is_some() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        anyhow::bail!("The supervisor did not start within 5 seconds; see {}", log_path(&config).display())
    }
}

#[cfg(windows)]
pub fn install(config: &Config) -> anyhow::Result<Status> {
    let _lock = lock()?;
    config.ensure_home()?;
    let exe = exe()?;
    // The Run key holds a command line. The supervisor names the home and port it serves, since
    // a sign-in starts it without the installing shell's variables.
    let environment = supervisor_environment(config, &passed_on(config)?)?;
    let wanted = plan::Definition { program: text(&exe, "beans")?.to_string(), environment };
    permanent(&wanted.program)?;
    windows_command_line(&wanted.program, &wanted.environment)?;
    let (found, served) = found(config)?;
    let plan = plan::install(&found, wanted, crate::config::VERSION)?;
    let done = plan::apply(&plan, &found, &mut RunKey { served, ports: vec![config.port], installed: installed_programs(&found.stored), definition: None })?;
    Ok(Status { notes: words(&[plan.notes, done].concat()), ..status(config) })
}

#[cfg(windows)]
pub fn refresh(config: &Config) -> anyhow::Result<Refreshed> {
    let _lock = lock()?;
    let exe = exe()?;
    let (found, served) = found(config)?;
    let program = text(&exe, "beans")?;
    // A Run key Beans did not write is replaced with this command's home and port.
    let fallback = passed_on(config).and_then(|environment| supervisor_environment(config, &environment)).ok()
        .map(|environment| plan::Definition { program: program.to_string(), environment });
    let plan = refresh_plan(&found, program, fallback);
    let ports = service_ports(config, &found.stored);
    let done = plan::apply(&plan, &found, &mut RunKey { served, ports, installed: installed_programs(&found.stored), definition: None })?;
    Ok(refreshed(&found, &plan, done))
}

#[cfg(windows)]
pub fn uninstall(config: &Config) -> anyhow::Result<Uninstalled> {
    let _lock = lock()?;
    let (found, served) = found(config)?;
    let plan = plan::uninstall(&found);
    plan::apply(&plan, &found, &mut RunKey { served: served.clone(), ports: Vec::new(), installed: installed_programs(&found.stored), definition: None })?;
    // What is left of a supervisor that stopped on its own is its record, whose process is gone
    // (a live one was proven and stopped above).
    if found.running.is_none() && supervisor::read_record(&served).ok().flatten().is_some() {
        let _ = std::fs::remove_file(pid_path(&served));
    }
    Ok(Uninstalled { removed: !plan.actions.is_empty(), notes: words(&plan.notes) })
}

/// Stops the supervisor and the `beans serve` it started.
#[cfg(windows)]
fn stop(config: &Config, installed: &[PathBuf]) -> anyhow::Result<()> {
    if let Some((record, handle)) = supervisor::bound(config, installed)? {
        // The held process handle cannot turn into a reused PID. Closing this supervisor's
        // job handle on exit stops only its own child tree.
        handle.terminate()?;
        if supervisor::read_record(config)?.as_ref() == Some(&record) {
            std::fs::remove_file(pid_path(config))?;
        }
    }
    Ok(())
}

#[cfg(windows)]
pub fn status(config: &Config) -> Status {
    match found(config) {
        Ok((found, served)) => described(Ok(found), log_path(&served).display().to_string()),
        Err(error) => described(Err(error), log_path(config).display().to_string()),
    }
}

#[cfg(windows)]
fn supervisor_environment(config: &Config, environment: &[(String, String)]) -> anyhow::Result<Vec<(String, String)>> {
    let mut environment: Vec<_> = environment.iter()
        .filter(|(name, _)| !name.eq_ignore_ascii_case("BEANS_HOME") && !name.eq_ignore_ascii_case("BEANS_PORT"))
        .cloned().collect();
    environment.push(("BEANS_HOME".into(), text(&std::fs::canonicalize(&config.home)?, "BEANS_HOME")?.to_string()));
    environment.push(("BEANS_PORT".into(), config.port.to_string()));
    Ok(environment)
}

/// `beans service run`, the supervisor on Windows: leaves the console a sign-in opened for it,
/// then runs `beans serve` from the installed binary until it is stopped, at once again after an
/// update ([`crate::update::RESTART_EXIT`]), and after a pause that grows while it keeps stopping
/// right after it starts. The log goes to `serve.log` in Beans's folder, which starts over past
/// [`LOG_LIMIT`].
#[cfg(windows)]
pub fn supervise(config: &Config, env: &[(String, String)]) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    // A supervisor running from a program the Run key names, or one the installer moved aside,
    // is this home's supervisor as much as one running from this binary.
    let installed = run_key_value().ok().flatten().map(|line| installed_programs(&plan::read_run_key(&line))).unwrap_or_default();
    if supervisor::bound(config, &installed).ok().flatten().is_some_and(|(record, _)| record.pid != std::process::id()) {
        return Ok(());
    }
    // Admission also precedes console detachment, PID replacement and log creation.
    config.validate_home()?;
    supervisor::bound(config, &installed)?;
    config.ensure_home()?;
    let exe = exe()?;
    let home = std::fs::canonicalize(&config.home)?;
    let env = supervisor_environment(config, env)?;
    let process = supervisor::Handle::open(std::process::id())?.ok_or_else(|| anyhow::anyhow!("Supervisor is not running"))?;
    let record = supervisor::Record { format: crate::config::Format::BeansV2, pid: std::process::id(), created_at: process.created_at()?, home };
    if !supervisor::matches(&record, record.created_at, &exe, &[exe.as_path()], &record.home, &std::env::args().collect::<Vec<_>>()) {
        // Manual invocation is normalized to the same explicit home-bound command as the
        // Run key. Never rely on an inherited environment to identify another process.
        let mut command = Command::new(&exe);
        command.raw_arg(windows_arguments(&env)?).creation_flags(CREATE_NO_WINDOW);
        run(&mut command)?;
        return Ok(());
    }
    unsafe { windows_sys::Win32::System::Console::FreeConsole() };
    let _job = supervisor::own_process_tree()?;
    crate::config::write_json_private(&pid_path(config), &record)?;
    let mut pause = std::time::Duration::from_secs(2);
    loop {
        let log = log_path(config);
        if std::fs::metadata(&log).is_ok_and(|m| m.len() > LOG_LIMIT) {
            let _ = std::fs::rename(&log, log.with_extension("log.1"));
        }
        let output = std::fs::OpenOptions::new().create(true).append(true).open(&log)?;
        let started = std::time::Instant::now();
        let mut command = Command::new(&exe);
        command.arg("serve").env("BEANS_SERVICE", "1").stdin(std::process::Stdio::null()).stdout(output.try_clone()?).stderr(output).creation_flags(CREATE_NO_WINDOW);
        for (name, value) in &env {
            command.env(name, value);
        }
        let code = match command.spawn().and_then(|mut child| child.wait()) {
            Ok(status) => status.code(),
            Err(_) => None,
        };
        if code == Some(crate::update::RESTART_EXIT) {
            continue;
        }
        pause = if started.elapsed() > std::time::Duration::from_secs(60) { std::time::Duration::from_secs(2) } else { (pause * 2).min(std::time::Duration::from_secs(60)) };
        std::thread::sleep(pause);
    }
}

// MARK: - Elsewhere

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn install(_config: &Config) -> anyhow::Result<Status> {
    anyhow::bail!("beans service runs on macOS, Linux, and Windows; start `beans serve` from this system's own init.")
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn refresh(_config: &Config) -> anyhow::Result<Refreshed> {
    Ok(Refreshed { installed: false, changed: false, notes: Vec::new() })
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn uninstall(_config: &Config) -> anyhow::Result<Uninstalled> {
    Ok(Uninstalled { removed: false, notes: Vec::new() })
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn status(_config: &Config) -> Status {
    Status { installed: false, running: None, log: String::new(), notes: Vec::new() }
}

fn run(command: &mut Command) -> anyhow::Result<()> {
    let output = command.output()?;
    if output.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let program = command.get_program().to_string_lossy().to_string();
    anyhow::bail!("{program} failed{}", if text.is_empty() { String::new() } else { format!(": {text}") })
}

#[cfg(test)]
mod tests {
    //! Every platform's definition, generated and read back on any platform. The readers below
    //! model each service manager's documented parsing; nothing here installs, starts, or asks a
    //! service manager anything.

    use super::*;

    /// Values that have broken service definitions: spaces, both quotes, backslashes (also last,
    /// before a closing quote), percent signs (also in `%NAME%` form), dollar signs (also in
    /// `$NAME` and `${NAME}` form), newlines, tabs, carriage returns, markup, and non-ASCII.
    const HARD: &[&str] = &[
        "",
        "plain",
        "with  two spaces ",
        " leading space",
        "double \" quote",
        "\"quoted\"",
        "single ' quote",
        r"C:\Users\me\",
        r"C:\Users\me\\",
        r#"back\"slash quote\\""#,
        "100% sure",
        "%PATH%",
        "%%",
        "%",
        "50%\\",
        "$HOME and ${HOME} and $$",
        "line one\nline two\n",
        "\n",
        "tab\there",
        "crlf\r\nand lone\rcr",
        "<tag attr=\"1\">&amp; ]]> &#65;",
        "https://relay.example/path?token=a%20b&x=$y#frag",
        "ünïcödé 🫘 日本",
        "trailing backslash\\",
    ];

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect()
    }

    fn hard_environment(value: &str) -> Vec<(String, String)> {
        env(&[("BEANS_HOME", value), ("BEANS_PORT", "4999"), ("BEANS_RELAY_URL", value)])
    }

    // MARK: Exactly what is written
    //
    // Every installed definition is compared byte for byte with what this version writes; any
    // difference makes `refresh` (at every launch of the app) rewrite and restart the service. A
    // change to these texts must be deliberate, and these tests say so when one is not.

    #[test]
    fn the_plist_is_written_exactly_so() {
        let environment = env(&[("BEANS_HOME", "/Users/me/Beans \"home\" 100%"), ("BEANS_PORT", "4999"), ("BEANS_RELAY_URL", "https://relay.example/?a=1&b=$x")]);
        let written = launchd_plist("app.beans.serve", "/Users/me/.local/bin/beans", "/Users/me/Library/Logs/Beans/serve.log", &environment).unwrap();
        assert_eq!(written, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>Label</key>\n\t<string>app.beans.serve</string>\n\t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>/Users/me/.local/bin/beans</string>\n\t\t<string>serve</string>\n\t</array>\n\t<key>EnvironmentVariables</key>\n\t<dict>\n\t\t<key>BEANS_SERVICE</key>\n\t\t<string>1</string>\n\t\t<key>BEANS_HOME</key>\n\t\t<string>/Users/me/Beans \"home\" 100%</string>\n\t\t<key>BEANS_PORT</key>\n\t\t<string>4999</string>\n\t\t<key>BEANS_RELAY_URL</key>\n\t\t<string>https://relay.example/?a=1&amp;b=$x</string>\n\t</dict>\n\t<key>RunAtLoad</key>\n\t<true/>\n\t<key>KeepAlive</key>\n\t<true/>\n\t<key>StandardOutPath</key>\n\t<string>/Users/me/Library/Logs/Beans/serve.log</string>\n\t<key>StandardErrorPath</key>\n\t<string>/Users/me/Library/Logs/Beans/serve.log</string>\n</dict>\n</plist>\n");
    }

    #[test]
    fn the_unit_is_written_exactly_so() {
        let environment = env(&[("BEANS_HOME", "/Users/me/Beans \"home\" 100%"), ("BEANS_PORT", "4999"), ("BEANS_RELAY_URL", "https://relay.example/?a=1&b=$x")]);
        let expected = "[Unit]\nDescription=Beans: this computer's Runner (beans serve)\n\n[Service]\nExecStart=\"/home/me/.local/bin/beans\" serve\nEnvironment=BEANS_SERVICE=1\nEnvironment=\"BEANS_HOME=/Users/me/Beans \\\"home\\\" 100%%\"\nEnvironment=\"BEANS_PORT=4999\"\nEnvironment=\"BEANS_RELAY_URL=https://relay.example/?a=1&b=$x\"\nRestart=always\nRestartSec=2\n\n[Install]\nWantedBy=default.target\n";
        assert_eq!(systemd_unit("/home/me/.local/bin/beans", &environment, Some(257)).unwrap(), expected);
        // The release decides only what is refused, never how a kept value is written.
        for release in [Some(237), Some(245), Some(300), None] {
            assert_eq!(systemd_unit("/home/me/.local/bin/beans", &environment, release).unwrap(), expected, "{release:?}");
        }
    }

    #[test]
    fn the_run_key_is_written_exactly_so() {
        let environment = env(&[("BEANS_HOME", r"C:\Users\me\.beans-v2"), ("BEANS_PORT", "4874"), ("BEANS_RELAY_URL", "https://relay.example/?a=1&b=%x%")]);
        assert_eq!(windows_command_line(r"C:\Users\me\.local\bin\beans.exe", &environment).unwrap(), "\"C:\\Users\\me\\.local\\bin\\beans.exe\" service run --env \"BEANS_HOME=C:\\Users\\me\\.beans-v2\" --env \"BEANS_PORT=4874\" --env \"BEANS_RELAY_URL=https://relay.example/?a=1&b=\"%\"x\"%\"\"");
    }

    // MARK: Inputs

    /// Where a test's installing command runs.
    fn cwd() -> PathBuf {
        PathBuf::from(if cfg!(windows) { r"C:\work" } else { "/work" })
    }

    #[test]
    fn passed_on_values_are_kept_exactly() {
        let default = Path::new("/home/me/.beans-v2");
        let config = Config { home: default.into(), port: crate::config::DEFAULT_PORT };
        assert!(environment(&config, Some(default), None, &cwd()).unwrap().is_empty());
        assert!(environment(&config, Some(default), Some(" \t".into()), &cwd()).unwrap().is_empty(), "a blank relay is no relay");
        for value in HARD.iter().filter(|value| !value.trim().is_empty()) {
            let config = Config { home: PathBuf::from(value), port: 4999 };
            let home = absolute_home(Path::new(value), &cwd());
            let expected = env(&[("BEANS_HOME", home.to_str().unwrap()), ("BEANS_PORT", "4999"), ("BEANS_RELAY_URL", value)]);
            assert_eq!(environment(&config, Some(default), Some(value.to_string()), &cwd()).unwrap(), expected, "{value:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_relative_home_is_the_folder_the_installing_command_meant() {
        let home = |path: &str| absolute_home(Path::new(path), Path::new("/work")).to_string_lossy().into_owned();
        assert_eq!(home("acct"), "/work/acct");
        assert_eq!(home("./acct"), "/work/acct");
        assert_eq!(home("~/.beans-v2"), "/work/~/.beans-v2", "a quoted ~ was never expanded: beans serve reads it so too");
        assert_eq!(home("../shared/acct"), "/work/../shared/acct", ".. stays: a symlink decides what it means");
        assert_eq!(home("/srv/./beans/"), "/srv/beans");
        assert_eq!(home("/srv/beans"), "/srv/beans");
        // The default home written another way is still the default: nothing to pass on.
        let config = Config { home: "/home/me/./.beans-v2".into(), port: crate::config::DEFAULT_PORT };
        assert!(environment(&config, Some(Path::new("/home/me/.beans-v2")), None, Path::new("/work")).unwrap().is_empty());
        let config = Config { home: ".beans-v2".into(), port: crate::config::DEFAULT_PORT };
        assert!(environment(&config, Some(Path::new("/home/me/.beans-v2")), None, Path::new("/home/me")).unwrap().is_empty(), "run from the home folder");
    }

    #[test]
    fn a_binary_replaced_while_it_runs_is_no_program_for_a_service() {
        assert_eq!(replaced_while_running("/home/me/.local/bin/beans (deleted)"), cfg!(target_os = "linux"));
        assert!(!replaced_while_running("/home/me/.local/bin/beans"));
        assert!(!replaced_while_running("/home/me/(deleted)/beans"));
    }

    #[test]
    fn a_beans_running_from_somewhere_that_goes_away_is_no_program_for_a_service() {
        let temp = if cfg!(windows) { r"C:\Users\me\AppData\Local\Temp\" } else { "/tmp/" };
        for program in [
            "/private/var/folders/xy/T/AppTranslocation/0123-4567/d/Beans.app/Contents/Resources/bin/beans",
            "/tmp/.mount_BeansAbC123/usr/bin/beans",
            "/home/me/Downloads/.mount_Beans/beans",
        ] {
            assert!(transient_location(program, temp).is_some(), "{program}");
        }
        let in_temp = if cfg!(windows) { r"c:\users\ME\appdata\local\temp\Temp1_beans.zip\beans.exe" } else { "/tmp/beans-unpacked/beans" };
        assert_eq!(transient_location(in_temp, temp), Some("the temporary folder"), "{in_temp}");
        assert_eq!(transient_location("/Volumes/Beans/Beans.app/Contents/Resources/bin/beans", temp).is_some(), cfg!(target_os = "macos"));
        for program in [
            "/Applications/Beans.app/Contents/Resources/bin/beans", "/home/me/.local/bin/beans", r"C:\Users\me\.local\bin\beans.exe",
            "/tmpfiles/beans", "/home/me/tmp/beans", "/opt/apptranslocation-tools/beans",
        ] {
            assert_eq!(transient_location(program, temp), None, "{program}");
        }
        assert_eq!(transient_location("/anything/beans", ""), None, "no temporary folder: nothing is under it");
    }

    #[cfg(unix)]
    #[test]
    fn a_home_that_is_not_unicode_is_refused_not_replaced() {
        use std::os::unix::ffi::OsStrExt;
        let home = PathBuf::from(std::ffi::OsStr::from_bytes(b"/home/me/\xffbeans"));
        let config = Config { home, port: crate::config::DEFAULT_PORT };
        let error = environment(&config, None, None, Path::new("/work")).unwrap_err().to_string();
        assert!(error.contains("BEANS_HOME") && error.contains("not valid Unicode"), "{error}");
    }

    #[test]
    fn nul_is_refused_by_every_definition() {
        let environment = env(&[("BEANS_RELAY_URL", "a\0b")]);
        let refused = |result: Result<String, Unrepresentable>| {
            let error = result.unwrap_err();
            assert_eq!((error.what.as_str(), error.character), ("BEANS_RELAY_URL", '\0'));
        };
        refused(launchd_plist("app.beans.serve", "/beans", "/serve.log", &environment));
        refused(systemd_unit("/beans", &environment, Some(257)));
        refused(windows_command_line(r"C:\beans.exe", &environment));
        refused(windows_arguments(&environment));
    }

    // MARK: launchd

    /// Reads text the way an XML parser does: line ends normalized first, then references.
    fn xml_text(raw: &str) -> String {
        let raw = raw.replace("\r\n", "\n").replace('\r', "\n");
        let mut text = String::new();
        let mut rest = raw.as_str();
        while let Some(start) = rest.find('&') {
            text.push_str(&rest[..start]);
            let end = rest[start..].find(';').expect("unterminated reference") + start;
            let reference = &rest[start + 1..end];
            text.push(match reference {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                _ => {
                    let number = reference.strip_prefix('#').expect("unknown entity");
                    let code = match number.strip_prefix('x') {
                        Some(hex) => u32::from_str_radix(hex, 16).unwrap(),
                        None => number.parse().unwrap(),
                    };
                    let c = char::from_u32(code).unwrap();
                    assert!(xml_char(c), "XML forbids the reference &{reference};");
                    c
                }
            });
            rest = &rest[end + 1..];
        }
        text.push_str(rest);
        text
    }

    /// The contents of every `<tag>` element, in order.
    fn elements<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
        let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
        let mut found = Vec::new();
        let mut rest = xml;
        while let Some(start) = rest.find(&open) {
            let body = &rest[start + open.len()..];
            let end = body.find(&close).expect("unclosed element");
            assert!(!body[..end].contains('<'), "markup inside <{tag}>: {:?}", &body[..end]);
            found.push(&body[..end]);
            rest = &body[end + close.len()..];
        }
        found
    }

    /// Program arguments, environment, and log path, as launchd reads them back.
    fn read_plist(xml: &str) -> (Vec<String>, Vec<(String, String)>, String) {
        assert!(xml.chars().all(xml_char), "a character XML cannot hold");
        let (_, after) = xml.split_once("<key>ProgramArguments</key>").unwrap();
        let (arguments, after) = after.split_once("</array>").unwrap();
        let arguments = elements(arguments, "string").into_iter().map(xml_text).collect();
        let (_, after) = after.split_once("<key>EnvironmentVariables</key>").unwrap();
        let (variables, after) = after.split_once("</dict>").unwrap();
        let names = elements(variables, "key").into_iter().map(xml_text);
        let environment = names.zip(elements(variables, "string").into_iter().map(xml_text)).collect();
        let log = xml_text(elements(after, "string")[0]);
        (arguments, environment, log)
    }

    #[test]
    fn plist_passes_hard_values_unchanged() {
        for value in HARD {
            let environment = hard_environment(value);
            let exe = format!("/Applications/Be ans{value}/beans");
            let log = format!("/Users/me/Library/Logs/{value}/serve.log");
            let plist = launchd_plist("app.beans.serve", &exe, &log, &environment).unwrap();
            let (arguments, read, read_log) = read_plist(&plist);
            assert_eq!(arguments, [exe.as_str(), "serve"], "{value:?}");
            let mut expected = env(&[("BEANS_SERVICE", "1")]);
            expected.extend(environment);
            assert_eq!(read, expected, "{value:?}");
            assert_eq!(read_log, log, "{value:?}");
        }
    }

    #[test]
    fn plist_carriage_return_is_a_reference_because_xml_reads_a_bare_one_as_a_newline() {
        assert_eq!(xml_text("a\r\nb\rc"), "a\nb\nc", "the XML rule this guards against");
        let plist = launchd_plist("app.beans.serve", "/beans", "/log", &env(&[("BEANS_RELAY_URL", "a\r\nb")])).unwrap();
        assert!(plist.contains("<string>a&#13;\nb</string>"), "{plist}");
        assert!(!plist.contains('\r'));
    }

    #[test]
    fn plist_markup_is_escaped() {
        let plist = launchd_plist("app.beans.serve", "/beans", "/log", &env(&[("BEANS_RELAY_URL", "</string><key>x</key>&")])).unwrap();
        assert!(plist.contains("<string>&lt;/string&gt;&lt;key&gt;x&lt;/key&gt;&amp;</string>"), "{plist}");
    }

    #[test]
    fn plist_refuses_characters_xml_cannot_hold_and_names_only_the_variable() {
        for c in ['\u{1}', '\u{8}', '\u{b}', '\u{c}', '\u{1b}', '\u{1f}', '\u{fffe}', '\u{ffff}'] {
            let value = format!("secret-token{c}");
            let error = launchd_plist("app.beans.serve", "/beans", "/log", &env(&[("BEANS_RELAY_URL", &value)])).unwrap_err();
            assert_eq!(error, Unrepresentable { what: "BEANS_RELAY_URL".into(), character: c, reason: "an XML property list cannot hold".into() });
            let message = error.to_string();
            assert!(message.contains("BEANS_RELAY_URL") && message.contains(&format!("U+{:04X}", c as u32)), "{message}");
            assert!(!message.contains("secret-token"), "the value is not repeated: {message}");
        }
        let error = launchd_plist("app.beans.serve", "/be\u{7}ans", "/log", &[]).unwrap_err();
        assert_eq!((error.what.as_str(), error.character), ("the path of beans", '\u{7}'));
        // DEL and C1 controls are legal XML 1.0 characters.
        let plist = launchd_plist("app.beans.serve", "/beans", "/log", &env(&[("BEANS_RELAY_URL", "\u{7f}\u{85}")])).unwrap();
        assert_eq!(read_plist(&plist).1[1].1, "\u{7f}\u{85}");
    }

    // MARK: systemd

    /// Releases that distributions ship: Ubuntu 18.04 (237), RHEL 8 (239), Ubuntu 20.04 (245),
    /// Debian 11 (247), Ubuntu 22.04 (249), Debian 12 and RHEL 9 (252), openSUSE Leap 15.6
    /// (254), Ubuntu 24.04 (255), Debian 13 (257), and one newer than any of them.
    const SYSTEMD_RELEASES: &[u32] = &[237, 239, 245, 247, 249, 252, 254, 255, 257, 300];

    /// Whether `release` keeps an `Environment=` value, per `env_value_is_valid` in systemd's
    /// sources: 237 refuses every control character but a tab, 245 also allows a newline, 247
    /// allows any. Releases between 237 and 245 are not confirmed and count as 237. Every release
    /// refuses the Unicode noncharacters as invalid UTF-8.
    fn systemd_keeps(value: &str, release: u32) -> bool {
        value.chars().all(|c| match c {
            c if ('\u{FDD0}'..='\u{FDEF}').contains(&c) || (c as u32 & 0xFFFE) == 0xFFFE => false,
            '\t' => true,
            '\n' => release >= 245,
            c if c.is_ascii_control() => release >= 247,
            _ => true,
        })
    }

    /// One setting's value, split into words the way systemd does for `Environment=` and
    /// `ExecStart=` (quotes removed, C escapes read), then with `%%` read back. A specifier the
    /// definition did not mean fails the test.
    fn systemd_words(value: &str) -> Vec<String> {
        let mut words = Vec::new();
        let mut chars = value.chars().peekable();
        loop {
            while chars.next_if(|c| *c == ' ' || *c == '\t').is_some() {}
            if chars.peek().is_none() {
                break;
            }
            let mut word = String::new();
            let mut quote = None;
            while let Some(c) = chars.next() {
                match (c, quote) {
                    ('\\', _) => word.push(match chars.next().expect("escape at the end of a line") {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        'x' => {
                            let hex: String = [chars.next().unwrap(), chars.next().unwrap()].into_iter().collect();
                            let code = u32::from_str_radix(&hex, 16).unwrap();
                            assert_ne!(code, 0, "systemd refuses \\x00");
                            char::from_u32(code).unwrap()
                        }
                        c @ ('\\' | '"' | '\'' | ' ') => c,
                        c => panic!("escape \\{c} was not written by systemd_word"),
                    }),
                    ('"' | '\'', None) => quote = Some(c),
                    (c, Some(open)) if c == open => quote = None,
                    (' ' | '\t', None) => break,
                    (c, _) => word.push(c),
                }
            }
            assert!(quote.is_none(), "unterminated quote in {value:?}");
            let mut resolved = String::new();
            let mut rest = word.chars();
            while let Some(c) = rest.next() {
                if c == '%' {
                    let next = rest.next();
                    assert_eq!(next, Some('%'), "%{next:?} would be read as a specifier in {word:?}");
                }
                resolved.push(c);
            }
            words.push(resolved);
        }
        words
    }

    /// What `release` runs for this unit: the program's path, its arguments, and the
    /// environment it keeps. A path systemd refuses fails the test, since the unit would not load.
    fn read_unit(unit: &str, release: u32) -> (String, Vec<String>, Vec<(String, String)>) {
        let (mut path, mut arguments, mut environment) = (None, Vec::new(), Vec::new());
        for line in unit.lines() {
            assert!(!line.ends_with('\\'), "a line that continues: {line:?}");
            assert!(!line.contains(|c: char| c.is_ascii_control()), "a control character in {line:?}");
            if let Some(value) = line.strip_prefix("ExecStart=") {
                let mut words = systemd_words(value).into_iter();
                let program = words.next().unwrap();
                // systemd's string_is_safe, then path_is_absolute.
                assert!(!program.contains(|c: char| matches!(c, '"' | '\'' | '\\') || c.is_ascii_control()), "systemd refuses {program:?}");
                assert!(systemd_keeps(&program.replace('\t', ""), 300), "systemd skips the line of {program:?}");
                assert!(program.starts_with('/'));
                // The arguments are expanded for `$`; `serve` has none.
                arguments = words.inspect(|word| assert!(!word.contains('$'))).collect();
                path = Some(program);
            } else if let Some(value) = line.strip_prefix("Environment=") {
                for word in systemd_words(value) {
                    let (name, value) = word.split_once('=').unwrap();
                    if systemd_keeps(value, release) {
                        environment.push((name.to_string(), value.to_string()));
                    }
                }
            } else {
                assert!(line.is_empty() || line.starts_with('[') || line.contains('='), "{line:?} is not a setting: a value broke its line");
            }
        }
        (path.expect("no ExecStart="), arguments, environment)
    }

    /// Checks one unit for one release: either it passes every variable exactly, or it was
    /// refused because that release would drop one.
    fn check_unit(exe: &str, environment: &[(String, String)], release: Option<u32>) {
        let oldest = release.unwrap_or(237);
        let all_kept = environment.iter().all(|(_, value)| systemd_keeps(value, oldest));
        match systemd_unit(exe, environment, release) {
            Ok(unit) => {
                assert!(all_kept, "a unit {release:?} would read without a variable:\n{unit}");
                let (path, arguments, read) = read_unit(&unit, oldest);
                assert_eq!((path.as_str(), arguments), (exe, vec!["serve".to_string()]), "{unit}");
                let mut expected = env(&[("BEANS_SERVICE", "1")]);
                expected.extend(environment.iter().cloned());
                assert_eq!(read, expected, "{unit}");
            }
            Err(error) => {
                assert!(!all_kept, "refused a unit {release:?} keeps whole: {error}");
                let c = error.character;
                assert!((c.is_ascii_control() && c != '\t') || !systemd_keeps(&c.to_string(), 300), "{error}");
            }
        }
    }

    #[test]
    fn systemd_unit_passes_hard_values_unchanged_on_every_release() {
        for release in SYSTEMD_RELEASES.iter().copied().map(Some).chain([None]) {
            for value in HARD {
                check_unit("/home/me/Be ans 100%/$HOME/${x}/beans", &hard_environment(value), release);
            }
        }
        for value in HARD {
            assert!(systemd_unit("/beans", &hard_environment(value), Some(247)).is_ok(), "{value:?}");
        }
    }

    #[test]
    fn systemd_refuses_what_the_installed_release_would_drop() {
        let unit = |value: &str, release| systemd_unit("/beans", &env(&[("BEANS_HOME", value)]), release).map_err(|error| error.to_string());
        let message = unit("a\nb", Some(237)).unwrap_err();
        assert!(message.contains("BEANS_HOME holds U+000A") && message.contains("systemd 237 would drop") && message.contains("systemd 245 and later"), "{message}");
        assert!(unit("a\nb", Some(245)).is_ok());
        assert!(unit("a\u{1b}b", Some(245)).unwrap_err().contains("systemd 247 and later"));
        assert!(unit("a\u{7f}b", Some(246)).is_err());
        assert!(unit("a\u{1b}\r\u{7f}b", Some(247)).is_ok());
        assert!(unit("a\nb", None).unwrap_err().contains("this computer's systemd would drop"));
        for release in [None, Some(237)] {
            assert!(unit("tab\there \u{85} \u{a0} ünï 🫘", release).is_ok(), "tabs and non-ASCII are kept by every release");
        }
    }

    #[test]
    fn systemd_program_path_keeps_dollar_and_refuses_what_systemd_refuses() {
        let unit = systemd_unit("/opt/100%/$HOME/${HOME}/be ans", &[], Some(257)).unwrap();
        // The path is not expanded for `$`; only `%` is a specifier there.
        assert!(unit.contains("ExecStart=\"/opt/100%%/$HOME/${HOME}/be ans\" serve\n"), "{unit}");
        for c in ['"', '\'', '\\', '\n', '\t', '\r', '\u{1}', '\u{1b}', '\u{7f}'] {
            let error = systemd_unit(&format!("/opt/a{c}b/beans"), &[], Some(257)).unwrap_err();
            assert_eq!((error.what.as_str(), error.character), ("the path of beans", c));
        }
    }

    #[test]
    fn systemd_refuses_noncharacters_every_release_reads_as_invalid_utf8() {
        for c in ['\u{FDD0}', '\u{FDEF}', '\u{FFFE}', '\u{FFFF}', '\u{1FFFE}', '\u{10FFFF}'] {
            for release in [Some(237), Some(257), None] {
                let error = systemd_unit("/beans", &env(&[("BEANS_RELAY_URL", &format!("a{c}b"))]), release).unwrap_err();
                assert_eq!((error.what.as_str(), error.character), ("BEANS_RELAY_URL", c));
                assert!(error.to_string().contains("invalid UTF-8"), "{error}");
            }
            let error = systemd_unit(&format!("/opt/a{c}b/beans"), &[], Some(257)).unwrap_err();
            assert_eq!((error.what.as_str(), error.character), ("the path of beans", c));
        }
        for c in ['\u{FDCF}', '\u{FDF0}', '\u{FFFD}', '\u{10000}', '\u{1FFFD}'] {
            assert!(systemd_unit(&format!("/opt/{c}/beans"), &env(&[("BEANS_RELAY_URL", &c.to_string())]), Some(237)).is_ok(), "{c:?}");
        }
    }

    #[test]
    fn systemd_newlines_and_controls_stay_inside_their_setting() {
        let unit = systemd_unit("/beans", &env(&[("BEANS_RELAY_URL", "a\nExecStart=/bin/evil\r\u{1b}[0m\u{7f}\\")]), Some(257)).unwrap();
        assert!(unit.contains("Environment=\"BEANS_RELAY_URL=a\\nExecStart=/bin/evil\\r\\x1b[0m\\x7f\\\\\"\n"), "{unit}");
        assert_eq!(unit.lines().filter(|line| line.starts_with("ExecStart=")).count(), 1);
    }

    #[test]
    fn systemd_release_is_read_from_systemctl_version() {
        for (output, release) in [
            ("systemd 237\n+PAM +AUDIT +SELINUX", Some(237)),
            ("systemd 239 (239-82.el8)\n+PAM", Some(239)),
            ("systemd 245 (245.4-4ubuntu3.24)", Some(245)),
            ("systemd 254 (254.5+suse.12.gd6e1a7e8d8)", Some(254)),
            ("systemd 257 (257.13-1~deb13u1)\n", Some(257)),
            ("systemd 258~rc1 (258~rc1-1)", Some(258)),
            ("", None),
            ("systemd\n", None),
            ("Failed to connect to bus", None),
            ("running in chroot, ignoring request\nsystemd 255", None),
        ] {
            assert_eq!(systemd_release(output), release, "{output:?}");
        }
    }

    #[test]
    fn systemd_show_output_is_read_as_systemctl_prints_it() {
        let output = "FragmentPath=/home/me/.config/systemd/user/beans.service\nLoadState=loaded\nActiveState=active\nUnitFileState=enabled\nMainPID=4242\n";
        assert_eq!(read_systemd_show(output), SystemdUnit {
            fragment: "/home/me/.config/systemd/user/beans.service".into(), drop_ins: vec![], load: "loaded".into(), active: "active".into(), enabled: "enabled".into(), pid: Some(4242),
        });
        let edited = read_systemd_show("DropInPaths=/home/me/.config/systemd/user/beans.service.d/override.conf /etc/systemd/user/beans.service.d/proxy.conf\n");
        assert_eq!(edited.drop_ins, ["/home/me/.config/systemd/user/beans.service.d/override.conf", "/etc/systemd/user/beans.service.d/proxy.conf"]);
        // A value holding `=`, and lines that are not properties.
        assert_eq!(read_systemd_show("FragmentPath=/a=b/beans.service\nnot a property\n=\n").fragment, "/a=b/beans.service");
        let gone = read_systemd_show("FragmentPath=\nLoadState=not-found\nActiveState=inactive\nUnitFileState=\nMainPID=0\n");
        assert_eq!((gone.fragment.as_str(), gone.pid), ("", None));
        assert_eq!(read_systemd_show(""), SystemdUnit::default(), "no user session: nothing known");
    }

    #[test]
    fn systemd_states_are_classified_from_the_file_and_the_manager() {
        let ours = "/home/me/.config/systemd/user/beans.service";
        let unit = |fragment: &str, load: &str, active: &str, enabled: &str, pid: Option<u32>| SystemdUnit {
            fragment: fragment.into(), drop_ins: vec![], load: load.into(), active: active.into(), enabled: enabled.into(), pid,
        };
        let none = |_: &str| None;
        let written = plan::read_systemd(&systemd_unit("/beans", &[], Some(257)).unwrap());
        // Installed and running.
        assert_eq!(systemd_found(written.clone(), ours, &unit(ours, "loaded", "active", "enabled", Some(7)), none), (written.clone(), true, Some(7)));
        // Written, but never enabled or started: an install that stopped partway.
        assert_eq!(systemd_found(written.clone(), ours, &unit(ours, "loaded", "inactive", "disabled", None), none), (written.clone(), false, None));
        // Failing to start over and over: enabled, activating, no process.
        assert_eq!(systemd_found(written.clone(), ours, &unit(ours, "loaded", "activating", "enabled", None), none), (written, true, None));
        // The file was deleted while the unit ran.
        assert_eq!(systemd_found(plan::Stored::Absent, ours, &unit("", "not-found", "active", "", Some(9)), none), (plan::Stored::Absent, true, Some(9)));
        // A unit of the same name a package or an administrator installed, Beans's or not.
        let theirs = "/usr/lib/systemd/user/beans.service";
        let (stored, ..) = systemd_found(plan::Stored::Absent, ours, &unit(theirs, "loaded", "active", "enabled", Some(3)), none);
        assert_eq!(stored, plan::Stored::Elsewhere { path: theirs.into(), definition: None });
        let beans = plan::Definition { program: "/opt/beans".into(), environment: vec![("BEANS_HOME".into(), "/srv/beans".into())] };
        let (stored, ..) = systemd_found(plan::Stored::Absent, ours, &unit(theirs, "loaded", "active", "enabled", Some(3)), |path| (path == theirs).then(|| beans.clone()));
        assert_eq!(stored, plan::Stored::Elsewhere { path: theirs.into(), definition: Some(beans) });
        // Nothing anywhere.
        assert_eq!(systemd_found(plan::Stored::Absent, ours, &SystemdUnit::default(), none), (plan::Stored::Absent, false, None));
        // Masked (`systemctl --user mask`): systemd loads /dev/null, which is not Beans's unit; the
        // masking link at Beans's own path reads as an empty, edited file and is replaced.
        let masked = plan::read_systemd("");
        assert!(matches!(masked, plan::Stored::Edited { .. }));
        assert_eq!(systemd_found(masked.clone(), ours, &unit("/dev/null", "masked", "inactive", "masked", None), none), (masked, false, None));
        // Enabled in other ways.
        for enabled in ["enabled-runtime", "linked", "linked-runtime"] {
            assert!(systemd_found(plan::Stored::Absent, ours, &unit(ours, "loaded", "inactive", enabled, None), none).1, "{enabled}");
        }
        for state in ["disabled", "static", "indirect", "generated", "bad", ""] {
            assert!(!systemd_found(plan::Stored::Absent, ours, &unit(ours, "loaded", "inactive", state, None), none).1, "{state}");
        }
    }

    // MARK: What answers on the port

    #[test]
    fn the_version_is_read_from_beans_version() {
        assert_eq!(version_output("beans 0.1.10\n").as_deref(), Some("0.1.10"));
        assert_eq!(version_output("beans 1.2.0-rc1").as_deref(), Some("1.2.0-rc1"));
        assert_eq!(version_output("other 1.0.0"), None);
        assert_eq!(version_output("beans: command not found"), None);
        assert_eq!(version_output(""), None);
    }

    #[test]
    fn the_service_port_comes_from_its_definition_then_this_command() {
        let config = Config { home: "/h".into(), port: 4874 };
        let stored = |port: &str| plan::Stored::Beans {
            definition: plan::Definition { program: "/beans".into(), environment: env(&[("BEANS_PORT", port)]) }, current: true, text: String::new(),
        };
        assert_eq!(service_ports(&config, &stored("5000")), [5000, 4874]);
        assert_eq!(service_ports(&config, &stored("4874")), [4874]);
        assert_eq!(service_ports(&config, &stored("not a port")), [4874]);
        assert_eq!(service_ports(&config, &plan::Stored::Absent), [4874]);
    }

    /// A stand-in for a `beans serve`, run in a child process by the test below: it answers
    /// `hello` on the port it is given, as the version it is given, until it is stopped.
    #[test]
    fn stand_in_serve() {
        use tokio_tungstenite::tungstenite::{self, Message};
        let Ok(port) = std::env::var("BEANS_STAND_IN_PORT") else { return };
        let version = std::env::var("BEANS_STAND_IN_VERSION").unwrap();
        let listener = std::net::TcpListener::bind(("127.0.0.1", port.parse::<u16>().unwrap())).unwrap();
        for stream in listener.incoming() {
            let (Ok(stream), version) = (stream, version.clone()) else { continue };
            std::thread::spawn(move || {
                let Ok(mut socket) = tungstenite::accept(stream) else { return };
                while let Ok(message) = socket.read() {
                    let Message::Text(text) = message else { continue };
                    let request: serde_json::Value = serde_json::from_str(&text).unwrap();
                    let result = serde_json::json!({
                        "version": version, "pid": std::process::id(),
                        "executable": std::env::current_exe().unwrap().to_string_lossy(),
                    });
                    let _ = socket.send(Message::Text(serde_json::json!({ "id": request["id"], "result": result }).to_string().into()));
                }
            });
        }
    }

    /// Starts [`stand_in_serve`] on a free port; its exit arrives on the receiver.
    #[cfg(unix)]
    fn start_stand_in(version: &str) -> (u16, u32, std::sync::mpsc::Receiver<std::process::ExitStatus>) {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "service::tests::stand_in_serve", "--nocapture", "--test-threads=1"])
            .env("BEANS_STAND_IN_PORT", port.to_string())
            .env("BEANS_STAND_IN_VERSION", version)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        let (exited, receiver) = std::sync::mpsc::channel();
        // Reaped at once, as a real leftover's parent would.
        std::thread::spawn(move || exited.send(child.wait().unwrap()));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while probe(port).is_none() {
            assert!(std::time::Instant::now() < deadline, "the stand-in never answered");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        (port, pid, receiver)
    }

    #[cfg(unix)]
    #[test]
    fn an_old_beans_serve_left_on_the_port_is_found_and_stopped() {
        let (port, pid, exited) = start_stand_in("0.0.1");
        let holder = probe(port).unwrap();
        assert_eq!((holder.pid, holder.version.as_deref(), holder.executable_exists), (Some(pid), Some("0.0.1"), true));
        let notes = clear_port(&[port], true).unwrap();
        assert_eq!(notes, [plan::Note::StoppedOld { port, pid, version: Some("0.0.1".into()) }]);
        let status = exited.recv_timeout(std::time::Duration::from_secs(20)).expect("the old one stopped");
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(libc::SIGTERM), "asked to stop, so a real one drains first");
        assert!(probe(port).is_none(), "the port is free for the service");
    }

    #[cfg(unix)]
    #[test]
    fn a_current_beans_serve_on_the_port_is_never_stopped() {
        for version in [crate::config::VERSION, "999.0.0"] {
            let (port, pid, exited) = start_stand_in(version);
            let error = clear_port(&[port], true).unwrap_err().to_string();
            assert!(error.contains(&format!("pid {pid}")) && error.contains("already answers"), "{error}");
            let notes = clear_port(&[port], false).unwrap();
            assert!(matches!(&notes[..], [plan::Note::PortHeld { pid: Some(p), .. }] if *p == pid), "{notes:?}");
            assert!(exited.try_recv().is_err() && probe(port).is_some(), "still running");
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            exited.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        }
    }

    #[test]
    fn nothing_on_the_port_is_nothing_to_clear() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert_eq!(probe(port), None);
        assert!(clear_port(&[port], true).unwrap().is_empty());
        // Something that is not a websocket.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                use std::io::Write;
                let _ = (&stream).write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok");
            }
        });
        assert_eq!(probe(port), None);
    }

    #[test]
    fn windows_paths_are_written_plainly_only_when_that_is_the_same_file() {
        let cases = [
            (r"\\?\C:\Users\me\.local\bin\beans.exe", r"C:\Users\me\.local\bin\beans.exe"),
            (r"\\?\C:\Program Files\Beans 100%\beans.exe", r"C:\Program Files\Beans 100%\beans.exe"),
            (r"\\?\UNC\server\share\tools\beans.exe", r"\\server\share\tools\beans.exe"),
            (r"\\?\d:\ünï\🫘\beans.exe", r"d:\ünï\🫘\beans.exe"),
            (r"C:\already\plain\beans.exe", r"C:\already\plain\beans.exe"),
            // Win32 would read these as another file, or not at all: kept verbatim.
            (r"\\?\C:\dir.\beans.exe", r"\\?\C:\dir.\beans.exe"),
            (r"\\?\C:\dir \beans.exe", r"\\?\C:\dir \beans.exe"),
            (r"\\?\C:\a\..\beans.exe", r"\\?\C:\a\..\beans.exe"),
            (r"\\?\C:\a\.\beans.exe", r"\\?\C:\a\.\beans.exe"),
            (r"\\?\C:\CON\beans.exe", r"\\?\C:\CON\beans.exe"),
            (r"\\?\C:\x\nul.txt\beans.exe", r"\\?\C:\x\nul.txt\beans.exe"),
            (r"\\?\C:\x\COM1\beans.exe", r"\\?\C:\x\COM1\beans.exe"),
            (r"\\?\C:\a*b\beans.exe", r"\\?\C:\a*b\beans.exe"),
            (r"\\?\C:\a\\beans.exe", r"\\?\C:\a\\beans.exe"),
            (r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\beans.exe", r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\beans.exe"),
            (r"\\?\GLOBALROOT\Device\HarddiskVolume1\beans.exe", r"\\?\GLOBALROOT\Device\HarddiskVolume1\beans.exe"),
        ];
        for (verbatim, written) in cases {
            assert_eq!(plain_windows_path(verbatim), written, "{verbatim}");
        }
        // COM0, COM10 and COMX are ordinary names.
        for name in ["COM0", "COM10", "COMX", "CONSOLE", "nullable"] {
            assert_eq!(plain_windows_path(&format!(r"\\?\C:\{name}\beans.exe")), format!(r"C:\{name}\beans.exe"), "{name}");
        }
        let long = format!(r"\\?\C:\{}\beans.exe", "d".repeat(300));
        assert_eq!(plain_windows_path(&long), long, "too long for ordinary paths");
        // Plain paths from here quote and read back as Run key programs.
        let line = windows_command_line(&plain_windows_path(cases[1].0), &[]).unwrap();
        assert_eq!(plan::read_run_key(&line), plan::Stored::Beans {
            definition: plan::Definition { program: cases[1].1.into(), environment: vec![] }, current: true, text: line.clone(),
        });
    }

    #[test]
    fn launchd_falls_back_from_the_login_session_to_the_users_domain() {
        let domains = ["gui/501".to_string(), "user/501".to_string()];
        let mut tried = Vec::new();
        assert_eq!(first_domain(&domains, |domain| { tried.push(domain.to_string()); Ok(()) }).unwrap(), "gui/501");
        assert_eq!(tried, ["gui/501"], "a GUI session is used when there is one");
        let over_ssh = first_domain(&domains, |domain| match domain {
            "gui/501" => anyhow::bail!("Bootstrap failed: 125: Domain does not support specified action"),
            _ => Ok(()),
        });
        assert_eq!(over_ssh.unwrap(), "user/501");
        let neither = first_domain(&domains, |domain| anyhow::bail!("refused in {domain}")).unwrap_err().to_string();
        assert!(neither.contains("gui/501: refused in gui/501") && neither.contains("user/501: refused in user/501"), "{neither}");
    }

    #[test]
    fn launchctl_print_gives_the_jobs_own_pid() {
        let running = "gui/501/app.beans.serve = {\n\tactive count = 1\n\tpath = /Users/me/Library/LaunchAgents/app.beans.serve.plist\n\tstate = running\n\n\tprogram = /Users/me/.local/bin/beans\n\targuments = {\n\t\t/Users/me/.local/bin/beans\n\t\tserve\n\t}\n\n\tendpoints = {\n\t\t\"x\" = {\n\t\t\tpid = 999\n\t\t}\n\t}\n\tpid = 4242\n\timmediate reason = speculative\n}\n";
        assert_eq!(launchd_pid(running), Some(4242), "not the nested block's pid");
        let waiting = "gui/501/app.beans.serve = {\n\tactive count = 0\n\tstate = not running\n\tlast exit code = 1\n}\n";
        assert_eq!(launchd_pid(waiting), None);
        assert_eq!(launchd_pid(""), None);
        assert_eq!(launchd_pid("gui/501/app.beans.serve = {\n\tpid = 0\n}"), None);
        assert_eq!(launchd_pid("gui/501/app.beans.serve = {\n\tpid = banana\n}"), None);
        assert_eq!(launchd_pid("pid = 7\n"), None, "outside the job's braces");
    }

    // MARK: Windows

    /// A command line split the way Rust's `std::env::args` and `CommandLineToArgvW` agree on:
    /// the program name up to the next quote, then arguments with the backslash-and-quote rules.
    /// `""` inside quotes, which they read differently, fails the test.
    fn windows_argv(line: &str) -> Vec<String> {
        let mut chars = line.chars().peekable();
        let mut program = String::new();
        if chars.next_if_eq(&'"').is_some() {
            program.extend(chars.by_ref().take_while(|c| *c != '"'));
        } else {
            while let Some(c) = chars.next_if(|c| *c != ' ' && *c != '\t') {
                program.push(c);
            }
        }
        let mut argv = vec![program];
        loop {
            while chars.next_if(|c| *c == ' ' || *c == '\t').is_some() {}
            if chars.peek().is_none() {
                return argv;
            }
            let (mut arg, mut quoted) = (String::new(), false);
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        let mut backslashes = 1;
                        while chars.next_if_eq(&'\\').is_some() {
                            backslashes += 1;
                        }
                        if chars.next_if_eq(&'"').is_some() {
                            arg.push_str(&"\\".repeat(backslashes / 2));
                            if backslashes % 2 == 1 {
                                arg.push('"');
                            } else {
                                assert!(!(quoted && chars.peek() == Some(&'"')), "\"\" inside quotes in {line:?}");
                                quoted = !quoted;
                            }
                        } else {
                            arg.push_str(&"\\".repeat(backslashes));
                        }
                    }
                    '"' => {
                        assert!(!(quoted && chars.peek() == Some(&'"')), "\"\" inside quotes in {line:?}");
                        quoted = !quoted;
                    }
                    ' ' | '\t' if !quoted => break,
                    c => arg.push(c),
                }
            }
            argv.push(arg);
        }
    }

    /// `ExpandEnvironmentStrings`, which a sign-in may apply to the Run key's command line:
    /// `%NAME%` for a defined NAME (in any case) is its value; anything else stays.
    fn expand(line: &str, defined: &[(&str, &str)]) -> String {
        let mut out = String::new();
        let mut rest = line;
        while let Some(start) = rest.find('%') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            match after.find('%') {
                Some(end) => match defined.iter().find(|(name, _)| name.eq_ignore_ascii_case(&after[..end])) {
                    Some((_, value)) => {
                        out.push_str(value);
                        rest = &after[end + 1..];
                    }
                    None => {
                        out.push('%');
                        out.push_str(&after[..end]);
                        rest = &after[end..];
                    }
                },
                None => {
                    out.push_str(&rest[start..]);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// The variables `beans service run` passes on, read as `main` reads its `--env` arguments.
    fn run_environment(argv: &[String]) -> Vec<(String, String)> {
        assert_eq!(argv[1..3], ["service", "run"]);
        argv[3..].chunks(2).map(|pair| {
            assert_eq!(pair[0], "--env");
            let (name, value) = pair[1].split_once('=').unwrap();
            (name.to_string(), value.to_string())
        }).collect()
    }

    #[test]
    fn windows_run_key_passes_hard_values_unchanged() {
        for value in HARD {
            let environment = hard_environment(value);
            let exe = format!(r"C:\Users\O'Brien {}\AppData\beans.exe", value.replace(['"', '%', '\r', '\n'], ""));
            let line = windows_command_line(&exe, &environment).unwrap();
            let argv = windows_argv(&line);
            assert_eq!(argv[0], exe, "{line}");
            assert_eq!(run_environment(&argv), environment, "{value:?}\n{line}");
        }
    }

    #[test]
    fn windows_run_key_is_unchanged_by_sign_in_expansion() {
        let defined: &[(&str, &str)] = &[("PATH", r"C:\Windows"), ("USERPROFILE", r"C:\Users\me"), ("HOME", "H"), ("A", "a"), ("", "empty")];
        assert_eq!(expand("%PATH%|%a%|%nope%x%A%", defined), r"C:\Windows|a|%nope%xa", "the model expands");
        for value in HARD.iter().copied().chain(["%USERPROFILE%\\x", "%a%%A%", "50%PATH%50", "%\"%"]) {
            let environment = hard_environment(value);
            // One % in the program name is allowed; it pairs with the arguments' first.
            let line = windows_command_line(r"C:\100%\beans.exe", &environment).unwrap();
            // Whichever way the expansion pairs them, the text between two % ends with a quote,
            // and no variable Windows or a user sets has a quote in its name.
            let percents: Vec<usize> = line.match_indices('%').map(|(at, _)| at).collect();
            for pair in percents.windows(2) {
                let name = &line[pair[0] + 1..pair[1]];
                assert!(name.ends_with('"'), "{name:?} could name a variable in {line}");
            }
            assert_eq!(expand(&line, defined), line, "{value:?}");
            assert_eq!(run_environment(&windows_argv(&line)), environment, "{value:?}");
        }
    }

    #[test]
    fn windows_backslashes_are_doubled_only_before_a_quote() {
        let argument = |value: &str| windows_arguments(&env(&[("BEANS_HOME", value)])).unwrap();
        assert_eq!(argument(r"C:\Users\me"), r#"service run --env "BEANS_HOME=C:\Users\me""#);
        assert_eq!(argument(r"C:\Users\me\"), r#"service run --env "BEANS_HOME=C:\Users\me\\""#);
        assert_eq!(argument(r#"a\"b"#), r#"service run --env "BEANS_HOME=a\\\"b""#);
        assert_eq!(argument(r"a\%b"), r#"service run --env "BEANS_HOME=a\\"%"b""#);
        assert_eq!(argument("%"), r#"service run --env "BEANS_HOME="%"""#);
    }

    #[test]
    fn windows_direct_start_uses_the_run_key_arguments() {
        let exe = r"C:\Program Files\Beans\beans.exe";
        for value in HARD {
            let environment = hard_environment(value);
            let arguments = windows_arguments(&environment).unwrap();
            let line = windows_command_line(exe, &environment).unwrap();
            // `install` starts the supervisor with `raw_arg(arguments)` after the program, which
            // is the Run key's line.
            assert_eq!(line, format!("\"{exe}\" {arguments}"));
            assert_eq!(windows_argv(&line), windows_argv(&format!("\"{exe}\" {arguments}")));
        }
    }

    #[test]
    fn windows_supervisor_recognizes_its_home_from_the_run_key() {
        for value in HARD.iter().filter(|value| !value.is_empty()) {
            let home = Path::new(value);
            let line = windows_command_line(r"C:\beans.exe", &hard_environment(value)).unwrap();
            let record = supervisor::Record { format: crate::config::Format::BeansV2, pid: 7, created_at: 1, home: home.into() };
            let image = Path::new(r"C:\beans.exe");
            assert!(supervisor::matches(&record, 1, image, &[image], home, &windows_argv(&line)), "{value:?}\n{line}");
        }
    }

    #[test]
    fn windows_refuses_a_program_name_it_cannot_quote() {
        let error = windows_command_line(r#"C:\a"b\beans.exe"#, &[]).unwrap_err();
        assert_eq!((error.what.as_str(), error.character), ("the path of beans", '"'));
        let error = windows_command_line(r"C:\%USERPROFILE%\beans.exe", &[]).unwrap_err();
        assert_eq!((error.what.as_str(), error.character), ("the path of beans", '%'));
        assert!(windows_command_line(r"C:\100%\beans.exe", &[]).is_ok(), "one % cannot name a variable");
    }

    // MARK: Every character

    /// Every ASCII character and the Unicode boundaries (C1 controls, the surrogate gap, the
    /// noncharacters in and at the edges of each range, the BOM, line and paragraph separators, the last plane), each alone and next
    /// to the characters each syntax treats specially; then 3000 pseudo-random strings weighted
    /// toward those characters. The same every run.
    fn corpus() -> Vec<String> {
        let boundaries = [
            '\u{80}', '\u{85}', '\u{9f}', '\u{a0}', '\u{ff}', '\u{100}', '\u{7ff}', '\u{800}', '\u{2028}', '\u{2029}',
            '\u{d7ff}', '\u{e000}', '\u{fdcf}', '\u{fdd0}', '\u{fdef}', '\u{fdf0}', '\u{feff}', '\u{fffd}', '\u{fffe}',
            '\u{ffff}', '\u{10000}', '\u{1f96b}', '\u{1fffd}', '\u{1fffe}', '\u{1ffff}', '\u{10fffd}', '\u{10ffff}',
        ];
        let mut values: Vec<String> = HARD.iter().map(|value| value.to_string()).collect();
        for c in (1..=0x7f_u8).map(char::from).chain(boundaries) {
            for context in ["{}", "a{}b", "\\{}", "{}\\", "\\\\{}\\\\", "%{}%", "\"{}\"", "{}{}", "$%{}", "{}%PATH%"] {
                values.push(context.replace("{}", &c.to_string()));
            }
        }
        let special = [
            '\\', '"', '\'', '%', '$', '{', '}', '\n', '\r', '\t', ' ', '&', '<', '>', ';', '=', '#', '/', ':', '@', '^', '!',
            '\u{1}', '\u{1b}', '\u{7f}', '\u{85}', '\u{fdd0}', '\u{fffe}', 'a', 'Z', '0', 'ü', '🫘',
        ];
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..3000 {
            let length = next() % 48;
            let value = (0..length).map(|_| {
                let roll = next();
                if roll % 5 != 0 {
                    special[(roll >> 8) as usize % special.len()]
                } else {
                    char::from_u32((roll >> 8) as u32 % 0x11_0000).filter(|c| *c != '\0').unwrap_or('\u{fffd}')
                }
            }).collect();
            values.push(value);
        }
        values
    }

    #[test]
    fn every_character_is_kept_exactly_or_refused_by_the_plist() {
        for value in corpus() {
            let environment = hard_environment(&value);
            match launchd_plist("app.beans.serve", "/beans", "/log", &environment) {
                Ok(plist) => {
                    assert_eq!(plist_refused(&value), None, "{value:?}");
                    assert_eq!(read_plist(&plist).1[1..], environment[..], "{value:?}");
                }
                Err(error) => {
                    assert_eq!(Some(error.character), plist_refused(&value), "{value:?}");
                }
            }
        }
    }

    #[test]
    fn every_character_is_kept_exactly_or_refused_by_every_systemd_release() {
        for value in corpus() {
            for release in [Some(237), Some(245), Some(247), Some(257), None] {
                check_unit("/opt/Be ans/beans", &hard_environment(&value), release);
            }
        }
    }

    #[test]
    fn every_character_is_kept_exactly_by_the_run_key() {
        let defined: &[(&str, &str)] = &[("PATH", r"C:\Windows"), ("USERPROFILE", r"C:\Users\me"), ("A", "a"), ("", "empty")];
        for value in corpus() {
            let environment = hard_environment(&value);
            let line = windows_command_line(r"C:\Program Files\Beans\beans.exe", &environment).unwrap();
            let percents: Vec<usize> = line.match_indices('%').map(|(at, _)| at).collect();
            for pair in percents.windows(2) {
                assert!(line[pair[0] + 1..pair[1]].ends_with('"'), "{value:?} in {line}");
            }
            assert_eq!(expand(&line, defined), line, "{value:?}");
            let argv = windows_argv(&line);
            assert_eq!(argv[0], r"C:\Program Files\Beans\beans.exe");
            assert_eq!(run_environment(&argv), environment, "{value:?}\n{line}");
        }
    }

    // MARK: The systems' own readers
    //
    // These run only where the reader exists, with no service manager involved: `plutil` is
    // CoreFoundation's property list reader; `CommandLineToArgvW`, `ExpandEnvironmentStringsW`,
    // and a child process's `std::env::args` are what Windows and Rust apply to the Run key.

    #[cfg(target_os = "macos")]
    #[test]
    fn plutil_reads_every_plist_back_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        for (i, value) in corpus().into_iter().filter(|value| plist_refused(value).is_none()).enumerate() {
            let environment = hard_environment(&value);
            let exe = format!("/Applications/{value}/beans");
            let path = dir.path().join(format!("{i}.plist"));
            std::fs::write(&path, launchd_plist("app.beans.serve", &exe, "/log", &environment).unwrap()).unwrap();
            let output = Command::new("/usr/bin/plutil").args(["-convert", "json", "-o", "-"]).arg(&path).output().unwrap();
            assert!(output.status.success(), "{value:?}: {}", String::from_utf8_lossy(&output.stderr));
            let plist: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(plist["ProgramArguments"], serde_json::json!([exe, "serve"]), "{value:?}");
            assert_eq!(plist["EnvironmentVariables"]["BEANS_SERVICE"], "1");
            for (name, value) in &environment {
                assert_eq!(plist["EnvironmentVariables"][name], value.as_str(), "{value:?}");
            }
        }
    }

    #[cfg(windows)]
    fn command_line_to_argv_w(line: &str) -> Vec<String> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::UI::Shell::CommandLineToArgvW;
        let wide: Vec<u16> = line.encode_utf16().chain([0]).collect();
        let mut count = 0;
        let argv = unsafe { CommandLineToArgvW(wide.as_ptr(), &mut count) };
        assert!(!argv.is_null());
        let args = (0..count as usize).map(|i| {
            let arg = unsafe { *argv.add(i) };
            let mut length = 0;
            while unsafe { *arg.add(length) } != 0 {
                length += 1;
            }
            String::from_utf16(unsafe { std::slice::from_raw_parts(arg, length) }).unwrap()
        }).collect();
        unsafe { LocalFree(argv.cast()) };
        args
    }

    #[cfg(windows)]
    fn expand_environment_strings_w(line: &str) -> String {
        use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
        let wide: Vec<u16> = line.encode_utf16().chain([0]).collect();
        let size = unsafe { ExpandEnvironmentStringsW(wide.as_ptr(), std::ptr::null_mut(), 0) };
        let mut expanded = vec![0u16; size as usize];
        let written = unsafe { ExpandEnvironmentStringsW(wide.as_ptr(), expanded.as_mut_ptr(), size) };
        assert!(written > 0 && written <= size, "ExpandEnvironmentStringsW failed");
        String::from_utf16(&expanded[..written as usize - 1]).unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn windows_reads_every_run_key_back_unchanged() {
        let exe = r"C:\Program Files\Beans 100%\beans.exe";
        for value in corpus() {
            let environment = hard_environment(&value);
            let line = windows_command_line(exe, &environment).unwrap();
            // Expanded with this process's real environment: PATH, USERPROFILE, and the rest.
            assert_eq!(expand_environment_strings_w(&line), line, "{value:?}");
            let argv = command_line_to_argv_w(&line);
            assert_eq!(argv[0], exe);
            assert_eq!(run_environment(&argv), environment, "{value:?}\n{line}");
        }
    }

    /// Run by the test below in a child process, where it records the arguments Rust read.
    #[cfg(windows)]
    #[test]
    fn windows_argv_probe() {
        if let Some(out) = std::env::var_os("BEANS_SERVICE_ARGV_PROBE") {
            std::fs::write(out, serde_json::to_string(&std::env::args().collect::<Vec<_>>()).unwrap()).unwrap();
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_rust_reads_the_direct_start_arguments_unchanged() {
        use std::os::windows::process::CommandExt;
        let dir = tempfile::tempdir().unwrap();
        // Each string is a child process, so HARD and every ASCII character, not all 3000.
        for (i, value) in corpus().into_iter().take(HARD.len() + 127 * 10).enumerate() {
            let environment = hard_environment(&value);
            let out = dir.path().join(format!("{i}.json"));
            // After `--`, the test harness takes every argument as a name filter, and with
            // `--exact` none of them names another test.
            let status = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "service::tests::windows_argv_probe", "--test-threads=1", "--quiet", "--"])
                .raw_arg(windows_arguments(&environment).unwrap())
                .env("BEANS_SERVICE_ARGV_PROBE", &out)
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "{value:?}");
            let argv: Vec<String> = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
            let after = argv.iter().position(|arg| arg == "--").unwrap();
            let mut read = vec![argv[0].clone()];
            read.extend(argv[after + 1..].iter().cloned());
            assert_eq!(run_environment(&read), environment, "{value:?}");
        }
    }
}
