//! Bringing a computer's Beans service to what it should be, whatever an earlier `beans` or an
//! interrupted command left: a definition that runs a `beans` since moved or replaced, one written
//! by an older version or edited by hand, a same-named unit from elsewhere, a job the service
//! manager still runs after its definition was deleted, one never loaded, and an old `beans serve`
//! still holding the port.
//!
//! [`Found`] is what the platform section found; [`install`], [`refresh`], [`uninstall`] and
//! [`findings`] decide from it alone, and [`apply`] carries a plan out through a [`Host`], putting
//! the previous definition back when a step fails. Nothing here reads the computer or runs a
//! command, so every case is tested on any platform.

use std::cmp::Ordering;
use std::fmt;

/// A Beans service definition, as written or read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Definition {
    /// The `beans` it runs.
    pub program: String,
    /// What it passes on, apart from the `BEANS_SERVICE=1` every definition sets.
    pub environment: Vec<(String, String)>,
}

/// What holds the service's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Stored {
    Absent,
    /// A definition Beans wrote. `current` when it is byte for byte what this version writes for
    /// it; an older `beans` wrote it otherwise. `text` is kept to restore it.
    Beans { definition: Definition, current: bool, text: String },
    /// The service's own file or value, but not as Beans writes it: edited by hand, or damaged.
    Edited { text: String },
    /// The service manager runs the name from another file, such as a unit a package or an
    /// administrator put in `/etc` or `/usr`; `definition` when that file is a Beans definition.
    /// Beans cannot delete it, but its own unit takes precedence over it. Only systemd looks for
    /// a name in more than one place.
    #[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
    Elsewhere { path: String, definition: Option<Definition> },
}

/// A `beans serve` that answered on the service's port, as it described itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Holder {
    pub port: u16,
    /// Its process; a `beans` from before `hello` named it does not say.
    pub pid: Option<u32>,
    pub version: Option<String>,
    /// The binary it started from, and whether that is still on disk.
    pub executable: Option<String>,
    pub executable_exists: bool,
}

/// What a [`Holder`] is, next to this `beans`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Holding {
    /// An older `beans`, or one whose binary is gone: a leftover, which Beans stops.
    Old,
    /// This version: the app's Runner, or one started by hand. Never stopped.
    Current,
    /// A newer `beans` than this one. Never stopped.
    Newer,
    /// Something that answered without saying which `beans` it is. Never stopped.
    Unknown,
}

/// A version's release numbers, padded to three: `0.1.10-rc1` is `[0, 1, 10]`.
fn release(version: &str) -> Option<[u64; 3]> {
    let mut numbers = [0; 3];
    let core = version.trim().trim_start_matches('v').split(['-', '+']).next()?;
    for (i, part) in core.split('.').enumerate() {
        *numbers.get_mut(i)? = part.parse().ok()?;
    }
    Some(numbers)
}

/// How `version` compares with `than`; `None` when either is not a version.
pub(crate) fn compare(version: &str, than: &str) -> Option<Ordering> {
    Some(release(version)?.cmp(&release(than)?))
}

/// What `holder` is to `beans` of `version`.
pub(crate) fn classify(holder: &Holder, version: &str) -> Holding {
    match holder.version.as_deref().and_then(|theirs| compare(theirs, version)) {
        None => Holding::Unknown,
        Some(Ordering::Less) => Holding::Old,
        Some(Ordering::Greater) => Holding::Newer,
        Some(Ordering::Equal) if holder.executable.is_some() && !holder.executable_exists => Holding::Old,
        Some(Ordering::Equal) => Holding::Current,
    }
}

/// What a platform section found for the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    pub stored: Stored,
    /// The service manager holds the job and would run it: launchd has it loaded, systemd has it
    /// enabled or running, or a supervisor Beans started runs on Windows.
    pub active: bool,
    pub running: Option<u32>,
    /// Whether the stored definition's program is still on disk.
    pub program_exists: bool,
    /// The stored definition's program's version, when it is another program than this one.
    pub program_version: Option<String>,
    /// A `beans serve` answering on the service's port, if any.
    pub holder: Option<Holder>,
    /// Files that change the definition without being it: systemd drop-ins, which `systemctl
    /// edit` writes. Beans keeps them (they may set a proxy, say) and says they are there.
    pub overrides: Vec<String>,
}

/// One step, carried out by a [`Host`] in the order of [`Plan::actions`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    /// Stop the job and take it out of the service manager, so no process of it is left.
    Stop,
    /// Stop an old `beans serve` still holding the port once the job is stopped. `strict`: one
    /// that is not old (this version's or a newer one) stops the plan; otherwise it is noted.
    ClearPort { strict: bool },
    /// Delete the definition.
    Remove,
    /// Write this definition, replacing any other in one step.
    Write(Definition),
    /// Load the definition and start it.
    Start,
}

/// Something found or done that a person should know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Note {
    /// The definition runs a `beans` that is no longer there, so the service cannot start.
    ProgramMissing { program: String },
    /// The definition runs another `beans` than this one.
    OtherProgram { was: String, now: String },
    /// These variables change; their values are not repeated, since a relay URL can hold a token.
    EnvironmentChanged { names: Vec<String> },
    /// An older `beans` wrote the definition.
    OlderFormat,
    /// The definition is there but the service manager does not run it: an install that stopped
    /// partway, or a service stopped by hand.
    Inactive,
    /// The service manager still runs the job, but its definition is gone.
    Orphaned,
    /// It should run but no process of it is running: it keeps failing to start.
    NotRunning,
    /// The service's file holds something Beans did not write.
    Edited,
    /// That definition is replaced.
    ReplacedEdited,
    /// The name comes from another file.
    Elsewhere { path: String },
    /// Beans's own unit now takes precedence over that file.
    Overrides { path: String },
    /// The service runs a newer `beans` than this one, so this one leaves it.
    NewerInstalled { program: String, version: String },
    /// An old `beans serve` holds the port: a leftover that `install` and `refresh` stop.
    OldProcess { port: u16, pid: Option<u32>, version: Option<String> },
    /// It was stopped.
    StoppedOld { port: u16, pid: u32, version: Option<String> },
    /// An old one holds the port but did not say its process, so it cannot be stopped here.
    OldUnidentified { port: u16, version: Option<String> },
    /// A `beans serve` that is not old holds the port: the app's Runner, one started by hand, or a
    /// newer one. The service retries, and takes over when it stops.
    PortHeld { port: u16, pid: Option<u32>, version: Option<String> },
    /// Drop-ins change what the service manager runs.
    Overridden { paths: Vec<String> },
    /// This `beans` runs from somewhere that goes away, so the service keeps its own program.
    Transient { program: String, reason: &'static str },
}

fn process(pid: Option<u32>, version: &Option<String>) -> String {
    let version = version.as_deref().map_or(String::from("an unknown version"), |version| format!("version {version}"));
    match pid {
        Some(pid) => format!("pid {pid}, {version}"),
        None => version,
    }
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Note::ProgramMissing { program } => write!(f, "The service ran {program}, which is no longer there, so it could not start."),
            Note::OtherProgram { was, now } => write!(f, "The service ran {was}; this beans is {now}."),
            Note::EnvironmentChanged { names } => write!(f, "The service's {} changes.", names.join(", ")),
            Note::OlderFormat => write!(f, "An older beans wrote the service's definition."),
            Note::Inactive => write!(f, "The service is defined but not loaded: an install stopped partway, or it was stopped by hand."),
            Note::Orphaned => write!(f, "The service manager still ran the service, but its definition was gone."),
            Note::NotRunning => write!(f, "The service should be running, but it is not: it keeps stopping, or cannot start."),
            Note::Edited => write!(f, "The service's definition was not written by beans (edited by hand, or damaged)."),
            Note::ReplacedEdited => write!(f, "The service's definition was not written by beans (edited by hand, or damaged); it is replaced."),
            Note::Elsewhere { path } => write!(f, "The service manager runs the service from {path}, which beans did not write."),
            Note::Overrides { path } => write!(f, "The service manager ran the service from {path}; beans's own definition now takes precedence over it."),
            Note::NewerInstalled { program, version } => write!(f, "The service runs {program}, version {version}, which is newer than this beans, so it is left as it is."),
            Note::OldProcess { port, pid, version } => write!(f, "An old beans serve ({}) holds port {port}, left over from before; beans service install or refresh stops it.", process(*pid, version)),
            Note::StoppedOld { port, pid, version } => write!(f, "Stopped an old beans serve ({}) left holding port {port}.", process(Some(*pid), version)),
            Note::OldUnidentified { port, version } => write!(f, "An old beans serve ({}) holds port {port} but does not say its process; stop it by hand.", process(None, version)),
            Note::PortHeld { port, pid, version } => write!(f, "Another beans serve ({}) answers on port {port}: the Beans app's, or one started by hand. The service keeps retrying and takes over when it stops.", process(*pid, version)),
            Note::Overridden { paths } => write!(f, "{} change what the service runs; beans leaves them as they are.", paths.join(", ")),
            Note::Transient { program, reason } => write!(f, "This beans runs from {program}, {reason}, so the service keeps the beans it runs."),
        }
    }
}

/// Why a plan changes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A `beans serve` that is not a leftover holds the port.
    InUse { port: u16, pid: Option<u32>, version: Option<String>, holding: Holding },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Refusal::InUse { port, pid, version, holding } = self;
        let who = match holding {
            Holding::Newer => "a newer beans serve, which this beans does not stop; install the service from that beans instead",
            Holding::Unknown => "something that does not say which beans it is",
            Holding::Current | Holding::Old => "the Beans app's Runner, or one started by hand. Stop it first; on a computer with the Beans app, the app runs beans serve itself",
        };
        write!(f, "A beans serve ({}) already answers on port {port}: {who}.", process(*pid, version))
    }
}

impl std::error::Error for Refusal {}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Plan {
    pub actions: Vec<Action>,
    pub notes: Vec<Note>,
}

/// What differs between the stored definition and `wanted`.
fn differences(found: &Found, wanted: &Definition) -> Vec<Note> {
    let mut notes = Vec::new();
    let Stored::Beans { definition, current, .. } = &found.stored else { return notes };
    if definition.program != wanted.program {
        notes.push(match found.program_exists {
            false => Note::ProgramMissing { program: definition.program.clone() },
            true => Note::OtherProgram { was: definition.program.clone(), now: wanted.program.clone() },
        });
    }
    let value = |environment: &[(String, String)], name: &str| environment.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone());
    let mut names: Vec<String> = definition.environment.iter().chain(&wanted.environment).map(|(name, _)| name.clone()).collect();
    names.sort();
    names.dedup();
    names.retain(|name| value(&definition.environment, name) != value(&wanted.environment, name));
    if !names.is_empty() {
        notes.push(Note::EnvironmentChanged { names });
    }
    if !current {
        notes.push(Note::OlderFormat);
    }
    notes
}

