//! Bundled marketplace index plus newer versioned feed from configured relay.
//! Validated indexes persist for offline starts; checks use ETags and never send credentials.

use std::sync::Arc;
use std::time::{Duration, Instant};
use parking_lot::{Mutex, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::App;
use crate::config::{self, Config};
use crate::model::{Bot, SetupPlugin, TemplateSetup};
use crate::plugins::Manifest;

/// The bundled index: first-party plugins and bots.
const BUNDLED_INDEX: &str = include_str!("../marketplace/index.json");

const INDEX_TTL_SECS: i64 = 3600;
const MISSING_EVERY: Duration = Duration::from_secs(300);
/// After a fetch that failed or a feed that was refused, unforced checks wait this long, so a
/// relay that is down does not hold every marketplace call for the fetch timeout.
const FAILED_BACKOFF: Duration = Duration::from_secs(300);
/// A saved index larger than this is not read: a feed is at most 1 MiB.
const MAX_CACHE_BYTES: u64 = 4 * 1024 * 1024;
/// A feed dated further ahead than this would win every later comparison, so it is refused.
const MAX_FUTURE_SECS: i64 = 24 * 3600;

/// What the marketplace offers, in index order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Index {
    /// When it last changed, as `YYYY-MM-DDTHH:MM:SSZ`; of two indexes the later one wins.
    pub updated: String,
    pub plugins: Vec<Manifest>,
    pub bots: Vec<BotTemplate>,
}

impl Index {
    pub fn plugin(&self, id: &str) -> Option<&Manifest> {
        self.plugins.iter().find(|p| p.id == id)
    }

    pub fn bot(&self, id: &str) -> Option<&BotTemplate> {
        self.bots.iter().find(|b| b.id == id)
    }
    /// User-supplied entries replace bundled entries by id; absent bundled entries remain.
    fn merge(&mut self, other: Index) {
        self.updated = other.updated;
        for plugin in other.plugins {
            match self.plugins.iter_mut().find(|existing| existing.id == plugin.id) {
                Some(existing) => *existing = plugin,
                None => self.plugins.push(plugin),
            }
        }
        for bot in other.bots {
            match self.bots.iter_mut().find(|existing| existing.id == bot.id) {
                Some(existing) => *existing = bot,
                None => self.bots.push(bot),
            }
        }
    }
}

/// A bot to add from the marketplace: the profile it starts with, the plugins it works with,
/// the routines it brings, and what it knows from the start. Adding one installs no plugin;
/// its first turn asks which to install.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BotTemplate {
    /// Lowercase letters, digits, and dashes.
    pub id: String,
    pub name: String,
    /// One line for the marketplace's rows.
    #[serde(default)]
    pub summary: String,
    /// What the bot does and how it should work: the new bot's description.
    pub description: String,
    #[serde(default = "default_symbol")]
    pub symbol_name: String,
    #[serde(default = "default_accent")]
    pub accent: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
    /// Listed under Featured Bots, in index order.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub featured: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    /// Marketplace plugin ids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<String>,
    /// Added paused with the bot; it asks whether to turn them on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routines: Vec<RoutineTemplate>,
    /// Facts the bot saves to its memory on its first turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutineTemplate {
    pub name: String,
    /// Anything `crate::schedule::parse` reads: `every 2h`, `0 9 * * 1-5`.
    pub schedule: String,
    pub prompt: String,
}

fn default_symbol() -> String {
    "sparkles".into()
}

fn default_accent() -> String {
    "indigo".into()
}

impl BotTemplate {
    /// Reads a template, refusing one that could not be added.
    pub fn parse(value: &Value) -> Result<BotTemplate, String> {
        let template: BotTemplate = serde_json::from_value(value.clone()).map_err(|e| format!("Not a bot template: {e}"))?;
        if !crate::plugins::is_id(&template.id) {
            return Err(format!("A bot template id is lowercase letters, digits, and dashes; {:?} is not.", template.id));
        }
        if template.name.trim().is_empty() || template.description.trim().is_empty() {
            return Err(format!("The bot template {} needs a name and a description.", template.id));
        }
        for routine in &template.routines {
            if routine.name.trim().is_empty() || routine.prompt.trim().is_empty() {
                return Err(format!("A routine of {} needs a name and a prompt.", template.name));
            }
            crate::schedule::parse(&routine.schedule).map_err(|e| format!("{} · {}: {e}", template.name, routine.name))?;
        }
        Ok(template)
    }
}

