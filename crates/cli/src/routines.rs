//! Routines: recurring tasks a bot runs on a schedule in its direct chat, after Grok Bot. The
//! roster carries them, so every Device lists them and can pause one; the bot's Runner runs
//! them, looking for due ones every half minute, and any Device can ask for a run now. A
//! routine with a check runs the check first, a script and no model, and the bot runs only on
//! what it found.

use std::sync::Arc;

use serde_json::Value;

use crate::app::App;
use crate::config::{now_secs, now_unix};
use crate::model::*;
use crate::runtime::{self, TurnOutcome};
use crate::schedule;

#[cfg(feature = "runner")]
use beans_agent::codemode::{CodemodeOptions, CodemodeTool, HostFunction};
#[cfg(feature = "runner")]
use beans_agent::{DirectRunner, Tool, ToolOutcome, ToolResult, ToolRunner};
#[cfg(feature = "runner")]
use tokio_util::sync::CancellationToken;

/// How often the Runner looks for due routines.
#[cfg(feature = "runner")]
const TICK: std::time::Duration = std::time::Duration::from_secs(30);

/// A week without a message from the user, and due routines are paused instead of run, so
/// they do not spend money on results nobody reads.
pub const AWAY_AFTER_SECS: i64 = 7 * 86_400;

pub const MAX_NAME_CHARS: usize = 60;
pub const MAX_PER_BOT: usize = 20;
/// The longest check a routine keeps: a check looks, and the run does the work.
pub const MAX_CHECK_CHARS: usize = 8_000;

// MARK: - Editing

/// Adds a routine for `bot_id`, checking the schedule, the name, and the check first.
pub fn create(
    app: &Arc<App>,
    bot_id: &str,
    name: &str,
    schedule_text: &str,
    prompt: &str,
    check: Option<&str>,
    enabled: bool,
) -> Result<Routine, String> {
    create_with_policy(
        app,
        bot_id,
        name,
        schedule_text,
        prompt,
        check,
        enabled,
        None,
        None,
    )
}

/// `create` with a timezone (this Runner's when `None`) and a missed-run policy (one run when
/// `None`).
#[allow(clippy::too_many_arguments)]
pub fn create_with_policy(
    app: &Arc<App>,
    bot_id: &str,
    name: &str,
    schedule_text: &str,
    prompt: &str,
    check: Option<&str>,
    enabled: bool,
    timezone: Option<&str>,
    missed_run_policy: Option<&str>,
) -> Result<Routine, String> {
    create_routine(
        app,
        bot_id,
        name,
        schedule_text,
        prompt,
        check,
        enabled,
        timezone,
        missed_run_policy,
        Triggers::default(),
    )
}

/// What places a routine's runs besides its schedule, as the bot or the local API gives it.
#[derive(Debug, Default, Clone, Copy)]
pub struct Triggers<'a> {
    /// The relay's receiver a routine on events listens through (`github`), and the subject
    /// its events are about (`acme/project#42`), as the service's skill names them.
    pub receiver: Option<&'a str>,
    pub subject: Option<&'a str>,
    /// The Calendar account of a schedule around events: its id or name; the only one when left
    /// out.
    pub calendar: Option<&'a str>,
    /// Words a matching event has in its title, description, place, or guests. `""` on edit
    /// matches every event.
    pub event_match: Option<&'a str>,
}

/// `create_with_policy` with what else places the runs: a receiver's events about a subject, or
/// the Calendar account and the words of events a schedule around events follows.
#[allow(clippy::too_many_arguments)]
pub fn create_routine(
    app: &Arc<App>,
    bot_id: &str,
    name: &str,
    schedule_text: &str,
    prompt: &str,
    check: Option<&str>,
    enabled: bool,
    timezone: Option<&str>,
    missed_run_policy: Option<&str>,
    triggers: Triggers,
) -> Result<Routine, String> {
    let name = clean_name(name)?;
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("Say what the routine should do on each run.".into());
    }
    let check = check.map(clean_check).transpose()?.flatten();
    // A routine on events takes no schedule: they start its runs.
    let listening = [triggers.receiver, triggers.subject]
        .iter()
        .any(|text| text.is_some_and(|text| !text.trim().is_empty()));
    let schedule_text = if listening && schedule_text.trim().is_empty() {
        "on events"
    } else {
        schedule_text
    };
    let schedule = schedule::parse(schedule_text)?;
    let timezone = match timezone {
        Some(zone) => schedule::timezone(zone)?.to_string(),
        None => schedule::local_timezone(),
    };
    let missed_run_policy = missed_run_policy
        .map(crate::routine_health::MissedRunPolicy::parse)
        .transpose()?
        .unwrap_or_default();
    let bot = app.bot(bot_id).ok_or("Unknown bot")?;
    let existing = app.routines_of(bot_id);
    if existing.len() >= MAX_PER_BOT {
        return Err(format!(
            "A bot keeps at most {MAX_PER_BOT} routines. Delete one first."
        ));
    }
    if existing.iter().any(|r| r.name.eq_ignore_ascii_case(&name)) {
        return Err(format!(
            "A routine named {name:?} already exists. Pick another name or edit that one."
        ));
    }
    let (events, calendar) = triggers_for(
        app,
        &bot,
        &schedule,
        check.as_deref(),
        &timezone,
        missed_run_policy,
        triggers,
        None,
    )?;
    let now = now_secs();
    let routine = Routine {
        id: String::new(),
        bot_id: bot_id.to_string(),
        name,
        prompt: prompt.to_string(),
        feedback_authorization_prompt: None,
        schedule: schedule.canonical(),
        timezone,
        missed_run_policy,
        last_scheduled_at: None,
        health: None,
        is_enabled: enabled,
        enabled_at: now,
        last_run_at: None,
        last_outcome: None,
        paused_reason: None,
        check,
        events,
        calendar,
        created_at: now,
    };
    let routine = app.insert_routine(routine).map_err(|e| e.to_string())?;
    // A routine on events listens through a subscription of its own on its Runner.
    #[cfg(feature = "runner")]
    if routine.events.is_some() {
        let opened = crate::routine_events::open(app, &routine).and_then(|subscription_id| {
            app.update_routine(&routine.id, |routine| {
                if let Some(events) = routine.events.as_mut() {
                    events.subscription_id = subscription_id;
                }
            })
            .map_err(|error| error.to_string())
        });
        return match opened {
            Ok(routine) => Ok(routine),
            Err(error) => {
                let _ = app.delete_routine(&routine.id);
                Err(error)
            }
        };
    }
    Ok(routine)
}

/// What a routine of `bot` with `schedule` listens to or follows, checked against the rest of
/// it: a one-time routine runs once, late if need be, with no check; a routine on events
/// listens to one receiver's subject and keeps it; a schedule around events follows one
/// Calendar account. `current` is the routine being edited, whose events and calendar carry
/// over what they read.
#[allow(clippy::too_many_arguments)]
fn triggers_for(
    app: &Arc<App>,
    bot: &Bot,
    schedule: &schedule::Schedule,
    check: Option<&str>,
    timezone: &str,
    policy: crate::routine_health::MissedRunPolicy,
    triggers: Triggers,
    current: Option<&Routine>,
) -> Result<
    (
        Option<crate::routine_events::RoutineEvents>,
        Option<crate::routine_triggers::CalendarTrigger>,
    ),
    String,
> {
    use crate::routine_triggers::CalendarTrigger;
    let on_runner = app.this_device_id().as_deref() == Some(bot.runner_id.as_str());
    let events = matches!(schedule, schedule::Schedule::Events(_));
    if !events
        && (triggers
            .calendar
            .is_some_and(|name| !name.trim().is_empty())
            || triggers
                .event_match
                .is_some_and(|words| !words.trim().is_empty()))
    {
        return Err("A calendar and event words go with a schedule around events, such as 15m before events.".into());
    }
    if let schedule::Schedule::Once(_) = schedule {
        if check.is_some() {
            return Err("A one-time routine runs once at its time and has no check.".into());
        }
        if policy == crate::routine_health::MissedRunPolicy::Skip {
            return Err("A one-time routine always runs, late if its Runner was off at its time. Leave missed_run_policy out.".into());
        }
        let at = schedule
            .once_instant(timezone)
            .ok_or("That time does not exist on the routine's clock.")?;
        let unchanged = current.is_some_and(|routine| {
            routine.schedule == schedule.canonical() && routine.timezone == timezone
        });
        if at <= now_unix() && !unchanged {
            return Err(format!(
                "That time has passed: it is {} in {timezone} now.",
                schedule::when_label(now_unix(), now_unix(), timezone)
            ));
        }
    }
    let listening = match current.and_then(|routine| routine.events.clone()) {
        Some(held) => {
            let asked = triggers
                .receiver
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .zip(
                    triggers
                        .subject
                        .map(str::trim)
                        .filter(|text| !text.is_empty()),
                );
            if asked.is_some_and(|(receiver, subject)| {
                !receiver.eq_ignore_ascii_case(&held.receiver) || subject != held.subject
            }) {
                return Err("A routine on events keeps what it listens to. Delete it and set up another for something else.".into());
            }
            if !matches!(schedule, schedule::Schedule::OnEvents) {
                return Err(
                    "A routine on events runs when they come and takes no schedule.".into(),
                );
            }
            Some(held)
        }
        None => match (
            triggers
                .receiver
                .map(str::trim)
                .filter(|text| !text.is_empty()),
            triggers
                .subject
                .map(str::trim)
                .filter(|text| !text.is_empty()),
        ) {
            (None, None) if matches!(schedule, schedule::Schedule::OnEvents) => {
                return Err("Give the receiver the events come through.".into())
            }
            (None, None) => None,
            (Some(receiver), subject) => {
                let subject = subject.unwrap_or_default();
                if current.is_some() {
                    return Err("Set up a new routine to run on events.".into());
                }
                if !matches!(schedule, schedule::Schedule::OnEvents) {
                    return Err("A routine on events runs when they come and takes no schedule: leave schedule out.".into());
                }
                if check.is_some() {
                    return Err(
                        "A routine on events has no check: its events start its runs.".into(),
                    );
                }
                if !on_runner {
                    return Err("Set up a routine on events on the bot's Runner.".into());
                }
                let (receiver, subject) = crate::routine_events::check(receiver, subject)?;
                Some(crate::routine_events::RoutineEvents {
                    receiver,
                    subject,
                    subscription_id: String::new(),
                    status: crate::routine_events::Listening::Pending,
                    source_name: String::new(),
                    title: String::new(),
                    url: String::new(),
                    endpoint: String::new(),
                    key: String::new(),
                    last_event: None,
                    ended_at: None,
                })
            }
            (None, Some(_)) => {
                return Err("Give the receiver the subject's events come through.".into())
            }
        },
    };
    let calendar = if events {
        if listening.is_some() {
            return Err("A routine either runs on a receiver's events or follows calendar events, not both.".into());
        }
        if check.is_some() {
            return Err("A routine around events has no check: the events start its runs.".into());
        }
        let matching = match triggers.event_match.map(str::trim) {
            Some("") => None,
            Some(words) => Some(words.chars().take(100).collect::<String>()),
            None => current
                .and_then(|routine| routine.calendar.as_ref())
                .and_then(|calendar| calendar.matching.clone()),
        };
        let held = current
            .and_then(|routine| routine.calendar.clone())
            .filter(|_| triggers.calendar.is_none_or(|name| name.trim().is_empty()));
        let mut calendar = match held {
            Some(calendar) => calendar,
            None => {
                if !on_runner {
                    return Err(
                        "Set up a routine around calendar events on the bot's Runner.".into(),
                    );
                }
                #[cfg(feature = "runner")]
                let (plugin_id, account) =
                    crate::routine_triggers::calendar_account(app, bot, triggers.calendar)?;
                #[cfg(not(feature = "runner"))]
                let (plugin_id, account) = (String::new(), String::new());
                CalendarTrigger {
                    plugin_id,
                    account,
                    matching: None,
                    upcoming: Vec::new(),
                    done: Vec::new(),
                    synced_at: None,
                }
            }
        };
        if calendar.matching != matching {
            // Other words find other events: read the calendar again.
            calendar.matching = matching;
            calendar.upcoming.clear();
            calendar.synced_at = None;
        }
        Some(calendar)
    } else {
        None
    };
    Ok((listening, calendar))
}

/// Changes a routine's name, schedule, prompt, or check. Only the fields given change; an empty
/// check removes it, and a new schedule counts from now.
pub fn edit(
    app: &Arc<App>,
    id: &str,
    name: Option<&str>,
    schedule_text: Option<&str>,
    prompt: Option<&str>,
    check: Option<&str>,
) -> Result<Routine, String> {
    edit_with_policy(app, id, name, schedule_text, prompt, check, None, None)
}

/// `edit` with a new timezone or missed-run policy as well. A new zone counts from now.
#[allow(clippy::too_many_arguments)]
pub fn edit_with_policy(
    app: &Arc<App>,
    id: &str,
    name: Option<&str>,
    schedule_text: Option<&str>,
    prompt: Option<&str>,
    check: Option<&str>,
    timezone: Option<&str>,
    missed_run_policy: Option<&str>,
) -> Result<Routine, String> {
    edit_routine(
        app,
        id,
        name,
        schedule_text,
        prompt,
        check,
        timezone,
        missed_run_policy,
        Triggers::default(),
    )
}

/// `edit_with_policy` with a new pull request to watch, or a new calendar or event words.
#[allow(clippy::too_many_arguments)]
pub fn edit_routine(
    app: &Arc<App>,
    id: &str,
    name: Option<&str>,
    schedule_text: Option<&str>,
    prompt: Option<&str>,
    check: Option<&str>,
    timezone: Option<&str>,
    missed_run_policy: Option<&str>,
    triggers: Triggers,
) -> Result<Routine, String> {
    let current = app.routine(id).ok_or("Unknown routine")?;
    let name = name.map(clean_name).transpose()?;
    if let Some(name) = &name {
        if app
            .routines_of(&current.bot_id)
            .iter()
            .any(|r| r.id != id && r.name.eq_ignore_ascii_case(name))
        {
            return Err(format!("A routine named {name:?} already exists."));
        }
    }
    let schedule = schedule_text.map(schedule::parse).transpose()?;
    let prompt = prompt
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    let check = check.map(clean_check).transpose()?;
    let timezone = timezone
        .map(schedule::timezone)
        .transpose()?
        .map(|zone| zone.to_string());
    let missed_run_policy = missed_run_policy
        .map(crate::routine_health::MissedRunPolicy::parse)
        .transpose()?;
    let triggered = triggers.receiver.is_some()
        || triggers.subject.is_some()
        || triggers.calendar.is_some()
        || triggers.event_match.is_some();
    if name.is_none()
        && schedule.is_none()
        && prompt.is_none()
        && check.is_none()
        && timezone.is_none()
        && missed_run_policy.is_none()
        && !triggered
    {
        return Err("Pass a new name, schedule, prompt, check, timezone, missed-run policy, calendar, or event words.".into());
    }
    // The routine as it would stand, checked whole.
    let bot = app.bot(&current.bot_id).ok_or("Unknown bot")?;
    let next_schedule = match &schedule {
        Some(schedule) => schedule.clone(),
        None => schedule::parse(&current.schedule)?,
    };
    let next_check = match &check {
        Some(check) => check.clone(),
        None => current.check.clone(),
    };
    let next_zone = timezone.clone().unwrap_or_else(|| current.timezone.clone());
    let next_policy = missed_run_policy.unwrap_or(current.missed_run_policy);
    let (events, calendar) = triggers_for(
        app,
        &bot,
        &next_schedule,
        next_check.as_deref(),
        &next_zone,
        next_policy,
        triggers,
        Some(&current),
    )?;
    let retargeted = current
        .calendar
        .as_ref()
        .map(|c| (&c.plugin_id, &c.matching))
        != calendar.as_ref().map(|c| (&c.plugin_id, &c.matching));
    app.update_routine(id, |routine| {
        if let Some(name) = name {
            routine.name = name;
        }
        if let Some(schedule) = schedule {
            routine.schedule = schedule.canonical();
            routine.enabled_at = now_secs();
        }
        if let Some(timezone) = timezone {
            routine.timezone = timezone;
            routine.enabled_at = now_secs();
        }
        if let Some(policy) = missed_run_policy {
            routine.missed_run_policy = policy;
        }
        if let Some(prompt) = prompt {
            // A task written here is the task the user asked for, feedback revisions included.
            if prompt != routine.prompt {
                routine.feedback_authorization_prompt = None;
            }
            routine.prompt = prompt;
        }
        if let Some(check) = check {
            routine.check = check;
            routine.health = None;
            routine.enabled_at = now_secs();
        }
        if retargeted {
            routine.health = None;
            routine.enabled_at = now_secs();
        }
        routine.events = events;
        routine.calendar = calendar;
    })
    .map_err(|e| e.to_string())
}

