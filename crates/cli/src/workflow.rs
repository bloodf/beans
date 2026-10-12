//! Saved workflows: a bot's ordered steps, kept in the roster so every Device lists them. A
//! definition names a bot's prompts and the tools each step may use; it grants nothing, and
//! nothing here runs one. The bot's Runner checks the tool references against the bot's live
//! capabilities when it saves a definition and each time it lists one.
//!
//! The roster keeps the definition as the raw JSON it was saved with, so a newer schema or
//! field passes through an older Device unchanged. A definition that fails a check later (a
//! revoked tool, a newer schema) is listed as unavailable, never deleted or rewritten.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::app::App;
use crate::config::now_secs;
use crate::model::{Bot, WorkflowRecord};

pub const SCHEMA_VERSION: u64 = 1;
pub const MAX_PER_BOT: usize = 20;
pub const MAX_NAME_CHARS: usize = 60;
pub const MAX_REVISION_CHARS: usize = 64;
pub const MAX_STEPS: usize = 20;
pub const MAX_PROMPT_CHARS: usize = 8_000;
pub const MAX_TOOLS_PER_STEP: usize = 16;
pub const MAX_TOOL_REF_CHARS: usize = 128;
pub const MAX_DEFINITION_BYTES: usize = 64 * 1024;

/// The bot's own tools a step may name; the rest of a bot's tools come from its plugins.
const BUILTIN_TOOLS: [&str; 9] = ["read", "write", "edit", "bash", "grep", "find", "ls", "bash_input", "bash_output"];

/// Checks the shape and bounds of a definition: version 1, a revision, and one to
/// `MAX_STEPS` steps of a prompt, optional tool references, and an optional approval flag.
/// Any other key is refused. Returns every tool reference with its step index.
fn check_shape(definition: &Value) -> Result<Vec<(usize, String)>, String> {
    if serde_json::to_vec(definition).map_or(true, |bytes| bytes.len() > MAX_DEFINITION_BYTES) {
        return Err(format!("Keep the definition under {} KiB.", MAX_DEFINITION_BYTES / 1024));
    }
    let object = definition.as_object().ok_or("The definition must be an object.")?;
    match object.get("version").and_then(Value::as_u64) {
        Some(SCHEMA_VERSION) => {}
        Some(other) => return Err(format!("Unsupported schema version {other}.")),
        None => return Err("version must be 1.".into()),
    }
    if let Some(key) = object.keys().find(|key| !matches!(key.as_str(), "version" | "revision" | "steps")) {
        return Err(format!("Unknown field {key:?} in the definition."));
    }
    let revision = object.get("revision").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if revision.is_empty() || revision.chars().count() > MAX_REVISION_CHARS || revision.chars().any(char::is_control) {
        return Err(format!("revision must be 1 to {MAX_REVISION_CHARS} characters."));
    }
    let steps = object.get("steps").and_then(Value::as_array).ok_or("steps must be a list.")?;
    if steps.is_empty() || steps.len() > MAX_STEPS {
        return Err(format!("A workflow has 1 to {MAX_STEPS} steps."));
    }
    let mut refs = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let step = step.as_object().ok_or_else(|| format!("steps[{index}] must be an object."))?;
        if let Some(key) = step.keys().find(|key| !matches!(key.as_str(), "prompt" | "tools" | "approval")) {
            return Err(format!("Unknown field {key:?} in steps[{index}]."));
        }
        let prompt = step.get("prompt").and_then(Value::as_str).map(str::trim).unwrap_or("");
        if prompt.is_empty() || prompt.chars().count() > MAX_PROMPT_CHARS {
            return Err(format!("steps[{index}].prompt must be 1 to {MAX_PROMPT_CHARS} characters."));
        }
        if step.get("approval").is_some_and(|approval| !approval.is_boolean()) {
            return Err(format!("steps[{index}].approval must be true or false."));
        }
        let tools = match step.get("tools") {
            None => &[][..],
            Some(Value::Array(tools)) => tools.as_slice(),
            Some(_) => return Err(format!("steps[{index}].tools must be a list.")),
        };
        if tools.len() > MAX_TOOLS_PER_STEP {
            return Err(format!("steps[{index}] names at most {MAX_TOOLS_PER_STEP} tools."));
        }
        let mut seen = std::collections::HashSet::new();
        for tool in tools {
            let tool = tool.as_str().filter(|name| {
                !name.is_empty() && name.len() <= MAX_TOOL_REF_CHARS && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            });
            let tool = tool.ok_or_else(|| format!("steps[{index}].tools holds a name that is not a tool reference."))?;
            if !seen.insert(tool) {
                return Err(format!("steps[{index}].tools names {tool:?} twice."));
            }
            refs.push((index, tool.to_string()));
        }
    }
    Ok(refs)
}

