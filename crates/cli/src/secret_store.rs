//! The Runner's secret store, and the `secrets` tool bots keep the user's secrets in.
//!
//! A bot setting something up for the user meets secrets it must carry from one place to
//! another: the client secret Google shows once when it creates an OAuth client, a token a
//! dashboard generates, a key in a downloaded file. It saves each one here by name, from the
//! clipboard after pressing a page's Copy button, from a file, from a placeholder the shield
//! already put in its place (`secret_shield`), or as a new random value, and never sees it.
//! It then uses the name as a placeholder, `{{secret:GOOGLE_CLIENT_SECRET}}`:
//! - in `write` and `edit`, or with the tool's `write_env`, to put it in a `.env` file, which
//!   needs no permission: the value stays on this computer;
//! - in a shell command (`vercel env add`, `gh secret set`, `fly secrets set`) or a plugin call
//!   (a browser typing into a dashboard), which the user allows on the card. Allowing it always
//!   grants that exact command in that folder, or that plugin tool for those secrets, so later
//!   runs, routines included, go ahead without asking; anything else asks again.
//!
//! The store is a file in the Beans home, encrypted with a key derived from this machine's own
//! secret, so it opens on this computer alone and is never synced or sent anywhere. Values never
//! leave it for a model: the tool's results name secrets and say where they came from, and every
//! result passes the shield. Replacing or deleting a secret asks the user.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use beans_agent::{Tool, ToolError, ToolResult, ToolUpdateFn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::model::Bot;
use crate::plugins::mcp::{self, Decision};
use crate::secret_shield::placeholder;

/// The associated data the store's envelope is sealed with.
const KIND: &str = "beans-secret-store-v1";
/// Files read for a secret are small: a key, a `.env`, a downloaded credentials file.
const MAX_FILE: u64 = 256 * 1024;

/// A secret the user keeps on this Runner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoredSecret {
    pub value: String,
    /// Where it came from: "clipboard", "file ~/Downloads/client_secret.json", "a random value made here".
    pub source: String,
    /// What it is for, in the bot's words.
    #[serde(default)]
    pub note: String,
    /// The bot that saved it.
    #[serde(default)]
    pub saved_by: String,
    pub updated_at: f64,
}

/// A use the user allowed always.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Grant {
    /// This exact command, placeholders and all, run in this folder.
    Command { folder: String, command: String, secrets: Vec<String>, granted_at: f64 },
    /// This plugin tool, sent these secrets.
    Tool { tool: String, secrets: Vec<String>, granted_at: f64 },
}