/// The note for what `Stored` replaces, if it is not a Beans definition.
fn replaced(stored: &Stored) -> Option<Note> {
    match stored {
        Stored::Edited { .. } => Some(Note::ReplacedEdited),
        Stored::Elsewhere { path, .. } => Some(Note::Overrides { path: path.clone() }),
        Stored::Absent | Stored::Beans { .. } => None,
    }
}

/// `beans service install`: the service runs `wanted` from now on, whatever was there: the job of
/// an earlier definition stops first, so two never run at once, and an old `beans serve` left on
/// the port stops too. It refuses only while a `beans serve` that is not old, and not the
/// service's own, holds the port.
pub(crate) fn install(found: &Found, wanted: Definition, version: &str) -> Result<Plan, Refusal> {
    if let Some(holder) = found.holder.as_ref().filter(|_| found.running.is_none()) {
        let holding = classify(holder, version);
        if holding != Holding::Old {
            return Err(Refusal::InUse { port: holder.port, pid: holder.pid, version: holder.version.clone(), holding });
        }
    }
    let mut plan = Plan { notes: differences(found, &wanted), ..Plan::default() };
    plan.notes.extend(replaced(&found.stored));
    if !found.overrides.is_empty() {
        plan.notes.push(Note::Overridden { paths: found.overrides.clone() });
    }
    if found.stored == Stored::Absent && found.active {
        plan.notes.push(Note::Orphaned);
    }
    if found.active || found.running.is_some() {
        plan.actions.push(Action::Stop);
    }
    plan.actions.extend([Action::ClearPort { strict: true }, Action::Write(wanted), Action::Start]);
    Ok(plan)
}

/// `beans service refresh`, which the installers run after replacing `beans` and the app runs at
/// every launch, so it changes only what needs it:
///
/// - an installed service moves onto `program`, keeping the home, port and relay it serves;
/// - it restarts only when its definition changed, its process is an old `beans` (the binary was
///   replaced under it), or it keeps failing to start; a healthy one is left running, and a
///   stopped one stays stopped;
/// - a definition Beans did not write is replaced with `fallback` (this command's home, port and
///   relay), and a same-named unit elsewhere that is a Beans definition is overridden;
/// - a service that runs a newer `beans` is left alone;
/// - an old `beans serve` still holding the port stops, service or not.
pub(crate) fn refresh(found: &Found, program: &str, fallback: Option<Definition>, version: &str) -> Plan {
    let mut plan = Plan::default();
    let running = found.active || found.running.is_some();
    let stale = found.holder.as_ref().is_some_and(|holder| classify(holder, version) == Holding::Old);
    let (wanted, notes) = match &found.stored {
        Stored::Absent => {
            if running {
                plan.notes.push(Note::Orphaned);
                plan.actions.push(Action::Stop);
            }
            if stale {
                plan.actions.push(Action::ClearPort { strict: false });
            }
            return plan;
        }
        Stored::Beans { definition, .. } => {
            let newer = found.program_version.as_deref().filter(|_| found.program_exists && definition.program != program)
                .filter(|theirs| compare(theirs, version) == Some(Ordering::Greater));
            if let Some(theirs) = newer {
                plan.notes.push(Note::NewerInstalled { program: definition.program.clone(), version: theirs.to_string() });
                // The service is not this beans's to change, but a leftover older than both, holding
                // the port instead of the service's own process, still goes.
                if stale && found.running.is_none() {
                    plan.actions.push(Action::ClearPort { strict: false });
                }
                return plan;
            }
            let wanted = Definition { program: program.to_string(), environment: definition.environment.clone() };
            (wanted.clone(), differences(found, &wanted))
        }
        Stored::Elsewhere { path, definition: Some(definition) } => {
            let wanted = Definition { program: program.to_string(), environment: definition.environment.clone() };
            (wanted, vec![Note::Overrides { path: path.clone() }])
        }
        // A same-named unit that is not a Beans definition is not a leftover of Beans's.
        Stored::Edited { .. } if fallback.is_some() => (fallback.unwrap(), vec![Note::ReplacedEdited]),
        Stored::Edited { .. } | Stored::Elsewhere { .. } => {
            plan.notes.push(match &found.stored {
                Stored::Elsewhere { path, .. } => Note::Elsewhere { path: path.clone() },
                _ => Note::Edited,
            });
            if stale {
                plan.actions.push(Action::ClearPort { strict: false });
            }
            return plan;
        }
    };
    plan.notes = notes;
    let rewrite = !matches!(&found.stored, Stored::Beans { definition, current: true, .. } if *definition == wanted);
    // Failing to start with the port free; while another beans serve holds it, a restart cannot
    // help, and restarting at every launch of the app would never settle.
    let failing = found.active && found.running.is_none() && found.holder.is_none();
    let restart = running && (rewrite || stale || failing);
    if !running {
        plan.notes.push(Note::Inactive);
    }
    if restart {
        plan.actions.push(Action::Stop);
    }
    if stale {
        plan.actions.push(Action::ClearPort { strict: false });
    }
    if rewrite {
        plan.actions.push(Action::Write(wanted));
    }
    if restart {
        plan.actions.push(Action::Start);
    }
    plan
}

/// `refresh` run by a `beans` from somewhere that goes away (see `transient_location`): the service
/// is not moved onto it, but a leftover older than it still stops when it, not the service's own
/// process, holds the port.
pub(crate) fn refresh_from_transient(found: &Found, program: &str, reason: &'static str, version: &str) -> Plan {
    let mut plan = Plan { notes: vec![Note::Transient { program: program.to_string(), reason }], ..Plan::default() };
    if found.running.is_none() && found.holder.as_ref().is_some_and(|holder| classify(holder, version) == Holding::Old) {
        plan.actions.push(Action::ClearPort { strict: false });
    }
    plan
}

/// `beans service uninstall`: no job and no definition under the service's name are left,
/// whatever state an earlier command left them in. A same-named file Beans cannot delete is
/// stopped and disabled.
pub(crate) fn uninstall(found: &Found) -> Plan {
    let mut plan = Plan::default();
    if found.active || found.running.is_some() {
        plan.actions.push(Action::Stop);
    }
    match &found.stored {
        Stored::Absent if found.active => plan.notes.push(Note::Orphaned),
        Stored::Absent => {}
        Stored::Beans { .. } => plan.actions.push(Action::Remove),
        Stored::Edited { .. } => {
            plan.notes.push(Note::Edited);
            plan.actions.push(Action::Remove);
        }
        Stored::Elsewhere { path, .. } => plan.notes.push(Note::Elsewhere { path: path.clone() }),
    }
    plan
}

/// `beans service status`: everything wrong with the service as found, for `program` of `version`.
pub(crate) fn findings(found: &Found, program: &str, version: &str) -> Vec<Note> {
    let mut notes = Vec::new();
    match &found.stored {
        Stored::Absent if found.active || found.running.is_some() => notes.push(Note::Orphaned),
        Stored::Absent => {}
        Stored::Edited { .. } => notes.push(Note::Edited),
        Stored::Elsewhere { path, .. } => notes.push(Note::Elsewhere { path: path.clone() }),
        Stored::Beans { definition, current, .. } => {
            if !found.program_exists {
                notes.push(Note::ProgramMissing { program: definition.program.clone() });
            } else if definition.program != program {
                notes.push(Note::OtherProgram { was: definition.program.clone(), now: program.to_string() });
            }
            if !current {
                notes.push(Note::OlderFormat);
            }
            if !found.active {
                notes.push(Note::Inactive);
            }
        }
    }
    if found.active && found.running.is_none() {
        notes.push(Note::NotRunning);
    }
    if !found.overrides.is_empty() {
        notes.push(Note::Overridden { paths: found.overrides.clone() });
    }
    if let Some(holder) = &found.holder {
        match classify(holder, version) {
            Holding::Old => notes.push(Note::OldProcess { port: holder.port, pid: holder.pid, version: holder.version.clone() }),
            // The service's own process, or another that keeps it from starting.
            _ if found.running.is_some() => {}
            _ => notes.push(Note::PortHeld { port: holder.port, pid: holder.pid, version: holder.version.clone() }),
        }
    }
    notes
}

/// What to do about one `beans serve` found on the port while clearing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Clearing {
    Stop { pid: u32, note: Note },
    Leave(Note),
    Refuse(Refusal),
}

/// Decides [`Action::ClearPort`] for `holder`, never for `this` process or its `parent`.
pub(crate) fn clearing(holder: &Holder, version: &str, strict: bool, this: u32, parent: Option<u32>) -> Clearing {
    let holding = classify(holder, version);
    let (port, pid, theirs) = (holder.port, holder.pid, holder.version.clone());
    match holding {
        Holding::Old => match pid.filter(|pid| *pid > 1 && *pid != this && Some(*pid) != parent) {
            Some(pid) => Clearing::Stop { pid, note: Note::StoppedOld { port, pid, version: theirs } },
            None => Clearing::Leave(Note::OldUnidentified { port, version: theirs }),
        },
        _ if strict => Clearing::Refuse(Refusal::InUse { port, pid, version: theirs, holding }),
        _ => Clearing::Leave(Note::PortHeld { port, pid, version: theirs }),
    }
}

/// Carries out a plan's steps on the computer.
pub(crate) trait Host {
    fn stop(&mut self) -> anyhow::Result<()>;
    /// Carries out [`Action::ClearPort`], returning what it stopped or left.
    fn clear_port(&mut self, strict: bool) -> anyhow::Result<Vec<Note>>;
    fn remove(&mut self) -> anyhow::Result<()>;
    /// Writes the definition whole, in one step: a write that fails leaves the previous one.
    fn write(&mut self, definition: &Definition) -> anyhow::Result<()>;
    /// Puts back a definition's exact text.
    fn restore(&mut self, text: &str) -> anyhow::Result<()>;
    fn start(&mut self) -> anyhow::Result<()>;
}

/// Carries out `plan`, returning what clearing the port did. When clearing the port, writing or
/// starting fails, the definition found before goes back (or, with none, the new one goes), and a
/// job that was running starts again, so a failed install leaves the service as it was rather than
/// half-replaced.
pub(crate) fn apply(plan: &Plan, found: &Found, host: &mut impl Host) -> anyhow::Result<Vec<Note>> {
    let mut notes = Vec::new();
    for action in &plan.actions {
        let result = match action {
            Action::Stop => host.stop(),
            Action::ClearPort { strict } => host.clear_port(*strict).map(|cleared| notes.extend(cleared)),
            Action::Remove => host.remove(),
            Action::Write(definition) => host.write(definition),
            Action::Start => host.start(),
        };
        let Err(error) = result else { continue };
        if matches!(action, Action::Stop | Action::Remove) {
            // Nothing new was written; what is left is what was there, less what already went.
            return Err(error);
        }
        return Err(match roll_back(found, host) {
            Ok(()) => error.context("the service was put back as it was"),
            Err(back) => error.context(format!("putting the previous service back also failed: {back:#}")),
        });
    }
    Ok(notes)
}