// MARK: - The index

/// Current index, checking configured feed when its hourly TTL expires.
pub async fn index(app: &Arc<App>) -> Index {
    refresh_selected(app);
    if source(app).is_some() {
        if let Err(error) = check(app, false).await {
            tracing::warn!(%error, "checking marketplace index");
        }
    }
    current(app)
}

/// Read without network; used at startup for installed marketplace plugins.
pub fn current(app: &App) -> Index {
    app.marketplace.current.read().clone()
}

/// Index state belonging to one App. Cache source must match current feed before reuse.
pub struct Updates {
    current: RwLock<Index>,
    source: Mutex<Option<String>>,
    checking: tokio::sync::Mutex<()>,
    missing_checked: Mutex<Option<Instant>>,
    refresh_selected: Mutex<bool>,
    /// The last check of a feed URL and how long unforced checks of it wait: the TTL after a
    /// check that completed, `FAILED_BACKOFF` after one that failed or was refused.
    attempted: Mutex<Option<(String, Instant, Duration)>>,
}

impl Updates {
    pub fn load(_config: &Config) -> Self {
        Self { current: RwLock::new(bundled()), source: Mutex::new(None), checking: tokio::sync::Mutex::new(()), missing_checked: Mutex::new(None), refresh_selected: Mutex::new(false), attempted: Mutex::new(None) }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    source: String,
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    checked_at: i64,
    index: Value,
}

/// The saved index, unless it is missing, unreadable, or larger than any feed could make it.
fn read_cache(path: &std::path::Path) -> Option<Cache> {
    if std::fs::metadata(path).ok()?.len() > MAX_CACHE_BYTES {
        return None;
    }
    config::read_json(path)
}

/// `YYYY-MM-DDTHH:MM:SSZ` for a Unix time, the form feeds date themselves in.
fn utc_text(unix: i64) -> String {
    let (days, secs) = (unix.div_euclid(86_400), unix.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs % 3600 / 60, secs % 60)
}

fn override_url(app: &App) -> Option<String> {
    std::env::var("BEANS_MARKETPLACE_URL").ok().filter(|url| !url.trim().is_empty())
        .or_else(|| app.settings.lock().unwrap().marketplace_url.clone().filter(|url| !url.trim().is_empty()))
}

fn source(app: &App) -> Option<String> {
    if std::env::var("BEANS_MARKETPLACE_FETCH").is_ok_and(|value| value.trim() == "0") {
        return None;
    }
    override_url(app).or_else(|| app.relay_url().map(|relay| format!("{}/marketplace/v1.json", relay.trim_end_matches('/'))))
}

/// Select configured source; a different relay starts with bundled index and its own validator.
pub fn enable(app: &App) {
    let selected = source(app);
    let mut source = app.marketplace.source.lock();
    if *source == selected { return; }
    *source = selected.clone();
    *app.marketplace.missing_checked.lock() = None;
    let mut index = bundled();
    if let Some(selected) = selected {
        if let Some(cache) = read_cache(&app.config.marketplace_path()) {
            if cache.source == selected {
                if let Ok(cached) = parse(&cache.index.to_string()) {
                    if cached.updated > index.updated { index = effective_index(cached, override_url(app).is_some()); }
                }
            }
        }
    }
    *app.marketplace.current.write() = index;
    *app.marketplace.refresh_selected.lock() = true;
}

fn refresh_selected(app: &Arc<App>) {
    let mut pending = app.marketplace.refresh_selected.lock();
    if *pending {
        let plugins = app.marketplace.current.read().plugins.clone();
        crate::plugins::refresh_installed(app, &plugins);
        *pending = false;
    }
}

pub fn check_in_background(app: &Arc<App>) {
    enable(app);
    refresh_selected(app);
    if app.marketplace.source.lock().is_none() { return; }
    let app = app.clone();
    tokio::spawn(async move {
        if let Err(error) = check(&app, false).await { tracing::warn!(%error, "checking marketplace index"); }
    });
}

pub async fn check_for_missing(app: &Arc<App>) -> bool {
    if source(app).is_none() { return false; }
    {
        let mut last = app.marketplace.missing_checked.lock();
        if last.is_some_and(|at| at.elapsed() < MISSING_EVERY) { return false; }
        *last = Some(Instant::now());
    }
    check(app, true).await.unwrap_or_else(|error| {
        tracing::warn!(%error, "checking marketplace index for missing entry");
        false
    })
}

/// Force bypasses hourly TTL. ETag and cache never cross feed URLs.
pub async fn check(app: &Arc<App>, force: bool) -> Result<bool, String> {
    let _guard = app.marketplace.checking.lock().await;
    enable(app);
    refresh_selected(app);
    let Some(url) = source(app) else { return Err("Marketplace updates are off".into()) };
    let parsed = reqwest::Url::parse(&url).map_err(|e| format!("Invalid marketplace URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Marketplace URL must be HTTP(S) without embedded credentials".into());
    }
    let path = app.config.marketplace_path();
    let mut cache: Cache = read_cache(&path)
        .filter(|cache: &Cache| cache.source == url && parse(&cache.index.to_string()).is_ok())
        .unwrap_or_default();
    let now = config::now_unix();
    if !force && (0..INDEX_TTL_SECS).contains(&(now - cache.checked_at)) { return Ok(false); }
    if !force && app.marketplace.attempted.lock().as_ref().is_some_and(|(at_url, at, wait)| *at_url == url && at.elapsed() < *wait) {
        return Ok(false);
    }
    let attempted = |wait: Duration| *app.marketplace.attempted.lock() = Some((url.clone(), Instant::now(), wait));
    // A public fetch uses an isolated client, never account or provider credentials.
    let fetched = match crate::served::fetch(&url, cache.etag.as_deref()).await {
        Ok(fetched) => fetched,
        Err(error) => {
            attempted(FAILED_BACKOFF);
            return Err(error);
        }
    };
    if source(app).as_deref() != Some(url.as_str()) {
        enable(app);
        return Err("Marketplace source changed while checking".into());
    }
    let changed = if let Some((etag, text)) = fetched {
        let index = match parse(&text).and_then(|index| {
            if index.updated > utc_text(now + MAX_FUTURE_SECS) { Err(format!("Marketplace index is dated in the future ({})", index.updated)) } else { Ok(index) }
        }) {
            Ok(index) => index,
            Err(error) => {
                attempted(FAILED_BACKOFF);
                return Err(error);
            }
        };
        let keep = index.updated >= current(app).updated;
        let changed = install_index(app, index);
        if keep {
            cache.index = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            cache.etag = etag;
        }
        changed
    } else {
        if cache.index.is_null() {
            attempted(FAILED_BACKOFF);
            return Err("Marketplace returned 304 without a cached index".into());
        }
        false
    };
    // Checked, even when the feed was older than what is here and nothing was kept.
    attempted(Duration::from_secs(INDEX_TTL_SECS as u64));
    cache.source = url;
    if !cache.index.is_null() { cache.checked_at = now; }
    if let Err(error) = config::write_json_private(&path, &cache) {
        tracing::warn!(%error, "saving marketplace index");
    }
    Ok(changed)
}

fn effective_index(index: Index, overlay: bool) -> Index {
    if !overlay { return index; }
    let mut combined = bundled();
    combined.merge(index);
    // A custom feed never changes pinned Browser, even if it lists that id.
    if let Some(browser) = bundled().plugin("playwright") {
        if let Some(entry) = combined.plugins.iter_mut().find(|plugin| plugin.id == "playwright") {
            *entry = browser.clone();
        }
    }
    combined
}

fn install_index(app: &Arc<App>, index: Index) -> bool {
    let index = effective_index(index, override_url(app).is_some());
    let plugins = index.plugins.clone();
    {
        let mut current = app.marketplace.current.write();
        // A feed that changed without changing `updated` still replaces what is here, so memory
        // never disagrees with the saved copy.
        if index.updated < current.updated || *current == index { return false; }
        *current = index;
    }
    crate::plugins::refresh_installed(app, &plugins);
    true
}

pub fn bundled() -> Index {
    parse(BUNDLED_INDEX).expect("bundled marketplace index must be valid")
}

#[derive(Deserialize)]
struct RawIndex {
    version: u64,
    updated: String,
    #[serde(default)]
    plugins: Vec<Value>,
    #[serde(default)]
    bots: Vec<Value>,
}

fn parse(text: &str) -> Result<Index, String> {
    let raw: RawIndex = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if raw.version != 1 { return Err(format!("Unsupported marketplace index version {}", raw.version)); }
    if !valid_utc_time(&raw.updated) { return Err(format!("Invalid marketplace updated time {:?}", raw.updated)); }
    let mut index = Index { updated: raw.updated, ..Index::default() };
    for entry in &raw.plugins {
        match Manifest::parse(entry) {
            Ok(manifest) if index.plugin(&manifest.id).is_none() => index.plugins.push(manifest),
            Ok(_) => tracing::warn!("skipping duplicate marketplace plugin"),
            Err(error) => tracing::warn!(%error, "skipping marketplace plugin"),
        }
    }
    for entry in &raw.bots {
        match BotTemplate::parse(entry) {
            Ok(bot) if index.bot(&bot.id).is_none() => index.bots.push(bot),
            Ok(_) => tracing::warn!("skipping duplicate marketplace bot"),
            Err(error) => tracing::warn!(%error, "skipping marketplace bot"),
        }
    }
    if index.plugins.is_empty() { return Err("Marketplace index has no valid plugins".into()); }
    // Pinned Chromium and MCP must move together; do not accept a remote Browser replacement.
    if text != BUNDLED_INDEX {
        let browser = bundled().plugin("playwright").cloned();
        if let Some(browser) = browser {
            match index.plugins.iter_mut().find(|plugin| plugin.id == "playwright") {
                Some(remote) => *remote = browser,
                None => index.plugins.push(browser),
            }
        }
    }
    Ok(index)
}

fn valid_utc_time(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 20 || !bytes.iter().zip(b"dddd-dd-ddTdd:dd:ddZ").all(|(c, pattern)| if *pattern == b'd' { c.is_ascii_digit() } else { c == pattern }) { return false; }
    let year = text[0..4].parse::<u32>().unwrap_or(0);
    let month = text[5..7].parse::<u32>().unwrap_or(0);
    let day = text[8..10].parse::<u32>().unwrap_or(0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    };
    year > 0 && day > 0 && day <= days && text[11..13].parse::<u32>().unwrap_or(99) < 24
        && text[14..16].parse::<u32>().unwrap_or(99) < 60 && text[17..19].parse::<u32>().unwrap_or(99) < 60
}

/// Plugins matching `query`: every word appears in the id, name, description, category, or
/// tags. All of them for an empty query.
pub fn search_plugins<'a>(plugins: &'a [Manifest], query: &str) -> Vec<&'a Manifest> {
    let words = words(query);
    plugins.iter().filter(|p| matches(&[&p.id, &p.name, &p.description, &p.category, &p.author, &p.tags.join(" ")], &words)).collect()
}