impl Grant {
    /// The grant without `name`: a command that used it is gone, a tool keeps its other secrets.
    fn without(mut self, name: &str) -> Option<Grant> {
        match &mut self {
            Grant::Command { secrets, .. } => (!secrets.iter().any(|used| used == name)).then_some(self),
            Grant::Tool { secrets, .. } => {
                secrets.retain(|used| used != name);
                (!secrets.is_empty()).then_some(self)
            }
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Contents {
    #[serde(default)]
    secrets: BTreeMap<String, StoredSecret>,
    #[serde(default)]
    grants: Vec<Grant>,
}

/// The store, read from disk on first use.
#[derive(Default)]
pub struct SecretStore {
    contents: Mutex<Option<Contents>>,
}

pub(crate) fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// A secret's name: an environment variable's, as `.env` files and dashboards want it.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 64 && chars.next().is_some_and(|c| c == '_' || c.is_ascii_alphabetic()) && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

impl SecretStore {
    fn path(app: &App) -> PathBuf {
        app.config.home.join("secrets.sealed")
    }

    fn key(app: &App) -> Result<[u8; 32], String> {
        let machine = app.machine.lock().unwrap();
        let machine = machine.as_ref().ok_or("This Runner has no machine key yet, so it cannot keep secrets. Finish setting up Beans first.")?;
        let secret = crate::keys::unb64_32(&machine.machine_secret).map_err(|_| "This Runner's machine key cannot be read.".to_string())?;
        Ok(crate::keys::secrets_key(&secret))
    }

    /// Runs `change` on the contents, reading them first, and writes them back when it says so.
    fn with<R>(&self, app: &App, change: impl FnOnce(&mut Contents) -> (R, bool)) -> Result<R, String> {
        let mut guard = self.contents.lock().unwrap();
        if guard.is_none() {
            let path = Self::path(app);
            *guard = Some(match std::fs::read(&path) {
                Ok(bytes) => crate::crypto::decrypt_json(&Self::key(app)?, KIND, &bytes)
                    // Never written over: a store this machine cannot open is left for the user.
                    .map_err(|_| format!("The secret store at {} cannot be opened with this machine's key.", path.display()))?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Contents::default(),
                Err(error) => return Err(format!("The secret store cannot be read: {error}")),
            });
        }
        let contents = guard.as_mut().expect("read above");
        let (result, changed) = change(contents);
        if changed {
            let bytes = crate::crypto::encrypt_json(&Self::key(app)?, KIND, contents).map_err(|error| error.to_string())?;
            crate::config::write_private(&Self::path(app), &bytes).map_err(|error| format!("The secret store cannot be written: {error}"))?;
        }
        Ok(result)
    }

    /// Every stored secret, by name.
    pub fn secrets(&self, app: &App) -> Result<BTreeMap<String, StoredSecret>, String> {
        self.with(app, |contents| (contents.secrets.clone(), false))
    }

    /// Every use the user allowed always.
    pub fn grants(&self, app: &App) -> Result<Vec<Grant>, String> {
        self.with(app, |contents| (contents.grants.clone(), false))
    }

    /// Keeps `secret` as `name`; the one it replaces, if any.
    pub fn save(&self, app: &App, name: &str, secret: StoredSecret) -> Result<Option<StoredSecret>, String> {
        if !valid_name(name) {
            return Err(format!("{name} is not a name a secret can have: use letters, digits, and _, starting with a letter, as in GOOGLE_CLIENT_SECRET."));
        }
        if secret.value.is_empty() {
            return Err("The secret is empty.".into());
        }
        let value = secret.value.clone();
        let replaced = self.with(app, |contents| (contents.secrets.insert(name.to_string(), secret), true))?;
        app.secrets.pin(name, &value);
        Ok(replaced)
    }

    /// Forgets `name` and every grant that names it. Whether it was there. Its placeholder no
    /// longer resolves; its value stays hidden wherever it still shows.
    pub fn delete(&self, app: &App, name: &str) -> Result<bool, String> {
        let removed = self.with(app, |contents| {
            let removed = contents.secrets.remove(name).is_some();
            let before = contents.grants.clone();
            contents.grants = std::mem::take(&mut contents.grants).into_iter().filter_map(|grant| grant.without(name)).collect();
            (removed, removed || before != contents.grants)
        })?;
        if removed {
            app.secrets.retire(name);
        }
        Ok(removed)
    }

    /// Keeps a use the user allowed always: a command once, and a tool's secrets together.
    pub fn grant(&self, app: &App, grant: Grant) -> Result<(), String> {
        self.with(app, |contents| {
            for known in contents.grants.iter_mut() {
                match (known, &grant) {
                    (Grant::Command { folder: a, command: b, .. }, Grant::Command { folder: c, command: d, .. }) if a == c && b == d => return ((), false),
                    (Grant::Tool { tool: a, secrets: known, .. }, Grant::Tool { tool: b, secrets: added, .. }) if a == b => {
                        let before = known.len();
                        for name in added {
                            if !known.contains(name) {
                                known.push(name.clone());
                            }
                        }
                        return ((), known.len() != before);
                    }
                    _ => {}
                }
            }
            contents.grants.push(grant);
            ((), true)
        })
    }

    /// Forgets what the grants allow for `name`, or every grant. How many grants changed.
    pub fn revoke(&self, app: &App, name: Option<&str>) -> Result<usize, String> {
        self.with(app, |contents| {
            let before = contents.grants.clone();
            contents.grants = match name {
                Some(name) => std::mem::take(&mut contents.grants).into_iter().filter_map(|grant| grant.without(name)).collect(),
                None => Vec::new(),
            };
            let changed = before.iter().filter(|grant| !contents.grants.contains(grant)).count();
            (changed, changed > 0)
        })
    }

    /// Whether the user allowed `command`, exactly, in `folder` always.
    pub fn command_granted(&self, app: &App, folder: &Path, command: &str) -> bool {
        let folder = folder.display().to_string();
        self.grants(app).unwrap_or_default().iter().any(|grant| matches!(grant, Grant::Command { folder: f, command: c, .. } if *f == folder && c == command.trim()))
    }

    /// Whether the user allowed sending each of `used` through the plugin tool `tool` always.
    pub fn tool_granted(&self, app: &App, tool: &str, used: &[String]) -> bool {
        let grants = self.grants(app).unwrap_or_default();
        !used.is_empty()
            && used.iter().all(|name| {
                grants.iter().any(|grant| matches!(grant, Grant::Tool { tool: t, secrets, .. } if t == tool && secrets.contains(name)))
            })
    }

    /// Gives each stored secret its placeholder in the shield.
    pub fn pin_all(&self, app: &App) {
        if let Ok(secrets) = self.secrets(app) {
            for (name, secret) in secrets {
                app.secrets.pin(&name, &secret.value);
            }
        }
    }
}

/// What the clipboard holds, from the system's own tool.
async fn read_clipboard() -> Result<String, String> {
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbpaste", &[])]
    } else if cfg!(windows) {
        &[("powershell", &["-NoProfile", "-Command", "Get-Clipboard -Raw"])]
    } else {
        &[("wl-paste", &["--no-newline"]), ("xclip", &["-selection", "clipboard", "-o"]), ("xsel", &["--clipboard", "--output"])]
    };
    for (program, args) in candidates {
        if let Ok(output) = tokio::process::Command::new(program).args(*args).stdin(std::process::Stdio::null()).output().await {
            if output.status.success() {
                return Ok(String::from_utf8_lossy(&output.stdout).trim_end_matches(['\r', '\n']).to_string());
            }
        }
    }
    Err("This computer's clipboard cannot be read (pbpaste, wl-paste, xclip, xsel, or PowerShell).".into())
}

/// Empties the clipboard, so nothing pastes or shows the secret later.
async fn clear_clipboard() {
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(windows) {
        &[("powershell", &["-NoProfile", "-Command", "Set-Clipboard -Value $null"])]
    } else {
        &[("wl-copy", &["--clear"]), ("xclip", &["-selection", "clipboard", "-i", "/dev/null"]), ("xsel", &["--clipboard", "--clear"])]
    };
    for (program, args) in candidates {
        let status = tokio::process::Command::new(program).args(*args).stdin(std::process::Stdio::null()).status().await;
        if status.is_ok_and(|status| status.success()) {
            return;
        }
    }
}

/// The value under `key` in a file: a dotted path in JSON (`web.client_secret`), or a
/// variable in a `.env` file (`export` and quotes are removed). The whole file, trimmed,
/// without a key: a private key or a token file.
pub fn value_in_file(text: &str, key: Option<&str>) -> Result<String, String> {
    let Some(key) = key.filter(|key| !key.is_empty()) else {
        return Ok(text.trim().to_string());
    };
    if let Ok(json) = serde_json::from_str::<Value>(text) {
        let mut at = &json;
        for part in key.split('.') {
            at = match at {
                Value::Object(fields) => fields.get(part),
                Value::Array(items) => part.parse::<usize>().ok().and_then(|index| items.get(index)),
                _ => None,
            }
            .ok_or_else(|| format!("The file has no {key}."))?;
        }
        return match at {
            Value::String(text) => Ok(text.clone()),
            Value::Number(number) => Ok(number.to_string()),
            _ => Err(format!("{key} in the file is not a single value.")),
        };
    }
    // A `.env` (`KEY=value`) or YAML (`key: value`) file; a later line wins, as dotenv reads it.
    let mut found = None;
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let pair = line.split_once('=').filter(|(name, _)| name.trim() == key).or_else(|| line.split_once(':').filter(|(name, _)| name.trim().trim_matches(['"', '\'']) == key));
        if let Some((_, value)) = pair {
            let value = value.trim();
            let quoted = value.strip_prefix('"').and_then(|v| v.rsplit_once('"')).map(|(v, _)| unescape_double(v))
                .or_else(|| value.strip_prefix('\'').and_then(|v| v.rsplit_once('\'')).map(|(v, _)| v.to_string()));
            // Unquoted, ` #` starts a comment.
            found = Some(quoted.unwrap_or_else(|| value.split(" #").next().unwrap_or(value).trim().to_string()));
        }
    }
    found.ok_or_else(|| format!("The file has no {key}."))
}

/// A double-quoted `.env` value as it reads: `\n`, `\r`, `\"`, and `\\` undone in one pass.
fn unescape_double(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other @ ('"' | '\\')) => out.push(other),
            Some(other) => out.extend(['\\', other]),
            None => out.push('\\'),
        }
    }
    out
}

/// `text`, a `.env` file, with `NAME=value` for each of `values`: an existing line replaced in
/// place, a new one added at the end. How many were added.
pub fn upsert_env(text: &str, values: &[(String, String)]) -> (String, usize) {
    // Plain when it can be; in single quotes, which dotenv reads literally (no `$` expansion),
    // unless it holds one or a line break; otherwise escaped in double quotes.
    let quote = |value: &str| {
        if !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric() || "_-./:@+=,".contains(c)) {
            value.to_string()
        } else if !value.contains(['\'', '\n', '\r']) {
            format!("'{value}'")
        } else {
            format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r"))
        }
    };
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut added = 0;
    for (name, value) in values {
        let line = format!("{name}={}", quote(value));
        let existing = lines.iter().position(|existing| {
            let existing = existing.trim_start();
            let existing = existing.strip_prefix("export ").unwrap_or(existing);
            existing.split_once('=').is_some_and(|(key, _)| key.trim() == name)
        });
        match existing {
            Some(at) => lines[at] = if lines[at].trim_start().starts_with("export ") { format!("export {line}") } else { line },
            None => {
                lines.push(line);
                added += 1;
            }
        }
    }
    let mut out = lines.join(newline);
    out.push_str(newline);
    (out, added)
}