fn roll_back(found: &Found, host: &mut impl Host) -> anyhow::Result<()> {
    match &found.stored {
        Stored::Beans { text, .. } | Stored::Edited { text } => host.restore(text)?,
        Stored::Absent | Stored::Elsewhere { .. } => host.remove()?,
    }
    // A job left without its definition has nothing to start again; it stays stopped.
    if found.active && found.stored != Stored::Absent {
        host.start()?;
    }
    Ok(())
}

/// What a stored definition passes on, from which `refresh` and `status` read the home and port
/// it serves when no shell names them; empty for anything but a Beans definition.
pub(crate) fn served_environment_of(stored: &Stored) -> &[(String, String)] {
    match stored {
        Stored::Beans { definition, .. } | Stored::Elsewhere { definition: Some(definition), .. } => &definition.environment,
        _ => &[],
    }
}

// MARK: - Reading definitions back

/// Text between XML tags as a parser reads it: line ends normalized, references resolved.
/// `None` for a reference or character the plists Beans writes never hold.
#[cfg(any(target_os = "macos", test))]
fn xml_text(raw: &str) -> Option<String> {
    let raw = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut text = String::new();
    let mut rest = raw.as_str();
    while let Some(start) = rest.find('&') {
        text.push_str(&rest[..start]);
        let end = rest[start..].find(';')? + start;
        let reference = &rest[start + 1..end];
        text.push(match reference {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let number = reference.strip_prefix('#')?;
                let code = match number.strip_prefix('x') {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => number.parse().ok()?,
                };
                char::from_u32(code).filter(|c| super::xml_char(*c))?
            }
        });
        rest = &rest[end + 1..];
    }
    text.push_str(rest);
    Some(text)
}

/// The contents of every `<tag>` in `xml`, in order; `None` if one holds markup.
#[cfg(any(target_os = "macos", test))]
fn xml_elements(xml: &str, tag: &str) -> Option<Vec<String>> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let body = &rest[start + open.len()..];
        let end = body.find(&close)?;
        if body[..end].contains('<') {
            return None;
        }
        found.push(xml_text(&body[..end])?);
        rest = &body[end + close.len()..];
    }
    Some(found)
}

/// What a launchd agent's plist holds, for the label `label` and log `log`.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn read_launchd(text: &str, label: &str, log: &str) -> Stored {
    let edited = || Stored::Edited { text: text.to_string() };
    let read = || -> Option<Definition> {
        let (_, after_label) = text.split_once("<key>Label</key>")?;
        if xml_elements(after_label.split("<key>").next()?, "string")? != [label] {
            return None;
        }
        let (_, after) = text.split_once("<key>ProgramArguments</key>")?;
        let (arguments, _) = after.split_once("</array>")?;
        let arguments = xml_elements(arguments, "string")?;
        let [program, serve] = <[String; 2]>::try_from(arguments).ok()?;
        if serve != "serve" {
            return None;
        }
        let (_, after) = text.split_once("<key>EnvironmentVariables</key>")?;
        let (variables, _) = after.split_once("</dict>")?;
        let names = xml_elements(variables, "key")?;
        let values = xml_elements(variables, "string")?;
        if names.len() != values.len() {
            return None;
        }
        let mut environment: Vec<(String, String)> = names.into_iter().zip(values).collect();
        if environment.first() != Some(&("BEANS_SERVICE".to_string(), "1".to_string())) {
            return None;
        }
        environment.remove(0);
        Some(Definition { program, environment })
    };
    match read() {
        Some(definition) => {
            let current = super::launchd_plist(label, &definition.program, log, &definition.environment).is_ok_and(|written| written == text);
            Stored::Beans { definition, current, text: text.to_string() }
        }
        None => edited(),
    }
}

/// One setting's value split into words as systemd splits it (quotes removed, C escapes and `%%`
/// read), for the subset Beans writes. `None` for anything else.
#[cfg(any(target_os = "linux", test))]
fn systemd_words(value: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut chars = value.chars().peekable();
    loop {
        while chars.next_if(|c| *c == ' ' || *c == '\t').is_some() {}
        if chars.peek().is_none() {
            return Some(words);
        }
        let mut word = String::new();
        let mut quote = None;
        while let Some(c) = chars.next() {
            match (c, quote) {
                ('\\', _) => word.push(match chars.next()? {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    'x' => {
                        let hex: String = [chars.next()?, chars.next()?].into_iter().collect();
                        char::from_u32(u32::from_str_radix(&hex, 16).ok()?).filter(|c| *c != '\0')?
                    }
                    c @ ('\\' | '"' | '\'' | ' ') => c,
                    _ => return None,
                }),
                ('"' | '\'', None) => quote = Some(c),
                (c, Some(open)) if c == open => quote = None,
                (' ' | '\t', None) => break,
                ('%', _) => {
                    if chars.next()? != '%' {
                        return None;
                    }
                    word.push('%');
                }
                (c, _) => word.push(c),
            }
        }
        if quote.is_some() {
            return None;
        }
        words.push(word);
    }
}

/// What a systemd unit Beans wrote holds.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn read_systemd(text: &str) -> Stored {
    let read = || -> Option<Definition> {
        let mut program = None;
        let mut environment = Vec::new();
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("ExecStart=") {
                let words = systemd_words(value)?;
                let [path, serve] = <[String; 2]>::try_from(words).ok()?;
                if serve != "serve" || program.replace(path).is_some() {
                    return None;
                }
            } else if let Some(value) = line.strip_prefix("Environment=") {
                for word in systemd_words(value)? {
                    let (name, value) = word.split_once('=')?;
                    environment.push((name.to_string(), value.to_string()));
                }
            }
        }
        if environment.first() != Some(&("BEANS_SERVICE".to_string(), "1".to_string())) {
            return None;
        }
        environment.remove(0);
        Some(Definition { program: program?, environment })
    };
    match read() {
        Some(definition) => {
            // The release only decides what to refuse, never how a kept value is written.
            let current = super::systemd_unit(&definition.program, &definition.environment, Some(u32::MAX)).is_ok_and(|written| written == text);
            Stored::Beans { definition, current, text: text.to_string() }
        }
        None => Stored::Edited { text: text.to_string() },
    }
}

/// A Windows command line split as `CommandLineToArgvW` and Rust's `std::env::args` both split
/// it. `None` for `""` inside quotes, where they differ, and which Beans never writes.
#[cfg(any(windows, test))]
fn windows_argv(line: &str) -> Option<Vec<String>> {
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
            return Some(argv);
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
                            if quoted && chars.peek() == Some(&'"') {
                                return None;
                            }
                            quoted = !quoted;
                        }
                    } else {
                        arg.push_str(&"\\".repeat(backslashes));
                    }
                }
                '"' => {
                    if quoted && chars.peek() == Some(&'"') {
                        return None;
                    }
                    quoted = !quoted;
                }
                ' ' | '\t' if !quoted => break,
                c => arg.push(c),
            }
        }
        argv.push(arg);
    }
}