/// A check as a routine keeps it, or `None` for an empty one, which removes it. A Runner reads
/// its options line now, so a bad one is refused before it is saved.
fn clean_check(code: &str) -> Result<Option<String>, String> {
    let code = code.trim();
    if code.is_empty() {
        return Ok(None);
    }
    if code.chars().count() > MAX_CHECK_CHARS {
        return Err(format!("Keep the check under {MAX_CHECK_CHARS} characters: it only looks, and the run does the work."));
    }
    #[cfg(feature = "runner")]
    beans_agent::codemode::parse_source(code)?;
    Ok(Some(code.to_string()))
}

/// A routine's check is its Runner's to change, and a build that does not know checks writes
/// the roster without them. When `incoming` leaves out the check that a routine of a bot on
/// `this_device` has in `current`, the check stays. True when one did, so the roster goes up
/// again with it. The same merge keeps a feedback revision's original task authority while the
/// task is still the revised one; a task edited on another Device is the user's own.
pub fn keep_checks(
    current: &[Routine],
    incoming: &mut [Routine],
    bots: &[Bot],
    this_device: &str,
) -> bool {
    let mut kept = false;
    for routine in incoming.iter_mut() {
        if !bots
            .iter()
            .any(|bot| bot.id == routine.bot_id && bot.runner_id == this_device)
        {
            continue;
        }
        let Some(held) = current.iter().find(|held| held.id == routine.id) else {
            continue;
        };
        kept |= crate::routine_triggers::keep_reads(held, routine);
        if routine.check.is_none() && held.check.is_some() {
            routine.check = held.check.clone();
            kept = true;
        }
        if routine.feedback_authorization_prompt.is_none()
            && held.feedback_authorization_prompt.is_some()
            && routine.prompt == held.prompt
        {
            routine.feedback_authorization_prompt = held.feedback_authorization_prompt.clone();
            kept = true;
        }
        // A schedule/check edit or explicit resume re-arms it. Otherwise preserve this
        // Runner's newest checkpoint when a Device publishes a roster it read earlier.
        if routine.enabled_at <= held.enabled_at {
            if held.last_scheduled_at > routine.last_scheduled_at {
                routine.last_scheduled_at = held.last_scheduled_at;
                kept = true;
            }
            if held.health.as_ref().map(|health| health.updated_at)
                > routine.health.as_ref().map(|health| health.updated_at)
            {
                routine.health = held.health.clone();
                kept = true;
            }
            if held.paused_reason.as_deref() == Some("authentication") && routine.is_enabled {
                routine.is_enabled = false;
                routine.paused_reason = held.paused_reason.clone();
                kept = true;
            }
        }
    }
    kept
}

/// Pauses or resumes a routine. A resumed schedule counts from now.
pub fn set_enabled(app: &Arc<App>, id: &str, enabled: bool) -> Result<Routine, String> {
    if enabled {
        crate::workflows::allow_enable(app, id)?;
    }
    app.update_routine(id, |routine| {
        if enabled && !routine.is_enabled {
            routine.enabled_at = now_secs();
            if let Some(health) = routine.health.as_mut() {
                health.resume();
            }
        }
        routine.is_enabled = enabled;
        routine.paused_reason = None;
    })
    .map_err(|e| e.to_string())
}

pub fn delete(app: &Arc<App>, id: &str) -> Result<(), String> {
    let events = app.routine(id).and_then(|routine| routine.events);
    app.delete_routine(id).map_err(|e| e.to_string())?;
    // A routine on events takes its subscription and its row on the relay with it.
    #[cfg(feature = "runner")]
    if let Some(events) = events.filter(|events| !events.subscription_id.is_empty()) {
        crate::routine_events::forget(app, &events.subscription_id);
    }
    #[cfg(not(feature = "runner"))]
    let _ = events;
    Ok(())
}

fn clean_name(name: &str) -> Result<String, String> {
    let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("Give the routine a name.".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("Keep the name under {MAX_NAME_CHARS} characters."));
    }
    Ok(name)
}

/// The schedule in words and the next firing from now, in `timezone` (this Runner's when
/// `None`), for a caller that wants to check a schedule and its policy before saving them.
pub fn describe(
    schedule_text: &str,
    timezone: Option<&str>,
    missed_run_policy: Option<&str>,
) -> Result<Value, String> {
    let schedule = schedule::parse(schedule_text)?;
    let timezone = match timezone {
        Some(zone) => schedule::timezone(zone)?.to_string(),
        None => schedule::local_timezone(),
    };
    let missed_run_policy = missed_run_policy
        .map(crate::routine_health::MissedRunPolicy::parse)
        .transpose()?
        .unwrap_or_default();
    Ok(serde_json::json!({
        "schedule": schedule.canonical(),
        "text": schedule.describe(),
        "timezone": timezone,
        "missed_run_policy": missed_run_policy,
        "next_run_at": schedule.next_after(now_unix(), &timezone).map(|t| t as f64),
    }))
}

/// Explicit local user authority, never called by the bot's routine tool or provisioning.
pub fn reauthorize(app: &Arc<App>, id: &str) -> Result<Routine, String> {
    let _policy = app.roster_edit.lock().unwrap();
    let _lifecycle = app.plugin_admission(None)?;
    let routine = app.routine(id).ok_or("Unknown routine")?;
    if !routine.is_enabled {
        return Err("Enable the routine before reauthorizing its schedule.".into());
    }
    let chat = app
        .state
        .lock()
        .unwrap()
        .chats
        .iter()
        .find(|c| c.meta.kind == "dm" && c.meta.bot_ids.contains(&routine.bot_id))
        .cloned()
        .ok_or("Routine DM unavailable")?;
    let job = job_in_chat(app, &routine, chat.meta.id);
    app.check_local_job(&job).map_err(|e| e.to_string())?;
    if !app.local_device().is_some_and(|d| d.is_runner()) {
        return Err("Local Runner required".into());
    }
    let chat = app.chat(&job.chat_id).ok_or("Routine DM unavailable")?;
    if chat.meta.kind != "dm" {
        return Err("Routine requires its direct chat.".into());
    }
    let lease = app.task_lease(&job).map_err(|e| e.to_string())?;
    let next = match routine.next_run_at() {
        Some(next) => next,
        None if routine.calendar.is_some() => i64::MAX,
        None => return Err("Schedule has no future occurrence".into()),
    };
    let binding = crate::local_store::schedule_binding(&app.state.lock().unwrap(), &routine)
        .map_err(|e| e.to_string())?;
    app.store
        .reauthorize_schedule(&lease, id, &binding, next)
        .map_err(|e| e.to_string())?;
    Ok(routine)
}

#[cfg(feature = "runner")]
fn capture_schedule(
    app: &App,
    routine: &Routine,
    now: i64,
) -> anyhow::Result<Option<crate::local_store::ScheduleLease>> {
    let binding = crate::local_store::schedule_binding(&app.state.lock().unwrap(), routine)?;
    let next = routine.next_run_after(now).unwrap_or(i64::MAX);
    app.store.schedule_lease(&routine.id, &binding, now, next)
}
// MARK: - Running

/// Starts a run of the routine now, here or on the bot's Runner through the relay. Refuses
/// while a run is going on.
pub fn run_now(app: &Arc<App>, id: &str) -> Result<(), String> {
    if app.is_paused() {
        return Err("Account paused. Resume to run routines.".into());
    }
    app.budgets.admit(app, "routine", id)?;
    let routine = app.routine(id).ok_or("Unknown routine")?;
    if routine.paused_reason.as_deref() == Some("authentication") {
        return Err(signed_out_text(&routine));
    }
    if app.is_routine_running(&routine.id) {
        return Err(format!("{} is running right now.", routine.name));
    }
    let job = job_for(app, &routine)?;
    let admission = app
        .update
        .try_admit()
        .ok_or(crate::update_control::UPDATING)?;
    runtime::start_turn(app, job, admission);
    Ok(())
}

/// Why a routine paused by failed sign-ins does not run, and what brings it back.
pub fn signed_out_text(routine: &Routine) -> String {
    format!(
        "{} is paused after three failed sign-ins in a row. Reconnect, then resume it.",
        routine.name
    )
}

/// The job that runs a routine: a `routine` turn in the bot's direct chat.
fn job_for(app: &Arc<App>, routine: &Routine) -> Result<Job, String> {
    let dm = app
        .dm_with(&routine.bot_id, None)
        .map_err(|e| e.to_string())?;
    Ok(job_in_chat(app, routine, dm.meta.id))
}

fn job_in_chat(app: &App, routine: &Routine, chat_id: String) -> Job {
    Job {
        id: format!("job-{}", uuid::Uuid::new_v4()),
        chat_id,
        bot_id: routine.bot_id.clone(),
        kind: "routine".into(),
        task_id: None,
        task_context: None,
        trigger_message_id: String::new(),
        handoff: None,
        routine_id: Some(routine.id.clone()),
        check: None,
        requested_by: app.this_device_id().unwrap_or_default(),
        from_bot_id: None,
        hops: 0,
        round: 0,
        is_winding_down: false,
        setup: None,
        created_at: now_secs(),
    }
}

/// A run began on this Runner: the schedule counts from now, so a slow run is not queued
/// again behind itself.
pub fn started(app: &Arc<App>, id: &str) {
    let _ = app.update_routine(id, |routine| routine.last_run_at = Some(now_secs()));
}

/// A run ended: what the panel shows as the last outcome.
pub fn finished(app: &Arc<App>, id: &str, outcome: TurnOutcome) {
    let outcome = match outcome {
        TurnOutcome::Sent => "sent",
        TurnOutcome::Pass => "pass",
        TurnOutcome::Skipped => "error",
    };
    let _ = app.update_routine(id, |routine| routine.last_outcome = Some(outcome.into()));
    end_if_finished(app, id);
}

/// Removes a routine that is done: a one-time routine after its run, a routine on events after
/// the run of the event that ended its subject. The chat keeps the run.
pub fn end_if_finished(app: &Arc<App>, id: &str) {
    if app
        .routine(id)
        .is_some_and(|routine| crate::routine_triggers::is_finished(&routine))
    {
        if let Err(error) = delete(app, id) {
            tracing::warn!(%error, routine = %id, "removing a finished routine");
        }
    }
}

/// A run ended, after the provider's own retries: a failure to reach or sign in to the provider
/// counts in the runs' streak (never in the checks'), and three failed sign-ins in a row pause
/// the routine until it is resumed. Anything else leaves the streak as it was.
pub fn model_result(app: &App, id: &str, error: Option<&str>) {
    let Some(routine) = app.routine(id) else {
        return;
    };
    let before = routine
        .health
        .as_ref()
        .map(|health| health.model.clone())
        .unwrap_or_default();
    let mut model = before.clone();
    match error {
        Some(error) => model.record_failure(now_secs(), crate::routine_health::classify(error)),
        None => model = Default::default(),
    }
    if model == before {
        return;
    }
    let _ = app.update_routine(id, |routine| {
        let health = routine.health.get_or_insert_with(Default::default);
        health.model = model;
        health.updated_at = now_secs();
        if health.model.authentication_failures >= 3 {
            routine.is_enabled = false;
            routine.paused_reason = Some("authentication".into());
        }
    });
}

/// The scheduler: every half minute, run the routines of this Runner's bots that are due.
#[cfg(feature = "runner")]
pub async fn run(app: Arc<App>) {
    loop {
        tokio::time::sleep(TICK).await;
        crate::feedback::tick(&app);
        tick(&app);
    }
}