/// A new random secret: `length` characters of hex, base64url, or letters and digits.
pub fn generate(length: usize, format: &str) -> Result<String, String> {
    use rand::Rng;
    let alphabet: &[u8] = match format {
        "hex" => b"0123456789abcdef",
        "alphanumeric" => b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
        "base64url" | "" => b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_",
        other => return Err(format!("{other} is not a format: use hex, base64url, or alphanumeric.")),
    };
    if !(16..=256).contains(&length) {
        return Err("A generated secret has 16 to 256 characters.".into());
    }
    let mut rng = rand::rngs::OsRng;
    Ok((0..length).map(|_| alphabet[rng.gen_range(0..alphabet.len())] as char).collect())
}

/// The `secrets` tool.
pub struct SecretsTool {
    pub app: Arc<App>,
    pub chat_id: String,
    pub bot: Bot,
    pub unattended: bool,
    pub workdir: PathBuf,
}

const DESCRIPTION: &str = "Keep the user's secrets (API keys, OAuth client secrets, tokens, passwords) on this computer, \
    without ever seeing their values. Each is a name, used as the placeholder {{secret:NAME}}. \
    list: the secrets kept, where each came from, and the uses the user allowed always. \
    save: keep a secret as name, from one of: clipboard (after pressing a page's Copy button; the clipboard is emptied \
    after), file (path, and key: a dotted JSON path such as web.client_secret or a .env variable; none for the whole file), \
    or placeholder (a {{secret:…}} a page, file, or command output showed you). \
    generate: keep a new random secret as name (length, default 32; format hex, base64url, or alphanumeric). \
    write_env: put the secrets in names into a .env file at path, as NAME=value, replacing existing lines. \
    delete: forget name (the user confirms). revoke: forget the uses the user allowed always for name, or for all. \
    Then use {{secret:NAME}} in write and edit, where the file gets the value with no question, or in a shell command \
    or plugin call, which the user allows on its card. Never take a screenshot while a secret shows on screen; copy it \
    and save it from the clipboard instead.";

#[async_trait]
impl Tool for SecretsTool {
    fn name(&self) -> &str {
        "secrets"
    }
    fn description(&self) -> &str {
        DESCRIPTION
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "save", "generate", "write_env", "delete", "revoke"] },
                "name": { "type": "string", "description": "The secret's name, as an environment variable: GOOGLE_CLIENT_SECRET" },
                "from": { "type": "string", "enum": ["clipboard", "file", "placeholder"], "description": "save: where the value is" },
                "placeholder": { "type": "string", "description": "save from placeholder: the {{secret:…}} you were shown" },
                "path": { "type": "string", "description": "save from file: the file; write_env: the .env file" },
                "key": { "type": "string", "description": "save from file: a dotted JSON path or a .env variable" },
                "note": { "type": "string", "description": "What the secret is for, for the user" },
                "length": { "type": "integer", "description": "generate: how many characters" },
                "format": { "type": "string", "enum": ["hex", "base64url", "alphanumeric"] },
                "names": { "type": "array", "items": { "type": "string" }, "description": "write_env: the secrets to write" },
                "keep_clipboard": { "type": "boolean", "description": "save from clipboard: leave it as it is" }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }
    fn execution_mode(&self) -> Option<beans_agent::agent_loop::ToolExecutionMode> {
        Some(beans_agent::agent_loop::ToolExecutionMode::Sequential)
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        // The environment and what the shield knows, so placeholders resolve.
        crate::secret_shield::vault(&self.app).await;
        let text = |key: &str| args[key].as_str().map(str::trim).filter(|value| !value.is_empty());
        let store = &self.app.secret_store;
        let reply = match text("action").unwrap_or("") {
            "list" => self.list(),
            "save" | "generate" => self.save(&args, &cancel).await,
            "write_env" => self.write_env(&args, &cancel).await,
            "delete" => {
                let name = text("name").ok_or("delete needs the secret's name.")?;
                if !store.secrets(&self.app)?.contains_key(name) {
                    return Err(format!("No secret named {name} is kept.").into());
                }
                self.confirm(&format!("Delete the secret {name}?"), &format!("The bot asks to delete {name} from this computer's secret store. It cannot be undone."), &args, &cancel).await?;
                store.delete(&self.app, name)?;
                Ok(format!("Deleted {name}, and the uses allowed always that named it."))
            }
            "revoke" => {
                let removed = store.revoke(&self.app, text("name"))?;
                Ok(format!("Forgot {removed} use{} allowed always. They ask again next time.", if removed == 1 { "" } else { "s" }))
            }
            other => Err(format!("{other} is not an action: use list, save, generate, write_env, delete, or revoke.")),
        }?;
        Ok(ToolResult { content: vec![beans_agent::ContentPart::text(reply)], ..ToolResult::default() })
    }
}

impl SecretsTool {
    fn list(&self) -> Result<String, String> {
        let store = &self.app.secret_store;
        let secrets = store.secrets(&self.app)?;
        if secrets.is_empty() {
            return Ok("No secrets are kept yet. Save one with action save or generate.".into());
        }
        let mut out = String::from("Secrets kept on this computer (values are never shown):\n");
        for (name, secret) in &secrets {
            let note = if secret.note.is_empty() { String::new() } else { format!(", for {}", secret.note) };
            out.push_str(&format!("- {}: {} characters, from {}{note}\n", placeholder(name), secret.value.chars().count(), secret.source));
        }
        let grants = store.grants(&self.app)?;
        if !grants.is_empty() {
            out.push_str("Uses the user allowed always:\n");
            for grant in grants {
                match grant {
                    Grant::Command { folder, command, .. } => out.push_str(&format!("- the command `{command}` in {folder}\n")),
                    Grant::Tool { tool, secrets, .. } => out.push_str(&format!("- {} through {tool}\n", secrets.join(", "))),
                }
            }
        }
        Ok(out)
    }

    /// Asks the user to confirm `what`; Err when they do not, or nobody is there.
    async fn confirm(&self, summary: &str, reason: &str, args: &Value, cancel: &CancellationToken) -> Result<(), String> {
        // Full access: the user chose to be asked nothing.
        if self.app.full_access() {
            return Ok(());
        }
        if self.unattended {
            return Err(format!("{reason} Nobody is here to confirm it, so nothing changed."));
        }
        match mcp::ask_with_rule(&self.app, &self.chat_id, &self.bot.id, "secrets", "Secrets", "secrets", summary, args.clone(), Some(reason.into()), None, cancel).await {
            Decision::Allowed | Decision::Always => Ok(()),
            Decision::Denied => Err("The user did not allow it; nothing changed. Do not retry it; ask what they want instead.".into()),
            Decision::Expired => Err("Nobody answered in time; nothing changed.".into()),
            Decision::Dismissed => Err("The user sent a new message instead of answering; nothing changed. Follow that message.".into()),
        }
    }