/// Whether `bot`'s live capabilities permit the tool `name` and the tool exists: one of the bot's
/// own tools, or a tool of a plugin installed on this Runner that the bot may use, from the tool
/// list the plugin last saved. Nothing connects or starts.
#[cfg(feature = "runner")]
fn tool_available(app: &Arc<App>, bot: &Bot, name: &str) -> bool {
    if BUILTIN_TOOLS.contains(&name) {
        return bot.capabilities.permits(name, None);
    }
    let catalog = crate::plugins::mcp::turn_catalog_for_bot(app, Vec::new(), &bot.id);
    catalog.plugin_id(name).is_some_and(|plugin_id| bot.capabilities.permits(name, Some(&plugin_id)))
}

#[cfg(not(feature = "runner"))]
fn tool_available(_app: &Arc<App>, _bot: &Bot, _name: &str) -> bool {
    false
}

/// The first tool reference the bot cannot use now, as a refusal.
fn check_tools(app: &Arc<App>, bot: &Bot, refs: &[(usize, String)]) -> Result<(), String> {
    match refs.iter().find(|(_, name)| !tool_available(app, bot, name)) {
        Some((index, name)) => Err(format!("steps[{index}] names {name:?}, which {} cannot use.", bot.name)),
        None => Ok(()),
    }
}

fn clean_name(name: &str) -> Result<String, String> {
    let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("Give the workflow a name.".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("Keep the name under {MAX_NAME_CHARS} characters."));
    }
    Ok(name)
}

/// Saves a workflow for `bot_id`. Only the bot's own Runner saves, because only it can say what
/// the bot's tools are; a Device that is not that Runner refuses. A refusal saves nothing.
pub fn create(app: &Arc<App>, bot_id: &str, name: &str, definition: Value) -> Result<WorkflowRecord, String> {
    let bot = app.bot(bot_id).ok_or("Unknown bot")?;
    if app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
        return Err(format!("{} runs on another Device; save its workflows there.", bot.name));
    }
    let name = clean_name(name)?;
    let refs = check_shape(&definition)?;
    check_tools(app, &bot, &refs)?;
    let existing = app.workflows_of(bot_id);
    if existing.len() >= MAX_PER_BOT {
        return Err(format!("A bot keeps at most {MAX_PER_BOT} workflows. Delete one first."));
    }
    if existing.iter().any(|w| w.name.eq_ignore_ascii_case(&name)) {
        return Err(format!("A workflow named {name:?} already exists. Pick another name."));
    }
    let record = WorkflowRecord {
        id: format!("wf-{}", &uuid::Uuid::new_v4().to_string()[..8]),
        bot_id: bot_id.to_string(),
        name,
        definition,
        created_at: now_secs(),
    };
    app.insert_workflow(record).map_err(|e| e.to_string())
}

/// The records of an incoming roster that fit the bounds, for the roster to keep. A record over
/// a bound is not taken: the one this Device already holds under its id stays (the last
/// accepted definition), and a new one is skipped, with a warning. Only that record is affected,
/// never the rest of the roster. Within bounds a definition is kept as it arrived, whatever its
/// schema: status is worked out here from the bot's capabilities and never comes from the wire.
pub fn admit(incoming: Vec<WorkflowRecord>, current: &[WorkflowRecord]) -> Vec<WorkflowRecord> {
    let fits = |record: &WorkflowRecord| {
        record.id.chars().count() <= 64
            && record.name.chars().count() <= MAX_NAME_CHARS
            && serde_json::to_vec(&record.definition).is_ok_and(|bytes| bytes.len() <= MAX_DEFINITION_BYTES)
    };
    let mut per_bot = std::collections::HashMap::<String, usize>::new();
    let mut admitted = Vec::with_capacity(incoming.len());
    for record in incoming {
        let record = if fits(&record) {
            record
        } else {
            tracing::warn!(workflow = %record.id.chars().take(64).collect::<String>(), "incoming workflow is over its bounds");
            match current.iter().find(|held| held.id == record.id) {
                Some(held) => held.clone(),
                None => continue,
            }
        };
        let count = per_bot.entry(record.bot_id.clone()).or_default();
        if *count >= MAX_PER_BOT {
            tracing::warn!("incoming workflows over the per-bot limit are skipped");
            continue;
        }
        *count += 1;
        admitted.push(record);
    }
    admitted
}