/// Starts every due routine of a bot on this Runner, or its check, unless the user has been
/// away, in which case the due ones are paused with a notice instead. A routine around calendar
/// events reads its calendar on its own clock, and runs once for each event that is due. A
/// routine that is done, but was not removed when its run ended (a restart), goes now.
#[cfg(feature = "runner")]
pub fn tick(app: &Arc<App>) {
    if !app.is_execution_owner() || app.is_paused() {
        return;
    }
    let Some(this) = app.this_device_id() else {
        return;
    };
    let now = now_unix();
    let all: Vec<Routine> = app.state.lock().unwrap().routines.clone();
    let mine: Vec<Routine> = all
        .into_iter()
        .filter(|r| app.bot(&r.bot_id).is_some_and(|b| b.runner_id == this))
        .collect();
    for routine in mine.iter().filter(|r| {
        crate::routine_triggers::is_finished(r)
            && !app.is_routine_running(&r.id)
            && !app.routine_checks.is_running(&r.id)
    }) {
        end_if_finished(app, &routine.id);
    }
    // Routines on events still waiting on the relay or on the user's setup try again, and the
    // subscriptions of ones that are gone go.
    crate::routine_events::retry_waiting(app, &mine);
    crate::routine_events::sweep(app);
    let enabled: Vec<Routine> = mine
        .into_iter()
        .filter(|r| r.is_enabled && !crate::routine_triggers::is_finished(r))
        .collect();
    // A routine stopped at its limits reads nothing either.
    for routine in enabled.iter().filter(|r| {
        crate::routine_triggers::calendar_sync_due(r, now)
            && !app.routine_checks.is_running(&r.id)
            && app.budgets.admit(app, "routine", &r.id).is_ok()
    }) {
        let Some(admission) = app.update.try_admit() else {
            return;
        };
        let app = app.clone();
        let routine = routine.clone();
        tokio::spawn(async move {
            let _admission = admission;
            let _ = check_now(&app, &routine, &CancellationToken::new()).await;
        });
    }
    let due: Vec<Routine> = enabled
        .into_iter()
        .filter(|r| {
            app.bot(&r.bot_id).is_some_and(|b| b.runner_id == this)
                && !app.is_routine_running(&r.id)
                && !app.routine_checks.is_running(&r.id)
        })
        .filter(|r| capture_schedule(app, r, now).ok().flatten().is_some())
        // A run the bot's old Runner still finishes after a move comes first.
        .filter(|r| !app.routine_running_elsewhere(&r.id))
        .filter(|r| app.budgets.admit(app, "routine", &r.id).is_ok())
        .collect();
    if due.is_empty() {
        return;
    }
    if user_away(app, now) {
        pause_while_away(app, &due);
        return;
    }
    for routine in due {
        // While an update holds new work back, a due routine stays due: the next tick after
        // the lease ends starts it.
        let Some(admission) = app.update.try_admit() else {
            return;
        };
        let schedule = match capture_schedule(app, &routine, now) {
            Ok(Some(schedule)) => schedule,
            Ok(None) => continue,
            Err(error) => {
                tracing::error!(%error,"Schedule capture refused");
                continue;
            }
        };
        if let Ok(schedule::Schedule::Events(offset)) = schedule::parse(&routine.schedule) {
            run_due_event(app, &routine, offset, now, &schedule, admission);
            continue;
        }
        if routine
            .missed_run_policy
            .should_skip(schedule.occurrence, now)
        {
            let result = (|| -> anyhow::Result<()> {
                let mut job = job_for(app, &routine).map_err(anyhow::Error::msg)?;
                let lease = app.task_lease(&job)?;
                commit_scheduled_check(
                    app,
                    &routine,
                    &mut job,
                    &lease,
                    &schedule,
                    &CheckRun {
                        found: None,
                        error: None,
                        result: String::new(),
                    },
                    &Default::default(),
                    &CancellationToken::new(),
                )?;
                app.record_routine(&routine.id, |r| r.last_scheduled_at = Some(now as f64))?;
                Ok(())
            })();
            if let Err(error) = result {
                tracing::error!(%error, "Skipped occurrence transaction refused");
            }
            continue;
        }
        if routine.check.is_some() {
            check_then_run(app, routine, schedule, admission);
            continue;
        }
        match job_for(app, &routine).and_then(|mut job| {
            let lease = app.task_lease(&job).map_err(|e| e.to_string())?;
            let run = commit_scheduled_check(
                app,
                &routine,
                &mut job,
                &lease,
                &schedule,
                &CheckRun {
                    found: None,
                    error: None,
                    result: String::new(),
                },
                &beans_agent::codemode::StoreWrites::default(),
                &CancellationToken::new(),
            )
            .map_err(|e| e.to_string())?;
            Ok((job, lease, run))
        }) {
            Ok((job, lease, run)) => {
                if run {
                    started(app, &routine.id);
                    runtime::spawn_admitted_local_job(app.clone(), job, None, admission, lease);
                }
            }
            Err(error) => tracing::warn!(%error, routine = %routine.name, "starting a routine"),
        }
    }
}

/// Runs a routine around calendar events for its earliest due event; the next one waits for the
/// next tick, after this run. Each event is taken before its run starts, so it runs once. One
/// whose run was due before it starts and which has started already is only taken, and so,
/// with `skip`, is one more than a minute late.
#[cfg(feature = "runner")]
fn run_due_event(
    app: &Arc<App>,
    routine: &Routine,
    offset: schedule::EventOffset,
    now: i64,
    schedule: &crate::local_store::ScheduleLease,
    admission: crate::update_control::Admission,
) {
    let Some((due_at, event)) = crate::routine_triggers::next_event_run(routine, offset)
        .map(|(due, event)| (due, event.clone()))
    else {
        return;
    };
    let skipped = (!offset.after && event.start as i64 <= now)
        || routine.missed_run_policy.should_skip(due_at, now);
    let result = (|| -> anyhow::Result<_> {
        let mut job = job_for(app, routine).map_err(anyhow::Error::msg)?;
        let lease = app.task_lease(&job)?;
        let found = CheckRun {
            found: Some(crate::routine_triggers::event_text(routine, &event)),
            error: None,
            result: String::new(),
        };
        let cancel = CancellationToken::new();
        if skipped {
            cancel.cancel();
        }
        // Consuming a skipped event is still authorized schedule work; no model task is queued.
        let run = commit_scheduled_event(
            app, routine, &mut job, &lease, schedule, &event, &found, skipped,
        )?;
        Ok((job, lease, run))
    })();
    match result {
        Ok((job, lease, run)) => {
            if let Err(error) = app.record_routine(&routine.id, |current| {
                if let Some(calendar) = current.calendar.as_mut() {
                    calendar.done.push(event.key());
                }
                current.last_scheduled_at = Some(now as f64);
            }) {
                tracing::error!(%error, "saving consumed calendar event; queued intent remains durable");
                return;
            }
            if run {
                runtime::spawn_admitted_local_job(app.clone(), job, None, admission, lease);
            }
        }
        Err(error) => tracing::error!(%error, "calendar schedule transaction refused"),
    }
}

#[cfg(feature = "runner")]
fn commit_scheduled_event(
    app: &App,
    routine: &Routine,
    job: &mut Job,
    lease: &crate::local_store::TaskLease,
    schedule: &crate::local_store::ScheduleLease,
    event: &crate::routine_triggers::CalendarEvent,
    found: &CheckRun,
    skipped: bool,
) -> anyhow::Result<bool> {
    let _policy = app.roster_edit.lock().unwrap();
    let _lifecycle = app
        .plugin_admission(Some(lease.incarnation))
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(!app.is_paused(), "Account paused");
    let mut current = app
        .routine(&routine.id)
        .ok_or_else(|| anyhow::anyhow!("Routine deleted"))?;
    anyhow::ensure!(
        current.is_enabled && crate::routine_triggers::same_target(&current, routine),
        "Routine changed during event admission"
    );
    let binding = crate::local_store::schedule_binding(&app.state.lock().unwrap(), &current)?;
    anyhow::ensure!(binding == schedule.binding, "Schedule revision changed");
    app.check_local_job(job)?;
    anyhow::ensure!(
        app.local_device().is_some_and(|d| d.is_runner()),
        "Local Runner required"
    );
    let offset = match schedule::parse(&current.schedule).map_err(anyhow::Error::msg)? {
        schedule::Schedule::Events(offset) => offset,
        _ => anyhow::bail!("Calendar schedule changed"),
    };
    anyhow::ensure!(
        crate::routine_triggers::next_event_run(&current, offset)
            .is_some_and(|(due, held)| due == schedule.occurrence && held == event),
        "Calendar occurrence changed during admission"
    );
    if let Some(calendar) = current.calendar.as_mut() {
        calendar.done.push(event.key());
    }
    let next = current
        .next_run_at()
        .filter(|next| *next > schedule.occurrence)
        .unwrap_or(i64::MAX);
    job.check = found.report();
    app.store.commit_scheduled_occurrence(
        lease,
        (!skipped).then_some(&*job),
        &routine.id,
        schedule,
        next,
        &job.chat_id,
        &job.bot_id,
        now_unix(),
        &Default::default(),
        &[],
    )?;
    Ok(!skipped)
}

// MARK: - Checks

/// How long a check may take, plugin servers starting included.
#[cfg(feature = "runner")]
const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// The most of what a check found that its run reads.
#[cfg(feature = "runner")]
const MAX_FOUND_CHARS: usize = 4_000;
/// The bot's file tools a check may call: the ones that only read.
#[cfg(feature = "runner")]
const CHECK_FILE_TOOLS: [&str; 4] = ["read", "grep", "find", "ls"];

/// In-flight check locks. Completed health and schedule anchors live in the durable roster.
#[cfg(feature = "runner")]
#[derive(Default)]
pub struct Checks(std::sync::Mutex<std::collections::HashMap<String, CheckState>>);

#[cfg(feature = "runner")]
#[derive(Default, Clone)]
struct CheckState {
    running: bool,
    cancel: Option<CancellationToken>,
}

#[cfg(feature = "runner")]
impl Checks {
    pub fn is_running(&self, id: &str) -> bool {
        self.0
            .lock()
            .unwrap()
            .get(id)
            .is_some_and(|state| state.running)
    }

    /// Marks the routine's check running; false when it already is.
    fn start(&self, id: &str, cancel: &CancellationToken) -> bool {
        let mut checks = self.0.lock().unwrap();
        let state = checks.entry(id.to_string()).or_default();
        if state.running {
            return false;
        }
        state.running = true;
        state.cancel = Some(cancel.clone());
        true
    }

    /// Marks the routine's check running once no other check of it runs; false when `cancel`
    /// stops the wait.
    async fn start_when_free(&self, id: &str, cancel: &CancellationToken) -> bool {
        while !self.start(id, cancel) {
            tokio::select! {
                _ = cancel.cancelled() => return false,
                _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
            }
        }
        true
    }

    pub fn cancel_all(&self) {
        for state in self.0.lock().unwrap().values() {
            if let Some(cancel) = &state.cancel {
                cancel.cancel();
            }
        }
    }

    fn finish(&self, id: &str) {
        self.0.lock().unwrap().insert(
            id.to_string(),
            CheckState {
                running: false,
                cancel: None,
            },
        );
    }
}

/// A check ended: its health is saved, so the schedule counts from it after a restart too. The
/// roster goes up when the check changed how the routine stands (it failed, or passed after a
/// failure); a quiet check after another changes nothing the other Devices show.
#[cfg(feature = "runner")]
fn checked(app: &App, routine: &Routine, run: &CheckRun) -> Result<(), String> {
    let failure = run.error.as_deref().map(crate::routine_health::classify);
    let mut changed = false;
    let saved = app.record_routine(&routine.id, |current| {
        if !crate::routine_triggers::same_target(current, routine)
            || current.enabled_at > routine.enabled_at
        {
            return;
        }
        let health = current.health.get_or_insert_with(Default::default);
        let before = health.status;
        health.record(now_secs(), run.found.is_some(), failure);
        changed = failure.is_some() || health.status != before;
        if health.authentication_failures >= 3 {
            current.is_enabled = false;
            current.paused_reason = Some("authentication".into());
        }
    });
    app.routine_checks.finish(&routine.id);
    if saved.is_ok() && changed {
        app.push_roster();
    }
    saved
        .map(|_| ())
        .map_err(|error| format!("Could not save the routine's check: {error}"))
}

/// When this Runner runs the routine next: its schedule from the last run, or from the last
/// check here, which counts as a run whether or not it started one.
#[cfg(feature = "runner")]
fn due_at(app: &App, routine: &Routine) -> Option<i64> {
    match app
        .store
        .routine_checked_at(&routine.id)
        .ok()
        .flatten()
        .or_else(|| app.routine_checks.last_at(&routine.id))
    {
        Some(at) => routine.next_run_after(at),
        None => routine.next_run_at(),
    }
}

/// When the routine runs next, as the apps and the bot say it. Only the Runner knows when a
/// routine's check last ran; elsewhere a routine with a check that is past due is due at its
/// next schedule time, since its Runner has been checking in between.
pub fn next_run_shown(app: &App, routine: &Routine) -> Option<i64> {
    #[cfg(feature = "runner")]
    if let Some(at) = app.routine_checks.last_at(&routine.id) {
        return routine.next_run_after(at);
    }
    #[cfg(not(feature = "runner"))]
    let _ = app;
    let next = routine.next_run_at()?;
    let now = now_unix();
    if next < now - 60 {
        return schedule::parse(&routine.schedule)
            .ok()
            .and_then(|schedule| schedule.next_after(now, &routine.timezone))
            .or(Some(next));
    }
    Some(next)
}

/// Runs a due routine's check, then the routine when the check found something or failed. A
/// quiet check publishes its completed health without a marker or model turn. Transport
/// failures back off; authentication failures pause after three attempts.
///
/// One check of a routine runs at a time. A run started by hand meanwhile waits for this check
/// before it runs its own (`check_now`), which no longer sees what this one found and stored as
/// seen; so what this one found still starts its run, after that one, as the chat runs one turn
/// at a time.
#[cfg(feature = "runner")]
fn check_then_run(
    app: &Arc<App>,
    routine: Routine,
    schedule: crate::local_store::ScheduleLease,
    admission: crate::update_control::Admission,
) {
    if !app.is_execution_owner() {
        return;
    }
    let cancel = CancellationToken::new();
    if !app.routine_checks.start(&routine.id, &cancel) {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let _admission = admission;
        let mut job = match job_for(&app, &routine) {
            Ok(job) => job,
            Err(_) => {
                app.routine_checks.finish(&routine.id);
                return;
            }
        };
        let lease = match app.task_lease(&job) {
            Ok(lease) => lease,
            Err(_) => {
                app.routine_checks.finish(&routine.id);
                return;
            }
        };
        let (found, writes) = run_check_staged(&app, &routine, &cancel).await;
        let committed = commit_scheduled_check(
            &app, &routine, &mut job, &lease, &schedule, &found, &writes, &cancel,
        );
        if committed.is_ok() && !cancel.is_cancelled() {
            let _ = checked(&app, &routine, &found);
        }
        app.routine_checks.finish(&routine.id);
        match committed {
            Ok(true) => {
                runtime::spawn_admitted_local_job(app.clone(), job, None, app.update.hold(), lease)
            }
            Ok(false) => {}
            Err(error) => tracing::error!(%error,"Routine check transaction refused"),
        }
    });
}

#[cfg(feature = "runner")]
fn commit_scheduled_check(
    app: &App,
    routine: &Routine,
    job: &mut Job,
    lease: &crate::local_store::TaskLease,
    schedule: &crate::local_store::ScheduleLease,
    found: &CheckRun,
    writes: &beans_agent::codemode::StoreWrites,
    cancel: &CancellationToken,
) -> anyhow::Result<bool> {
    let _policy = app.roster_edit.lock().unwrap();
    let _lifecycle = app
        .plugin_admission(Some(lease.incarnation))
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        !cancel.is_cancelled() && !app.is_paused(),
        "Routine check was cancelled"
    );
    let current = app
        .routine(&routine.id)
        .ok_or_else(|| anyhow::anyhow!("Routine was deleted"))?;
    anyhow::ensure!(
        current.is_enabled
            && crate::routine_triggers::same_target(&current, routine)
            && current.bot_id == routine.bot_id,
        "Routine changed during check"
    );
    app.check_local_job(job)?;
    anyhow::ensure!(
        app.local_device().is_some_and(|d| d.is_runner()),
        "Local Runner required"
    );
    let binding = crate::local_store::schedule_binding(&app.state.lock().unwrap(), &current)?;
    anyhow::ensure!(binding == schedule.binding, "Schedule revision changed");
    let now = now_unix();
    let next = schedule::parse(&current.schedule)
        .map_err(anyhow::Error::msg)?
        .next_after(now.max(schedule.occurrence), &current.timezone)
        .unwrap_or(i64::MAX);
    job.check = found.report();
    let blocked = found.error.as_deref().is_some_and(|error| {
        matches!(
            crate::routine_health::classify(error),
            crate::routine_health::Failure::Connection
                | crate::routine_health::Failure::Authentication
                | crate::routine_health::Failure::Blocked
        )
    });
    let run = !blocked
        && !routine
            .missed_run_policy
            .should_skip(schedule.occurrence, now)
        && (routine.check.is_none() || job.check.is_some());
    app.store.commit_scheduled_occurrence(
        lease,
        run.then_some(&*job),
        &routine.id,
        schedule,
        next,
        &job.chat_id,
        &job.bot_id,
        now,
        &writes.set,
        &writes.delete,
    )?;
    Ok(run)
}