    fn resolve_path(&self, path: &str) -> PathBuf {
        match path.strip_prefix("~/") {
            Some(rest) => dirs::home_dir().map(|home| home.join(rest)).unwrap_or_else(|| PathBuf::from(path)),
            None => self.workdir.join(path),
        }
    }

    async fn save(&self, args: &Value, cancel: &CancellationToken) -> Result<String, String> {
        let text = |key: &str| args[key].as_str().map(str::trim).filter(|value| !value.is_empty());
        let name = text("name").ok_or("save and generate need the secret's name, such as GOOGLE_CLIENT_SECRET.")?;
        if !valid_name(name) {
            return Err(format!("{name} is not a name a secret can have: use letters, digits, and _, starting with a letter."));
        }
        let (value, source) = if args["action"] == "generate" {
            let length = args["length"].as_u64().unwrap_or(32) as usize;
            (generate(length, text("format").unwrap_or("base64url"))?, "a random value made here".to_string())
        } else {
            match text("from") {
                Some("clipboard") => {
                    let value = read_clipboard().await?;
                    if value.trim().is_empty() {
                        return Err("The clipboard is empty. Press the page's Copy button next to the secret first.".into());
                    }
                    if args["keep_clipboard"] != true {
                        clear_clipboard().await;
                    }
                    (value.trim().to_string(), "clipboard".to_string())
                }
                Some("file") => {
                    let path = text("path").ok_or("save from file needs its path.")?;
                    let resolved = self.resolve_path(path);
                    let metadata = std::fs::metadata(&resolved).map_err(|_| format!("{path} cannot be read."))?;
                    if metadata.len() > MAX_FILE {
                        return Err(format!("{path} is too large to hold one secret."));
                    }
                    let contents = std::fs::read_to_string(&resolved).map_err(|_| format!("{path} cannot be read as text."))?;
                    let value = value_in_file(&contents, text("key"))?;
                    (value, format!("file {path}{}", text("key").map(|key| format!(" ({key})")).unwrap_or_default()))
                }
                Some("placeholder") => {
                    let shown = text("placeholder").ok_or("save from placeholder needs the {{secret:…}} you were shown.")?;
                    // Exactly one placeholder, or its bare name: text around it would be saved too.
                    let names = crate::secret_shield::placeholders(shown);
                    let name_shown = match names.as_slice() {
                        [only] if shown == placeholder(only) => only.clone(),
                        [] if shown.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')) => shown.to_string(),
                        _ => return Err("Give exactly one placeholder, such as {{secret:secret}}, with nothing around it.".into()),
                    };
                    let value = self.app.secrets.value(&name_shown).ok_or_else(|| format!("{} is not a secret this Runner knows now. Show it again, then save the placeholder you get.", placeholder(&name_shown)))?;
                    (value, "a placeholder the bot was shown".to_string())
                }
                _ => return Err("save needs from: clipboard, file, or placeholder.".into()),
            }
        };
        if value.is_empty() {
            return Err("The secret is empty.".into());
        }
        let existing = self.app.secret_store.secrets(&self.app)?.remove(name);
        if existing.as_ref().is_some_and(|existing| existing.value != value) {
            self.confirm(&format!("Replace the secret {name}?"), &format!("The bot asks to replace the secret {name} kept on this computer with a new value, from {source}."), args, cancel).await?;
        }
        let note = text("note").unwrap_or_default().to_string();
        let secret = StoredSecret { value: value.clone(), source: source.clone(), note, saved_by: self.bot.name.clone(), updated_at: crate::secret_store::now() };
        self.app.secret_store.save(&self.app, name, secret)?;
        // A short value cannot be told apart from ordinary words in output.
        let warning = match value.chars().count() {
            0..4 => " It is too short to hide where a command prints it: tell the user.",
            4..8 => " It is short, so it is hidden in output only as a whole word.",
            _ => "",
        };
        Ok(format!(
            "Saved {} ({} characters, from {source}). Use it in write or edit, or with write_env, to put it in a file; in a shell command or plugin call the user allows it first.{warning}",
            placeholder(name),
            value.chars().count()
        ))
    }