/// What the Run key's command line, as Beans writes it, holds.
#[cfg(any(windows, test))]
pub(crate) fn read_run_key(line: &str) -> Stored {
    let read = || -> Option<Definition> {
        let argv = windows_argv(line)?;
        if argv.get(1..3)? != ["service", "run"] {
            return None;
        }
        let mut environment = Vec::new();
        for pair in argv[3..].chunks(2) {
            let [flag, assignment] = pair else { return None };
            let (name, value) = assignment.split_once('=').filter(|_| flag == "--env")?;
            environment.push((name.to_string(), value.to_string()));
        }
        Some(Definition { program: argv[0].clone(), environment })
    };
    match read() {
        Some(definition) => {
            let current = super::windows_command_line(&definition.program, &definition.environment).is_ok_and(|written| written == line);
            Stored::Beans { definition, current, text: line.to_string() }
        }
        None => Stored::Edited { text: line.to_string() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW: &str = "/example/user/.local/bin/beans";
    const OLD: &str = "/example/user/.cargo/bin/beans";
    const VERSION: &str = "0.2.0";

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect()
    }

    fn definition(program: &str, home: &str) -> Definition {
        Definition { program: program.into(), environment: env(&[("BEANS_HOME", home), ("BEANS_PORT", "4874")]) }
    }

    fn beans(program: &str, home: &str, current: bool) -> Stored {
        Stored::Beans { definition: definition(program, home), current, text: format!("text of {program} {home}") }
    }

    fn found(stored: Stored, active: bool, running: Option<u32>, program_exists: bool) -> Found {
        Found { stored, active, running, program_exists, program_version: None, holder: None, overrides: Vec::new() }
    }

    fn holder(pid: u32, version: &str) -> Holder {
        Holder { port: 4874, pid: Some(pid), version: Some(version.into()), executable: Some(NEW.into()), executable_exists: true }
    }

    fn holding(found: Found, holder: Holder) -> Found {
        Found { holder: Some(holder), ..found }
    }

    // MARK: Which processes are leftovers

    #[test]
    fn versions_compare_by_release_numbers() {
        assert_eq!(compare("0.1.9", "0.1.10"), Some(Ordering::Less));
        assert_eq!(compare("0.1.10", "0.1.10"), Some(Ordering::Equal));
        assert_eq!(compare("1.0", "1.0.0"), Some(Ordering::Equal));
        assert_eq!(compare("v0.2.0-rc1", "0.2.0"), Some(Ordering::Equal));
        assert_eq!(compare("1.0.0+build", "0.9.99"), Some(Ordering::Greater));
        assert_eq!(compare("banana", "0.2.0"), None);
        assert_eq!(compare("1.2.3.4", "1.2.3"), None);
    }

    #[test]
    fn only_an_older_beans_or_one_whose_binary_is_gone_is_a_leftover() {
        assert_eq!(classify(&holder(1, "0.1.10"), VERSION), Holding::Old);
        assert_eq!(classify(&holder(1, VERSION), VERSION), Holding::Current);
        assert_eq!(classify(&holder(1, "0.3.0"), VERSION), Holding::Newer);
        assert_eq!(classify(&Holder { version: None, ..holder(1, VERSION) }, VERSION), Holding::Unknown);
        assert_eq!(classify(&Holder { version: Some("not beans".into()), ..holder(1, VERSION) }, VERSION), Holding::Unknown);
        // This version, but its binary was deleted: nothing will start it again as it is.
        assert_eq!(classify(&Holder { executable_exists: false, ..holder(1, VERSION) }, VERSION), Holding::Old);
        // An older beans, before hello named its binary, is still old.
        assert_eq!(classify(&Holder { executable: None, executable_exists: false, ..holder(1, "0.1.0") }, VERSION), Holding::Old);
    }

    #[test]
    fn clearing_stops_leftovers_and_never_this_process_its_parent_or_a_current_one() {
        let old = holder(41, "0.1.0");
        assert_eq!(clearing(&old, VERSION, true, 1, Some(2)), Clearing::Stop { pid: 41, note: Note::StoppedOld { port: 4874, pid: 41, version: Some("0.1.0".into()) } });
        assert!(matches!(clearing(&old, VERSION, true, 41, None), Clearing::Leave(Note::OldUnidentified { .. })), "never itself");
        assert!(matches!(clearing(&old, VERSION, true, 1, Some(41)), Clearing::Leave(Note::OldUnidentified { .. })), "never the app that ran it");
        assert!(matches!(clearing(&Holder { pid: None, ..old }, VERSION, true, 1, None), Clearing::Leave(Note::OldUnidentified { .. })));
        for (theirs, holding) in [(VERSION, Holding::Current), ("9.0.0", Holding::Newer)] {
            let other = holder(7, theirs);
            assert_eq!(clearing(&other, VERSION, true, 1, None), Clearing::Refuse(Refusal::InUse { port: 4874, pid: Some(7), version: Some(theirs.into()), holding }));
            assert_eq!(clearing(&other, VERSION, false, 1, None), Clearing::Leave(Note::PortHeld { port: 4874, pid: Some(7), version: Some(theirs.into()) }));
        }
    }

    // MARK: Install

    #[test]
    fn install_on_a_clean_computer_writes_and_starts() {
        let plan = install(&found(Stored::Absent, false, None, false), definition(NEW, "/h"), VERSION).unwrap();
        assert_eq!(plan.actions, [Action::ClearPort { strict: true }, Action::Write(definition(NEW, "/h")), Action::Start]);
        assert!(plan.notes.is_empty());
    }

    #[test]
    fn install_over_a_running_service_stops_it_before_replacing_it() {
        let before = holding(found(beans(NEW, "/h", true), true, Some(7), true), holder(7, VERSION));
        let plan = install(&before, definition(NEW, "/h"), VERSION).unwrap();
        assert_eq!(plan.actions, [Action::Stop, Action::ClearPort { strict: true }, Action::Write(definition(NEW, "/h")), Action::Start]);
        assert!(plan.notes.is_empty(), "the same definition: nothing to report");
    }

    #[test]
    fn install_over_a_service_of_a_moved_beans_replaces_it_and_says_so() {
        let plan = install(&found(beans(OLD, "/h", true), true, None, false), definition(NEW, "/h"), VERSION).unwrap();
        assert_eq!(plan.actions[0], Action::Stop);
        assert_eq!(plan.notes, [Note::ProgramMissing { program: OLD.into() }]);
    }

    #[test]
    fn install_over_a_service_of_another_beans_names_both() {
        let plan = install(&found(beans(OLD, "/h", true), true, Some(3), true), definition(NEW, "/h"), VERSION).unwrap();
        assert_eq!(plan.notes, [Note::OtherProgram { was: OLD.into(), now: NEW.into() }]);
        assert_eq!(plan.actions[0], Action::Stop, "the old beans stops before the new one starts");
    }

    #[test]
    fn install_over_another_home_or_an_older_format_reports_what_changes_without_values() {
        let mut wanted = definition(NEW, "/other");
        wanted.environment.push(("BEANS_RELAY_URL".into(), "https://relay/?token=secret".into()));
        let plan = install(&found(beans(NEW, "/h", false), true, Some(1), true), wanted, VERSION).unwrap();
        assert_eq!(plan.notes, [Note::EnvironmentChanged { names: vec!["BEANS_HOME".into(), "BEANS_RELAY_URL".into()] }, Note::OlderFormat]);
        assert!(!plan.notes.iter().map(ToString::to_string).collect::<String>().contains("secret"));
    }

    #[test]
    fn install_after_an_interrupted_install_finishes_it() {
        let plan = install(&found(beans(NEW, "/h", true), false, None, true), definition(NEW, "/h"), VERSION).unwrap();
        assert_eq!(plan.actions, [Action::ClearPort { strict: true }, Action::Write(definition(NEW, "/h")), Action::Start]);
    }

    #[test]
    fn install_stops_a_job_left_running_without_its_definition() {
        let plan = install(&found(Stored::Absent, true, Some(9), false), definition(NEW, "/h"), VERSION).unwrap();
        assert_eq!(plan.actions, [Action::Stop, Action::ClearPort { strict: true }, Action::Write(definition(NEW, "/h")), Action::Start]);
        assert_eq!(plan.notes, [Note::Orphaned]);
    }

    #[test]
    fn install_replaces_a_definition_beans_did_not_write() {
        let plan = install(&found(Stored::Edited { text: "hand-made".into() }, true, Some(2), false), definition(NEW, "/h"), VERSION).unwrap();
        assert_eq!(plan.actions, [Action::Stop, Action::ClearPort { strict: true }, Action::Write(definition(NEW, "/h")), Action::Start]);
        assert_eq!(plan.notes, [Note::ReplacedEdited]);
        let path = "/etc/systemd/user/beans.service";
        let plan = install(&found(Stored::Elsewhere { path: path.into(), definition: None }, true, Some(2), false), definition(NEW, "/h"), VERSION).unwrap();
        assert_eq!(plan.actions, [Action::Stop, Action::ClearPort { strict: true }, Action::Write(definition(NEW, "/h")), Action::Start]);
        assert_eq!(plan.notes, [Note::Overrides { path: path.into() }]);
    }

    #[test]
    fn install_stops_an_old_beans_left_on_the_port_but_refuses_a_current_one() {
        let plan = install(&holding(found(Stored::Absent, false, None, false), holder(50, "0.1.0")), definition(NEW, "/h"), VERSION).unwrap();
        assert_eq!(plan.actions[0], Action::ClearPort { strict: true });
        for (theirs, holding_kind) in [(VERSION, Holding::Current), ("1.0.0", Holding::Newer)] {
            let error = install(&holding(found(Stored::Absent, false, None, false), holder(50, theirs)), definition(NEW, "/h"), VERSION).unwrap_err();
            assert_eq!(error, Refusal::InUse { port: 4874, pid: Some(50), version: Some(theirs.into()), holding: holding_kind });
        }
        let unknown = Holder { version: None, ..holder(50, VERSION) };
        assert!(install(&holding(found(Stored::Absent, false, None, false), unknown), definition(NEW, "/h"), VERSION).is_err());
        // While the service runs, the process on the port is its own.
        assert!(install(&holding(found(beans(NEW, "/h", true), true, Some(8), true), holder(9, VERSION)), definition(NEW, "/h"), VERSION).is_ok());
    }

    // MARK: Refresh

    #[test]
    fn refresh_without_a_service_does_nothing() {
        assert_eq!(refresh(&found(Stored::Absent, false, None, false), NEW, None, VERSION), Plan::default());
        assert_eq!(refresh(&holding(found(Stored::Absent, false, None, false), holder(5, VERSION)), NEW, None, VERSION), Plan::default(), "the app's own Runner stays");
    }

    #[test]
    fn refresh_without_a_service_still_stops_an_old_beans_on_the_port() {
        let plan = refresh(&holding(found(Stored::Absent, false, None, false), holder(5, "0.1.0")), NEW, None, VERSION);
        assert_eq!(plan.actions, [Action::ClearPort { strict: false }]);
    }

    #[test]
    fn refresh_leaves_a_healthy_current_service_running() {
        // The app runs refresh at every launch: nothing restarts without a reason.
        let before = holding(found(beans(NEW, "/h", true), true, Some(4), true), holder(4, VERSION));
        assert_eq!(refresh(&before, NEW, None, VERSION), Plan::default());
    }

    #[test]
    fn refresh_restarts_a_service_whose_binary_was_replaced_under_it() {
        // The installer renamed a new binary over the same path; the process is still the old one.
        let before = holding(found(beans(NEW, "/h", true), true, Some(4), true), holder(4, "0.1.10"));
        let plan = refresh(&before, NEW, None, VERSION);
        assert_eq!(plan.actions, [Action::Stop, Action::ClearPort { strict: false }, Action::Start]);
        assert!(plan.notes.is_empty());
    }

    #[test]
    fn refresh_restarts_a_service_that_keeps_failing_to_start() {
        let plan = refresh(&found(beans(NEW, "/h", true), true, None, true), NEW, None, VERSION);
        assert_eq!(plan.actions, [Action::Stop, Action::Start]);
    }

    #[test]
    fn refresh_does_not_restart_a_service_the_apps_runner_keeps_from_starting() {
        let before = holding(found(beans(NEW, "/h", true), true, None, true), holder(9, VERSION));
        let plan = refresh(&before, NEW, None, VERSION);
        assert!(plan.actions.is_empty(), "{plan:?}");
    }

    #[test]
    fn refresh_still_stops_a_leftover_when_the_service_runs_a_newer_beans() {
        // Stopped, or loaded but kept from starting by the leftover on its port.
        for active in [false, true] {
            let mut before = holding(found(beans(OLD, "/h", true), active, None, true), holder(9, "0.0.1"));
            before.program_version = Some("9.0.0".into());
            let plan = refresh(&before, NEW, None, VERSION);
            assert_eq!(plan.actions, [Action::ClearPort { strict: false }], "active: {active}");
            assert_eq!(plan.notes, [Note::NewerInstalled { program: OLD.into(), version: "9.0.0".into() }]);
        }
    }

    #[test]
    fn refresh_from_a_temporary_location_leaves_the_service_and_clears_only_leftovers() {
        let before = holding(found(beans(OLD, "/h", true), true, None, false), holder(9, "0.0.1"));
        let plan = refresh_from_transient(&before, "/tmp/.mount_x/beans", "an AppImage's temporary mount", VERSION);
        assert_eq!(plan.actions, [Action::ClearPort { strict: false }], "even a service whose program is gone is not moved there");
        let running = holding(found(beans(OLD, "/h", true), true, Some(3), true), holder(3, "0.0.1"));
        assert!(refresh_from_transient(&running, "/tmp/x/beans", "the temporary folder", VERSION).actions.is_empty(), "the service's own process");
        assert!(refresh_from_transient(&found(Stored::Absent, false, None, false), "/tmp/x/beans", "the temporary folder", VERSION).actions.is_empty());
    }

    #[test]
    fn refresh_moves_a_service_onto_this_beans_keeping_its_home() {
        let plan = refresh(&found(beans(OLD, "/account", true), true, Some(4), false), NEW, None, VERSION);
        assert_eq!(plan.actions, [Action::Stop, Action::Write(definition(NEW, "/account")), Action::Start]);
        assert_eq!(plan.notes, [Note::ProgramMissing { program: OLD.into() }]);
    }

    #[test]
    fn refresh_never_moves_a_service_onto_an_older_beans() {
        let mut before = found(beans(OLD, "/h", true), true, Some(4), true);
        before.program_version = Some("0.9.0".into());
        let plan = refresh(&before, NEW, None, VERSION);
        assert!(plan.actions.is_empty());
        assert_eq!(plan.notes, [Note::NewerInstalled { program: OLD.into(), version: "0.9.0".into() }]);
        before.program_version = Some("0.1.0".into());
        assert_eq!(refresh(&before, NEW, None, VERSION).actions, [Action::Stop, Action::Write(definition(NEW, "/h")), Action::Start]);
    }

    #[test]
    fn refresh_rewrites_an_older_format_in_place() {
        let plan = refresh(&found(beans(NEW, "/h", false), true, Some(4), true), NEW, None, VERSION);
        assert_eq!(plan.actions, [Action::Stop, Action::Write(definition(NEW, "/h")), Action::Start]);
        assert_eq!(plan.notes, [Note::OlderFormat]);
    }

    #[test]
    fn refresh_keeps_a_stopped_service_stopped_but_points_it_at_this_beans() {
        let plan = refresh(&found(beans(OLD, "/h", true), false, None, true), NEW, None, VERSION);
        assert_eq!(plan.actions, [Action::Write(definition(NEW, "/h"))]);
        assert_eq!(plan.notes, [Note::OtherProgram { was: OLD.into(), now: NEW.into() }, Note::Inactive]);
        let plan = refresh(&found(beans(NEW, "/h", true), false, None, true), NEW, None, VERSION);
        assert!(plan.actions.is_empty());
    }

    #[test]
    fn refresh_stops_a_job_left_without_its_definition() {
        let plan = refresh(&found(Stored::Absent, true, Some(5), false), NEW, None, VERSION);
        assert_eq!(plan.actions, [Action::Stop]);
        assert_eq!(plan.notes, [Note::Orphaned]);
    }

    #[test]
    fn refresh_replaces_a_damaged_definition_with_this_commands_service() {
        let fallback = definition(NEW, "/default");
        let plan = refresh(&found(Stored::Edited { text: "x".into() }, true, Some(5), false), NEW, Some(fallback.clone()), VERSION);
        assert_eq!(plan.actions, [Action::Stop, Action::Write(fallback.clone()), Action::Start]);
        assert_eq!(plan.notes, [Note::ReplacedEdited]);
        let plan = refresh(&found(Stored::Edited { text: "x".into() }, false, None, false), NEW, Some(fallback.clone()), VERSION);
        assert_eq!(plan.actions, [Action::Write(fallback)]);
        // Without a service of its own to put there, it is left as it is.
        let plan = refresh(&found(Stored::Edited { text: "x".into() }, true, Some(5), false), NEW, None, VERSION);
        assert!(plan.actions.is_empty());
        assert_eq!(plan.notes, [Note::Edited]);
    }

    #[test]
    fn refresh_overrides_a_beans_unit_elsewhere_but_not_another_programs() {
        let path = "/etc/systemd/user/beans.service";
        let theirs = Stored::Elsewhere { path: path.into(), definition: Some(definition(OLD, "/h")) };
        let plan = refresh(&found(theirs, true, Some(5), true), NEW, Some(definition(NEW, "/default")), VERSION);
        assert_eq!(plan.actions, [Action::Stop, Action::Write(definition(NEW, "/h")), Action::Start], "keeps that unit's home");
        assert_eq!(plan.notes, [Note::Overrides { path: path.into() }]);
        let other = Stored::Elsewhere { path: path.into(), definition: None };
        let plan = refresh(&found(other, true, Some(5), true), NEW, Some(definition(NEW, "/default")), VERSION);
        assert!(plan.actions.is_empty());
        assert_eq!(plan.notes, [Note::Elsewhere { path: path.into() }]);
    }

    // MARK: Uninstall

    #[test]
    fn uninstall_removes_every_state_an_earlier_command_left() {
        let cases = [
            (found(beans(NEW, "/h", true), true, Some(1), true), vec![Action::Stop, Action::Remove]),
            (found(beans(NEW, "/h", true), false, None, true), vec![Action::Remove]),
            (found(beans(OLD, "/h", false), true, None, false), vec![Action::Stop, Action::Remove]),
            (found(Stored::Absent, true, Some(1), false), vec![Action::Stop]),
            (found(Stored::Absent, false, Some(1), false), vec![Action::Stop]),
            (found(Stored::Edited { text: "x".into() }, false, None, false), vec![Action::Remove]),
            (found(Stored::Absent, false, None, false), vec![]),
        ];
        for (found, actions) in cases {
            assert_eq!(uninstall(&found).actions, actions, "{found:?}");
        }
    }

    #[test]
    fn uninstall_stops_but_cannot_delete_a_unit_elsewhere() {
        let path = "/etc/systemd/user/beans.service";
        let plan = uninstall(&found(Stored::Elsewhere { path: path.into(), definition: None }, true, Some(1), true));
        assert_eq!(plan.actions, [Action::Stop]);
        assert_eq!(plan.notes, [Note::Elsewhere { path: path.into() }]);
    }

    // MARK: Status

    #[test]
    fn status_names_each_problem() {
        assert!(findings(&found(Stored::Absent, false, None, false), NEW, VERSION).is_empty());
        assert!(findings(&holding(found(beans(NEW, "/h", true), true, Some(1), true), holder(1, VERSION)), NEW, VERSION).is_empty());
        assert_eq!(findings(&found(beans(OLD, "/h", false), true, None, false), NEW, VERSION),
            [Note::ProgramMissing { program: OLD.into() }, Note::OlderFormat, Note::NotRunning]);
        assert_eq!(findings(&found(beans(OLD, "/h", true), false, None, true), NEW, VERSION),
            [Note::OtherProgram { was: OLD.into(), now: NEW.into() }, Note::Inactive]);
        assert_eq!(findings(&found(Stored::Absent, true, Some(3), false), NEW, VERSION), [Note::Orphaned]);
        assert_eq!(findings(&found(Stored::Edited { text: "x".into() }, true, Some(3), false), NEW, VERSION), [Note::Edited]);
        assert_eq!(findings(&holding(found(Stored::Absent, false, None, false), holder(6, "0.1.0")), NEW, VERSION),
            [Note::OldProcess { port: 4874, pid: Some(6), version: Some("0.1.0".into()) }]);
        assert_eq!(findings(&holding(found(beans(NEW, "/h", true), true, None, true), holder(6, VERSION)), NEW, VERSION),
            [Note::NotRunning, Note::PortHeld { port: 4874, pid: Some(6), version: Some(VERSION.into()) }], "why it cannot start");
    }

    // MARK: Carrying a plan out

    /// A service manager and a port in memory: what they hold, and every call, failing the step named.
    #[derive(Default)]
    struct FakeHost {
        stored: Option<String>,
        active: bool,
        /// The process on the port: killed by a clear when it is a leftover.
        port: Option<Holder>,
        calls: Vec<String>,
        fail: Option<&'static str>,
    }

    impl FakeHost {
        fn step(&mut self, name: &'static str) -> anyhow::Result<()> {
            self.calls.push(name.to_string());
            if self.fail == Some(name) {
                self.fail = None;
                anyhow::bail!("{name} failed");
            }
            Ok(())
        }
    }

    impl Host for FakeHost {
        fn stop(&mut self) -> anyhow::Result<()> {
            self.step("stop")?;
            self.active = false;
            Ok(())
        }
        fn clear_port(&mut self, strict: bool) -> anyhow::Result<Vec<Note>> {
            self.step("clear")?;
            let Some(holder) = self.port.clone() else { return Ok(Vec::new()) };
            match clearing(&holder, VERSION, strict, 1, None) {
                Clearing::Stop { note, .. } => {
                    self.port = None;
                    Ok(vec![note])
                }
                Clearing::Leave(note) => Ok(vec![note]),
                Clearing::Refuse(refusal) => Err(refusal.into()),
            }
        }
        fn remove(&mut self) -> anyhow::Result<()> {
            self.step("remove")?;
            self.stored = None;
            Ok(())
        }
        fn write(&mut self, definition: &Definition) -> anyhow::Result<()> {
            self.step("write")?;
            self.stored = Some(format!("text of {} {}", definition.program, definition.environment[0].1));
            Ok(())
        }
        fn restore(&mut self, text: &str) -> anyhow::Result<()> {
            self.step("restore")?;
            self.stored = Some(text.to_string());
            Ok(())
        }
        fn start(&mut self) -> anyhow::Result<()> {
            self.step("start")?;
            self.active = true;
            Ok(())
        }
    }

    fn host_for(found: &Found) -> FakeHost {
        let stored = match &found.stored {
            Stored::Beans { text, .. } | Stored::Edited { text } => Some(text.clone()),
            _ => None,
        };
        FakeHost { stored, active: found.active, port: found.holder.clone(), ..FakeHost::default() }
    }

    #[test]
    fn a_replacement_that_starts_leaves_only_the_new_service() {
        let before = found(beans(OLD, "/h", true), true, Some(1), true);
        let mut host = host_for(&before);
        apply(&install(&before, definition(NEW, "/h"), VERSION).unwrap(), &before, &mut host).unwrap();
        assert_eq!(host.calls, ["stop", "clear", "write", "start"]);
        assert_eq!((host.stored.as_deref(), host.active), (Some("text of /example/user/.local/bin/beans /h"), true));
    }

    #[test]
    fn an_old_beans_left_on_the_port_is_stopped_before_the_service_starts() {
        let before = holding(found(Stored::Absent, false, None, false), holder(77, "0.1.0"));
        let mut host = host_for(&before);
        let notes = apply(&install(&before, definition(NEW, "/h"), VERSION).unwrap(), &before, &mut host).unwrap();
        assert_eq!(host.calls, ["clear", "write", "start"]);
        assert_eq!(notes, [Note::StoppedOld { port: 4874, pid: 77, version: Some("0.1.0".into()) }]);
        assert!(host.port.is_none());
    }

    #[test]
    fn a_current_beans_found_on_the_port_only_after_stopping_the_service_puts_it_back() {
        // The process on the port turned out not to be the service's: it is still there.
        let before = holding(found(beans(OLD, "/h", true), true, Some(1), true), holder(2, VERSION));
        let mut host = host_for(&before);
        let error = apply(&install(&before, definition(NEW, "/h"), VERSION).unwrap(), &before, &mut host).unwrap_err();
        assert_eq!(host.calls, ["stop", "clear", "restore", "start"]);
        assert_eq!((host.stored.as_deref(), host.active), (Some("text of /example/user/.cargo/bin/beans /h"), true));
        assert!(format!("{error:#}").contains("already answers on port 4874"), "{error:#}");
        assert!(host.port.is_some(), "never stopped");
    }

    #[test]
    fn a_replacement_that_cannot_start_puts_the_previous_service_back_and_running() {
        let before = found(beans(OLD, "/h", true), true, Some(1), true);
        let mut host = FakeHost { fail: Some("start"), ..host_for(&before) };
        let error = apply(&install(&before, definition(NEW, "/h"), VERSION).unwrap(), &before, &mut host).unwrap_err();
        assert_eq!(host.calls, ["stop", "clear", "write", "start", "restore", "start"]);
        assert_eq!((host.stored.as_deref(), host.active), (Some("text of /example/user/.cargo/bin/beans /h"), true));
        let message = format!("{error:#}");
        assert!(message.contains("start failed") && message.contains("put back as it was"), "{message}");
    }

    #[test]
    fn a_first_install_that_cannot_start_leaves_nothing_behind() {
        let before = found(Stored::Absent, false, None, false);
        let mut host = FakeHost { fail: Some("start"), ..host_for(&before) };
        assert!(apply(&install(&before, definition(NEW, "/h"), VERSION).unwrap(), &before, &mut host).is_err());
        assert_eq!(host.calls, ["clear", "write", "start", "remove"]);
        assert_eq!((host.stored.as_deref(), host.active), (None, false), "no half-installed service");
    }

    #[test]
    fn a_write_that_fails_restarts_the_previous_service() {
        let before = found(beans(NEW, "/h", false), true, Some(1), true);
        let mut host = FakeHost { fail: Some("write"), ..host_for(&before) };
        assert!(apply(&refresh(&before, NEW, None, VERSION), &before, &mut host).is_err());
        assert_eq!(host.calls, ["stop", "write", "restore", "start"]);
        assert_eq!((host.stored.as_deref(), host.active), (Some("text of /example/user/.local/bin/beans /h"), true));
    }

    #[test]
    fn a_stop_that_fails_changes_nothing() {
        let before = found(beans(OLD, "/h", true), true, Some(1), true);
        let mut host = FakeHost { fail: Some("stop"), ..host_for(&before) };
        assert!(apply(&install(&before, definition(NEW, "/h"), VERSION).unwrap(), &before, &mut host).is_err());
        assert_eq!(host.calls, ["stop"]);
        assert_eq!((host.stored.as_deref(), host.active), (Some("text of /example/user/.cargo/bin/beans /h"), true));
    }

    #[test]
    fn a_rollback_that_also_fails_says_both() {
        let before = found(beans(OLD, "/h", true), true, Some(1), true);
        struct Twice(FakeHost);
        impl Host for Twice {
            fn stop(&mut self) -> anyhow::Result<()> { self.0.stop() }
            fn clear_port(&mut self, strict: bool) -> anyhow::Result<Vec<Note>> { self.0.clear_port(strict) }
            fn remove(&mut self) -> anyhow::Result<()> { self.0.remove() }
            fn write(&mut self, definition: &Definition) -> anyhow::Result<()> { self.0.write(definition) }
            fn restore(&mut self, _: &str) -> anyhow::Result<()> { anyhow::bail!("disk full") }
            fn start(&mut self) -> anyhow::Result<()> { self.0.start() }
        }
        let mut twice = Twice(FakeHost { fail: Some("start"), ..host_for(&before) });
        let message = format!("{:#}", apply(&install(&before, definition(NEW, "/h"), VERSION).unwrap(), &before, &mut twice).unwrap_err());
        assert!(message.contains("start failed") && message.contains("putting the previous service back also failed: disk full"), "{message}");
    }

    #[test]
    fn uninstall_then_install_again_from_any_state_ends_clean() {
        let states = [
            found(beans(OLD, "/h", false), true, None, false),
            found(Stored::Absent, true, Some(1), false),
            found(Stored::Edited { text: "x".into() }, false, None, false),
            found(beans(NEW, "/h", true), false, None, true),
        ];
        for before in states {
            let mut host = host_for(&before);
            apply(&uninstall(&before), &before, &mut host).unwrap();
            assert_eq!((host.stored.as_deref(), host.active), (None, false), "{before:?}");
            let clean = found(Stored::Absent, false, None, false);
            apply(&install(&clean, definition(NEW, "/h"), VERSION).unwrap(), &clean, &mut host).unwrap();
            assert_eq!((host.stored.as_deref(), host.active), (Some("text of /example/user/.local/bin/beans /h"), true));
        }
    }

    #[test]
    fn refresh_twice_changes_nothing_the_second_time() {
        // The app runs refresh at every launch: the second run must find nothing to do.
        let before = holding(found(beans(OLD, "/h", false), true, Some(1), false), holder(1, "0.1.0"));
        let mut host = host_for(&before);
        apply(&refresh(&before, NEW, None, VERSION), &before, &mut host).unwrap();
        let text = host.stored.clone().unwrap();
        let after = holding(found(Stored::Beans { definition: definition(NEW, "/h"), current: true, text }, true, Some(2), true), holder(2, VERSION));
        assert_eq!(refresh(&after, NEW, None, VERSION), Plan::default());
    }


    // MARK: Every state, every failure
    //
    // A computer simulated whole: the service's file or value, whether the service manager has
    // the job and runs it, the process on the port, which program is where, and a step that fails.
    // Random starting states, every step made to fail in turn, and the properties that must hold
    // after any command from any of them.

    /// A deterministic generator, so a failure names a seed that reproduces it.
    struct Dice(u64);

    impl Dice {
        fn roll(&mut self, sides: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % sides
        }
        fn chance(&mut self, one_in: u64) -> bool {
            self.roll(one_in) == 0
        }
    }

    /// What runs on the port.
    #[derive(Debug, Clone, PartialEq)]
    enum Process {
        /// The service's own `beans serve`, of this version or not.
        Service { version: String },
        /// Something else: a leftover, the app's Runner, a newer `beans`, a stranger.
        Other(Holder),
    }

    #[derive(Debug, Clone)]
    struct World {
        stored: Stored,
        /// A same-named unit elsewhere, which Beans's own unit covers but cannot delete: removing
        /// Beans's brings it back.
        elsewhere: Option<Stored>,
        /// The service manager has the job.
        loaded: bool,
        port: Option<Process>,
        /// Programs on disk, with their versions.
        programs: Vec<(String, String)>,
        /// The host call that fails (counting from 0), if any.
        fail_at: Option<usize>,
        calls: usize,
    }

    fn text_of(definition: &Definition) -> String {
        format!("{}|{:?}", definition.program, definition.environment)
    }

    impl World {
        fn version_of(&self, program: &str) -> Option<String> {
            self.programs.iter().find(|(path, _)| path == program).map(|(_, version)| version.clone())
        }

        fn found(&self) -> Found {
            let program = match &self.stored {
                Stored::Beans { definition, .. } => Some(definition.program.clone()),
                _ => None,
            };
            let program_exists = program.as_deref().is_some_and(|program| self.version_of(program).is_some());
            let program_version = program.as_deref().filter(|program| *program != NEW).and_then(|program| self.version_of(program));
            let running = matches!(self.port, Some(Process::Service { .. })).then_some(100);
            let holder = self.port.as_ref().map(|process| match process {
                Process::Service { version } => Holder { pid: Some(100), version: Some(version.clone()), ..holder(100, VERSION) },
                Process::Other(holder) => holder.clone(),
            });
            Found { stored: self.stored.clone(), active: self.loaded, running, program_exists, program_version, holder, overrides: Vec::new() }
        }

        fn call(&mut self, name: &str) -> anyhow::Result<()> {
            let this = self.calls;
            self.calls += 1;
            if self.fail_at == Some(this) {
                anyhow::bail!("{name} failed");
            }
            Ok(())
        }

        /// What `start` runs: the stored definition's program, when there is one to run.
        fn startable(&self) -> Option<String> {
            match &self.stored {
                Stored::Beans { definition, .. } | Stored::Elsewhere { definition: Some(definition), .. } => self.version_of(&definition.program),
                Stored::Edited { .. } => Some(VERSION.into()),
                _ => None,
            }
        }
    }

    impl Host for World {
        fn stop(&mut self) -> anyhow::Result<()> {
            self.call("stop")?;
            self.loaded = false;
            if matches!(self.port, Some(Process::Service { .. })) {
                self.port = None;
            }
            Ok(())
        }
        fn clear_port(&mut self, strict: bool) -> anyhow::Result<Vec<Note>> {
            self.call("clear")?;
            let Some(holder) = self.found().holder else { return Ok(Vec::new()) };
            match clearing(&holder, VERSION, strict, 1, Some(2)) {
                Clearing::Stop { note, .. } => {
                    self.port = None;
                    Ok(vec![note])
                }
                Clearing::Leave(note) => Ok(vec![note]),
                Clearing::Refuse(refusal) => Err(refusal.into()),
            }
        }
        fn remove(&mut self) -> anyhow::Result<()> {
            self.call("remove")?;
            self.stored = self.elsewhere.clone().unwrap_or(Stored::Absent);
            Ok(())
        }
        fn write(&mut self, definition: &Definition) -> anyhow::Result<()> {
            self.call("write")?;
            self.stored = Stored::Beans { definition: definition.clone(), current: true, text: text_of(definition) };
            Ok(())
        }
        fn restore(&mut self, text: &str) -> anyhow::Result<()> {
            self.call("restore")?;
            self.stored = match &self.stored {
                // The simulation restores by remembering, not parsing.
                _ if text.starts_with("edited") => Stored::Edited { text: text.into() },
                _ => RESTORE.with(|restore| restore.borrow().clone()).expect("restored a text never read"),
            };
            Ok(())
        }
        fn start(&mut self) -> anyhow::Result<()> {
            self.call("start")?;
            let version = self.startable().ok_or_else(|| anyhow::anyhow!("nothing to start"))?;
            self.loaded = true;
            // It runs unless something else holds the port.
            if self.port.is_none() {
                self.port = Some(Process::Service { version });
            }
            Ok(())
        }
    }

    thread_local! {
        /// What `restore` puts back: the definition found before the command ran.
        static RESTORE: std::cell::RefCell<Option<Stored>> = const { std::cell::RefCell::new(None) };
    }

    fn random_world(dice: &mut Dice) -> World {
        let homes = ["/h", "/other home"];
        let home = homes[dice.roll(2) as usize];
        let mut programs = vec![(NEW.to_string(), VERSION.to_string())];
        if dice.chance(2) {
            programs.push((OLD.to_string(), ["0.1.0", VERSION, "0.9.0"][dice.roll(3) as usize].to_string()));
        }
        let stored = match dice.roll(6) {
            0 => Stored::Absent,
            1 | 2 => {
                let program = if dice.chance(2) { NEW } else { OLD };
                Stored::Beans { definition: definition(program, home), current: dice.chance(2), text: format!("text {program} {home}") }
            }
            3 => Stored::Edited { text: "edited by hand".into() },
            4 => Stored::Elsewhere { path: "/etc/systemd/user/beans.service".into(), definition: dice.chance(2).then(|| definition(OLD, home)) },
            _ => Stored::Beans { definition: definition(NEW, home), current: true, text: format!("text {NEW} {home}") },
        };
        let loaded = stored != Stored::Absent && dice.chance(2) || dice.chance(8);
        let port = match dice.roll(7) {
            0 | 1 => None,
            2 if loaded => Some(Process::Service { version: ["0.1.0", VERSION][dice.roll(2) as usize].into() }),
            2 | 3 => Some(Process::Other(holder(50, "0.1.0"))),
            4 => Some(Process::Other(holder(51, VERSION))),
            5 => Some(Process::Other(holder(52, "9.9.9"))),
            _ => Some(Process::Other(Holder { version: None, ..holder(53, VERSION) })),
        };
        let elsewhere = matches!(stored, Stored::Elsewhere { .. }).then(|| stored.clone());
        World { stored, elsewhere, loaded, port, programs, fail_at: None, calls: 0 }
    }

    enum Command {
        Install,
        Refresh,
        Uninstall,
    }

    fn plan_for(command: &Command, found: &Found) -> Option<Plan> {
        match command {
            Command::Install => install(found, definition(NEW, "/h"), VERSION).ok(),
            Command::Refresh => Some(refresh(found, NEW, Some(definition(NEW, "/fallback")), VERSION)),
            Command::Uninstall => Some(uninstall(found)),
        }
    }

    fn other_on_port(world: &World) -> Option<Holder> {
        match &world.port {
            Some(Process::Other(holder)) => Some(holder.clone()),
            _ => None,
        }
    }

    #[test]
    fn every_plan_is_well_ordered() {
        let mut dice = Dice(0x5eed_0001);
        for _ in 0..20_000 {
            let world = random_world(&mut dice);
            let found = world.found();
            for command in [Command::Install, Command::Refresh, Command::Uninstall] {
                let Some(plan) = plan_for(&command, &found) else { continue };
                let position = |wanted: fn(&Action) -> bool| plan.actions.iter().position(wanted);
                let count = |wanted: fn(&Action) -> bool| plan.actions.iter().filter(|action| wanted(action)).count();
                for kind in [|a: &Action| *a == Action::Stop, |a: &Action| matches!(a, Action::ClearPort { .. }), |a: &Action| matches!(a, Action::Write(_)), |a: &Action| *a == Action::Start, |a: &Action| *a == Action::Remove] {
                    assert!(count(kind) <= 1, "a step twice: {plan:?}");
                }
                // Stop first: nothing new starts, and nothing is stopped from the port, while the old job runs.
                if let Some(stop) = position(|a| *a == Action::Stop) {
                    assert_eq!(stop, 0, "{plan:?}");
                }
                if let Some(start) = position(|a| *a == Action::Start) {
                    assert_eq!(start, plan.actions.len() - 1, "start last: {plan:?}");
                }
                if let (Some(clear), Some(write)) = (position(|a| matches!(a, Action::ClearPort { .. })), position(|a| matches!(a, Action::Write(_)))) {
                    assert!(clear < write, "the port is cleared before anything is written: {plan:?}");
                }
                assert!(!(position(|a| *a == Action::Remove).is_some() && position(|a| matches!(a, Action::Write(_))).is_some()), "{plan:?}");
                if let Some(Action::Write(written)) = plan.actions.iter().find(|a| matches!(a, Action::Write(_))) {
                    assert_eq!(written.program, NEW, "only this beans is ever written: {plan:?}");
                }
            }
        }
    }

    #[test]
    fn every_command_from_every_state_ends_as_promised() {
        let mut dice = Dice(0x5eed_0002);
        for round in 0..20_000 {
            let before = random_world(&mut dice);
            for command in [Command::Install, Command::Refresh, Command::Uninstall] {
                let found = before.found();
                RESTORE.with(|restore| *restore.borrow_mut() = Some(before.stored.clone()));
                let Some(plan) = plan_for(&command, &found) else {
                    // Install refuses only for a process that is not a leftover and not the service's.
                    let holder = other_on_port(&before).expect("refused without a reason");
                    assert_ne!(classify(&holder, VERSION), Holding::Old, "round {round}");
                    continue;
                };
                let mut after = before.clone();
                let result = apply(&plan, &found, &mut after);
                let old_left = other_on_port(&after).is_some_and(|holder| classify(&holder, VERSION) == Holding::Old);
                // What is not a leftover is never stopped.
                if let Some(holder) = other_on_port(&before).filter(|holder| classify(holder, VERSION) != Holding::Old) {
                    assert_eq!(other_on_port(&after), Some(holder), "round {round}: {plan:?}");
                }
                match command {
                    Command::Install => {
                        if result.is_err() {
                            // Refused once the port was checked: the service as it was.
                            assert_eq!(after.stored, before.stored, "round {round}");
                            continue;
                        }
                        assert_eq!(after.stored, Stored::Beans { definition: definition(NEW, "/h"), current: true, text: text_of(&definition(NEW, "/h")) }, "round {round}");
                        assert!(after.loaded && !old_left, "round {round}: {after:?}");
                    }
                    Command::Refresh => {
                        result.unwrap();
                        // A stopped service stays stopped.
                        if !before.loaded && !matches!(before.port, Some(Process::Service { .. })) {
                            assert!(!after.loaded, "round {round}: {plan:?}");
                        }
                        // The home, port and relay a Beans service serves are kept.
                        if let (Stored::Beans { definition: was, .. }, Stored::Beans { definition: now, .. }) = (&before.stored, &after.stored) {
                            assert_eq!(was.environment, now.environment, "round {round}");
                        }
                        // Never onto an older beans than the one the service runs.
                        if let Stored::Beans { definition: now, .. } = &after.stored {
                            let running = after.version_of(&now.program).unwrap_or_default();
                            let was = match &before.stored { Stored::Beans { definition, .. } => before.version_of(&definition.program), _ => None };
                            if let Some(was) = was {
                                assert_ne!(compare(&was, &running), Some(Ordering::Greater), "round {round}: downgraded");
                            }
                        }
                        assert!(!old_left, "round {round}: a leftover survived refresh: {after:?}");
                        // The app runs refresh at every launch: the second finds nothing to do.
                        let again = refresh(&after.found(), NEW, Some(definition(NEW, "/fallback")), VERSION);
                        assert!(again.actions.is_empty(), "round {round}: not settled: {again:?} after {plan:?} from {before:?}");
                    }
                    Command::Uninstall => {
                        result.unwrap();
                        assert!(matches!(after.stored, Stored::Absent | Stored::Elsewhere { .. }), "round {round}");
                        assert!(!after.loaded && !matches!(after.port, Some(Process::Service { .. })), "round {round}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_failure_at_any_step_leaves_the_service_as_it_was() {
        let mut dice = Dice(0x5eed_0003);
        for round in 0..5_000 {
            let before = random_world(&mut dice);
            for command in [Command::Install, Command::Refresh] {
                let found = before.found();
                let Some(plan) = plan_for(&command, &found) else { continue };
                for fail_at in 0..plan.actions.len() {
                    RESTORE.with(|restore| *restore.borrow_mut() = Some(before.stored.clone()));
                    let mut after = World { fail_at: Some(fail_at), ..before.clone() };
                    if apply(&plan, &found, &mut after).is_ok() {
                        continue;
                    }
                    // The definition is the one found before, whichever step failed.
                    assert_eq!(after.stored, before.stored, "round {round}, step {fail_at} of {plan:?}");
                    // A service that ran, with a definition to run, runs again.
                    if before.loaded && before.startable().is_some() && fail_at > 0 {
                        assert!(after.loaded, "round {round}, step {fail_at} of {plan:?}");
                    }
                    // A job left without a definition is not started again by the rollback.
                    if before.stored == Stored::Absent {
                        assert!(!(after.loaded && !before.loaded), "round {round}");
                    }
                }
            }
        }
    }

    // MARK: Readers against anything

    /// Random damage to a definition's text: characters dropped, repeated or swapped for ones each
    /// syntax treats specially, lines lost or doubled.
    fn damage(text: &str, dice: &mut Dice) -> String {
        let special = ['"', '\\', '<', '>', '&', ';', '%', '$', '=', ' ', '\n', '\t', '\r', '\'', '#', '[', ']', '/', 'é', '🫘', '\u{0}', '\u{fffe}'];
        let mut chars: Vec<char> = text.chars().collect();
        for _ in 0..=dice.roll(4) {
            if chars.is_empty() {
                break;
            }
            let at = dice.roll(chars.len() as u64) as usize;
            match dice.roll(5) {
                0 => { chars.remove(at); }
                1 => chars.insert(at, special[dice.roll(special.len() as u64) as usize]),
                2 => chars[at] = special[dice.roll(special.len() as u64) as usize],
                3 => { let c = chars[at]; chars.insert(at, c); }
                _ => {
                    let lines: Vec<String> = chars.iter().collect::<String>().lines().map(str::to_string).collect();
                    if !lines.is_empty() {
                        let mut lines = lines;
                        let line = dice.roll(lines.len() as u64) as usize;
                        if dice.chance(2) { lines.remove(line); } else { let copy = lines[line].clone(); lines.insert(line, copy); }
                        chars = lines.join("\n").chars().collect();
                    }
                }
            }
        }
        chars.into_iter().collect()
    }

    #[test]
    fn readers_never_panic_and_call_current_only_what_they_would_write() {
        let mut dice = Dice(0x5eed_0004);
        let values = ["", "/h", "C:\\Users\\me\\", "a \"b\" %c% $d", "line\nbreak", "ünï 🫘"];
        for round in 0..30_000 {
            let value = values[dice.roll(values.len() as u64) as usize];
            let environment = env(&[("BEANS_HOME", value), ("BEANS_PORT", "4874")]);
            let originals = [
                super::super::launchd_plist("app.beans.serve", "/Apps/beans", "/log", &environment).unwrap(),
                super::super::systemd_unit("/opt/beans", &environment, Some(257)).unwrap(),
                super::super::windows_command_line(r"C:\Beans\beans.exe", &environment).unwrap(),
            ];
            for (kind, original) in originals.iter().enumerate() {
                let text = if dice.chance(4) { original.clone() } else { damage(original, &mut dice) };
                let read = match kind {
                    0 => read_launchd(&text, "app.beans.serve", "/log"),
                    1 => read_systemd(&text),
                    _ => read_run_key(&text),
                };
                match read {
                    Stored::Beans { definition, current: true, text: kept } => {
                        assert_eq!(kept, text);
                        let written = match kind {
                            0 => super::super::launchd_plist("app.beans.serve", &definition.program, "/log", &definition.environment).unwrap(),
                            1 => super::super::systemd_unit(&definition.program, &definition.environment, Some(u32::MAX)).unwrap(),
                            _ => super::super::windows_command_line(&definition.program, &definition.environment).unwrap(),
                        };
                        assert_eq!(written, text, "round {round}: current but not what would be written");
                    }
                    Stored::Beans { definition, current: false, .. } => {
                        assert!(!definition.program.is_empty() || kind == 2, "round {round}: {text:?}");
                    }
                    Stored::Edited { text: kept } => assert_eq!(kept, text),
                    other => panic!("round {round}: a reader returned {other:?}"),
                }
                if text == *original {
                    assert!(matches!(read_again(kind, &text), Stored::Beans { current: true, .. }), "round {round}: its own text is not current");
                }
            }
        }
    }

    fn read_again(kind: usize, text: &str) -> Stored {
        match kind {
            0 => read_launchd(text, "app.beans.serve", "/log"),
            1 => read_systemd(text),
            _ => read_run_key(text),
        }
    }

    #[test]
    fn versions_never_panic_and_order_consistently() {
        let mut dice = Dice(0x5eed_0005);
        let parts = ["0", "1", "10", "01", "9", "99999999999999999999", "", "v", "-", "+", "rc1", "beta", ".", " "];
        for _ in 0..20_000 {
            let mut make = || (0..=dice.roll(5)).map(|_| parts[dice.roll(parts.len() as u64) as usize]).collect::<String>();
            let (a, b) = (make(), make());
            match (compare(&a, &b), compare(&b, &a)) {
                (Some(one), Some(other)) => assert_eq!(one, other.reverse(), "{a:?} {b:?}"),
                (None, None) => {}
                (one, other) => panic!("{a:?} {b:?}: {one:?} but {other:?}"),
            }
            assert_ne!(compare(&a, &a), Some(Ordering::Less));
        }
        assert_eq!(compare("99999999999999999999", "1"), None, "too large is not a version");
        assert_eq!(compare("", "1"), None);
        assert_eq!(compare("1..2", "1"), None);
        assert_eq!(compare("01.002.0003", "1.2.3"), Some(Ordering::Equal));
    }

    // MARK: Reading definitions back

    fn hard_values() -> Vec<String> {
        [
            "", "plain", "with  spaces ", "double \" quote", "single ' quote", r"C:\Users\me\", "100% sure", "%PATH%", "%%", "$HOME ${x} $$",
            "line\nbreak", "tab\there", "crlf\r\n", "<tag>&amp; ]]>", "ünïcödé 🫘", "trailing backslash\\",
        ].map(String::from).into()
    }

    #[test]
    fn each_platform_reads_back_exactly_what_it_wrote() {
        for value in hard_values() {
            let environment = env(&[("BEANS_HOME", &value), ("BEANS_PORT", "4999"), ("BEANS_RELAY_URL", &value)]);
            let plist = super::super::launchd_plist("app.beans.serve", "/Apps/beans", "/log", &environment).unwrap();
            assert_eq!(read_launchd(&plist, "app.beans.serve", "/log"),
                Stored::Beans { definition: Definition { program: "/Apps/beans".into(), environment: environment.clone() }, current: true, text: plist.clone() }, "{value:?}");
            let unit = super::super::systemd_unit("/opt/be ans 100%/$x/beans", &environment, Some(257)).unwrap();
            assert_eq!(read_systemd(&unit),
                Stored::Beans { definition: Definition { program: "/opt/be ans 100%/$x/beans".into(), environment: environment.clone() }, current: true, text: unit.clone() }, "{value:?}");
            let line = super::super::windows_command_line(r"C:\Program Files\Beans 100%\beans.exe", &environment).unwrap();
            assert_eq!(read_run_key(&line),
                Stored::Beans { definition: Definition { program: r"C:\Program Files\Beans 100%\beans.exe".into(), environment }, current: true, text: line.clone() }, "{value:?}");
        }
    }

    #[test]
    fn definitions_from_an_older_beans_are_still_recognized_as_beans() {
        // As this CLI wrote them before its escaping was fixed.
        let plist = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n<dict>\n\t<key>Label</key>\n\t<string>app.beans.serve</string>\n\t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>/old/beans</string>\n\t\t<string>serve</string>\n\t</array>\n\t<key>EnvironmentVariables</key>\n\t<dict>\n\t\t<key>BEANS_SERVICE</key>\n\t\t<string>1</string>\n\t\t<key>BEANS_HOME</key>\n\t\t<string>/h</string>\n\t</dict>\n</dict>\n</plist>\n";
        let Stored::Beans { definition, current, .. } = read_launchd(plist, "app.beans.serve", "/log") else { panic!() };
        assert_eq!((definition, current), (Definition { program: "/old/beans".into(), environment: env(&[("BEANS_HOME", "/h")]) }, false));
        let unit = "[Unit]\nDescription=Beans\n\n[Service]\nExecStart=\"/old/beans\" serve\nEnvironment=BEANS_SERVICE=1\nEnvironment=\"BEANS_HOME=/h\"\nRestart=always\n";
        let Stored::Beans { definition, current, .. } = read_systemd(unit) else { panic!() };
        assert_eq!((definition, current), (Definition { program: "/old/beans".into(), environment: env(&[("BEANS_HOME", "/h")]) }, false));
        let line = r#""C:\old\beans.exe" service run --env "BEANS_HOME=C:\h" --env "BEANS_PORT=4874""#;
        let Stored::Beans { definition, current, .. } = read_run_key(line) else { panic!() };
        assert_eq!((definition, current), (Definition { program: r"C:\old\beans.exe".into(), environment: env(&[("BEANS_HOME", r"C:\h"), ("BEANS_PORT", "4874")]) }, true));
    }

    #[test]
    fn definitions_a_newer_beans_writes_are_still_beans_and_keep_their_variables() {
        // A later version may add settings or variables; an older one still knows the service.
        let environment = env(&[("BEANS_HOME", "/h"), ("BEANS_NEW_SETTING", "on")]);
        let plist = super::super::launchd_plist("app.beans.serve", "/beans", "/log", &environment).unwrap()
            .replace("\t<key>RunAtLoad</key>", "\t<key>ProcessType</key>\n\t<string>Background</string>\n\t<key>RunAtLoad</key>");
        let Stored::Beans { definition, current, .. } = read_launchd(&plist, "app.beans.serve", "/log") else { panic!("{plist}") };
        assert_eq!((definition.environment, current), (environment.clone(), false));
        let unit = super::super::systemd_unit("/beans", &environment, Some(257)).unwrap().replace("RestartSec=2\n", "RestartSec=2\nTimeoutStopSec=30\nNice=5\n");
        let Stored::Beans { definition, current, .. } = read_systemd(&unit) else { panic!("{unit}") };
        assert_eq!((definition.environment, current), (environment.clone(), false));
        let line = format!("{} --env \"BEANS_NEW=1\"", super::super::windows_command_line(r"C:\beans.exe", &environment).unwrap());
        let Stored::Beans { definition, .. } = read_run_key(&line) else { panic!("{line}") };
        assert_eq!(definition.environment.last(), Some(&("BEANS_NEW".to_string(), "1".to_string())));
    }

    #[test]
    fn a_unit_edited_in_ways_systemd_allows_is_recognized_or_replaced_but_never_misread() {
        let unit = super::super::systemd_unit("/beans", &env(&[("BEANS_HOME", "/h")]), Some(257)).unwrap();
        let edits = [
            unit.replace("ExecStart=", "# a comment\nExecStart="),
            unit.replace("ExecStart=", "; a comment\nExecStart="),
            unit.replace("ExecStart=\"/beans\" serve", "ExecStart= \"/beans\"  serve "),
            unit.replace("Environment=BEANS_SERVICE=1", "Environment=\"BEANS_SERVICE=1\""),
            unit.replace("Environment=\"BEANS_HOME=/h\"", "Environment='BEANS_HOME=/h'"),
            unit.replace("Environment=\"BEANS_HOME=/h\"", "Environment=BEANS_HOME=/h"),
            unit.replace("\n", "\r\n"),
        ];
        for edited in edits {
            match read_systemd(&edited) {
                Stored::Beans { definition, current, .. } => {
                    assert_eq!(definition, Definition { program: "/beans".into(), environment: env(&[("BEANS_HOME", "/h")]) }, "misread: {edited:?}");
                    assert!(!current, "{edited:?}");
                }
                Stored::Edited { .. } => {}
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn definitions_beans_did_not_write_are_recognized_as_edited() {
        let edited = |stored: Stored| assert!(matches!(stored, Stored::Edited { .. }), "{stored:?}");
        let plist = super::super::launchd_plist("app.beans.serve", "/beans", "/log", &[]).unwrap();
        edited(read_launchd(&plist.replace("<string>serve</string>", "<string>serve</string>\n\t\t<string>--debug</string>"), "app.beans.serve", "/log"));
        edited(read_launchd(&plist.replace("<string>1</string>", "<string>0</string>"), "app.beans.serve", "/log"));
        edited(read_launchd(&plist, "com.example.other", "/log"));
        edited(read_launchd("not a plist", "app.beans.serve", "/log"));
        edited(read_launchd(&plist.replace("/beans", "/be&unknown;ans"), "app.beans.serve", "/log"));
        let unit = super::super::systemd_unit("/beans", &[], Some(257)).unwrap();
        edited(read_systemd(&unit.replace(" serve", " serve --verbose")));
        edited(read_systemd(&unit.replace("ExecStart=", "ExecStart=/bin/sh -c ")));
        edited(read_systemd(&unit.replace("Environment=BEANS_SERVICE=1\n", "")));
        edited(read_systemd(&format!("{unit}ExecStart=\"/other\" serve\n")));
        edited(read_systemd(&unit.replace("\"/beans\"", "\"/be%hans\"")));
        edited(read_systemd(""));
        edited(read_run_key(r#""C:\beans.exe" service status"#));
        edited(read_run_key(r#""C:\beans.exe" service run --env"#));
        edited(read_run_key(r#""C:\beans.exe" service run --other "A=b""#));
        edited(read_run_key(r#""C:\beans.exe" service run --env "A""#));
        edited(read_run_key(r#""C:\beans.exe" service run --env "A=b""c""#));
        edited(read_run_key(r#"C:\Windows\other.exe /start"#));
    }

    #[test]
    fn an_edited_definition_with_the_same_meaning_is_beans_but_not_current() {
        let unit = super::super::systemd_unit("/beans", &env(&[("BEANS_HOME", "/h")]), Some(257)).unwrap();
        let Stored::Beans { current, .. } = read_systemd(&unit.replace("RestartSec=2", "RestartSec=30")) else { panic!() };
        assert!(!current, "refresh rewrites it to what this version writes");
    }
}