/// How a check went.
#[cfg(feature = "runner")]
#[derive(Debug, Clone, PartialEq)]
pub struct CheckRun {
    /// What it returned, as its run reads it; `None` when it returned nothing to report.
    pub found: Option<String>,
    /// How it failed, when it did.
    pub error: Option<String>,
    /// Its whole result, what it printed and its calls included, for the bot that wrote it.
    pub result: String,
}

#[cfg(feature = "runner")]
impl CheckRun {
    /// What the run reads about the check, when the check calls for a run.
    pub fn report(&self) -> Option<CheckReport> {
        (self.found.is_some() || self.error.is_some()).then(|| CheckReport {
            found: self.found.clone().unwrap_or_default(),
            error: self.error.clone(),
        })
    }
}

/// Runs a routine's check now, for a run started by hand or a check just saved: after a check of
/// the routine already running, and counted like a due one, so the schedule counts from it.
#[cfg(feature = "runner")]
pub async fn check_now(app: &Arc<App>, routine: &Routine, cancel: &CancellationToken) -> CheckRun {
    let failed = || CheckRun {
        found: None,
        error: Some("Check admission or storage failed.".into()),
        result: "Check admission or storage failed.".into(),
    };
    if !app.is_execution_owner() || app.is_paused() {
        return failed();
    }
    let Some(_admission) = app.update.try_admit() else {
        return failed();
    };
    if !app
        .routine_checks
        .start_when_free(&routine.id, cancel)
        .await
    {
        return failed();
    }
    let job = match job_for(app, routine) {
        Ok(job) => job,
        Err(_) => {
            app.routine_checks.finish(&routine.id);
            return failed();
        }
    };
    let lease = match app.task_lease(&job) {
        Ok(lease) => lease,
        Err(_) => {
            app.routine_checks.finish(&routine.id);
            return failed();
        }
    };
    let binding = match crate::local_store::schedule_binding(&app.state.lock().unwrap(), routine) {
        Ok(binding) => binding,
        Err(_) => {
            app.routine_checks.finish(&routine.id);
            return failed();
        }
    };
    let exhausted = if routine.calendar.is_some() {
        match app
            .store
            .schedule_lease(&routine.id, &binding, i64::MAX, i64::MAX)
        {
            Ok(schedule) => schedule.filter(|s| s.occurrence == i64::MAX),
            Err(_) => {
                app.routine_checks.finish(&routine.id);
                return failed();
            }
        }
    } else {
        None
    };
    let (found, writes) = run_check_staged(app, routine, cancel).await;
    let committed = (|| -> anyhow::Result<()> {
        let _policy = app.roster_edit.lock().unwrap();
        let _lifecycle = app
            .plugin_admission(Some(lease.incarnation))
            .map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            !cancel.is_cancelled()
                && !app.is_paused()
                && app
                    .routine(&routine.id)
                    .is_some_and(|r| r.check == routine.check),
            "Check changed"
        );
        let current = app
            .routine(&routine.id)
            .ok_or_else(|| anyhow::anyhow!("Routine deleted"))?;
        anyhow::ensure!(
            crate::local_store::schedule_binding(&app.state.lock().unwrap(), &current)? == binding,
            "Schedule changed during read"
        );
        app.store.commit_routine_check(
            &lease,
            None,
            &routine.id,
            &job.chat_id,
            &job.bot_id,
            now_unix(),
            &writes.set,
            &writes.delete,
        )?;
        if found.error.is_none() {
            if let (Some(schedule), Some(next)) = (&exhausted, current.next_run_at()) {
                app.store
                    .rearm_calendar(&lease, &routine.id, schedule, next)?;
            }
        }
        Ok(())
    })();
    app.routine_checks.finish(&routine.id);
    if committed.is_err() {
        failed()
    } else {
        if !cancel.is_cancelled() {
            let _ = checked(app, routine, &found);
        }
        found
    }
}

/// Runs a routine's check: its script in a codemode sandbox of its own, with the bot's file
/// tools that only read, the read-only tools of this Runner's plugins, the values the bot's
/// scripts keep in its direct chat, and `models.ask`. No model of the bot's runs and nobody is
/// asked anything: a call that could change something ends the check.
#[cfg(feature = "runner")]
pub async fn run_check(app: &Arc<App>, routine: &Routine, cancel: &CancellationToken) -> CheckRun {
    let (result, _) = run_check_staged(app, routine, cancel).await;
    result
}

#[cfg(feature = "runner")]
async fn run_check_staged(
    app: &Arc<App>,
    routine: &Routine,
    cancel: &CancellationToken,
) -> (CheckRun, beans_agent::codemode::StoreWrites) {
    let failed = |error: String| {
        (
            CheckRun {
                found: None,
                error: Some(error.clone()),
                result: error,
            },
            Default::default(),
        )
    };
    let budget = match crate::budgets::for_routine(app, routine) {
        Ok(budget) => budget,
        Err(error) => return failed(error),
    };
    match budget
        .run(cancel, run_budgeted_check_staged(app, routine, cancel))
        .await
    {
        Ok(result) => result,
        Err(error) => failed(error),
    }
}

#[cfg(feature = "runner")]
async fn run_budgeted_check_staged(
    app: &Arc<App>,
    routine: &Routine,
    cancel: &CancellationToken,
) -> (CheckRun, beans_agent::codemode::StoreWrites) {
    let failed = |error: String| {
        (
            CheckRun {
                found: None,
                error: Some(error.clone()),
                result: error,
            },
            beans_agent::codemode::StoreWrites::default(),
        )
    };
    if !app.is_execution_owner() || app.is_paused() || cancel.is_cancelled() {
        return failed("Routine execution is unavailable.".into());
    }
    let Some(bot) = app.bot(&routine.bot_id) else {
        return failed("The routine's bot is gone.".into());
    };
    let dm = match app.dm_with(&bot.id, None) {
        Ok(dm) => dm,
        Err(error) => return failed(error.to_string()),
    };
    // A calendar reads through its plugin alone, by the same rules as a check.
    if routine.calendar.is_some() {
        let catalog = crate::plugins::mcp::bot_catalog(app, &bot, &dm.meta.id, Vec::new());
        let runner = CheckRunner {
            app: app.clone(),
            catalog: catalog.clone(),
            bot,
            chat_id: dm.meta.id.clone(),
            budget: crate::budgets::current(),
        };
        return (
            crate::routine_triggers::read_calendar(app, routine, &catalog, &runner, cancel).await,
            Default::default(),
        );
    }
    let Some(code) = routine.check.as_deref() else {
        return (
            CheckRun {
                found: None,
                error: None,
                result: String::new(),
            },
            Default::default(),
        );
    };
    let files: Vec<Arc<dyn Tool>> =
        beans_agent::tools::coding_tools(bot.working_directory(&app.config.home))
            .into_iter()
            .filter(|tool| CHECK_FILE_TOOLS.contains(&tool.name()))
            .collect();
    let catalog = crate::plugins::mcp::bot_catalog(app, &bot, &dm.meta.id, files);
    let values = match app.store.codemode_values(&dm.meta.id, &bot.id) {
        Ok(values) => values,
        Err(_) => return failed("Check storage is unavailable.".into()),
    };
    let store = Arc::new(crate::scripts::CheckStore::new(values));
    let functions: Vec<Arc<dyn HostFunction>> =
        crate::scripts::ModelsAsk::new(app, &dm.meta.id, &bot.provider)
            .map(|ask| Arc::new(ask) as Arc<dyn HostFunction>)
            .into_iter()
            .collect();
    let options = CodemodeOptions {
        mcp_types: !crate::plugins::mcp::plugin_briefs(app, Some(&bot.id)).is_empty(),
        timeout: CHECK_TIMEOUT,
        ..CodemodeOptions::default()
    };
    let codemode = CodemodeTool::new(catalog.clone(), options)
        .with_store(store.clone())
        .with_functions(functions);
    let runner = CheckRunner {
        app: app.clone(),
        catalog,
        bot,
        chat_id: dm.meta.id.clone(),
        budget: crate::budgets::current(),
    };
    match codemode
        .run_script(
            &format!("check-{}", routine.id),
            code,
            cancel.clone(),
            &runner,
        )
        .await
    {
        Err(error) => failed(error.0),
        Ok(run) => {
            let result = run.result.text_content();
            if run.result.is_error {
                (
                    CheckRun {
                        found: None,
                        error: Some(clipped(&result, MAX_FOUND_CHARS)),
                        result,
                    },
                    Default::default(),
                )
            } else {
                (
                    CheckRun {
                        found: run.returned.as_ref().and_then(found_text),
                        error: None,
                        result,
                    },
                    store.writes(),
                )
            }
        }
    }
}

