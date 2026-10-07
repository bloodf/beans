//! A model catalog served by the selected relay, with the bundled catalog as offline fallback.

use std::sync::Arc;
use std::time::{Duration, Instant};
use parking_lot::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::App;
use crate::config::{self, Config};

const FRESH_SECS: i64 = 60 * 60;
const UNKNOWN_MODEL_EVERY: Duration = Duration::from_secs(5 * 60);

/// Per-Device catalog checks. No network until `enable` has a selected relay or explicit URL.
#[derive(Default)]
pub struct Updates {
    source: Mutex<Option<String>>,
    checking: tokio::sync::Mutex<()>,
    unknown_checked: Mutex<Option<Instant>>,
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    source: String,
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    checked_at: i64,
    catalog: Value,
}

fn source(app: &App) -> Option<String> {
    std::env::var("BEANS_MODELS_URL").ok().map(|url| url.trim().to_string()).filter(|url| !url.is_empty())
        .or_else(|| app.relay_url().map(|relay| format!("{}/models/v1.json", relay.trim_end_matches('/'))))
}

/// Select source after App construction. A relay switch drops its previous catalog and ETag.
pub fn enable(app: &App) {
    let selected = source(app);
    let mut current = app.catalog.source.lock();
    if *current == selected { return; }
    beans_models::reset();
    *app.catalog.unknown_checked.lock() = None;
    *current = selected;
    if let Some(url) = current.as_deref() { load_cached_from(&app.config, url); }
}

/// Load only a cache belonging to App's currently selected source. Call after `enable`.
pub fn load_cached(app: &App) {
    if let Some(url) = app.catalog.source.lock().as_deref() {
        load_cached_from(&app.config, url);
    }
}

fn read_cache(config: &Config) -> Option<Cache> {
    use std::io::Read;
    let file = std::fs::File::open(config.catalog_path()).ok()?;
    let mut bytes = Vec::new();
    file.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 2 * 1024 * 1024 { return None; }
    serde_json::from_slice(&bytes).ok()
}

fn load_cached_from(config: &Config, url: &str) {
    let Some(cache) = read_cache(config).filter(|cache| cache.source == url) else { return };
    let Ok(text) = serde_json::to_string(&cache.catalog) else { return };
    match beans_models::parse(&text) {
        Ok(catalog) => { beans_models::install(catalog); }
        Err(error) => tracing::warn!(%error, "reading cached model catalog"),
    }
}

/// Check once when startup/bootstrap calls this; later calls respect the one-hour freshness.
pub fn check_in_background(app: &Arc<App>) {
    enable(app);
    if fetch_disabled() || app.catalog.source.lock().is_none() { return; }
    let app = Arc::clone(app);
    tokio::spawn(async move {
        if let Err(error) = check(&app, false).await {
            tracing::warn!(%error, "checking model catalog");
        }
    });
}

/// Unknown built-in models can be learned before a turn, at most every five minutes.
pub async fn check_for_model(app: &Arc<App>, provider: &str, model: Option<&str>) {
    let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) else { return };
    enable(app);
    if fetch_disabled() || app.catalog.source.lock().is_none()
        || beans_models::default_model(provider).is_none()
        || beans_models::find(provider, model).is_some() { return; }
    {
        let mut last = app.catalog.unknown_checked.lock();
        if last.is_some_and(|at| at.elapsed() < UNKNOWN_MODEL_EVERY) { return; }
        *last = Some(Instant::now());
    }
    if let Err(error) = check(app, true).await {
        tracing::warn!(%error, provider, model, "checking catalog for unknown model");
    }
}

fn fetch_disabled() -> bool {
    std::env::var("BEANS_MODELS_FETCH").is_ok_and(|value| value.trim() == "0")
}