/// A workflow as the apps see it: the stored fields plus `state`. `valid` means this Device is
/// the bot's Runner and the definition passes every check against the bot's capabilities now;
/// `unavailable` (with a `reason`) means it fails one; `unverified` means this Device cannot
/// check it. The stored definition is never changed.
pub fn out(app: &Arc<App>, record: &WorkflowRecord) -> Value {
    let mut out = serde_json::to_value(record).unwrap_or_default();
    let (state, reason) = match check_shape(&record.definition) {
        Err(reason) => ("unavailable", Some(reason)),
        Ok(refs) => match app.bot(&record.bot_id) {
            None => ("unavailable", Some("Unknown bot".to_string())),
            Some(bot) if app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) => ("unverified", None),
            Some(bot) => match check_tools(app, &bot, &refs) {
                Ok(()) => ("valid", None),
                Err(reason) => ("unavailable", Some(reason)),
            },
        },
    };
    out["state"] = json!(state);
    if let Some(reason) = reason {
        out["reason"] = json!(reason);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Removes a scratch home when the test ends.
    struct Home(std::path::PathBuf);
    impl Home {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("beans-workflows-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Home(path)
        }
    }
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn bot(id: &str, runner_id: &str, capabilities: crate::model::Capabilities) -> Bot {
        Bot {
            id: id.into(), name: id.into(), description: String::new(), symbol_name: String::new(), accent: String::new(),
            avatar: None, look: None, runner_id: runner_id.into(), provider: "deepseek".into(), model: None, thinking: None,
            legacy_instructions: String::new(), workdir: None, capabilities, created_at: 0.0,
        }
    }

    /// An account on a scratch home with this Device as its Runner and bots `b1`, `b2`
    /// (`b2` without shell), and `far` (assigned to another Device).
    fn scratch(home: &Home) -> Arc<App> {
        let app = App::load(crate::config::Config { home: home.0.clone(), port: 0 }).unwrap();
        if !app.has_identity() {
            crate::identity::create(&app, Some("Workbench".into())).unwrap();
            let this = app.this_device_id().unwrap();
            let mut state = app.state.lock().unwrap();
            state.bots = vec![
                bot("b1", &this, Default::default()),
                bot("b2", &this, crate::model::Capabilities { shell: false, ..Default::default() }),
                bot("far", "another-device", Default::default()),
            ];
            state.chats.clear();
        }
        app
    }

    fn definition(tools: Value) -> Value {
        json!({ "version": 1, "revision": "r1", "steps": [{ "prompt": "Summarize", "tools": tools }, { "prompt": "Review", "approval": true }] })
    }

    fn with(definition: &Value, edit: impl FnOnce(&mut Value)) -> Value {
        let mut copy = definition.clone();
        edit(&mut copy);
        copy
    }

    #[test]
    fn definitions_are_checked_saved_and_kept() {
        let home = Home::new();
        let app = &scratch(&home);
        let good = definition(json!(["read", "bash"]));
        let cases: Vec<(&str, &str, &str, Value, Option<&str>)> = vec![
            ("valid", "b1", "Morning", good.clone(), None),
            ("valid without tools", "b1", "Plain", json!({ "version": 1, "revision": "r1", "steps": [{ "prompt": "x" }] }), None),
            ("unknown bot", "nobody", "N", good.clone(), Some("Unknown bot")),
            ("not this Runner", "far", "N", good.clone(), Some("another Device")),
            ("blank name", "b1", "  ", good.clone(), Some("name")),
            ("duplicate name", "b1", "morning", good.clone(), Some("already exists")),
            ("version 2", "b1", "V2", with(&good, |d| d["version"] = json!(2)), Some("Unsupported schema version 2")),
            ("no version", "b1", "V0", with(&good, |d| { d.as_object_mut().unwrap().remove("version"); }), Some("version must be 1")),
            ("no revision", "b1", "R0", with(&good, |d| d["revision"] = json!(" ")), Some("revision")),
            ("no steps", "b1", "E", with(&good, |d| d["steps"] = json!([])), Some("1 to 20 steps")),
            ("21 steps", "b1", "S21", with(&good, |d| d["steps"] = json!(vec![json!({ "prompt": "x" }); 21])), Some("1 to 20 steps")),
            ("blank prompt", "b1", "P0", with(&good, |d| d["steps"][0]["prompt"] = json!("  ")), Some("prompt")),
            ("long prompt", "b1", "P8", with(&good, |d| d["steps"][0]["prompt"] = json!("x".repeat(8001))), Some("prompt")),
            ("approval not bool", "b1", "A", with(&good, |d| d["steps"][1]["approval"] = json!("allow")), Some("approval")),
            ("edge field", "b1", "Next", with(&good, |d| d["steps"][0]["next"] = json!(0)), Some("Unknown field \"next\"")),
            ("shell field", "b1", "Sh", with(&good, |d| d["steps"][0]["shell"] = json!("echo")), Some("Unknown field \"shell\"")),
            ("top-level field", "b1", "Br", with(&good, |d| d["branches"] = json!([])), Some("Unknown field \"branches\"")),
            ("duplicate tool", "b1", "D", definition(json!(["read", "read"])), Some("twice")),
            ("17 tools", "b1", "T17", definition(json!((0..17).map(|i| format!("t{i}")).collect::<Vec<_>>())), Some("at most 16")),
            ("unknown tool", "b1", "U", definition(json!(["nonesuch"])), Some("cannot use")),
            ("unknown plugin tool", "b1", "UP", definition(json!(["github__create_issue"])), Some("cannot use")),
            ("tool the bot lacks", "b2", "L", definition(json!(["bash"])), Some("cannot use")),
            ("tool the bot has", "b2", "H", definition(json!(["read"])), None),
        ];
        for (label, bot_id, name, definition, expected) in cases {
            let before = app.state.lock().unwrap().workflows.clone();
            match (create(app, bot_id, name, definition.clone()), expected) {
                (Ok(saved), None) => {
                    assert_eq!(saved.definition, definition, "{label}: kept as given");
                    assert_eq!(app.state.lock().unwrap().workflows.len(), before.len() + 1, "{label}");
                }
                (Err(error), Some(fragment)) => {
                    assert!(error.contains(fragment), "{label}: {error}");
                    assert_eq!(app.state.lock().unwrap().workflows, before, "{label}: a refusal saves nothing");
                }
                (other, _) => panic!("{label}: {other:?}"),
            }
        }
        let twenty = scratch_with_many(app);
        assert!(twenty.contains("at most 20"), "{twenty}");
    }

    fn scratch_with_many(app: &Arc<App>) -> String {
        for i in 0..MAX_PER_BOT {
            let _ = create(app, "b1", &format!("Many {i}"), definition(json!([])));
        }
        create(app, "b1", "One too many", definition(json!([]))).unwrap_err()
    }

    #[test]
    fn saved_workflows_survive_reload_and_stay_per_account_and_bot() {
        let (home, other_home) = (Home::new(), Home::new());
        let second = scratch(&other_home);
        let first = scratch(&home);
        let app = &first;
        let kept = create(app, "b1", "Keep", definition(json!(["read"]))).unwrap();
        create(app, "b2", "Other bot", definition(json!(["read"]))).unwrap();
        assert_eq!(app.workflows_of("b1").len(), 1);
        assert_eq!(app.workflows_of("b2").len(), 1);
        assert!(second.workflows_of("b1").is_empty(), "another account sees nothing");

        // A definition from a newer schema is kept byte for byte and listed unavailable.
        let newer = json!({ "version": 2, "revision": "r9", "steps": [{ "prompt": "x", "future": [1, 2] }], "extra": true });
        app.insert_workflow(WorkflowRecord { id: "wf-new".into(), bot_id: "b1".into(), name: "Newer".into(), definition: newer.clone(), created_at: kept.created_at + 1.0 }).unwrap();

        let this = app.this_device_id();
        let listed: Vec<Value> = app.workflows_of("b1").iter().map(|w| out(app, w)).collect();
        assert_eq!(listed[0]["state"], "valid");
        assert_eq!(listed[1]["state"], "unavailable");
        assert!(listed[1]["reason"].as_str().unwrap().contains("Unsupported schema version 2"));

        drop(first);
        let app = scratch(&home);
        assert_eq!(app.this_device_id(), this);
        let stored = app.workflows_of("b1");
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0], kept);
        assert_eq!(stored[1].definition, newer, "an unknown schema survives a reload untouched");
        assert_eq!(app.workflows_of("b2").len(), 1);

        // A revoked tool makes the definition unavailable, not gone, and grants stay as set.
        let app = &app;
        let caps = crate::model::Capabilities { shell: false, ..Default::default() };
        create(app, "b1", "Shell", definition(json!(["bash"]))).unwrap();
        app.state.lock().unwrap().bots.iter_mut().find(|b| b.id == "b1").unwrap().capabilities = caps.clone();
        let shell = app.workflows_of("b1").into_iter().find(|w| w.name == "Shell").unwrap();
        let shown = out(app, &shell);
        assert_eq!(shown["state"], "unavailable");
        assert_eq!(app.workflows_of("b1").len(), 3);
        assert_eq!(app.bot("b1").unwrap().capabilities, caps);

        // A Device that is not the bot's Runner cannot check it and does not call it valid.
        assert_eq!(out(app, &WorkflowRecord { bot_id: "far".into(), ..stored[0].clone() })["state"], "unverified");

        // Deleting a bot takes its workflows; deleting one workflow leaves the rest.
        app.delete_workflow(&shell.id).unwrap();
        assert!(app.delete_workflow(&shell.id).is_err());
        app.delete_bot("b2").unwrap();
        assert!(app.workflows_of("b2").is_empty());
        assert_eq!(app.workflows_of("b1").len(), 2);
    }
}