/// Bots matching `query` the same way, over the id, name, summary, description, and category.
pub fn search_bots<'a>(bots: &'a [BotTemplate], query: &str) -> Vec<&'a BotTemplate> {
    let words = words(query);
    bots.iter().filter(|b| matches(&[&b.id, &b.name, &b.summary, &b.description, &b.category, &b.author], &words)).collect()
}

fn words(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

fn matches(fields: &[&str], words: &[String]) -> bool {
    let haystack = fields.join(" ").to_lowercase();
    words.iter().all(|w| haystack.contains(w))
}

// MARK: - Adding a bot

/// The template `id`, and the plugins it names as the index describes them.
pub async fn template(app: &Arc<App>, id: &str) -> Result<(BotTemplate, Vec<SetupPlugin>), String> {
    let index = index(app).await;
    let template = index.bot(id).cloned().ok_or_else(|| format!("No bot {id} in the marketplace"))?;
    let plugins = template
        .plugins
        .iter()
        .filter_map(|id| index.plugin(id))
        .map(|p| SetupPlugin { id: p.id.clone(), name: p.name.clone(), description: p.description.clone() })
        .collect();
    Ok((template, plugins))
}

/// A bot just added from `template`, with its direct chat: its routines, added paused, and a
/// first turn in which it greets the user and sets itself up, as Grok Bot's template import
/// does. `greeting` is the user's first message, worded by the app in the user's language.
pub fn welcome(app: &Arc<App>, bot: &Bot, chat_id: &str, template: &BotTemplate, plugins: Vec<SetupPlugin>, greeting: Option<String>, admission: crate::update_control::Admission) -> anyhow::Result<()> {
    let mut routines = Vec::new();
    for routine in &template.routines {
        match crate::routines::create(app, &bot.id, &routine.name, &routine.schedule, &routine.prompt, None, false) {
            Ok(created) => routines.push(created.name),
            Err(error) => tracing::warn!(%error, routine = %routine.name, "adding a template's routine"),
        }
    }
    let setup = TemplateSetup { template: template.name.clone(), plugins, routines, memory: template.memory.clone() };
    let greeting = greeting.filter(|g| !g.trim().is_empty()).unwrap_or_else(|| format!("Hi {}, introduce yourself.", bot.name));
    crate::runtime::greet_new_bot(app, chat_id, &bot.id, &greeting, setup, admission)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A feed server answering each request with the next response (the last one repeats),
    /// counting requests and keeping their heads.
    async fn feed(responses: Vec<String>) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/marketplace/v1.json", listener.local_addr().unwrap());
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let requests = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut head = Vec::new();
                let mut chunk = [0u8; 2048];
                while !head.windows(4).any(|part| part == b"\r\n\r\n") {
                    match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(count) => head.extend_from_slice(&chunk[..count]),
                    }
                }
                let index = {
                    let mut requests = requests.lock().unwrap();
                    requests.push(String::from_utf8_lossy(&head).to_ascii_lowercase());
                    requests.len() - 1
                };
                let response = responses.get(index).or(responses.last()).unwrap().clone();
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        (url, seen)
    }

    fn ok(body: &str, etag: &str) -> String {
        format!("HTTP/1.1 200 OK\r\nETag: {etag}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
    }

    fn status(line: &str) -> String {
        format!("HTTP/1.1 {line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    }

    /// The bundled index dated `updated`, with its first plugin's description changed to `note`.
    fn index_dated(updated: &str, note: &str) -> String {
        let mut index: Value = serde_json::from_str(BUNDLED_INDEX).unwrap();
        index["updated"] = updated.into();
        index["plugins"][0]["description"] = note.into();
        index.to_string()
    }

    struct Scratch(Arc<App>, std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    fn runner_with_feed(url: &str) -> Scratch {
        let home = std::env::temp_dir().join(format!("beans-marketplace-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&home).unwrap();
        let app = App::load(Config { home: home.clone(), port: 0 }).unwrap();
        app.settings.lock().unwrap().marketplace_url = Some(url.into());
        enable(&app);
        Scratch(app, home)
    }

    #[tokio::test]
    async fn a_failing_feed_is_not_fetched_again_on_every_call() {
        let (url, seen) = feed(vec![status("500 Internal Server Error")]).await;
        let scratch = runner_with_feed(&url);
        assert!(check(&scratch.0, false).await.unwrap_err().contains("500"));
        for _ in 0..3 {
            assert!(!check(&scratch.0, false).await.unwrap(), "backs off");
        }
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert!(check(&scratch.0, true).await.is_err(), "a forced check still asks");
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn an_older_or_broken_feed_counts_as_checked() {
        let (url, seen) = feed(vec![ok(&index_dated("2000-01-01T00:00:00Z", "old"), "\"old\"")]).await;
        let scratch = runner_with_feed(&url);
        assert!(!check(&scratch.0, false).await.unwrap());
        assert!(!check(&scratch.0, false).await.unwrap());
        assert_eq!(seen.lock().unwrap().len(), 1, "an older feed is not fetched again within the hour");
        assert_eq!(current(&scratch.0).updated, bundled().updated);

        let (url, seen) = feed(vec![ok("{not json", "\"x\"")]).await;
        let scratch = runner_with_feed(&url);
        assert!(check(&scratch.0, false).await.is_err());
        assert!(!check(&scratch.0, false).await.unwrap());
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_newer_feed_installs_and_revalidates_with_its_etag() {
        let now = config::now_unix();
        let newer = index_dated(&utc_text(now - 60), "newer");
        let (url, seen) = feed(vec![ok(&newer, "\"v1\""), status("304 Not Modified")]).await;
        let scratch = runner_with_feed(&url);
        assert!(check(&scratch.0, false).await.unwrap());
        assert_eq!(current(&scratch.0).updated, utc_text(now - 60));
        assert!(!check(&scratch.0, false).await.unwrap(), "fresh within the hour");
        assert!(!check(&scratch.0, true).await.unwrap(), "304: nothing new");
        let requests = seen.lock().unwrap().clone();
        assert_eq!(requests.len(), 2);
        assert!(!requests[0].contains("if-none-match") && requests[1].contains("if-none-match: \"v1\""));
        let cache: Cache = read_cache(&scratch.0.config.marketplace_path()).unwrap();
        assert_eq!((cache.source.as_str(), cache.etag.as_deref()), (url.as_str(), Some("\"v1\"")));
    }

    #[tokio::test]
    async fn a_feed_dated_in_the_future_is_refused() {
        let (url, _) = feed(vec![ok(&index_dated("9999-12-31T23:59:59Z", "frozen"), "\"f\"")]).await;
        let scratch = runner_with_feed(&url);
        assert!(check(&scratch.0, true).await.unwrap_err().contains("future"));
        assert_eq!(current(&scratch.0).updated, bundled().updated);
    }

    #[tokio::test]
    async fn a_changed_feed_with_the_same_date_replaces_what_is_in_memory() {
        let updated = utc_text(config::now_unix() - 60);
        let (url, _) = feed(vec![ok(&index_dated(&updated, "first"), "\"a\""), ok(&index_dated(&updated, "second"), "\"b\"")]).await;
        let scratch = runner_with_feed(&url);
        assert!(check(&scratch.0, true).await.unwrap());
        assert!(check(&scratch.0, true).await.unwrap());
        let first = bundled().plugins[0].id.clone();
        assert_eq!(current(&scratch.0).plugin(&first).unwrap().description, "second");
        let cache: Cache = read_cache(&scratch.0.config.marketplace_path()).unwrap();
        assert_eq!(cache.index["plugins"][0]["description"], "second", "memory and disk agree");
    }

    #[test]
    fn an_oversized_saved_index_is_not_read() {
        let home = std::env::temp_dir().join(format!("beans-marketplace-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&home).unwrap();
        let app = App::load(Config { home: home.clone(), port: 0 }).unwrap();
        let path = app.config.marketplace_path();
        std::fs::write(&path, vec![b' '; MAX_CACHE_BYTES as usize + 1]).unwrap();
        assert!(read_cache(&path).is_none());
        app.settings.lock().unwrap().marketplace_url = Some("http://127.0.0.1:9/marketplace/v1.json".into());
        enable(&app);
        assert_eq!(current(&app).updated, bundled().updated);
        drop(app);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn utc_text_formats_unix_times() {
        assert_eq!(utc_text(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_text(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(utc_text(1_763_812_345), "2025-11-22T11:52:25Z");
        assert!(valid_utc_time(&utc_text(config::now_unix())));
    }

    #[test]
    fn the_bundled_index_parses() {
        let index = bundled();
        let text: Value = serde_json::from_str(BUNDLED_INDEX).unwrap();
        assert_eq!(index.plugins.len(), text["plugins"].as_array().unwrap().len(), "every bundled plugin reads");
        assert_eq!(index.bots.len(), text["bots"].as_array().unwrap().len(), "every bundled bot reads");
        let mut ids = std::collections::HashSet::new();
        for plugin in &index.plugins {
            assert!(ids.insert(plugin.id.as_str()), "{} is listed twice", plugin.id);
            assert!(!plugin.icon.is_empty() && !plugin.description.is_empty() && !plugin.category.is_empty(), "{} is incomplete", plugin.id);
            assert!(!plugin.author.is_empty() && plugin.homepage.as_deref().is_some_and(|h| h.starts_with("https://")), "{} needs its maker and website", plugin.id);
            for (name, server) in &plugin.servers {
                if let crate::plugins::ServerSpec::Http { url, .. } = server {
                    assert!(url.starts_with("https://"), "{}'s server {name} is not HTTPS", plugin.id);
                }
            }
        }
        for bot in &index.bots {
            assert!(!bot.summary.is_empty() && !bot.category.is_empty(), "{} is incomplete", bot.id);
            for id in &bot.plugins {
                assert!(index.plugin(id).is_some(), "{} names the unknown plugin {id}", bot.id);
            }
        }
        assert!(index.plugins.iter().any(|p| p.featured) && index.bots.iter().any(|b| b.featured));
        assert_eq!(search_plugins(&index.plugins, "PULL requests").iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["github"]);
        assert_eq!(search_plugins(&index.plugins, "").len(), index.plugins.len());
        assert!(search_plugins(&index.plugins, "nothing-like-this").is_empty());
        assert!(search_bots(&index.bots, "pull requests").iter().any(|b| b.id == "pr-reviewer"));
        assert_eq!(search_bots(&index.bots, "").len(), index.bots.len());
    }

    #[test]
    fn versioned_index_skips_bad_entries_without_losing_valid_plugins() {
        let text = r#"{ "version": 1, "updated": "2026-10-03T00:00:00Z", "plugins": [
            { "id": "new", "name": "New", "servers": { "s": { "type": "http", "url": "https://new.test/mcp" } } },
            { "id": "Bad Id", "name": "Invalid" },
            { "id": "new", "name": "Duplicate", "servers": { "s": { "type": "http", "url": "https://new.test/mcp" } } }
        ], "bots": [ { "id": "good", "name": "Good", "description": "Works" }, { "id": "Bad Id" } ] }"#;
        let parsed = parse(text).unwrap();
        assert_eq!(parsed.plugins.iter().map(|plugin| plugin.name.as_str()).collect::<Vec<_>>(), ["New", "Browser"]);
        assert_eq!(parsed.bots.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), ["good"]);
        for updated in ["2026-02-30T00:00:00Z", "2026-13-01T00:00:00Z", "2026-10-03T25:00:00Z"] {
            assert!(parse(&text.replace("2026-10-03T00:00:00Z", updated)).is_err());
        }
        assert!(parse(&text.replace("\"version\": 1", "\"version\": 2")).is_err());
        assert!(parse(&text.replace("\"version\": 1, ", "")).is_err());
        let mut remote: Value = serde_json::from_str(BUNDLED_INDEX).unwrap();
        remote["plugins"].as_array_mut().unwrap().iter_mut().find(|plugin| plugin["id"] == "playwright").unwrap()["servers"]["browser"]["args"] = serde_json::json!(["-y", "@playwright/mcp@latest"]);
        let remote = parse(&remote.to_string()).unwrap();
        assert_eq!(remote.plugin("playwright").unwrap(), bundled().plugin("playwright").unwrap());
    }

    #[test]
    fn cache_is_bound_to_feed_and_only_newer_indexes_win() {
        let home = std::env::temp_dir().join(format!("beans-marketplace-cache-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&home).unwrap();
        let config = Config { home: home.clone(), port: 0 };
        config.ensure_home().unwrap();
        let mut cached: Value = serde_json::from_str(BUNDLED_INDEX).unwrap();
        cached["updated"] = Value::from("2999-01-01T00:00:00Z");
        cached["plugins"].as_array_mut().unwrap().iter_mut().find(|plugin| plugin["id"] == "github").unwrap()["description"] = Value::from("Selected feed GitHub");
        let newer = Cache { source: "https://one.test/marketplace/v1.json".into(), etag: Some("\"new\"".into()), checked_at: 1, index: cached };
        config::write_json_private(&home.join("marketplace.json"), &newer).unwrap();
        let app = App::load(config).unwrap();
        crate::plugins::install(&app, bundled().plugin("github").unwrap().clone(), "marketplace").unwrap();
        assert_eq!(current(&app).updated, bundled().updated);
        assert!(bundled().plugin("playwright").unwrap().servers.values().any(|server| matches!(server, crate::plugins::ServerSpec::Stdio { args, .. } if args.iter().any(|arg| arg == "@playwright/mcp@0.0.83"))));
        app.settings.lock().unwrap().marketplace_url = Some(newer.source.clone());
        enable(&app);
        assert_eq!(current(&app).updated, "2999-01-01T00:00:00Z");
        refresh_selected(&app);
        assert_eq!(app.plugins.lock().unwrap().get("github").unwrap().manifest.description, "Selected feed GitHub");
        app.settings.lock().unwrap().marketplace_url = Some("https://two.test/marketplace/v1.json".into());
        enable(&app);
        assert_eq!(current(&app).updated, bundled().updated);
        refresh_selected(&app);
        assert_eq!(app.plugins.lock().unwrap().get("github").unwrap().manifest.description, bundled().plugin("github").unwrap().description);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn newer_index_replaces_old_entries_and_older_one_cannot_downgrade() {
        let home = std::env::temp_dir().join(format!("beans-marketplace-version-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&home).unwrap();
        let app = App::load(Config { home: home.clone(), port: 0 }).unwrap();
        let mut newer: Value = serde_json::from_str(BUNDLED_INDEX).unwrap();
        newer["updated"] = Value::from("2999-01-01T00:00:00Z");
        newer["plugins"] = serde_json::json!([{
            "id": "new", "name": "New", "servers": { "s": { "type": "http", "url": "https://new.test/mcp" } }
        }]);
        let changed = parse(&newer.to_string()).unwrap();
        assert!(install_index(&app, changed));
        assert!(current(&app).plugin("new").is_some());
        assert!(current(&app).plugin("github").is_none(), "new index replaces prior entries");
        assert!(!install_index(&app, bundled()));
        assert!(current(&app).plugin("new").is_some(), "older bundled index cannot downgrade");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn explicit_feed_merges_bundled_entries_and_replaces_matching_ids() {
        let mut remote: Value = serde_json::from_str(BUNDLED_INDEX).unwrap();
        remote["updated"] = Value::from("2999-01-01T00:00:00Z");
        let mut github = remote["plugins"].as_array().unwrap().iter().find(|entry| entry["id"] == "github").unwrap().clone();
        github["description"] = Value::from("Custom GitHub");
        remote["plugins"] = serde_json::json!([github, { "id": "new", "name": "New", "servers": { "s": { "type": "http", "url": "https://new.test/mcp" } } }]);
        remote["bots"] = serde_json::json!([]);
        let parsed = parse(&remote.to_string()).unwrap();
        let overlaid = effective_index(parsed.clone(), true);
        assert_eq!(overlaid.plugin("github").unwrap().description, "Custom GitHub");
        assert!(overlaid.plugin("new").is_some());
        assert!(overlaid.plugin("playwright").is_some());
        assert!(overlaid.bot("pr-reviewer").is_some());
        assert!(effective_index(parsed, false).plugin("github").is_some());
        assert!(effective_index(parse(&remote.to_string()).unwrap(), false).plugin("playwright").is_some(), "Playwright stays pinned even in relay feed");
    }

    #[tokio::test]
    async fn a_bot_added_from_a_template_starts_with_paused_routines_and_a_greeting() {
        let home = std::env::temp_dir().join(format!("beans-marketplace-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&home).unwrap();
        let app = App::load_owner(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        crate::identity::create(&app, Some("Workbench".into())).unwrap();
        let runner_id = app.this_device_id().unwrap();
        let params = serde_json::json!({ "template_id": "pr-reviewer", "runner_id": runner_id, "greeting": "Hi PR Reviewer, introduce yourself." });
        let out = crate::api::dispatch(&app, "bots.create", params).await.unwrap();
        let bot = app.bot(out["bot"]["id"].as_str().unwrap()).unwrap();
        assert_eq!((bot.name.as_str(), bot.symbol_name.as_str(), bot.accent.as_str()), ("PR Reviewer", "checklist", "blue"));
        assert!(bot.description.starts_with("You review pull requests"));
        let routines = app.routines_of(&bot.id);
        assert_eq!(routines.iter().map(|r| (r.name.as_str(), r.is_enabled)).collect::<Vec<_>>(), [("Review new pull requests", false)]);
        let chat_id = out["chat_id"].as_str().unwrap();
        let (messages, _) = app.store.page(chat_id, None, 10).unwrap();
        let greeted = messages.iter().any(|m| {
            m.author == crate::model::Author::You && matches!(&m.body, crate::model::Body::Text { text, .. } if text == "Hi PR Reviewer, introduce yourself.")
        });
        assert!(greeted, "the user's greeting opens the chat");
        let unknown = serde_json::json!({ "template_id": "nope", "runner_id": runner_id });
        assert!(crate::api::dispatch(&app, "bots.create", unknown).await.unwrap_err().contains("No bot nope"));
        let _ = std::fs::remove_dir_all(&home);
    }
}