/// What a check returned, as its run reads it: a string as it is, anything else as JSON.
/// Nothing, `null`, `false`, and an empty string, list, or object have nothing to report.
#[cfg(feature = "runner")]
fn found_text(value: &Value) -> Option<String> {
    let text = match value {
        Value::Null | Value::Bool(false) => return None,
        Value::Array(items) if items.is_empty() => return None,
        Value::Object(fields) if fields.is_empty() => return None,
        Value::String(text) => text.trim().to_string(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    (!text.is_empty()).then(|| clipped(&text, MAX_FOUND_CHARS))
}

#[cfg(feature = "runner")]
fn clipped(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        format!("{}…", text.chars().take(max).collect::<String>())
    } else {
        text.to_string()
    }
}

/// Runs a check's calls: the read-only ones the bot's Access allows, and no other. A refused
/// call ends the check.
#[cfg(feature = "runner")]
struct CheckRunner {
    app: Arc<App>,
    catalog: Arc<crate::plugins::mcp::PluginCatalog>,
    bot: Bot,
    chat_id: String,
    budget: Option<crate::budgets::BudgetContext>,
}

#[cfg(feature = "runner")]
#[async_trait::async_trait]
impl ToolRunner for CheckRunner {
    async fn run(
        &self,
        tool: Arc<dyn Tool>,
        tool_call_id: String,
        args: Value,
        cancel: CancellationToken,
    ) -> ToolOutcome {
        if self.app.is_paused() || cancel.is_cancelled() {
            return ToolOutcome {
                result: ToolResult {
                    is_error: true,
                    ..ToolResult::text("Account paused.")
                },
                is_error: true,
                blocked: true,
            };
        }
        if let Some(reason) = self.budget.as_ref().and_then(|budget| budget.check().err()) {
            return ToolOutcome {
                result: ToolResult {
                    is_error: true,
                    ..ToolResult::text(reason)
                },
                is_error: true,
                blocked: true,
            };
        }
        let allowed = if CHECK_FILE_TOOLS.contains(&tool.name()) {
            crate::permissions::check_tool(&self.app, &self.bot, tool.name())
        } else {
            crate::plugins::mcp::authorize_catalog_tool(
                &self.app,
                &self.catalog,
                &self.bot,
                tool.name(),
                &cancel,
            )
            .await
        };
        if let Err(denied) = allowed {
            let refusal = crate::permissions::refuse(&self.app, &self.chat_id, &self.bot, denied);
            return ToolOutcome {
                result: ToolResult {
                    is_error: true,
                    ..ToolResult::text(refusal.reason.unwrap_or_default())
                },
                is_error: true,
                blocked: true,
            };
        }
        let reads = CHECK_FILE_TOOLS.contains(&tool.name())
            || crate::plugins::mcp::is_read_only(&self.app, &self.catalog, tool.name(), &cancel)
                .await;
        if !reads {
            let refusal = format!("{} can change things, and a check only looks: leave it to the run the check starts.", tool.name());
            return ToolOutcome {
                result: ToolResult {
                    is_error: true,
                    ..ToolResult::text(refusal)
                },
                is_error: true,
                blocked: true,
            };
        }
        let plugin_id = self.catalog.plugin_id(tool.name());
        let allowed = self
            .app
            .bot(&self.catalog.bot_id().unwrap_or_default())
            .is_some_and(|bot| bot.capabilities.permits(tool.name(), plugin_id.as_deref()));
        if self.app.is_paused() || !allowed {
            return ToolOutcome {
                result: ToolResult {
                    is_error: true,
                    ..ToolResult::text("Account paused or tool access removed.")
                },
                is_error: true,
                blocked: true,
            };
        }
        let mut outcome = DirectRunner.run(tool, tool_call_id, args, cancel).await;
        // As a turn's results do, what a check reads has this Runner's saved secrets replaced
        // by their placeholders before the script, the run it starts, or the roster sees it.
        if let Some(clean) = crate::secrets::scrub_result(&self.app, &outcome.result) {
            if let Some(content) = clean.content {
                outcome.result.content = content;
            }
            if let Some(details) = clean.details {
                outcome.result.details = details;
            }
            if clean.structured.is_some() {
                outcome.result.structured = clean.structured;
            }
        }
        outcome
    }
}

/// True when the user has not written in any chat for a week. A user who never wrote counts
/// from the newest routine, so a fresh account is never "away".
#[cfg(any(feature = "runner", test))]
fn user_away(app: &Arc<App>, now: i64) -> bool {
    let last_routine = app
        .state
        .lock()
        .unwrap()
        .routines
        .iter()
        .map(|r| r.created_at.max(r.enabled_at) as i64)
        .max();
    let last_message = app.store.last_user_at().unwrap_or(None);
    let last_activity = last_message.max(last_routine).unwrap_or(now);
    now - last_activity > AWAY_AFTER_SECS
}

/// Pauses the routines with a notice in each bot's chat, in Grok Bot's words.
#[cfg(feature = "runner")]
fn pause_while_away(app: &Arc<App>, routines: &[Routine]) {
    let mut by_bot: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for routine in routines {
        let _ = app.update_routine(&routine.id, |r| {
            r.is_enabled = false;
            r.paused_reason = Some("away".into());
        });
        by_bot
            .entry(routine.bot_id.clone())
            .or_default()
            .push(routine.name.clone());
    }
    for (bot_id, names) in by_bot {
        let Ok(dm) = app.dm_with(&bot_id, None) else {
            continue;
        };
        let days = AWAY_AFTER_SECS / 86_400;
        app.notice(
            &dm.meta.id,
            format!(
                "Routines paused while you were away: {}. Nobody has written in {days} days, so they stopped to avoid wasted spend. Turn them back on from this chat's inspector, or ask here.",
                names.join(", ")
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ScratchApp(Arc<App>, std::path::PathBuf);
    impl Drop for ScratchApp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    /// An App with an identity on a scratch home, so this Device is a Runner, and one bot
    /// `b1` (Chef) assigned to it.
    fn scratch_app() -> ScratchApp {
        let home = std::env::temp_dir().join(format!("beans-routines-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&home).unwrap();
        let app = App::load_owner(crate::config::Config {
            home: home.clone(),
            port: 0,
        })
        .unwrap();
        crate::identity::create(&app, Some("Workbench".into())).unwrap();
        let runner_id = app.this_device_id().unwrap();
        {
            let mut state = app.state.lock().unwrap();
            state.bots.clear();
            state.chats.clear();
            state.bots.push(Bot {
                id: "b1".into(),
                name: "Chef".into(),
                description: String::new(),
                symbol_name: String::new(),
                shape: String::new(),
                accent: String::new(),
                avatar: None,
                look: None,
                runner_id,
                provider: "deepseek".into(),
                model: None,
                thinking: None,
                legacy_instructions: String::new(),
                workdir: None,
                capabilities: Default::default(),
                permissions: None,
                created_at: 0.0,
            });
        }
        ScratchApp(app, home)
    }

    #[cfg(feature = "runner")]
    #[tokio::test]
    async fn explicit_schedule_authority_barrier_duplicate_and_reopen() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let routine = create(
            app,
            "b1",
            "Watch CAS",
            "every 10m",
            "Report",
            Some("return 'new';"),
            true,
        )
        .unwrap();
        let now = now_unix();
        app.dm_with("b1", None).unwrap();
        assert!(capture_schedule(app, &routine, now + 600)
            .unwrap()
            .is_none());
        crate::api::dispatch(
            app,
            "routines.reauthorize",
            serde_json::json!({"id":routine.id}),
        )
        .await
        .unwrap();
        let arm = |at| {
            app.store
                .connection
                .lock()
                .unwrap()
                .execute(
                    "UPDATE routine_schedule_authority SET next_at=?1 WHERE routine_id=?2",
                    rusqlite::params![at, routine.id],
                )
                .unwrap();
            capture_schedule(app, &routine, at).unwrap().unwrap()
        };
        let schedule = arm(now);
        let mut job = job_for(app, &routine).unwrap();
        let lease = app.task_lease(&job).unwrap();
        let writes = beans_agent::codemode::StoreWrites {
            set: std::collections::BTreeMap::from([("seen".into(), serde_json::json!(true))]),
            delete: vec![],
        };
        let found = CheckRun {
            found: Some("new".into()),
            error: None,
            result: "new".into(),
        };
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let worker = app.clone();
        let captured = routine.clone();
        let old = schedule.clone();
        let pending = tokio::spawn(async move {
            ready_tx.send(()).unwrap();
            release_rx.await.unwrap();
            commit_scheduled_check(
                &worker,
                &captured,
                &mut job,
                &lease,
                &old,
                &found,
                &writes,
                &CancellationToken::new(),
            )
        });
        ready_rx.await.unwrap();
        crate::api::dispatch(
            app,
            "routines.reauthorize",
            serde_json::json!({"id":routine.id}),
        )
        .await
        .unwrap();
        release_tx.send(()).unwrap();
        assert!(pending.await.unwrap().is_err());
        let dm = app.dm_with("b1", None).unwrap();
        assert!(!app
            .store
            .codemode_values(&dm.meta.id, "b1")
            .unwrap()
            .contains_key("seen"));
        let schedule = arm(now);
        let mut job = job_for(app, &routine).unwrap();
        let lease = app.task_lease(&job).unwrap();
        let found = CheckRun {
            found: Some("new".into()),
            error: None,
            result: "new".into(),
        };
        let writes = beans_agent::codemode::StoreWrites {
            set: std::collections::BTreeMap::from([("seen".into(), serde_json::json!(true))]),
            delete: vec![],
        };
        app.store.connection.lock().unwrap().execute_batch("CREATE TRIGGER reject_schedule BEFORE INSERT ON routine_check_commits BEGIN SELECT RAISE(ABORT,'synthetic rejection'); END;").unwrap();
        assert!(commit_scheduled_check(
            app,
            &routine,
            &mut job,
            &lease,
            &schedule,
            &found,
            &writes,
            &CancellationToken::new()
        )
        .is_err());
        assert_eq!(app.store.task_state(&lease).unwrap(), None);
        assert!(!app
            .store
            .codemode_values(&dm.meta.id, "b1")
            .unwrap()
            .contains_key("seen"));
        app.store
            .connection
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_schedule;")
            .unwrap();
        assert!(commit_scheduled_check(
            app,
            &routine,
            &mut job,
            &lease,
            &schedule,
            &found,
            &writes,
            &CancellationToken::new()
        )
        .unwrap());
        assert_eq!(
            app.store.task_state(&lease).unwrap(),
            Some(crate::local_store::TaskState::Queued)
        );
        assert_eq!(
            app.store
                .codemode_values(&dm.meta.id, "b1")
                .unwrap()
                .get("seen"),
            Some(&serde_json::json!(true))
        );
        assert!(commit_scheduled_check(
            app,
            &routine,
            &mut job,
            &lease,
            &schedule,
            &found,
            &writes,
            &CancellationToken::new()
        )
        .is_err());
        assert!(
            reauthorize(app, &routine.id).is_err(),
            "pending intent blocks new authority"
        );
        assert!(app.store.start_task(&lease).unwrap());
        let binding = crate::local_store::InvocationBinding {
            invocation_id: "invocation-schedule".into(),
            parent_invocation_id: None,
            ordinal: 0,
            attempt_id: "attempt-schedule".into(),
            receipt_id: "receipt-schedule".into(),
            revision: 1,
            digest: [42; 32],
        };
        assert!(app.store.insert_invocation(&lease, &binding).unwrap());
        assert!(app
            .store
            .decide_invocation(
                &lease,
                &binding,
                crate::local_store::AuthorizationDecision::Authorized,
                crate::local_store::AuthorizationKind::UserOnce,
                None,
                None,
                now
            )
            .unwrap());
        assert!(app.store.admit_invocation(&lease, &binding).unwrap());
        let reopened =
            crate::local_store::LocalStore::open(&scratch.1.join("beans.sqlite3")).unwrap();
        reopened.recover_task_owner("replacement-owner").unwrap();
        let count: i64 = reopened
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM routine_schedule_occurrences WHERE routine_id=?1",
                [&routine.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            reopened.task_state(&lease).unwrap(),
            Some(crate::local_store::TaskState::NeedsReview)
        );
        let mut recovered = lease.clone();
        recovered.owner_epoch = "replacement-owner".into();
        assert!(
            reopened
                .reauthorize_schedule(&recovered, &routine.id, &schedule.binding, now + 600)
                .is_err(),
            "recovered intent blocks new authority"
        );
        let history = reopened
            .history_task_lease("replacement-owner", &job.id, lease.incarnation)
            .unwrap()
            .unwrap();
        assert_eq!(
            history.owner_epoch, lease.owner_epoch,
            "history retains original execution identity"
        );
        assert!(reopened.resolve_task(&history, now).unwrap());
        assert!(
            reopened
                .reauthorize_schedule(&recovered, &routine.id, &schedule.binding, now + 600)
                .is_err(),
            "resolved history does not erase unknown receipt"
        );
        let state: String = reopened
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM task_effect_receipts WHERE receipt_id=?1",
                [&binding.receipt_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "unknown");
        reopened.prune_tasks(now + 30 * 24 * 60 * 60 + 1).unwrap();
        assert!(
            reopened
                .reauthorize_schedule(&recovered, &routine.id, &schedule.binding, now + 600)
                .is_err(),
            "retention cannot erase matching unknown denial"
        );
        let retained: String = reopened
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT state FROM task_effect_receipts WHERE receipt_id=?1",
                [&binding.receipt_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(retained, "unknown");
        let next_at: i64 = reopened
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT next_at FROM routine_schedule_authority WHERE routine_id=?1",
                [&routine.id],
                |r| r.get(0),
            )
            .unwrap();
        let next = reopened
            .schedule_lease(&routine.id, &schedule.binding, next_at, next_at + 600)
            .unwrap()
            .unwrap();
        let mut future = job.clone();
        future.id = format!("job-{}", uuid::Uuid::new_v4());
        let mut future_lease = recovered.clone();
        future_lease.task_id = future.id.clone();
        future_lease.execution_id = uuid::Uuid::new_v4().to_string();
        assert!(
            reopened
                .commit_scheduled_occurrence(
                    &future_lease,
                    Some(&future),
                    &routine.id,
                    &next,
                    next.occurrence + 600,
                    &future.chat_id,
                    &future.bot_id,
                    next_at,
                    &writes.set,
                    &[]
                )
                .is_err(),
            "unknown blocks a fresh occurrence under the current owner after pruning"
        );
        assert_eq!(reopened.task_state(&future_lease).unwrap(), None);
        assert!(reopened
            .commit_scheduled_occurrence(
                &lease,
                Some(&job),
                &routine.id,
                &schedule,
                now + 600,
                &job.chat_id,
                &job.bot_id,
                now,
                &writes.set,
                &[]
            )
            .is_err());
    }

    #[cfg(feature = "runner")]
    #[tokio::test]
    async fn scheduled_check_reassignment_wins_before_atomic_commit() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let routine = create(
            app,
            "b1",
            "Watch",
            "every 10m",
            "Report changes",
            Some("store('seen',true); return 'new';"),
            true,
        )
        .unwrap();
        app.dm_with("b1", None).unwrap();
        reauthorize(app, &routine.id).unwrap();
        let future = now_unix() + 600;
        app.store
            .connection
            .lock()
            .unwrap()
            .execute("UPDATE routine_schedule_authority SET next_at=?1", [future])
            .unwrap();
        let schedule = capture_schedule(app, &routine, future).unwrap().unwrap();
        let mut job = job_for(app, &routine).unwrap();
        let lease = app.task_lease(&job).unwrap();
        let writes = beans_agent::codemode::StoreWrites {
            set: std::collections::BTreeMap::from([("seen".into(), serde_json::json!(true))]),
            delete: vec![],
        };
        let found = CheckRun {
            found: Some("new".into()),
            error: None,
            result: "new".into(),
        };
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let worker = app.clone();
        let task_id = job.id.clone();
        let chat_id = job.chat_id.clone();
        let pending = tokio::spawn(async move {
            ready_tx.send(()).unwrap();
            release_rx.await.unwrap();
            commit_scheduled_check(
                &worker,
                &routine,
                &mut job,
                &lease,
                &schedule,
                &found,
                &writes,
                &CancellationToken::new(),
            )
        });
        ready_rx.await.unwrap();
        app.update_bot("b1", |bot| bot.runner_id = "other-runner".into())
            .unwrap();
        release_tx.send(()).unwrap();
        assert!(pending.await.unwrap().is_err());
        assert!(!app
            .store
            .codemode_values(&chat_id, "b1")
            .unwrap()
            .contains_key("seen"));
        let count: i64 = app
            .store
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM local_tasks WHERE task_id=?1",
                [task_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bot_permissions_apply_to_unattended_check_reads() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let routine = create(
            app,
            "b1",
            "Inbox",
            "every 1h",
            "Report changes",
            Some("return await tools.ls({});"),
            true,
        )
        .unwrap();
        let policy =
            serde_json::from_value(serde_json::json!({"filesystem":"none","shell":false})).unwrap();
        app.update_bot("b1", |bot| bot.permissions = Some(policy))
            .unwrap();
        let result = run_check(app, &routine, &CancellationToken::new()).await;
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("reading files is off")),
            "{:?}",
            result.error
        );
        // The routine shows as blocked, not as a check its bot can fix.
        assert_eq!(
            crate::routine_health::classify(result.error.as_deref().unwrap()),
            crate::routine_health::Failure::Blocked
        );
    }

    #[test]
    fn routines_are_checked_and_named_once() {
        let scratch = scratch_app();
        let app = &scratch.0;
        assert!(create(app, "b1", "Brief", "every 2m", "x", None, true)
            .unwrap_err()
            .contains("too often"));
        assert!(create(app, "b1", "", "every 1h", "x", None, true)
            .unwrap_err()
            .contains("name"));
        assert!(create(app, "b1", "Brief", "every 1h", "  ", None, true)
            .unwrap_err()
            .contains("should do"));
        let brief = create(
            app,
            "b1",
            "  Morning   brief ",
            "0 9 * * 1-5",
            "Summarize the inbox.",
            None,
            true,
        )
        .unwrap();
        assert_eq!(brief.name, "Morning brief");
        assert_eq!(brief.schedule, "0 9 * * 1-5");
        assert!(brief.is_enabled && brief.next_run_at().is_some());
        assert!(
            create(app, "b1", "morning BRIEF", "every 1h", "x", None, true)
                .unwrap_err()
                .contains("already exists")
        );
        assert!(create(app, "b2", "Other", "every 1h", "x", None, true)
            .unwrap_err()
            .contains("Unknown bot"));

        let edited = edit(app, &brief.id, None, Some("every 2h"), None, None).unwrap();
        assert_eq!(edited.schedule, "every 2h");
        assert!(edited.enabled_at >= brief.enabled_at);
        assert!(edit(app, &brief.id, None, None, None, None)
            .unwrap_err()
            .contains("Pass a new"));

        let paused = set_enabled(app, &brief.id, false).unwrap();
        assert!(!paused.is_enabled && paused.next_run_at().is_none());
        let resumed = set_enabled(app, &brief.id, true).unwrap();
        assert!(resumed.is_enabled && resumed.paused_reason.is_none());
        assert_eq!(app.routines_of("b1").len(), 1);
        delete(app, &brief.id).unwrap();
        assert!(app.routines_of("b1").is_empty());
        assert!(delete(app, &brief.id).is_err());
    }

    #[test]
    fn the_next_run_counts_from_the_last_one() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let hourly = create(app, "b1", "Hourly", "every 1h", "x", None, true).unwrap();
        assert_eq!(hourly.next_run_at(), Some(hourly.enabled_at as i64 + 3600));
        let ran = app
            .update_routine(&hourly.id, |r| {
                r.last_run_at = Some(hourly.enabled_at + 7200.0)
            })
            .unwrap();
        assert_eq!(ran.next_run_at(), Some(hourly.enabled_at as i64 + 10_800));
        // A resume after a long pause counts from the resume, not the old run.
        let paused = set_enabled(app, &hourly.id, false).unwrap();
        assert_eq!(paused.next_run_at(), None);
        let resumed = set_enabled(app, &hourly.id, true).unwrap();
        assert!(resumed.next_run_at().unwrap() >= resumed.enabled_at as i64 + 3600 - 1);
    }

    #[cfg(feature = "runner")]
    #[tokio::test]
    async fn execution_configuration_edits_revoke_schedule_authority() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let routine = create(app, "b1", "Bound", "every 1h", "x", None, true).unwrap();
        app.dm_with("b1", None).unwrap();
        crate::api::dispatch(
            app,
            "routines.reauthorize",
            serde_json::json!({"id": routine.id}),
        )
        .await
        .unwrap();
        let before = capture_schedule(app, &routine, i64::MAX).unwrap().unwrap();
        let changed = app
            .update_routine(&routine.id, |r| r.timezone = "UTC".into())
            .unwrap();
        // Force a different timezone even on a UTC host.
        let changed = if changed.timezone == routine.timezone {
            app.update_routine(&routine.id, |r| r.timezone = "Asia/Tokyo".into())
                .unwrap()
        } else {
            changed
        };
        assert!(capture_schedule(app, &changed, i64::MAX).unwrap().is_none());
        let mut job = job_for(app, &changed).unwrap();
        let lease = app.task_lease(&job).unwrap();
        assert!(commit_scheduled_check(
            app,
            &changed,
            &mut job,
            &lease,
            &before,
            &CheckRun {
                found: None,
                error: None,
                result: String::new()
            },
            &Default::default(),
            &CancellationToken::new()
        )
        .is_err());
        crate::api::dispatch(
            app,
            "routines.reauthorize",
            serde_json::json!({"id": routine.id}),
        )
        .await
        .unwrap();
        let policy = app
            .update_routine(&routine.id, |r| {
                r.missed_run_policy = crate::routine_health::MissedRunPolicy::Skip
            })
            .unwrap();
        assert!(capture_schedule(app, &policy, i64::MAX).unwrap().is_none());
    }

    /// The scheduler fires a due routine once, counts the schedule from that run, and refuses
    /// to queue it again while the first run has not ended. Without a provider the run ends in
    /// a notice and an error outcome, which is what the panel would show.
    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_tick_runs_a_due_routine_once() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let hourly = create(app, "b1", "Hourly", "every 1h", "x", None, true).unwrap();
        tick(app);
        assert_eq!(
            app.routine(&hourly.id).unwrap().last_run_at,
            None,
            "not due yet"
        );
        // Armed an hour and a bit ago: due now.
        let now = now_secs();
        app.update_routine(&hourly.id, |r| r.enabled_at = now - 3700.0)
            .unwrap();
        app.dm_with("b1", None).unwrap();
        reauthorize(app, &hourly.id).unwrap();
        app.store
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE routine_schedule_authority SET next_at=?1",
                [now_unix()],
            )
            .unwrap();
        tick(app);
        let ran = app.routine(&hourly.id).unwrap();
        let first_run = ran.last_run_at.expect("started");
        assert!(
            ran.next_run_at().unwrap() > now_unix() + 3500,
            "the next run counts from this one"
        );
        tick(app);
        // The run itself stamps the start too, a moment after the tick did.
        let again = app.routine(&hourly.id).unwrap().last_run_at.unwrap();
        assert!(
            (again - first_run).abs() < 5.0,
            "not started again: {again} vs {first_run}"
        );
        // The run ends: no provider on this Runner, so the chat says so and the outcome is an error.
        let dm = app.dm_with("b1", None).unwrap();
        for _ in 0..100 {
            if app.routine(&hourly.id).unwrap().last_outcome.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(
            app.routine(&hourly.id).unwrap().last_outcome.as_deref(),
            Some("error")
        );
        let notices: Vec<String> = app
            .messages(&dm.meta.id)
            .iter()
            .filter_map(|m| match &m.body {
                Body::Notice { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(notices[0], "Routine · Hourly", "the marker opens the run");
        assert!(notices[1].starts_with("Chef cannot run yet"), "{notices:?}");
        assert_eq!(notices.len(), 2, "one run, one marker: {notices:?}");
    }

    /// A due routine of a user who has been away for a week is paused with a notice, not run.
    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn away_pauses_due_routines_with_a_notice() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let brief = create(app, "b1", "Brief", "every 1h", "x", None, true).unwrap();
        let long_ago = now_secs() - (AWAY_AFTER_SECS as f64) - 7200.0;
        app.update_routine(&brief.id, |r| {
            r.created_at = long_ago;
            r.enabled_at = long_ago;
        })
        .unwrap();
        app.dm_with("b1", None).unwrap();
        reauthorize(app, &brief.id).unwrap();
        app.store
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE routine_schedule_authority SET next_at=?1 WHERE routine_id=?2",
                rusqlite::params![now_unix(), brief.id],
            )
            .unwrap();
        tick(app);
        let paused = app.routine(&brief.id).unwrap();
        assert!(
            !paused.is_enabled
                && paused.paused_reason.as_deref() == Some("away")
                && paused.last_run_at.is_none()
        );
        let dm = app.dm_with("b1", None).unwrap();
        let messages = app.messages(&dm.meta.id);
        let Body::Notice { text, routine_id } = &messages[0].body else {
            panic!("a notice")
        };
        assert!(
            text.starts_with("Routines paused while you were away: Brief."),
            "{text}"
        );
        assert_eq!(routine_id, &None);
        // Resuming arms it from now, so it is not due again at once.
        let resumed = set_enabled(app, &brief.id, true).unwrap();
        assert!(resumed.next_run_at().unwrap() > now_unix() + 3500);
        tick(app);
        assert_eq!(app.routine(&brief.id).unwrap().last_run_at, None);
    }

    #[test]
    fn a_week_of_silence_is_away() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let now = now_unix();
        assert!(!user_away(app, now), "a fresh account is not away");
        create(app, "b1", "Hourly", "every 1h", "x", None, true).unwrap();
        assert!(!user_away(app, now));
        assert!(user_away(app, now + AWAY_AFTER_SECS + 60));
        let dm = app.dm_with("b1", None).unwrap();
        let mut message = Message::new(&dm.meta.id, Author::You, Body::text("hi"));
        message.created_at = (now + AWAY_AFTER_SECS) as f64;
        app.upsert_message(message, false);
        assert!(!user_away(app, now + AWAY_AFTER_SECS + 60));
    }

    fn plain(id: &str, bot_id: &str, check: Option<&str>) -> Routine {
        Routine {
            id: id.into(),
            bot_id: bot_id.into(),
            name: id.into(),
            prompt: "x".into(),
            feedback_authorization_prompt: None,
            schedule: "every 1h".into(),
            timezone: "UTC".into(),
            missed_run_policy: Default::default(),
            last_scheduled_at: None,
            health: None,
            is_enabled: true,
            enabled_at: 0.0,
            last_run_at: None,
            last_outcome: None,
            paused_reason: None,
            check: check.map(str::to_string),
            events: None,
            calendar: None,
            created_at: 0.0,
        }
    }

    #[test]
    fn a_check_is_kept_as_written_and_removed_when_empty() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let watch = create(
            app,
            "b1",
            "Watch",
            "every 10m",
            "Tell me what is new.",
            Some("  return load('seen');  "),
            true,
        )
        .unwrap();
        assert_eq!(watch.check.as_deref(), Some("return load('seen');"));
        assert!(create(
            app,
            "b1",
            "Long",
            "every 1h",
            "x",
            Some(&"x".repeat(MAX_CHECK_CHARS + 1)),
            true
        )
        .unwrap_err()
        .contains("under"));
        #[cfg(feature = "runner")]
        assert!(create(
            app,
            "b1",
            "Bad",
            "every 1h",
            "x",
            Some("// @options: {\"retries\": 3}\nreturn 1"),
            true
        )
        .unwrap_err()
        .contains("@options"));
        assert_eq!(
            create(app, "b1", "Plain", "every 1h", "x", Some("   "), true)
                .unwrap()
                .check,
            None
        );
        assert_eq!(
            edit(app, &watch.id, None, None, None, Some(""))
                .unwrap()
                .check,
            None,
            "an empty check removes it"
        );
    }

    /// Only a Runner writes its routines' checks, so one an incoming roster leaves out was
    /// dropped by a build that does not know them.
    #[test]
    fn a_roster_without_checks_keeps_this_runners_checks() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let mut bots = app.state.lock().unwrap().bots.clone();
        let here = bots[0].runner_id.clone();
        let mut away = bots[0].clone();
        away.id = "b2".into();
        away.runner_id = "another-runner".into();
        bots.push(away);
        let current = vec![
            plain("r1", "b1", Some("return 1")),
            plain("r2", "b2", Some("return 2")),
        ];
        let mut incoming = vec![
            plain("r1", "b1", None),
            plain("r2", "b2", None),
            plain("r3", "b1", None),
        ];
        assert!(keep_checks(&current, &mut incoming, &bots, &here));
        assert_eq!(incoming[0].check.as_deref(), Some("return 1"));
        assert_eq!(incoming[1].check, None, "another Runner keeps its own");
        assert_eq!(incoming[2].check, None);
        // A check that arrives stands.
        let mut incoming = vec![plain("r1", "b1", Some("return 9"))];
        assert!(!keep_checks(&current, &mut incoming, &bots, &here));
        assert_eq!(incoming[0].check.as_deref(), Some("return 9"));
    }

    /// A due time past for more than a minute (an offline Runner, or quiet checks the Runner did
    /// not announce) is shown as the next time from now; one just due is shown as it is.
    #[test]
    fn a_routine_past_due_is_shown_due_from_now() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let mut routine = plain("r1", "b1", Some("return null"));
        routine.enabled_at = now_secs() - 7200.0;
        assert!(next_run_shown(app, &routine).unwrap() > now_unix());
        routine.check = None;
        assert!(
            next_run_shown(app, &routine).unwrap() > now_unix(),
            "with or without a check"
        );
        routine.enabled_at = now_secs() - 3610.0;
        assert!(
            next_run_shown(app, &routine).unwrap() <= now_unix(),
            "one just due is about to run"
        );
    }

    #[cfg(feature = "runner")]
    #[test]
    fn nothing_returned_is_nothing_found() {
        use serde_json::json;
        for quiet in [
            json!(null),
            json!(false),
            json!(""),
            json!("  "),
            json!([]),
            json!({}),
        ] {
            assert_eq!(found_text(&quiet), None, "{quiet}");
        }
        assert_eq!(found_text(&json!(" two new ")).as_deref(), Some("two new"));
        assert_eq!(
            found_text(&json!({ "new": [1] })).as_deref(),
            Some("{\n  \"new\": [\n    1\n  ]\n}")
        );
        assert!(found_text(&json!("y".repeat(MAX_FOUND_CHARS + 10)))
            .unwrap()
            .ends_with('…'));
    }

    /// A check reads with the bot's file tools, keeps values with store(), and returns what it
    /// found; a tool that writes is not there to call, and its failure is what the run reads.
    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_check_looks_and_says_what_it_found() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let bot = app.bot("b1").unwrap();
        let workdir = bot.working_directory(&app.config.home);
        std::fs::create_dir_all(&workdir).unwrap();
        std::fs::write(workdir.join("notes.txt"), "hello").unwrap();
        let looks = "const seen = load('runs') ?? 0;\nstore('runs', seen + 1);\nconst files = await tools.ls({});\nreturn files.includes('notes.txt') ? 'notes.txt is here' : null;";
        let watch = create(
            app,
            "b1",
            "Watch",
            "every 10m",
            "Tell me what is new.",
            Some(looks),
            true,
        )
        .unwrap();
        let checked = run_check(app, &watch, &CancellationToken::new()).await;
        assert_eq!(
            (checked.found.as_deref(), checked.error.as_deref()),
            (Some("notes.txt is here"), None),
            "{}",
            checked.result
        );
        assert_eq!(
            checked.report(),
            Some(CheckReport {
                found: "notes.txt is here".into(),
                error: None
            })
        );
        let dm = app.dm_with("b1", None).unwrap();
        assert!(
            !app.store
                .codemode_values(&dm.meta.id, "b1")
                .unwrap()
                .contains_key("runs"),
            "a staged check does not consume values"
        );
        let committed = check_now(app, &watch, &CancellationToken::new()).await;
        assert_eq!(
            committed, checked,
            "explicit execution commits the same check result"
        );
        assert_eq!(
            app.store.codemode_values(&dm.meta.id, "b1").unwrap()["runs"],
            serde_json::json!(1)
        );

        let writes = edit(
            app,
            &watch.id,
            None,
            None,
            None,
            Some("await tools.write({ path: 'x.txt', content: 'y' });"),
        )
        .unwrap();
        let checked = run_check(app, &writes, &CancellationToken::new()).await;
        assert!(
            checked
                .error
                .as_deref()
                .is_some_and(|error| error.contains("Unknown tool")),
            "{checked:?}"
        );
        assert!(checked.report().unwrap().error.is_some());
        assert!(!workdir.join("x.txt").exists());
    }

    /// A quiet due check records and publishes health without a model run. Findings start a run.
    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_due_check_starts_the_run_only_when_it_finds_something() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let quiet = create(
            app,
            "b1",
            "Quiet",
            "every 10m",
            "Tell me what is new.",
            Some("return null;"),
            true,
        )
        .unwrap();
        let now = now_secs();
        app.update_routine(&quiet.id, |r| r.enabled_at = now - 700.0)
            .unwrap();
        app.dm_with("b1", None).unwrap();
        reauthorize(app, &quiet.id).unwrap();
        app.store
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE routine_schedule_authority SET next_at=?1 WHERE routine_id=?2",
                rusqlite::params![now_unix(), quiet.id],
            )
            .unwrap();
        let mut events = app.events.subscribe();
        tick(app);
        for _ in 0..200 {
            if app
                .routine(&quiet.id)
                .unwrap()
                .health
                .as_ref()
                .and_then(|health| health.last_check_at)
                .is_some()
                && !app.routine_checks.is_running(&quiet.id)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let checked_at = app
            .routine(&quiet.id)
            .unwrap()
            .health
            .unwrap()
            .last_check_at
            .unwrap() as i64;
        let after = app.routine(&quiet.id).unwrap();
        assert_eq!(
            (after.last_run_at, after.last_outcome.as_deref()),
            (None, None),
            "a check is not a run"
        );
        assert_eq!(after.next_run_at(), after.next_run_after(checked_at));
        assert!(after.next_run_at().unwrap() > now_unix() + 500);
        let dm = app.dm_with("b1", None).unwrap();
        assert!(app.messages(&dm.meta.id).is_empty(), "no marker, no turn");
        // The roster event carries the persisted next check to apps.
        let mut heard = false;
        while let Ok(event) = events.try_recv() {
            if let crate::events::Event::RosterChanged { routines, .. } = event {
                heard |= routines.iter().any(|r| {
                    r["id"] == quiet.id.as_str()
                        && r["next_run_at"]
                            .as_f64()
                            .is_some_and(|next| next > (now_unix() + 500) as f64)
                });
            }
        }
        assert!(heard, "a roster event with the next check");

        let found = create(
            app,
            "b1",
            "Found",
            "every 10m",
            "Tell me what is new.",
            Some("return 'Two new pull requests';"),
            true,
        )
        .unwrap();
        app.update_routine(&found.id, |r| r.enabled_at = now - 700.0)
            .unwrap();
        reauthorize(app, &found.id).unwrap();
        app.store
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE routine_schedule_authority SET next_at=?1 WHERE routine_id=?2",
                rusqlite::params![now_unix(), found.id],
            )
            .unwrap();
        tick(app);
        for _ in 0..200 {
            if app.routine(&found.id).unwrap().last_outcome.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let ran = app.routine(&found.id).unwrap();
        assert!(ran.last_run_at.is_some());
        // No provider on this Runner: the run opens with its marker and ends in a notice.
        assert_eq!(ran.last_outcome.as_deref(), Some("error"));
        let notices: Vec<String> = app
            .messages(&dm.meta.id)
            .iter()
            .filter_map(|m| match &m.body {
                Body::Notice { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(notices[0], "Routine · Found");
    }

    /// A check on save or by hand waits for a due check already running, and counts as the
    /// routine's last check, so an overdue routine is not checked again at the next tick.
    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_check_by_hand_waits_for_a_running_one_and_counts_as_the_last() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let watch = create(
            app,
            "b1",
            "Watch",
            "every 10m",
            "Tell me what is new.",
            Some("return 'new';"),
            true,
        )
        .unwrap();
        app.update_routine(&watch.id, |r| r.enabled_at = now_secs() - 7200.0)
            .unwrap();
        let watch = app.routine(&watch.id).unwrap();
        assert!(
            app.routine_checks
                .start(&watch.id, &CancellationToken::new()),
            "a due check holds the routine"
        );
        let waiting = {
            let (app, watch) = (app.clone(), watch.clone());
            tokio::spawn(async move { check_now(&app, &watch, &CancellationToken::new()).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert!(!waiting.is_finished(), "it waits for the running check");
        app.routine_checks.finish(&watch.id);
        let checked = waiting.await.unwrap();
        assert_eq!(checked.found.as_deref(), Some("new"), "{}", checked.result);
        assert!(!app.routine_checks.is_running(&watch.id));
        assert!(
            app.routine(&watch.id).unwrap().next_run_at().unwrap() > now_unix() + 500,
            "the schedule counts from it"
        );
    }

    #[cfg(feature = "runner")]
    #[test]
    fn a_routines_run_reads_its_checks_findings_as_data() {
        let found = crate::turns::check_cue(&CheckReport {
            found: "Two new pull requests".into(),
            error: None,
        });
        assert!(
            found.contains("not instructions") && found.contains("Two new pull requests"),
            "{found}"
        );
        let failed = crate::turns::check_cue(&CheckReport {
            found: String::new(),
            error: Some("GitHub is down".into()),
        });
        assert!(
            failed.contains("check failed")
                && failed.contains("GitHub is down")
                && failed.contains("fix the check"),
            "{failed}"
        );
    }

    #[test]
    fn policies_validate_and_preserve_one_assigned_runner() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let runner = app.bot("b1").unwrap().runner_id;
        assert!(create_with_policy(
            app,
            "b1",
            "Bad",
            "0 9 * * *",
            "x",
            None,
            true,
            Some("Mars/Base"),
            None
        )
        .is_err());
        assert!(create_with_policy(
            app,
            "b1",
            "Bad",
            "0 9 * * *",
            "x",
            None,
            true,
            None,
            Some("catch_up")
        )
        .is_err());
        let local = create(app, "b1", "Here", "0 9 * * *", "x", None, true).unwrap();
        assert_eq!(
            local.timezone,
            schedule::local_timezone(),
            "a routine keeps the Runner's zone from its creation on"
        );
        let routine = create_with_policy(
            app,
            "b1",
            "Brief",
            "0 9 * * *",
            "x",
            None,
            true,
            Some("America/New_York"),
            Some("skip"),
        )
        .unwrap();
        assert_eq!(routine.timezone, "America/New_York");
        assert_eq!(app.routine_out(&routine)["state"], "on");
        let edited = edit_with_policy(
            app,
            &routine.id,
            None,
            None,
            None,
            None,
            Some("Asia/Singapore"),
            Some("coalesce"),
        )
        .unwrap();
        assert_eq!(edited.timezone, "Asia/Singapore");
        assert_eq!(app.bot("b1").unwrap().runner_id, runner);
        app.state.lock().unwrap().bots[0].runner_id = "offline-runner".into();
        assert_eq!(
            app.routine_out(&edited)["state"],
            "waiting_for_runner",
            "no other Runner takes it over"
        );
    }

    /// Check health survives a restart, and the roster carries it to the other Devices when it
    /// changes how the routine stands; one more quiet check uploads nothing.
    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn quiet_health_survives_restart_and_syncs_when_it_changes() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let routine = create(
            app,
            "b1",
            "Quiet",
            "every 10m",
            "x",
            Some("return null"),
            true,
        )
        .unwrap();
        app.update_routine(&routine.id, |routine| {
            routine.enabled_at = now_secs() - 700.0
        })
        .unwrap();
        let routine = app.routine(&routine.id).unwrap();
        check_now(app, &routine, &CancellationToken::new()).await;
        let after = app.routine(&routine.id).unwrap();
        let health = after.health.as_ref().unwrap();
        assert!(health.last_check_at.is_some() && health.last_success_at.is_some());
        assert_eq!(
            health.status,
            Some(crate::routine_health::CheckStatus::Quiet)
        );
        assert_eq!(
            (after.last_run_at, after.last_outcome.as_ref()),
            (None, None)
        );
        let restarted = App::load(crate::config::Config {
            home: scratch.1.clone(),
            port: 0,
        })
        .unwrap();
        assert_eq!(restarted.routine(&routine.id).unwrap(), after);
        tick(&restarted);
        assert!(
            !restarted.routine_checks.is_running(&routine.id),
            "not checked again after restart"
        );
        let roster = || {
            app.store
                .outbox()
                .unwrap()
                .into_iter()
                .find(|blob| blob.kind == "roster")
                .unwrap()
                .ciphertext
        };
        let uploaded: RosterBlob =
            crate::crypto::decrypt_json(&app.dek().unwrap(), "roster", &roster()).unwrap();
        assert_eq!(
            uploaded.routines[0].health, after.health,
            "the first quiet check goes up"
        );
        let before = roster();
        check_now(app, &after, &CancellationToken::new()).await;
        assert!(
            app.routine(&routine.id)
                .unwrap()
                .health
                .unwrap()
                .last_check_at
                >= after.health.unwrap().last_check_at
        );
        assert_eq!(roster(), before, "another quiet check uploads nothing");
        let failing = edit(
            app,
            &routine.id,
            None,
            None,
            None,
            Some("throw new Error('connection refused')"),
        )
        .unwrap();
        check_now(app, &failing, &CancellationToken::new()).await;
        assert_ne!(roster(), before, "a failed check goes up");
    }

    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn missed_runs_skip_or_coalesce_without_a_burst() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let skip = create_with_policy(
            app,
            "b1",
            "Skip",
            "every 10m",
            "x",
            Some("return 'new'"),
            true,
            None,
            Some("skip"),
        )
        .unwrap();
        let coalesce = create(
            app,
            "b1",
            "Coalesce",
            "every 10m",
            "x",
            Some("store('checks', (load('checks') ?? 0) + 1); return null"),
            true,
        )
        .unwrap();
        for routine in [&skip, &coalesce] {
            app.update_routine(&routine.id, |routine| {
                routine.enabled_at = now_secs() - 5.0 * 86_400.0
            })
            .unwrap();
            app.dm_with("b1", None).unwrap();
            crate::api::dispatch(
                app,
                "routines.reauthorize",
                serde_json::json!({"id": routine.id}),
            )
            .await
            .unwrap();
        }
        tick(app);
        for _ in 0..200 {
            if app.routine(&coalesce.id).unwrap().health.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(app.routine(&skip.id).unwrap().health.is_none());
        assert!(app.routine(&skip.id).unwrap().last_scheduled_at.is_some());
        for _ in 0..5 {
            tick(app);
        }
        let dm = app.dm_with("b1", None).unwrap();
        assert_eq!(
            app.store.codemode_values(&dm.meta.id, "b1").unwrap()["checks"],
            serde_json::json!(1)
        );
        assert!(app.messages(&dm.meta.id).is_empty());
        for routine in [&skip, &coalesce] {
            assert!(app.routine(&routine.id).unwrap().next_run_at().unwrap() > now_unix());
        }
    }

    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authentication_pause_and_connection_backoff_survive_restart() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let watch = create(
            app,
            "b1",
            "Watch",
            "every 5m",
            "x",
            Some("throw new Error('HTTP 401 Unauthorized')"),
            true,
        )
        .unwrap();
        for _ in 0..3 {
            let current = app.routine(&watch.id).unwrap();
            check_now(app, &current, &CancellationToken::new()).await;
        }
        let paused = app.routine(&watch.id).unwrap();
        assert_eq!(paused.paused_reason.as_deref(), Some("authentication"));
        assert!(!paused.is_enabled);
        assert!(run_now(app, &watch.id).unwrap_err().contains("resume it"));
        assert_eq!(app.routine_out(&paused)["state"], "blocked");
        let restarted = App::load(crate::config::Config {
            home: scratch.1.clone(),
            port: 0,
        })
        .unwrap();
        assert_eq!(restarted.routine(&watch.id).unwrap(), paused);
        let resumed = set_enabled(&restarted, &watch.id, true).unwrap();
        assert!(resumed.is_enabled && resumed.paused_reason.is_none());
        assert_eq!(resumed.health.unwrap().authentication_failures, 0);
        let connection = edit(
            app,
            &watch.id,
            None,
            None,
            None,
            Some("throw new Error('connection refused')"),
        )
        .unwrap();
        for _ in 0..3 {
            check_now(app, &connection, &CancellationToken::new()).await;
        }
        let after = app.routine(&watch.id).unwrap();
        assert_eq!(after.health.as_ref().unwrap().connection_failures, 3);
        assert!(
            after.next_run_at().is_none(),
            "editing does not implicitly resume a paused routine"
        );
        let mut current = after.clone();
        current.is_enabled = true;
        assert!(current.next_run_at().unwrap() >= now_unix() + 1190);
    }

    #[test]
    fn stale_roster_edits_preserve_runner_check_health_and_cursor() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let current = create(
            app,
            "b1",
            "Watch",
            "every 10m",
            "x",
            Some("return null"),
            true,
        )
        .unwrap();
        let mut stale = current.clone();
        let held = app
            .update_routine(&current.id, |routine| {
                routine.last_scheduled_at = Some(now_secs());
                routine.health = Some(Default::default());
                routine
                    .health
                    .as_mut()
                    .unwrap()
                    .record(now_secs(), false, None);
            })
            .unwrap();
        stale.name = "Renamed elsewhere".into();
        let mut incoming = vec![stale];
        assert!(keep_checks(
            std::slice::from_ref(&held),
            &mut incoming,
            &app.state.lock().unwrap().bots,
            &app.this_device_id().unwrap()
        ));
        assert_eq!(incoming[0].health, held.health);
        assert_eq!(incoming[0].last_scheduled_at, held.last_scheduled_at);
        assert_eq!(incoming[0].name, "Renamed elsewhere");
    }

    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_checkpoint_does_not_admit_scheduled_work() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let routine = create(
            app,
            "b1",
            "Watch",
            "every 10m",
            "x",
            Some("store('ran', true); return null"),
            true,
        )
        .unwrap();
        let due = app
            .update_routine(&routine.id, |routine| {
                routine.enabled_at = now_secs() - 700.0
            })
            .unwrap();
        app.dm_with("b1", None).unwrap();
        crate::api::dispatch(
            app,
            "routines.reauthorize",
            serde_json::json!({"id": routine.id}),
        )
        .await
        .unwrap();
        let connection = rusqlite::Connection::open(app.config.database_path()).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_routines BEFORE INSERT ON routine_check_commits BEGIN SELECT RAISE(FAIL, 'routine writes unavailable'); END;").unwrap();
        tick(app);
        assert_eq!(
            app.routine(&routine.id).unwrap(),
            due,
            "failed checkpoint rolls back in-memory admission"
        );
        assert!(!app.routine_checks.is_running(&routine.id));
        let dm = app.dm_with("b1", None).unwrap();
        assert!(!app
            .store
            .codemode_values(&dm.meta.id, "b1")
            .unwrap()
            .contains_key("ran"));
        connection
            .execute_batch("DROP TRIGGER reject_routines")
            .unwrap();
        tick(app);
        for _ in 0..200 {
            if app.routine(&routine.id).unwrap().health.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(
            app.store.codemode_values(&dm.meta.id, "b1").unwrap()["ran"],
            serde_json::json!(true)
        );
    }

    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_job_queued_before_authentication_pause_requires_resume() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let routine = create(app, "b1", "Brief", "every 1h", "x", None, true).unwrap();
        let queued = job_for(app, &routine).unwrap();
        for _ in 0..3 {
            model_result(app, &routine.id, Some("HTTP 401 Unauthorized"));
        }
        let outcome = crate::turns::run_job(app, &queued, CancellationToken::new()).await;
        assert_eq!(outcome, TurnOutcome::Skipped);
        let paused = app.routine(&routine.id).unwrap();
        assert_eq!(paused.last_run_at, None, "no model run started");
        assert_eq!(
            paused.health.as_ref().unwrap().last_check_at,
            None,
            "a provider failure is not a check"
        );
        assert_eq!(
            paused
                .health
                .as_ref()
                .unwrap()
                .model
                .authentication_failures,
            3
        );
        assert!(app.messages(&queued.chat_id).iter().any(|message| matches!(&message.body, Body::Notice { text, .. } if text.contains("then resume it"))));
    }

    /// Waits until `done` holds, up to five seconds.
    #[cfg(feature = "runner")]
    async fn until(mut done: impl FnMut() -> bool) {
        for _ in 0..200 {
            if done() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("timed out");
    }

    fn notices(app: &App) -> Vec<String> {
        let dm = app.dm_with("b1", None).unwrap();
        app.messages(&dm.meta.id)
            .iter()
            .filter_map(|m| match &m.body {
                Body::Notice { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// A one-time routine runs once at its time, late if need be, and is gone afterwards; a run
    /// by hand before its time leaves it waiting for that time.
    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_one_time_routine_runs_once_and_is_gone() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let zone = schedule::local_timezone();
        let soon = chrono::Utc::now().with_timezone(&schedule::timezone(&zone).unwrap())
            + chrono::Duration::hours(2);
        let at = format!("once {}", soon.format("%Y-%m-%d %H:%M"));
        assert!(
            create(app, "b1", "Past", "once 2020-01-01 09:00", "x", None, true)
                .unwrap_err()
                .contains("passed")
        );
        assert!(
            create(app, "b1", "Checked", &at, "x", Some("return 1"), true)
                .unwrap_err()
                .contains("no check")
        );
        assert!(create_with_policy(
            app,
            "b1",
            "Skipped",
            &at,
            "x",
            None,
            true,
            None,
            Some("skip")
        )
        .unwrap_err()
        .contains("always runs"));
        let reminder = create(
            app,
            "b1",
            "Call the dentist",
            &at,
            "Remind me to call the dentist.",
            None,
            true,
        )
        .unwrap();
        assert_eq!(
            reminder.next_run_at(),
            schedule::parse(&at).unwrap().once_instant(&zone)
        );
        assert!(app.routine_out(&reminder)["schedule_text"]
            .as_str()
            .unwrap()
            .starts_with("Once on "));
        assert!(app.routine_out(&reminder)["once_at"].as_f64().is_some());
        // Run Now before its time: it still runs at its time.
        app.update_routine(&reminder.id, |r| r.last_run_at = Some(now_secs()))
            .unwrap();
        assert!(app.routine(&reminder.id).unwrap().next_run_at().is_some());
        assert!(!crate::routine_triggers::is_finished(
            &app.routine(&reminder.id).unwrap()
        ));
        // Its Runner was off at its time: it runs when the Runner is back, then goes.
        app.update_routine(&reminder.id, |r| {
            r.schedule = "once 2020-01-01 09:00".into();
            r.last_run_at = None;
        })
        .unwrap();
        app.dm_with("b1", None).unwrap();
        crate::api::dispatch(
            app,
            "routines.reauthorize",
            serde_json::json!({"id": reminder.id}),
        )
        .await
        .unwrap();
        tick(app);
        until(|| app.routine(&reminder.id).is_none()).await;
        assert_eq!(
            notices(app)[..1],
            ["Routine · Call the dentist".to_string()]
        );
        tick(app);
        assert_eq!(
            notices(app)
                .iter()
                .filter(|text| text.starts_with("Routine ·"))
                .count(),
            1,
            "one run"
        );
        // A restart between taking its time and removing it removes it at the next tick.
        let left = create(app, "b1", "Left over", &at, "x", None, true).unwrap();
        app.record_routine(&left.id, |r| {
            r.schedule = "once 2020-01-01 09:00".into();
            r.last_scheduled_at = Some(now_secs());
        })
        .unwrap();
        assert!(crate::routine_triggers::is_finished(
            &app.routine(&left.id).unwrap()
        ));
        tick(app);
        assert!(app.routine(&left.id).is_none());
    }

    /// A check stopped with the turn it ran in records nothing: Stop is not a failed check.
    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_check_stopped_with_its_turn_leaves_health_alone() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let watch = create(
            app,
            "b1",
            "Watch",
            "every 10m",
            "x",
            Some("throw new Error('boom')"),
            true,
        )
        .unwrap();
        let stopped = CancellationToken::new();
        stopped.cancel();
        check_now(app, &watch, &stopped).await;
        let after = app.routine(&watch.id).unwrap();
        assert_eq!(after.health, None, "nothing recorded");
        assert!(
            !app.routine_checks.is_running(&watch.id),
            "the check's hold is released"
        );
        check_now(app, &after, &CancellationToken::new()).await;
        assert_eq!(
            app.routine(&watch.id).unwrap().health.unwrap().status,
            Some(crate::routine_health::CheckStatus::Failed),
            "a check that ran counts"
        );
    }

    /// A streamable-HTTP MCP server for the tests: `tools` to list, and each call recorded and
    /// answered with what `answer` holds then.
    #[cfg(all(feature = "runner", feature = "server"))]
    async fn fake_mcp(
        tools: Value,
        answer: Arc<std::sync::Mutex<Value>>,
        calls: Arc<std::sync::Mutex<Vec<Value>>>,
    ) -> String {
        use axum::{routing::post, Json, Router};
        let router = Router::new().route(
            "/mcp",
            post(move |Json(request): Json<Value>| {
                let (tools, answer, calls) = (tools.clone(), answer.clone(), calls.clone());
                async move {
                    let result = match request["method"].as_str() {
                        Some("initialize") => serde_json::json!({ "protocolVersion": request["params"]["protocolVersion"], "serverInfo": { "name": "fake", "version": "1" }, "capabilities": { "tools": {} } }),
                        Some("tools/list") => serde_json::json!({ "tools": tools }),
                        Some("tools/call") => {
                            calls.lock().unwrap().push(request["params"].clone());
                            let text = answer.lock().unwrap().to_string();
                            serde_json::json!({ "content": [{ "type": "text", "text": text }], "isError": false })
                        }
                        _ => return axum::http::StatusCode::ACCEPTED.into_response(),
                    };
                    Json(serde_json::json!({ "jsonrpc": "2.0", "id": request["id"], "result": result })).into_response()
                }
            }),
        );
        use axum::response::IntoResponse;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        url
    }

    /// Signs an event for a subscription with its secret and hands it to the inbox, as the
    /// relay's receiver or a gateway does.
    #[cfg(feature = "runner")]
    fn deliver(
        app: &Arc<App>,
        subscription_id: &str,
        delivery_id: &str,
        event_type: &str,
        payload: Value,
    ) -> Value {
        let (secret, generation) = crate::event_triggers::secret_of(app, subscription_id).unwrap();
        let mut event = crate::event_triggers::Envelope {
            version: 1,
            subscription_id: subscription_id.into(),
            generation,
            delivery_id: delivery_id.into(),
            occurred_at: now_unix(),
            event_type: event_type.into(),
            payload: payload.to_string(),
            signature: String::new(),
        };
        event.sign(&secret).unwrap();
        crate::event_triggers::receive(app, event).unwrap()
    }

    /// A routine on events listens through a subscription of its own: each event about its
    /// subject starts one run, a repeat of a delivery runs nothing, and the run of an event
    /// that ends the subject is its last.
    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_routine_on_events_runs_once_per_event_and_ends_with_its_subject() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let on = |receiver: Option<&'static str>, subject: Option<&'static str>| Triggers {
            receiver,
            subject,
            ..Default::default()
        };
        assert!(create_routine(
            app,
            "b1",
            "Bad",
            "every 1h",
            "x",
            None,
            true,
            None,
            None,
            on(Some("github"), Some("acme/project#42"))
        )
        .unwrap_err()
        .contains("no schedule"));
        assert!(create_routine(
            app,
            "b1",
            "Bad",
            "",
            "x",
            None,
            true,
            None,
            None,
            on(None, Some("acme/project#42"))
        )
        .unwrap_err()
        .contains("receiver"));
        assert!(create_routine(
            app,
            "b1",
            "Bad",
            "",
            "x",
            Some("return 1"),
            true,
            None,
            None,
            on(Some("github"), Some("acme/project#42"))
        )
        .unwrap_err()
        .contains("no check"));
        let watch = create_routine(
            app,
            "b1",
            "Login PR",
            "",
            "Tell me what happened.",
            None,
            true,
            None,
            None,
            on(Some("GitHub"), Some("acme/project#42")),
        )
        .unwrap();
        let events = watch.events.clone().unwrap();
        assert_eq!(
            (
                watch.schedule.as_str(),
                events.receiver.as_str(),
                events.status
            ),
            (
                "on events",
                "github",
                crate::routine_events::Listening::Pending
            )
        );
        assert!(!events.subscription_id.is_empty() && watch.next_run_at().is_none());
        assert_eq!(
            app.routine_out(&watch)["schedule_text"],
            "Watches acme/project#42"
        );
        assert!(edit_routine(
            app,
            &watch.id,
            None,
            Some("every 1h"),
            None,
            None,
            None,
            None,
            Triggers::default()
        )
        .unwrap_err()
        .contains("no schedule"));
        // Another subject's event is filtered; this one's runs once, however often it comes.
        assert_eq!(
            deliver(
                app,
                &events.subscription_id,
                "d-0",
                "github",
                serde_json::json!({ "subject": "acme/project#7", "summary": "Merged", "ends": true })
            )["status"],
            "filtered"
        );
        let pushed = serde_json::json!({ "subject": "acme/project#42", "kind": "commits", "summary": "New commits pushed", "title": "Add passkey sign-in", "url": "https://github.com/acme/project/pull/42" });
        assert_eq!(
            deliver(
                app,
                &events.subscription_id,
                "d-1",
                "github",
                pushed.clone()
            )["status"],
            "queued"
        );
        assert_eq!(
            deliver(app, &events.subscription_id, "d-1", "github", pushed)["status"],
            "duplicate"
        );
        crate::event_triggers::tick(app).unwrap();
        until(|| app.routine(&watch.id).unwrap().last_outcome.is_some()).await;
        let ran = app.routine(&watch.id).unwrap();
        let heard = ran.events.as_ref().unwrap();
        assert_eq!(
            (
                heard.last_event.as_ref().unwrap().summary.as_str(),
                heard.title.as_str()
            ),
            ("New commits pushed", "Add passkey sign-in")
        );
        assert!(ran.last_run_at.is_some());
        assert_eq!(notices(app)[0], "Event · Login PR");
        // With no provider here the run failed, which holds later events until the user discards
        // or retries it.
        let listed =
            crate::event_triggers::serve(app, "events.list", &serde_json::json!({})).unwrap();
        let failed = listed["subscriptions"][0]["deliveries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["state"] == "failed")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        crate::event_triggers::serve(app, "events.discard", &serde_json::json!({ "id": failed }))
            .unwrap();
        // The subject ends: its run is the last, and the routine and its subscription go.
        assert_eq!(
            deliver(
                app,
                &events.subscription_id,
                "d-2",
                "github",
                serde_json::json!({ "subject": "acme/project#42", "kind": "merged", "summary": "Merged", "ends": true })
            )["status"],
            "queued"
        );
        crate::event_triggers::tick(app).unwrap();
        until(|| app.routine(&watch.id).is_none()).await;
        until(|| {
            crate::event_triggers::receiver_subscriptions(app)
                .unwrap()
                .is_empty()
        })
        .await;
        assert_eq!(
            notices(app)
                .iter()
                .filter(|text| text.starts_with("Event ·"))
                .count(),
            2,
            "one run per event"
        );

        // A webhook's routine has no subject: every request its receiver delivers runs it.
        let hook = create_routine(
            app,
            "b1",
            "Deploys",
            "",
            "Tell me if a deploy failed.",
            None,
            true,
            None,
            None,
            on(Some("webhook"), None),
        )
        .unwrap();
        let hook_events = hook.events.clone().unwrap();
        assert_eq!(
            app.routine_out(&hook)["schedule_text"],
            "When its webhook is called"
        );
        assert_eq!(
            deliver(
                app,
                &hook_events.subscription_id,
                "r-1",
                "webhook",
                serde_json::json!({ "kind": "request", "summary": "Webhook request", "data": { "deploy": "failed" } })
            )["status"],
            "queued"
        );
        // Deleting the routine removes its subscription.
        delete(app, &hook.id).unwrap();
        until(|| {
            crate::event_triggers::receiver_subscriptions(app)
                .unwrap()
                .is_empty()
        })
        .await;
        // Another Device's older roster keeps the Runner's view of what the routine listens with.
        let mut held = watch.clone();
        held.events.as_mut().unwrap().key = "k".into();
        held.events.as_mut().unwrap().status = crate::routine_events::Listening::Subscribed;
        let mut incoming = watch.clone();
        assert!(crate::routine_triggers::keep_reads(&held, &mut incoming));
        assert_eq!(incoming.events.as_ref().unwrap().key, "k");
    }

    /// A routine around events reads its Calendar account, keeps the matching events of the next
    /// day, and runs once for each when its time comes, with the event.
    #[cfg(all(feature = "runner", feature = "server"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_routine_around_events_runs_once_for_each_matching_event() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let iso = |at: i64| {
            chrono::DateTime::from_timestamp(at, 0)
                .unwrap()
                .to_rfc3339()
        };
        let now = now_unix();
        let event = |id: &str, title: &str, start: i64| serde_json::json!({ "id": id, "summary": title, "start": { "dateTime": iso(start) }, "end": { "dateTime": iso(start + 1800) }, "htmlLink": format!("https://calendar.google.com/{id}") });
        // Due a minute ago, in an hour, and an event that does not match.
        let answer = Arc::new(std::sync::Mutex::new(serde_json::json!({ "events": [
            event("soon", "Customer call: Acme", now + 14 * 60),
            event("later", "Customer call: Globex", now + 75 * 60),
            event("lunch", "Lunch", now + 20 * 60),
        ] })));
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let tools = serde_json::json!([{ "name": "list_events", "description": "List events", "inputSchema": { "type": "object" }, "annotations": { "readOnlyHint": true } }]);
        let url = fake_mcp(tools, answer.clone(), calls.clone()).await;
        let triggers = Triggers {
            event_match: Some("customer call"),
            ..Default::default()
        };
        assert!(create_routine(
            app,
            "b1",
            "Prep",
            "15m before events",
            "Prepare me.",
            None,
            true,
            None,
            None,
            triggers
        )
        .unwrap_err()
        .contains("not connected"));
        let manifest = crate::plugins::Manifest::parse(&serde_json::json!({ "id": "google-calendar", "name": "Google Calendar", "named_accounts": true, "servers": { "calendar": { "type": "http", "url": url } } })).unwrap();
        crate::plugins::accounts::install(app, manifest, "inline", Some("Work")).unwrap();
        assert!(create_routine(
            app, "b1", "Prep", "every 1h", "x", None, true, None, None, triggers
        )
        .unwrap_err()
        .contains("around events"));
        let prep = create_routine(
            app,
            "b1",
            "Prep",
            "15m before events",
            "Prepare me for the call.",
            None,
            true,
            None,
            None,
            triggers,
        )
        .unwrap();
        assert_eq!(
            prep.calendar.as_ref().unwrap().account,
            "Google Calendar · Work"
        );
        assert_eq!(
            app.routine_out(&prep)["schedule_text"],
            "15 minutes before events matching “customer call”"
        );
        assert_eq!(prep.next_run_at(), None, "nothing read yet");
        // Set up an hour ago, so the event due a minute ago is its to run.
        app.update_routine(&prep.id, |r| r.enabled_at = now_secs() - 3600.0)
            .unwrap();
        app.dm_with("b1", None).unwrap();
        crate::api::dispatch(
            app,
            "routines.reauthorize",
            serde_json::json!({"id": prep.id}),
        )
        .await
        .unwrap();
        tick(app);
        until(|| {
            app.routine(&prep.id)
                .unwrap()
                .calendar
                .unwrap()
                .synced_at
                .is_some()
        })
        .await;
        let args = calls.lock().unwrap()[0]["arguments"].clone();
        assert_eq!(
            (args["fullText"].as_str(), args["orderBy"].as_str()),
            (Some("customer call"), Some("startTime"))
        );
        let read = app.routine(&prep.id).unwrap();
        assert_eq!(
            read.calendar
                .as_ref()
                .unwrap()
                .upcoming
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            ["soon", "later"]
        );
        assert_eq!(read.next_run_at(), Some(now + 14 * 60 - 15 * 60));
        assert_eq!(
            app.routine_out(&read)["calendar"]["next_event"]["title"],
            "Customer call: Acme"
        );
        tick(app);
        until(|| app.routine(&prep.id).unwrap().last_outcome.is_some()).await;
        assert_eq!(notices(app)[0], "Routine · Prep");
        let after = app.routine(&prep.id).unwrap();
        assert_eq!(after.calendar.as_ref().unwrap().done.len(), 1);
        assert_eq!(
            after.next_run_at(),
            Some(now + 75 * 60 - 15 * 60),
            "the next event's turn"
        );
        for _ in 0..3 {
            tick(app);
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(
            notices(app)
                .iter()
                .filter(|text| text.starts_with("Routine ·"))
                .count(),
            1,
            "one run per event"
        );
        // The roster carries the Runner's read and runs over a copy another Device wrote earlier.
        let mut incoming = vec![prep.clone()];
        assert!(keep_checks(
            std::slice::from_ref(&after),
            &mut incoming,
            &app.state.lock().unwrap().bots,
            &app.this_device_id().unwrap()
        ));
        assert_eq!(incoming[0].calendar, after.calendar);
    }
}