/// Forced checks bypass freshness, not origin or version validation. True when installed.
pub async fn check(app: &Arc<App>, force: bool) -> Result<bool, String> {
    let _checking = app.catalog.checking.lock().await;
    enable(app);
    if fetch_disabled() { return Err("Model catalog updates are off".into()); }
    let url = app.catalog.source.lock().clone().ok_or("Model catalog updates are off")?;
    let path = app.config.catalog_path();
    let mut cache = read_cache(&app.config).filter(|cache| cache.source == url).unwrap_or_else(|| Cache {
        source: url.clone(), ..Cache::default()
    });
    // An invalid cache cannot support a 304 or suppress a fresh check.
    let valid = serde_json::to_string(&cache.catalog).ok()
        .and_then(|text| beans_models::parse(&text).ok()).is_some();
    if !valid { cache.etag = None; cache.checked_at = 0; cache.catalog = Value::Null; }
    let now = config::now_unix();
    if !force && (0..FRESH_SECS).contains(&(now - cache.checked_at)) { return Ok(false); }
    let fetched = crate::served::fetch(&url, cache.etag.as_deref().filter(|_| valid)).await;
    // `enable` can change the selected relay during a request. Never install its old response.
    let current = app.catalog.source.lock();
    if current.as_deref() != Some(url.as_str()) || source(app).as_deref() != Some(url.as_str()) {
        return Ok(false);
    }
    cache.checked_at = now;
    let installed = match fetched {
        Ok(Some((etag, text))) => {
            let catalog = beans_models::parse(&text)?;
            let incoming = serde_json::from_str(&text).map_err(|error| error.to_string())?;
            let installed = beans_models::install(catalog);
            // A downgrade cannot replace persisted content or its validator. The next
            // request must not revalidate an older response as though it were current.
            if installed || (valid && incoming_newer_than_cache(&incoming, &cache.catalog)) {
                cache.catalog = incoming;
                cache.etag = etag;
            }
            installed
        }
        Ok(None) => false,
        Err(error) => return Err(error),
    };
    if let Err(error) = config::write_json_private(&path, &cache) {
        tracing::warn!(%error, "saving model catalog");
    }
    if installed { app.emit(app.roster_summary()); }
    Ok(installed)
}

fn incoming_newer_than_cache(incoming: &Value, cached: &Value) -> bool {
    incoming["updated"].as_str() > cached["updated"].as_str()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_rejects_foreign_origin_and_invalid_content() {
        let home = std::env::temp_dir().join(format!("beans-catalog-{}", uuid::Uuid::new_v4()));
        let config = Config { home: home.clone(), port: 0 };
        let mut later: Value = serde_json::from_str(beans_models::BUNDLED).unwrap();
        later["updated"] = Value::from("2999-01-01T00:00:00Z");
        let cache = Cache { source: "https://other.example/models/v1.json".into(), catalog: later, ..Cache::default() };
        config::write_json_private(&config.catalog_path(), &cache).unwrap();
        load_cached_from(&config, "https://selected.example/models/v1.json");
        assert_ne!(beans_models::updated(), "2999-01-01T00:00:00Z");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn older_response_never_replaces_cached_payload() {
        let newer = serde_json::json!({"updated": "2027-01-01T00:00:00Z"});
        let older = serde_json::json!({"updated": "2026-01-01T00:00:00Z"});
        assert!(!incoming_newer_than_cache(&older, &newer));
        assert!(incoming_newer_than_cache(&newer, &older));
        assert!(!incoming_newer_than_cache(&newer, &newer));
    }

    #[tokio::test]
    async fn fetches_persists_and_revalidates_from_selected_relay() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mut later: Value = serde_json::from_str(beans_models::BUNDLED).unwrap();
        later["updated"] = Value::from("2999-01-01T00:00:00Z");
        let body = later.to_string();
        let server = std::thread::spawn(move || {
            for expected_conditional in [false, true] {
                let (mut socket, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut chunk = [0; 2048];
                while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                    let count = socket.read(&mut chunk).unwrap();
                    if count == 0 { break; }
                    request.extend_from_slice(&chunk[..count]);
                }
                let text = String::from_utf8(request).unwrap().to_ascii_lowercase();
                assert_eq!(text.contains("if-none-match: \"new\""), expected_conditional);
                if expected_conditional {
                    socket.write_all(b"HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                } else {
                    let header = format!("HTTP/1.1 200 OK\r\nETag: \"new\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    socket.write_all(header.as_bytes()).unwrap();
                    socket.write_all(body.as_bytes()).unwrap();
                }
            }
        });
        let home = std::env::temp_dir().join(format!("beans-catalog-{}", uuid::Uuid::new_v4()));
        let app = App::load(Config { home: home.clone(), port: 0 }).unwrap();
        app.settings.lock().unwrap().relay_url = Some(base.clone());
        enable(&app);
        assert!(check(&app, false).await.unwrap());
        assert_eq!(beans_models::updated(), "2999-01-01T00:00:00Z");
        assert!(!check(&app, false).await.unwrap());
        assert!(!check(&app, true).await.unwrap());
        server.join().unwrap();
        let cached = read_cache(&app.config).unwrap();
        assert_eq!(cached.source, format!("{base}/models/v1.json"));
        assert_eq!(cached.etag.as_deref(), Some("\"new\""));
        app.settings.lock().unwrap().relay_url = Some("http://127.0.0.1:1".into());
        enable(&app);
        assert_ne!(beans_models::updated(), "2999-01-01T00:00:00Z");
        beans_models::reset();
        drop(app);
        std::fs::remove_dir_all(home).unwrap();
    }
}