    async fn write_env(&self, args: &Value, cancel: &CancellationToken) -> Result<String, String> {
        if !self.app.bot(&self.bot.id).is_some_and(|bot| bot.capabilities.permits("write", None)) {
            return Err("write_env writes a file, which this bot is not permitted to do.".into());
        }
        let path = args["path"].as_str().map(str::trim).filter(|path| !path.is_empty()).ok_or("write_env needs the .env file's path.")?;
        let names: Vec<String> = args["names"].as_array().map(|names| names.iter().filter_map(|name| name.as_str().map(str::to_string)).collect()).unwrap_or_default();
        if names.is_empty() {
            return Err("write_env needs names: the secrets to write.".into());
        }
        let stored = self.app.secret_store.secrets(&self.app)?;
        let mut values = Vec::new();
        for name in &names {
            let value = stored.get(name).map(|secret| secret.value.clone()).or_else(|| self.app.secrets.value(name)).ok_or_else(|| format!("No secret named {name} is kept. Save it first."))?;
            let variable = if valid_name(name) { name.clone() } else { return Err(format!("{name} cannot be a variable's name in a .env file.")) };
            values.push((variable, value));
        }
        // A link is followed, so the file it points to is the one written, and kept a link.
        let mut resolved = self.resolve_path(path);
        if std::fs::symlink_metadata(&resolved).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            resolved = std::fs::canonicalize(&resolved).map_err(|_| format!("{path} is a link to a file that is not there."))?;
        }
        let place = crate::secret_shield::risky_destination(&resolved, &self.workdir)
            .or_else(|| (!crate::secret_shield::secrets_file(&resolved)).then(|| "which is not a file for secrets: programs may run or send what it holds".to_string()));
        if let Some(place) = place {
            self.confirm(
                &format!("Write {} to {path}?", names.join(", ")),
                &format!("The bot asks to write the secrets {} to {}, {place}.", names.join(", "), resolved.display()),
                args,
                cancel,
            )
            .await?;
        }
        let existing = std::fs::read_to_string(&resolved).unwrap_or_default();
        let (text, added) = upsert_env(&existing, &values);
        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent).map_err(|error| format!("{path} cannot be written: {error}"))?;
        }
        crate::config::write_private(&resolved, text.as_bytes()).map_err(|error| format!("{path} cannot be written: {error}"))?;
        self.app.secrets.mark_file(path);
        Ok(format!("Wrote {} to {path} ({added} added, {} replaced). The file is readable by this user alone.", names.join(", "), values.len() - added))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_app() -> (Arc<App>, PathBuf) {
        let home = std::env::temp_dir().join(format!("beans-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        *app.machine.lock().unwrap() = Some(crate::keys::MachineFile {
            format: crate::config::Format::BeansV2,
            machine_secret: crate::keys::b64(&crate::keys::random_32()),
            identity_pubkey: "identity".into(),
            content_pubkey: "content".into(),
            account_dek: crate::keys::b64(&crate::keys::random_32()),
            name: "Runner".into(),
            os: "macos".into(),
            os_version: String::new(),
            model: String::new(),
            registered: true,
            relay_url: None,
            created_at: 1,
        });
        (app, home)
    }

    fn secret(value: &str) -> StoredSecret {
        StoredSecret { value: value.into(), source: "clipboard".into(), note: String::new(), saved_by: "Bot".into(), updated_at: 1.0 }
    }

    #[test]
    fn the_store_is_encrypted_for_this_machine_and_survives_a_restart() {
        let (app, home) = scratch_app();
        let store = &app.secret_store;
        assert!(store.secrets(&app).unwrap().is_empty());
        assert_eq!(store.save(&app, "GOOGLE_CLIENT_SECRET", secret("GOCSPX-abcdefghijklmnopqrstuv")).unwrap(), None);
        let bytes = std::fs::read(home.join("secrets.sealed")).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("GOCSPX") && !String::from_utf8_lossy(&bytes).contains("GOOGLE_CLIENT_SECRET"), "names and values are sealed");
        // The shield shows its name from now on, and resolves it.
        assert_eq!(app.secrets.redact("is GOCSPX-abcdefghijklmnopqrstuv"), "is {{secret:GOOGLE_CLIENT_SECRET}}");
        // A second App on the same home and machine key reads it.
        let machine = app.machine.lock().unwrap().clone();
        let again = App::load(crate::config::Config { home: home.clone(), port: 0 });
        if let Ok(again) = again {
            *again.machine.lock().unwrap() = machine;
            assert_eq!(again.secret_store.secrets(&again).unwrap()["GOOGLE_CLIENT_SECRET"].value, "GOCSPX-abcdefghijklmnopqrstuv");
            again.secret_store.pin_all(&again);
            assert_eq!(again.secrets.resolve("{{secret:GOOGLE_CLIENT_SECRET}}").unwrap(), "GOCSPX-abcdefghijklmnopqrstuv");
        }
        // Another machine's key cannot open it, and does not write over it.
        let (other, other_home) = scratch_app();
        std::fs::copy(home.join("secrets.sealed"), other_home.join("secrets.sealed")).unwrap();
        assert!(other.secret_store.secrets(&other).unwrap_err().contains("cannot be opened"));
        assert!(other.secret_store.save(&other, "X", secret("another-value")).is_err());
        assert_eq!(std::fs::read(other_home.join("secrets.sealed")).unwrap(), bytes);
        let _ = std::fs::remove_dir_all(home);
        let _ = std::fs::remove_dir_all(other_home);
    }

    #[test]
    fn grants_are_exact_and_deleting_a_secret_forgets_them() {
        let (app, home) = scratch_app();
        let store = &app.secret_store;
        store.save(&app, "VERCEL_TOKEN", secret("vercel-token-value-1234")).unwrap();
        let folder = PathBuf::from("/work/app");
        let command = "printf %s '{{secret:VERCEL_TOKEN}}' | vercel env add TOKEN production";
        assert!(!store.command_granted(&app, &folder, command));
        store.grant(&app, Grant::Command { folder: folder.display().to_string(), command: command.into(), secrets: vec!["VERCEL_TOKEN".into()], granted_at: 1.0 }).unwrap();
        assert!(store.command_granted(&app, &folder, command));
        assert!(store.command_granted(&app, &folder, &format!("  {command}\n")));
        assert!(!store.command_granted(&app, &folder, &command.replace("production", "preview")), "another command asks again");
        assert!(!store.command_granted(&app, Path::new("/elsewhere"), command), "another folder asks again");
        store.grant(&app, Grant::Tool { tool: "playwright__browser_type".into(), secrets: vec!["VERCEL_TOKEN".into()], granted_at: 1.0 }).unwrap();
        assert!(store.tool_granted(&app, "playwright__browser_type", &["VERCEL_TOKEN".into()]));
        assert!(!store.tool_granted(&app, "playwright__browser_type", &["VERCEL_TOKEN".into(), "OTHER".into()]));
        assert!(!store.tool_granted(&app, "other__tool", &["VERCEL_TOKEN".into()]));
        assert!(!store.tool_granted(&app, "playwright__browser_type", &[]));
        assert!(store.delete(&app, "VERCEL_TOKEN").unwrap());
        assert!(store.grants(&app).unwrap().is_empty());
        assert!(!store.delete(&app, "VERCEL_TOKEN").unwrap());
        let _ = std::fs::remove_dir_all(home);
    }

    async fn call(tool: &SecretsTool, args: Value) -> Result<String, String> {
        let noop: ToolUpdateFn = Arc::new(|_| {});
        tool.execute("1", args, CancellationToken::new(), noop).await.map(|result| result.content[0].as_text().unwrap().to_string()).map_err(|error| error.0)
    }

    /// An OAuth client set up end to end: the secret comes from a downloaded file and a page,
    /// goes into the store and a `.env` file, and never into a result.
    #[tokio::test]
    async fn a_bot_keeps_and_places_secrets_without_seeing_them() {
        let (app, home) = scratch_app();
        let work = home.join("project");
        std::fs::create_dir_all(&work).unwrap();
        let bot: Bot = serde_json::from_value(json!({
            "id": "bot", "name": "Bot", "description": "", "symbol_name": "", "accent": "", "runner_id": "runner", "provider": "deepseek", "created_at": 0.0
        }))
        .unwrap();
        app.state.lock().unwrap().bots.push(bot.clone());
        app.state.lock().unwrap().chats.push(serde_json::from_value(json!({ "id": "chat", "kind": "dm", "bot_ids": ["bot"], "created_at": 0.0 })).unwrap());
        let tool = SecretsTool { app: app.clone(), chat_id: "chat".into(), bot: bot.clone(), unattended: false, workdir: work.clone() };
        let secret = "GOCSPX-AbCdEfGhIjKlMnOpQrStUvWx";
        std::fs::write(home.join("client_secret.json"), json!({ "web": { "client_id": "1-x.apps.googleusercontent.com", "client_secret": secret } }).to_string()).unwrap();

        let saved = call(&tool, json!({ "action": "save", "name": "GOOGLE_CLIENT_SECRET", "from": "file", "path": home.join("client_secret.json").display().to_string(), "key": "web.client_secret", "note": "Google sign-in" })).await.unwrap();
        assert!(saved.contains("{{secret:GOOGLE_CLIENT_SECRET}}") && !saved.contains(secret), "{saved}");
        // A value the shield hid on a page, kept under a name of its own.
        let shown = app.secrets.redact("Signing secret: whsec_Zx8Kq2Lm9Pv4Rt7Nw1Yb3Jd6");
        let placeholder_shown = shown.trim_start_matches("Signing secret: ").to_string();
        assert!(placeholder_shown.starts_with("{{secret:"), "{shown}");
        call(&tool, json!({ "action": "save", "name": "STRIPE_WEBHOOK_SECRET", "from": "placeholder", "placeholder": placeholder_shown })).await.unwrap();
        call(&tool, json!({ "action": "generate", "name": "NEXTAUTH_SECRET", "length": 48 })).await.unwrap();
        assert!(call(&tool, json!({ "action": "save", "name": "X", "from": "placeholder", "placeholder": "{{secret:NEVER_SHOWN}}" })).await.unwrap_err().contains("not a secret this Runner knows"));
        assert!(call(&tool, json!({ "action": "save", "name": "bad-name", "from": "file", "path": "x" })).await.unwrap_err().contains("not a name"));

        let listed = call(&tool, json!({ "action": "list" })).await.unwrap();
        assert!(listed.contains("{{secret:GOOGLE_CLIENT_SECRET}}: 31 characters, from file") && listed.contains("for Google sign-in") && listed.contains("NEXTAUTH_SECRET"), "{listed}");
        let stored = app.secret_store.secrets(&app).unwrap();
        for value in stored.values() {
            assert!(!listed.contains(&value.value), "list shows no value");
        }
        assert_eq!(stored["NEXTAUTH_SECRET"].value.len(), 48);
        assert_eq!(stored["STRIPE_WEBHOOK_SECRET"].value, "whsec_Zx8Kq2Lm9Pv4Rt7Nw1Yb3Jd6");

        // Into a .env file, beside what it held.
        std::fs::write(work.join(".env"), "PORT=3000
GOOGLE_CLIENT_SECRET=old
").unwrap();
        let wrote = call(&tool, json!({ "action": "write_env", "path": ".env", "names": ["GOOGLE_CLIENT_SECRET", "NEXTAUTH_SECRET"] })).await.unwrap();
        assert!(wrote.contains("1 added, 1 replaced") && !wrote.contains(secret), "{wrote}");
        let env = std::fs::read_to_string(work.join(".env")).unwrap();
        assert!(env.starts_with(&format!("PORT=3000
GOOGLE_CLIENT_SECRET={secret}
NEXTAUTH_SECRET=")), "{env}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(work.join(".env")).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(app.secrets.files().contains(&(String::new(), ".env".to_string())), "a command naming .env now asks");
        assert!(call(&tool, json!({ "action": "write_env", "path": ".env", "names": ["MISSING"] })).await.unwrap_err().contains("No secret named MISSING"));

        // Replacing and deleting ask the user; with nobody there, nothing changes.
        let away = SecretsTool { unattended: true, ..SecretsTool { app: app.clone(), chat_id: "chat".into(), bot: bot.clone(), unattended: false, workdir: work.clone() } };
        assert!(call(&away, json!({ "action": "delete", "name": "NEXTAUTH_SECRET" })).await.unwrap_err().contains("Nobody is here"));
        assert!(call(&away, json!({ "action": "generate", "name": "NEXTAUTH_SECRET" })).await.unwrap_err().contains("Nobody is here"));
        assert_eq!(app.secret_store.secrets(&app).unwrap()["NEXTAUTH_SECRET"].value, stored["NEXTAUTH_SECRET"].value);
        // Bounded, so a card that is never asked fails the test instead of hanging it.
        let answer = |decision: Decision| {
            let app = app.clone();
            async move {
                let started = std::time::Instant::now();
                loop {
                    assert!(started.elapsed() < std::time::Duration::from_secs(10), "the card was never asked");
                    let pending = app.pending_permissions.lock().unwrap().keys().next().cloned();
                    if let Some(id) = pending {
                        assert!(mcp::answer(&app, &id, decision));
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }
        };
        let (denied, _) = tokio::join!(call(&tool, json!({ "action": "delete", "name": "NEXTAUTH_SECRET" })), answer(Decision::Denied));
        assert!(denied.unwrap_err().contains("did not allow"));
        let (deleted, _) = tokio::join!(call(&tool, json!({ "action": "delete", "name": "NEXTAUTH_SECRET" })), answer(Decision::Allowed));
        assert!(deleted.unwrap().starts_with("Deleted NEXTAUTH_SECRET"));
        assert!(!app.secret_store.secrets(&app).unwrap().contains_key("NEXTAUTH_SECRET"));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn deleting_retires_the_placeholder_and_grants_merge_and_persist() {
        let (app, home) = scratch_app();
        let store = &app.secret_store;
        store.save(&app, "A_TOKEN", secret("a-token-value-123456")).unwrap();
        store.save(&app, "B_TOKEN", secret("b-token-value-123456")).unwrap();
        assert_eq!(app.secrets.resolve("{{secret:A_TOKEN}}").unwrap(), "a-token-value-123456");
        let tool = |secrets: &[&str]| Grant::Tool { tool: "t".into(), secrets: secrets.iter().map(|s| s.to_string()).collect(), granted_at: 1.0 };
        store.grant(&app, tool(&["A_TOKEN"])).unwrap();
        store.grant(&app, tool(&["A_TOKEN"])).unwrap();
        store.grant(&app, tool(&["B_TOKEN"])).unwrap();
        let command = Grant::Command { folder: "/w".into(), command: "deploy {{secret:A_TOKEN}}".into(), secrets: vec!["A_TOKEN".into()], granted_at: 1.0 };
        store.grant(&app, command.clone()).unwrap();
        store.grant(&app, command).unwrap();
        assert_eq!(store.grants(&app).unwrap().len(), 2, "a tool's grants merge, a command's is kept once");
        assert!(store.tool_granted(&app, "t", &["A_TOKEN".into(), "B_TOKEN".into()]));

        // Grants survive a restart, read with the same machine key.
        let machine = app.machine.lock().unwrap().clone();
        let again = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        *again.machine.lock().unwrap() = machine;
        assert!(again.secret_store.command_granted(&again, Path::new("/w"), "deploy {{secret:A_TOKEN}}"));
        drop(again);

        // Deleting forgets its grants, and its placeholder; its value stays hidden.
        assert!(store.delete(&app, "A_TOKEN").unwrap());
        assert!(!store.command_granted(&app, Path::new("/w"), "deploy {{secret:A_TOKEN}}"));
        assert!(!store.tool_granted(&app, "t", &["A_TOKEN".into()]) && store.tool_granted(&app, "t", &["B_TOKEN".into()]));
        assert!(app.secrets.resolve("{{secret:A_TOKEN}}").is_err());
        assert_eq!(app.secrets.redact("a-token-value-123456"), "{{secret:A_TOKEN}}");
        // Pinning the rest again, as each call does, does not bring it back.
        store.pin_all(&app);
        assert!(app.secrets.resolve("{{secret:A_TOKEN}}").is_err());

        assert_eq!(store.revoke(&app, Some("B_TOKEN")).unwrap(), 1);
        store.grant(&app, tool(&["B_TOKEN"])).unwrap();
        assert_eq!(store.revoke(&app, None).unwrap(), 1);
        assert_eq!(store.revoke(&app, None).unwrap(), 0);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_damaged_store_or_a_missing_key_changes_nothing() {
        let (app, home) = scratch_app();
        std::fs::write(home.join("secrets.sealed"), b"not an envelope at all").unwrap();
        assert!(app.secret_store.secrets(&app).unwrap_err().contains("cannot be opened"));
        assert!(app.secret_store.save(&app, "X", secret("a-value-1234")).is_err());
        assert_eq!(std::fs::read(home.join("secrets.sealed")).unwrap(), b"not an envelope at all");

        // No machine key yet: nothing is read or written, and it works once there is one.
        let (fresh, fresh_home) = scratch_app();
        let machine = fresh.machine.lock().unwrap().take();
        assert!(fresh.secret_store.save(&fresh, "X", secret("a-value-1234")).unwrap_err().contains("no machine key"));
        assert!(!fresh_home.join("secrets.sealed").exists());
        *fresh.machine.lock().unwrap() = machine;
        fresh.secret_store.save(&fresh, "X", secret("a-value-1234")).unwrap();
        assert!(fresh_home.join("secrets.sealed").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(fresh_home.join("secrets.sealed")).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(home);
        let _ = std::fs::remove_dir_all(fresh_home);
    }

    #[test]
    fn env_and_yaml_files_read_as_their_tools_read_them() {
        let env = "A=plain # a comment\nB=\"quoted # not a comment\"\nC='single # literal'\nD=first\nD=second\nE=\"line\\nbreak \\\"q\\\" back\\\\slash\"\nexport F = spaced \nG=\n";
        for (key, value) in [("A", "plain"), ("B", "quoted # not a comment"), ("C", "single # literal"), ("D", "second"), ("E", "line\nbreak \"q\" back\\slash"), ("F", "spaced"), ("G", "")] {
            assert_eq!(value_in_file(env, Some(key)).unwrap(), value, "{key}");
        }
        assert!(value_in_file(env, Some("H")).is_err());
        let yaml = "database:\n  host: db\npassword: \"s3cret value\"\napi_key: abc123\n'quoted_key': v1\n";
        assert_eq!(value_in_file(yaml, Some("password")).unwrap(), "s3cret value");
        assert_eq!(value_in_file(yaml, Some("api_key")).unwrap(), "abc123");
        assert_eq!(value_in_file(yaml, Some("quoted_key")).unwrap(), "v1");
        let json = r#"{"a": [{"b": "x"}], "n": 5, "t": true, "z": null}"#;
        assert_eq!(value_in_file(json, Some("a.0.b")).unwrap(), "x");
        assert_eq!(value_in_file(json, Some("n")).unwrap(), "5");
        assert!(value_in_file(json, Some("t")).is_err() && value_in_file(json, Some("z")).is_err() && value_in_file(json, Some("a.9.b")).is_err());

        // Whatever a value holds, what write_env writes reads back the same.
        // `C:\new` with a quote is written escaped: `\\n` must read back as a backslash and n.
        for value in ["plain-123", "with space", "dollar $HOME and `cmd`", "it's quoted", "line\nbreak", "back\\slash \"both\" 'kinds'", "C:\\new 'dir'", "ends with \\", "#hash", "=equals=", "ünïcode"] {
            let (text, _) = upsert_env("", &[("K".into(), value.into())]);
            assert_eq!(value_in_file(&text, Some("K")).unwrap(), value, "{text}");
        }
        let (text, added) = upsert_env("A=1\r\nB=2\r\n", &[("B".into(), "3".into()), ("C".into(), "4".into())]);
        assert_eq!((text.as_str(), added), ("A=1\r\nB=3\r\nC=4\r\n", 1), "Windows line ends are kept");
        assert_eq!(upsert_env("", &[("K".into(), "$HOME".into())]).0, "K='$HOME'\n", "single quotes keep a dollar literal");
        assert_eq!(upsert_env("", &[("K".into(), String::new())]).0, "K=''\n");
    }

    #[tokio::test]
    async fn the_tool_refuses_what_it_should() {
        let (app, home) = scratch_app();
        let work = home.join("project");
        std::fs::create_dir_all(&work).unwrap();
        let mut bot: Bot = serde_json::from_value(json!({
            "id": "bot", "name": "Bot", "description": "", "symbol_name": "", "accent": "", "runner_id": "runner", "provider": "deepseek", "created_at": 0.0
        }))
        .unwrap();
        app.state.lock().unwrap().bots.push(bot.clone());
        let tool = SecretsTool { app: app.clone(), chat_id: "chat".into(), bot: bot.clone(), unattended: true, workdir: work.clone() };
        app.secrets.pin("shown", "a-value-the-page-showed-1");
        for (args, error) in [
            (json!({ "action": "teleport" }), "not an action"),
            (json!({ "action": "save", "name": "X" }), "save needs from"),
            (json!({ "action": "save", "from": "placeholder", "placeholder": "{{secret:shown}}" }), "need the secret's name"),
            (json!({ "action": "save", "name": "X", "from": "placeholder", "placeholder": "key {{secret:shown}}" }), "exactly one placeholder"),
            (json!({ "action": "save", "name": "X", "from": "placeholder", "placeholder": "{{secret:shown}}{{secret:shown}}" }), "exactly one placeholder"),
            (json!({ "action": "save", "name": "X", "from": "file", "path": "missing.json" }), "cannot be read"),
            (json!({ "action": "generate", "name": "X", "length": 8 }), "16 to 256"),
            (json!({ "action": "generate", "name": "X", "format": "emoji" }), "not a format"),
            (json!({ "action": "delete", "name": "NOPE" }), "No secret named NOPE"),
            (json!({ "action": "write_env", "path": ".env" }), "needs names"),
            (json!({ "action": "write_env", "names": ["shown"] }), "needs the .env file's path"),
            (json!({ "action": "write_env", "path": ".env", "names": ["bad-name"] }), "No secret named bad-name"),
        ] {
            let got = call(&tool, args.clone()).await.unwrap_err();
            assert!(got.contains(error), "{args}: {got}");
        }
        // A bare name works as well as its placeholder.
        assert!(call(&tool, json!({ "action": "save", "name": "FROM_PAGE", "from": "placeholder", "placeholder": "shown" })).await.unwrap().contains("{{secret:FROM_PAGE}}"));
        // Short values say how well they hide.
        std::fs::write(work.join("pin.txt"), "4821").unwrap();
        assert!(call(&tool, json!({ "action": "save", "name": "PIN", "from": "file", "path": "pin.txt" })).await.unwrap().contains("only as a whole word"));
        std::fs::write(work.join("tiny.txt"), "ab").unwrap();
        assert!(call(&tool, json!({ "action": "save", "name": "TINY", "from": "file", "path": "tiny.txt" })).await.unwrap().contains("too short to hide"));
        std::fs::write(work.join("big.txt"), "x".repeat(300 * 1024)).unwrap();
        assert!(call(&tool, json!({ "action": "save", "name": "BIG", "from": "file", "path": "big.txt" })).await.unwrap_err().contains("too large"));

        // Outside the working folder, in a synced folder, or through a link that leads out:
        // nobody here to allow it, so nothing is written.
        let outside = home.join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        let refused = call(&tool, json!({ "action": "write_env", "path": outside.join(".env").display().to_string(), "names": ["FROM_PAGE"] })).await.unwrap_err();
        assert!(refused.contains("outside the bot's working folder") && !outside.join(".env").exists(), "{refused}");
        assert!(call(&tool, json!({ "action": "write_env", "path": "Dropbox/.env", "names": ["FROM_PAGE"] })).await.unwrap_err().contains("cloud service syncs"));
        assert!(call(&tool, json!({ "action": "write_env", "path": "Makefile", "names": ["FROM_PAGE"] })).await.unwrap_err().contains("not a file for secrets"));
        #[cfg(unix)]
        {
            std::fs::write(outside.join("shared.env"), "A=1\n").unwrap();
            std::os::unix::fs::symlink(outside.join("shared.env"), work.join("linked.env")).unwrap();
            assert!(call(&tool, json!({ "action": "write_env", "path": "linked.env", "names": ["FROM_PAGE"] })).await.unwrap_err().contains("outside"));
            // A link inside the folder is written through, and stays a link.
            std::fs::write(work.join("real.env"), "A=1\n").unwrap();
            std::os::unix::fs::symlink(work.join("real.env"), work.join("inner.env")).unwrap();
            call(&tool, json!({ "action": "write_env", "path": "inner.env", "names": ["FROM_PAGE"] })).await.unwrap();
            assert!(std::fs::symlink_metadata(work.join("inner.env")).unwrap().file_type().is_symlink());
            assert_eq!(std::fs::read_to_string(work.join("real.env")).unwrap(), "A=1\nFROM_PAGE=a-value-the-page-showed-1\n");
        }
        // Full access asks nothing: with nobody here, a replace and a delete go through.
        let mut review = app.auto_review();
        review.full_access = true;
        app.set_auto_review(review.clone());
        std::fs::write(work.join("new-pin.txt"), "9999").unwrap();
        assert!(call(&tool, json!({ "action": "save", "name": "PIN", "from": "file", "path": "new-pin.txt" })).await.is_ok());
        assert!(call(&tool, json!({ "action": "delete", "name": "PIN" })).await.unwrap().starts_with("Deleted PIN"));
        review.full_access = false;
        app.set_auto_review(review);
        // A bot that may not write files cannot write one this way either.
        bot.capabilities.write = false;
        app.state.lock().unwrap().bots = vec![bot.clone()];
        assert!(call(&tool, json!({ "action": "write_env", "path": ".env", "names": ["FROM_PAGE"] })).await.unwrap_err().contains("not permitted"));

        // With no clipboard program, the error says so.
        let none = ["pbpaste", "wl-paste", "xclip", "xsel", "powershell"].iter().all(|program| std::process::Command::new("which").arg(program).output().map_or(true, |out| !out.status.success()));
        if none {
            assert!(call(&tool, json!({ "action": "save", "name": "CLIP", "from": "clipboard" })).await.unwrap_err().contains("clipboard cannot be read"));
        }
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn generated_secrets_use_their_whole_alphabet_only() {
        for (format, alphabet) in [("hex", "0123456789abcdef"), ("alphanumeric", "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"), ("base64url", "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_")] {
            let mut seen = std::collections::HashSet::new();
            for _ in 0..200 {
                let value = generate(64, format).unwrap();
                assert_eq!(value.len(), 64);
                assert!(value.chars().all(|c| alphabet.contains(c)), "{value}");
                seen.extend(value.chars());
            }
            assert_eq!(seen.len(), alphabet.len(), "{format} uses every character");
        }
        assert_eq!(generate(16, "hex").unwrap().len(), 16);
        assert_eq!(generate(256, "").unwrap().len(), 256);
        assert!(generate(257, "hex").is_err() && generate(15, "hex").is_err());
    }

    #[test]
    fn names_files_and_generated_values() {
        for (name, valid) in [("GOOGLE_CLIENT_SECRET", true), ("_x", true), ("a1", true), ("1A", false), ("A-B", false), ("", false), ("A B", false), (&"A".repeat(65), false)] {
            assert_eq!(valid_name(name), valid, "{name}");
        }
        let google = r#"{"web":{"client_id":"1-abc.apps.googleusercontent.com","client_secret":"GOCSPX-xyz","redirect_uris":["http://localhost"]}}"#;
        assert_eq!(value_in_file(google, Some("web.client_secret")).unwrap(), "GOCSPX-xyz");
        assert_eq!(value_in_file(google, Some("web.redirect_uris.0")).unwrap(), "http://localhost");
        assert!(value_in_file(google, Some("web.missing")).is_err());
        assert!(value_in_file(google, Some("web")).is_err());
        let env = "# comment\nexport A=\"quoted value\"\nB='single'\nC=plain\n";
        assert_eq!(value_in_file(env, Some("A")).unwrap(), "quoted value");
        assert_eq!(value_in_file(env, Some("B")).unwrap(), "single");
        assert_eq!(value_in_file(env, Some("C")).unwrap(), "plain");
        assert_eq!(value_in_file("  -----BEGIN KEY-----\nabc\n-----END KEY-----\n", None).unwrap(), "-----BEGIN KEY-----\nabc\n-----END KEY-----");

        let (text, added) = upsert_env("# app\nexport A=old\nB=keep\n", &[("A".into(), "new value".into()), ("C".into(), "c-1".into())]);
        assert_eq!((text.as_str(), added), ("# app\nexport A='new value'\nB=keep\nC=c-1\n", 1));
        assert_eq!(upsert_env("", &[("Q".into(), "a\"b\\c".into())]).0, "Q='a\"b\\c'\n", "single quotes keep quotes and backslashes literal");

        let generated = generate(32, "base64url").unwrap();
        assert_eq!(generated.len(), 32);
        assert_ne!(generated, generate(32, "base64url").unwrap());
        assert!(generate(40, "hex").unwrap().chars().all(|c| c.is_ascii_hexdigit()));
        assert!(generate(8, "hex").is_err() && generate(32, "emoji").is_err());
    }
}
