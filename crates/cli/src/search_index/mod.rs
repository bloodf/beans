//! Semantic search: messages and images found by meaning, beside exact search (`crate::search`).
//!
//! A Runner embeds the same visible text exact search reads (message text, handoff reasons,
//! notices, permission summaries), each attachment's name, and what a vision model said its
//! images show, with the text model the user chose: a local catalog model, or an embedding
//! provider the user configured and confirmed. With a local image model it also embeds the
//! images themselves, so a text query finds pictures. Tool output and hidden thinking are never
//! read. Vectors stay in the Runner's store (`local_store::vectors`) and never sync.
//!
//! `chats.search_semantic` answers on this Device when it is a Runner and otherwise asks a
//! Runner through the relay. Results only propose messages: each is confirmed against current
//! rows and the current roster before it is returned, as exact search does.

#[cfg(feature = "runner")]
mod describe;
#[cfg(feature = "runner")]
pub mod runner;
#[cfg(all(test, feature = "runner"))]
mod tests;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::app::App;
use crate::local_store::search_body_text;
use crate::model::{Attachment, Body, Message};

/// Names how messages are cut into chunks. Part of every text space: changing the chunker
/// embeds everything again.
pub const CHUNKER: &str = "beans.search.chunks.v1";
/// The most characters in one chunk, about 200 tokens: under every catalog model's limit.
const CHUNK_CHARS: usize = 800;
/// How much of a chunk the next one repeats, so a thought split at a boundary stays whole.
const OVERLAP_CHARS: usize = 120;
/// A longer message is embedded by its first chunks only.
const MAX_TEXT_CHUNKS: usize = 32;
/// The most attachments a message contributes, by name, description, or image.
const MAX_ATTACHMENTS: usize = 16;
/// Chunk numbers by what they hold, each kind from its base by attachment index.
pub const FILE_CHUNK_BASE: u32 = 1 << 20;
pub const DESCRIPTION_CHUNK_BASE: u32 = 2 << 20;
pub const IMAGE_CHUNK_BASE: u32 = 3 << 20;
/// The longest query embedded.
const MAX_QUERY_CHARS: usize = 1000;
/// Results under this share of the best are left out: a space always has a nearest neighbour,
/// and past the close ones they are noise. Absolute scores differ per model, so callers may
/// add their own `min_score`.
const RELATIVE_FLOOR: f32 = 0.5;
/// How long a Device waits for one Runner's answer, and for all it asks in turn.
const REMOTE_WAIT: Duration = Duration::from_secs(10);
const REMOTE_WAIT_TOTAL: Duration = Duration::from_secs(20);

/// The space of a model's exact files (or a provider's exact profile) for one role.
pub fn space_id(role: &str, model: &str, fingerprint: &str) -> String {
    let mut hash = Sha256::new();
    for part in [CHUNKER, role, model, fingerprint] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

/// A message's attachments the search reads, by index, at most `MAX_ATTACHMENTS`.
fn attachments(message: &Message) -> impl Iterator<Item = (u32, &Attachment)> {
    let list: &[Attachment] = match &message.body {
        Body::Text { attachments, .. } => attachments,
        _ => &[],
    };
    list.iter().take(MAX_ATTACHMENTS).enumerate().map(|(index, attachment)| (index as u32, attachment))
}

/// A message's images, with the chunk number each is embedded under.
pub fn images(message: &Message) -> Vec<(u32, &Attachment)> {
    attachments(message).filter(|(_, a)| a.is_image()).map(|(index, a)| (IMAGE_CHUNK_BASE + index, a)).collect()
}

/// What of a message the text space embeds, by chunk number: its search text in overlapping
/// chunks, each attachment's name, and each image description (`(attachment id, text)`).
/// Deterministic, so a chunk number found later names the same words.
pub fn text_chunks(message: &Message, descriptions: &[(String, String)]) -> Vec<(u32, String)> {
    let mut chunks: Vec<(u32, String)> =
        search_body_text(&message.body).map(split).unwrap_or_default().into_iter().take(MAX_TEXT_CHUNKS).enumerate().map(|(i, text)| (i as u32, text)).collect();
    for (index, attachment) in attachments(message) {
        let name = attachment.name.trim();
        if !name.is_empty() {
            chunks.push((FILE_CHUNK_BASE + index, name.to_string()));
        }
        if let Some((_, description)) = descriptions.iter().find(|(id, text)| id == &attachment.id && !text.trim().is_empty()) {
            chunks.push((DESCRIPTION_CHUNK_BASE + index, description.trim().to_string()));
        }
    }
    chunks
}

/// Whitespace-normalized text in chunks of at most `CHUNK_CHARS` characters, ending at a space
/// where one falls in a chunk's last quarter, each repeating about `OVERLAP_CHARS` of the one
/// before. Text without spaces (Chinese, Japanese) is cut by characters.
fn split(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().collect();
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut end = (start + CHUNK_CHARS).min(chars.len());
        if end < chars.len() {
            if let Some(space) = (start + CHUNK_CHARS * 3 / 4..end).rev().find(|&i| chars[i] == ' ') {
                end = space;
            }
        }
        let chunk = chars[start..end].iter().collect::<String>().trim().to_string();
        if !chunk.is_empty() {
            chunks.push(chunk);
        }
        if end >= chars.len() {
            break;
        }
        let back = end.saturating_sub(OVERLAP_CHARS).max(start + 1);
        start = (back..end).find(|&i| chars[i] == ' ').map_or(back, |i| i + 1);
    }
    chunks
}

/// A chunk as a result shows it: its first words.
fn snippet(text: &str) -> String {
    const WORDS: usize = 32;
    const CHARS: usize = 240;
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut snippet = words.iter().take(WORDS).copied().collect::<Vec<_>>().join(" ");
    let mut cut = words.len() > WORDS;
    if snippet.chars().count() > CHARS {
        snippet = snippet.chars().take(CHARS).collect();
        cut = true;
    }
    if cut {
        snippet.push_str(" …");
    }
    snippet
}

/// A confirmed result: `kind` is `message`, `file` (an attachment's name matched), or `image`
/// (its description or its pixels matched), with the attachment for the last two.
fn result(message: &Message, chunk: u32, score: f32, descriptions: &[(String, String)]) -> Value {
    let text = text_chunks(message, descriptions).into_iter().find(|(number, _)| *number == chunk).map(|(_, text)| text).unwrap_or_default();
    let mut value = json!({
        "kind": "message", "chat_id": message.chat_id, "message_id": message.id, "snippet": snippet(&text),
        "author": message.author, "created_at": message.created_at, "score": (f64::from(score) * 10_000.0).round() / 10_000.0,
    });
    let (kind, base) = match chunk {
        c if c >= IMAGE_CHUNK_BASE => ("image", IMAGE_CHUNK_BASE),
        c if c >= DESCRIPTION_CHUNK_BASE => ("image", DESCRIPTION_CHUNK_BASE),
        c if c >= FILE_CHUNK_BASE => ("file", FILE_CHUNK_BASE),
        _ => return value,
    };
    if let Some((_, attachment)) = attachments(message).find(|(index, _)| *index == chunk - base) {
        value["kind"] = kind.into();
        value["attachment_id"] = attachment.id.clone().into();
        value["name"] = attachment.name.clone().into();
        if chunk >= IMAGE_CHUNK_BASE {
            value["snippet"] = attachment.name.clone().into();
        }
    }
    value
}

/// Up to `limit` messages nearest `query` (a vector of `space`), best first, one per message,
/// each confirmed against current rows and the roster, at least `min_score` and
/// `RELATIVE_FLOOR` of the best.
pub fn related(app: &App, space: &str, query: &[f32], limit: usize, min_score: f32) -> Result<Vec<Value>, String> {
    use crate::local_store::vectors::Quantized;
    let Some(query) = Quantized::unit(query) else { return Ok(Vec::new()) };
    let roster: HashSet<String> = app.state.lock().unwrap().chats.iter().map(|chat| chat.meta.id.clone()).collect();
    let failed = |_| "search_failed".to_string();
    let hits = app.store.nearest_vectors(space, query.data.len(), &query, limit * 8 + 16).map_err(failed)?;
    // The floor comes from the best confirmed hit: a stale candidate never sets it.
    let mut floor = min_score.max(f32::MIN_POSITIVE);
    let mut seen = HashSet::new();
    let mut results = Vec::new();
    for hit in hits {
        if results.len() >= limit || hit.score < floor {
            break;
        }
        if seen.contains(&hit.message_id) {
            continue;
        }
        let Some((message, chunk)) = app.store.vector_hit(hit.vector_id).map_err(failed)? else { continue };
        if !roster.contains(&message.chat_id) {
            continue;
        }
        let descriptions = if (DESCRIPTION_CHUNK_BASE..IMAGE_CHUNK_BASE).contains(&chunk) { app.store.descriptions(&message.id).map_err(failed)? } else { Vec::new() };
        if results.is_empty() {
            floor = floor.max(hit.score * RELATIVE_FLOOR);
        }
        seen.insert(hit.message_id);
        results.push(result(&message, chunk, hit.score, &descriptions));
    }
    Ok(results)
}

/// The `search_index.*` verbs. Each runs on the Runner it names, this Device by default.
pub fn is_method(method: &str) -> bool {
    matches!(method, "search_index.status" | "search_index.select")
}

/// Routes a `search_index.*` call from the user's apps to its Runner.
pub async fn dispatch(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    if !is_method(method) {
        return Err("unknown_search_index_method".into());
    }
    let me = app.this_device_id().ok_or("identity_required")?;
    let runner_id = params["runner_id"].as_str().map_or_else(|| me.clone(), str::to_owned);
    let mut body = params;
    if let Some(body) = body.as_object_mut() {
        body.remove("runner_id");
    }
    if runner_id == me {
        return serve_as(app, method, body, &me).await;
    }
    if !app.device(&runner_id).ok_or("runner_unknown")?.is_runner() {
        return Err("runner_required".into());
    }
    crate::requests::ask(app, &runner_id, method, body).await
}

/// Serves a `search_index.*` call on this Runner for `actor`, this Device or a paired one.
pub async fn serve_as(app: &Arc<App>, method: &str, params: Value, actor: &str) -> Result<Value, String> {
    #[cfg(feature = "runner")]
    return runner::serve(app, method, params, actor).await;
    #[cfg(not(feature = "runner"))]
    {
        let _ = (app, method, params, actor);
        Err("runner_required".into())
    }
}

/// The Runners a Device asks, in order: itself when it is one (and no other), else every paired
/// Runner, online ones first. Search is chosen per Runner, so the first may have none.
fn candidate_runners(app: &App) -> Vec<String> {
    if app.local_device().is_some_and(|device| device.is_runner()) {
        return app.this_device_id().into_iter().collect();
    }
    let state = app.state.lock().unwrap();
    runners_to_ask(&state.devices, |id| state.listed_machines.contains_key(id), |id| state.device_online.contains(id))
}

/// The Runners among `devices` still paired (`listed`), online ones first, each group by id.
fn runners_to_ask(devices: &[crate::model::Device], listed: impl Fn(&str) -> bool, online: impl Fn(&str) -> bool) -> Vec<String> {
    let mut runners: Vec<(bool, &str)> =
        devices.iter().filter(|device| device.is_runner() && listed(&device.id)).map(|device| (!online(&device.id), device.id.as_str())).collect();
    runners.sort();
    runners.dedup();
    runners.into_iter().map(|(_, id)| id.to_string()).collect()
}

/// Whether an answer searched at least one space.
fn searched(answer: &Value) -> bool {
    answer["state"] == "ready" || answer["image_state"] == "ready"
}

/// A query as both sides read it.
pub struct Query {
    pub text: String,
    pub limit: usize,
    pub min_score: f32,
}

impl Query {
    fn read(params: &Value) -> Result<Query, String> {
        Ok(Query {
            text: params["query"].as_str().ok_or("missing query")?.trim().chars().take(MAX_QUERY_CHARS).collect(),
            limit: params["limit"].as_u64().unwrap_or(20).clamp(1, 50) as usize,
            min_score: params["min_score"].as_f64().filter(|s| s.is_finite()).unwrap_or(0.0).clamp(-1.0, 1.0) as f32,
        })
    }
}

/// No results, and why: `state` for text, `image_state` for images.
fn unavailable(state: &str, runner_id: Option<&str>, error: Option<String>) -> Value {
    json!({ "state": state, "related": [], "image_state": state, "images": [], "runner_id": runner_id, "error": error })
}

/// `chats.search_semantic`: `{query, limit?, min_score?, runner_id?}` to `{state, related,
/// image_state, images, runner_id, error}`. A state is `ready` when the Runner searched that
/// space; otherwise it says why its list is empty: `off`, `loading`, `not_loaded`, `failed`,
/// `runner_required`, `no_runner`, or `unreachable`.
pub async fn semantic(app: &Arc<App>, params: Value) -> Result<Value, String> {
    let query = Query::read(&params)?;
    let targets = match params["runner_id"].as_str() {
        Some(id) => vec![id.to_string()],
        None => candidate_runners(app),
    };
    let me = app.this_device_id();
    let query = &query;
    first_searched(targets, REMOTE_WAIT, REMOTE_WAIT_TOTAL, |target, wait| {
        let here = me.as_deref() == Some(target.as_str());
        async move {
            if here {
                serve_semantic(app, query).await
            } else {
                Ok(ask(app, &target, query, wait).await)
            }
        }
    })
    .await
}

/// Asks `targets` in turn, each for at most `each` and all of them within `total`, until one
/// has searched: Runners that are off or unreachable must not keep a search waiting one after
/// another. The last answer when none searched; `no_runner` when there was none to ask.
async fn first_searched<F, Answer>(targets: Vec<String>, each: Duration, total: Duration, mut ask: F) -> Result<Value, String>
where
    F: FnMut(String, Duration) -> Answer,
    Answer: std::future::Future<Output = Result<Value, String>>,
{
    let mut last = unavailable("no_runner", None, None);
    let deadline = tokio::time::Instant::now() + total;
    for target in targets {
        let wait = deadline.saturating_duration_since(tokio::time::Instant::now()).min(each);
        if wait.is_zero() {
            break;
        }
        last = ask(target, wait).await?;
        if searched(&last) {
            break;
        }
    }
    Ok(last)
}

/// `query` answered by another Runner, kept to the chats this Device has.
async fn ask(app: &Arc<App>, target: &str, query: &Query, wait: Duration) -> Value {
    let body = json!({ "query": query.text, "limit": query.limit, "min_score": query.min_score });
    match crate::requests::ask_within(app, target, "chats.search_semantic", body, wait).await {
        Ok(mut answer) => {
            // The Runner checked its roster; this Device shows only chats it has.
            let roster: HashSet<String> = app.state.lock().unwrap().chats.iter().map(|chat| chat.meta.id.clone()).collect();
            for list in ["related", "images"] {
                if let Some(hits) = answer[list].as_array_mut() {
                    hits.retain(|hit| hit["chat_id"].as_str().is_some_and(|id| roster.contains(id)));
                }
            }
            answer["runner_id"] = target.into();
            answer
        }
        Err(error) => unavailable("unreachable", Some(target), Some(error)),
    }
}

/// `chats.search_semantic` asked by a paired Device: answered here, never passed on.
pub async fn serve_semantic_as(app: &Arc<App>, params: Value, actor: &str) -> Result<Value, String> {
    if !app.state.lock().unwrap().listed_machines.contains_key(actor) && app.this_device_id().as_deref() != Some(actor) {
        return Err("requester_revoked".into());
    }
    serve_semantic(app, &Query::read(&params)?).await
}

async fn serve_semantic(app: &Arc<App>, query: &Query) -> Result<Value, String> {
    #[cfg(feature = "runner")]
    return runner::semantic(app, query).await;
    #[cfg(not(feature = "runner"))]
    {
        let _ = query;
        Ok(unavailable("runner_required", app.this_device_id().as_deref(), None))
    }
}

#[cfg(test)]
mod chunk_tests {
    use super::*;
    use crate::model::Author;

    fn attachment(id: &str, name: &str, mime: &str) -> Attachment {
        Attachment { id: id.into(), name: name.into(), mime: mime.into(), size: 1, width: None, height: None }
    }

    #[test]
    fn chunks_cover_long_text_with_overlap() {
        let words: Vec<String> = (0..400).map(|i| format!("word{i}")).collect();
        let parts = split(&words.join(" \n "));
        assert!(parts.len() > 2);
        assert!(parts.iter().all(|p| p.chars().count() <= CHUNK_CHARS && !p.starts_with(' ')));
        for pair in parts.windows(2) {
            assert!(pair[1].contains(pair[0].split(' ').next_back().unwrap()), "the next chunk repeats the end of the one before");
        }
        assert!(words.iter().all(|word| parts.iter().any(|p| p.split(' ').any(|w| w == word))));
        // No spaces: cut by characters, still overlapping, never looping.
        let parts = split(&"上海的会议记录".repeat(300));
        assert!(parts.len() >= 3 && parts[..parts.len() - 1].iter().all(|p| p.chars().count() == CHUNK_CHARS));
        assert!(split("   ").is_empty());
    }

    #[test]
    fn every_attachment_has_its_chunks_and_results_name_it() {
        let mut message = Message::new("chat", Author::You, Body::Text {
            text: "Quarterly numbers".into(),
            attachments: vec![attachment("a", "Ledger.xlsx", "application/vnd.ms-excel"), attachment("b", "  ", "image/png"), attachment("c", "beach.jpg", "image/jpeg")],
            mentions: vec![], reply_to: None,
        });
        let descriptions = vec![("c".to_string(), "Two people on a sandy beach".to_string()), ("elsewhere".into(), "ignored".into())];
        assert_eq!(text_chunks(&message, &descriptions), vec![
            (0, "Quarterly numbers".to_string()),
            (FILE_CHUNK_BASE, "Ledger.xlsx".to_string()),
            (FILE_CHUNK_BASE + 2, "beach.jpg".to_string()),
            (DESCRIPTION_CHUNK_BASE + 2, "Two people on a sandy beach".to_string()),
        ]);
        assert_eq!(images(&message).iter().map(|(chunk, a)| (*chunk, a.id.as_str())).collect::<Vec<_>>(), [(IMAGE_CHUNK_BASE + 1, "b"), (IMAGE_CHUNK_BASE + 2, "c")]);
        let file = result(&message, FILE_CHUNK_BASE, 0.123456, &[]);
        assert_eq!((file["kind"].as_str(), file["attachment_id"].as_str(), file["score"].as_f64()), (Some("file"), Some("a"), Some(0.1235)));
        let described = result(&message, DESCRIPTION_CHUNK_BASE + 2, 0.5, &descriptions);
        assert_eq!((described["kind"].as_str(), described["attachment_id"].as_str(), described["snippet"].as_str()), (Some("image"), Some("c"), Some("Two people on a sandy beach")));
        let pixels = result(&message, IMAGE_CHUNK_BASE + 2, 0.5, &[]);
        assert_eq!((pixels["kind"].as_str(), pixels["snippet"].as_str()), (Some("image"), Some("beach.jpg")));
        message.body = serde_json::from_value(json!({"kind": "tool", "name": "bash", "summary": "s", "detail": "d", "is_running": false})).unwrap();
        assert!(text_chunks(&message, &[]).is_empty() && images(&message).is_empty(), "tool rows are never embedded");
    }

    #[test]
    fn chunk_boundaries_are_exact_and_never_split_a_character() {
        let exactly = "x".repeat(CHUNK_CHARS);
        assert_eq!(split(&exactly), std::slice::from_ref(&exactly), "exactly one chunk's worth");
        let one_more = "x".repeat(CHUNK_CHARS + 1);
        let parts = split(&one_more);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].chars().count(), OVERLAP_CHARS + 1, "the rest, with the overlap before it");
        // A word longer than a chunk is cut inside it; multi-byte characters stay whole.
        let emoji = "😀".repeat(CHUNK_CHARS * 2);
        assert!(split(&emoji).iter().all(|p| p.chars().all(|c| c == '😀') && p.chars().count() <= CHUNK_CHARS));
        let mixed = format!("start {} end", "長".repeat(CHUNK_CHARS * 2));
        let parts = split(&mixed);
        assert!(parts.first().unwrap().starts_with("start ") && parts.last().unwrap().ends_with(" end"));
        // Tabs, newlines and runs of spaces read as single spaces.
        assert_eq!(split("a\t\tb\n\n c  "), ["a b c"]);
        assert!(split("").is_empty() && split("\n\t ").is_empty());
    }

    #[test]
    fn long_messages_and_many_attachments_are_capped() {
        let long = Message::new("chat", Author::You, Body::text("word ".repeat(CHUNK_CHARS * MAX_TEXT_CHUNKS)));
        let chunks = text_chunks(&long, &[]);
        assert_eq!(chunks.len(), MAX_TEXT_CHUNKS, "only the first chunks of a very long message");
        assert_eq!(chunks.last().unwrap().0, MAX_TEXT_CHUNKS as u32 - 1);
        let many: Vec<Attachment> = (0..40).map(|i| attachment(&format!("a{i}"), &format!("photo {i}.png"), "image/png")).collect();
        let message = Message::new("chat", Author::You, Body::Text { text: String::new(), attachments: many, mentions: vec![], reply_to: None });
        assert_eq!(images(&message).len(), MAX_ATTACHMENTS);
        assert_eq!(text_chunks(&message, &[]).len(), MAX_ATTACHMENTS, "names only, and no chunk for the empty text");
        assert_eq!(images(&message).last().unwrap().0, IMAGE_CHUNK_BASE + MAX_ATTACHMENTS as u32 - 1);
    }

    #[test]
    fn snippets_and_results_hold_at_their_edges() {
        assert_eq!(snippet(""), "");
        assert_eq!(snippet(&"w ".repeat(32)), "w ".repeat(32).trim());
        assert_eq!(snippet(&"w ".repeat(33)), format!("{} …", "w ".repeat(32).trim()));
        let giant = "x".repeat(1000);
        assert_eq!(snippet(&giant).chars().count(), 240 + 2, "a single long word is cut too");
        // A chunk number past the message's attachments (a message edited since) reads as text.
        let message = Message::new("chat", Author::You, Body::Text { text: "hi".into(), attachments: vec![attachment("a", "a.png", "image/png")], mentions: vec![], reply_to: None });
        for chunk in [FILE_CHUNK_BASE + 5, DESCRIPTION_CHUNK_BASE + 5, IMAGE_CHUNK_BASE + 5, 99] {
            let value = result(&message, chunk, 0.5, &[]);
            assert_eq!((value["kind"].as_str(), value["snippet"].as_str()), (Some("message"), Some("")), "{chunk}");
        }
        assert_eq!(result(&message, 0, -0.25, &[])["score"], -0.25);
    }

    #[test]
    fn a_space_is_stable_across_releases() {
        // Changing this hash embeds every user's history again: do it only on purpose.
        assert_eq!(space_id("text", "model", "files"), "3f660d7423560acffc7b823754cd7805cda53b2a13d7935d050a7e038401f83b");
    }

    #[test]
    fn spaces_differ_by_role_model_files_and_chunker() {
        assert_ne!(space_id("text", "a", "x"), space_id("text", "a", "y"));
        assert_ne!(space_id("text", "a", "x"), space_id("text", "b", "x"));
        assert_ne!(space_id("text", "a", "x"), space_id("image", "a", "x"));
        assert_ne!(space_id("text", "ab", "c"), space_id("text", "a", "bc"), "parts are length-delimited");
    }

    #[test]
    fn a_phone_asks_paired_runners_online_first() {
        let device = |id: &str, os: &str| crate::model::Device { id: id.into(), os: os.into(), ..Default::default() };
        let devices = [
            device("phone", "ios"),
            device("b-offline", "linux"),
            device("c-online", "macos"),
            device("a-online", "windows"),
            device("revoked", "macos"),
            device("c-online", "macos"),
        ];
        let listed = |id: &str| id != "revoked";
        let online = |id: &str| id.ends_with("-online") || id == "phone";
        assert_eq!(runners_to_ask(&devices, listed, online), ["a-online", "c-online", "b-offline"], "no phone, no revoked Runner, each once");
        assert!(runners_to_ask(&devices, |_| false, online).is_empty(), "nothing paired, nothing to ask");
        assert!(runners_to_ask(&[], listed, online).is_empty());
    }

    /// A fake Runner for `first_searched`: answers with `state`, or takes the whole wait and is
    /// unreachable.
    fn answer(state: &'static str) -> Option<Value> {
        (state != "unreachable").then(|| json!({ "state": state, "image_state": "off" }))
    }

    async fn ask_in_turn(runners: &[(&'static str, &'static str)]) -> (Value, Vec<(String, Duration)>) {
        let asked = std::sync::Mutex::new(Vec::new());
        let answers: std::collections::HashMap<&str, &str> = runners.iter().copied().collect();
        let targets = runners.iter().map(|(id, _)| id.to_string()).collect();
        let result = first_searched(targets, REMOTE_WAIT, REMOTE_WAIT_TOTAL, |target, wait| {
            asked.lock().unwrap().push((target.clone(), wait));
            let state = answers[target.as_str()];
            async move {
                match answer(state) {
                    Some(found) if state == "broken" => Err(found["state"].as_str().unwrap().to_string()),
                    Some(found) => Ok(found),
                    None => {
                        tokio::time::sleep(wait).await;
                        Ok(unavailable("unreachable", Some(&target), Some("timed out".into())))
                    }
                }
            }
        })
        .await
        .unwrap_or_else(|error| json!({ "error": error }));
        (result, asked.into_inner().unwrap())
    }

    #[tokio::test(start_paused = true)]
    async fn runners_are_asked_in_turn_until_one_has_searched() {
        let (found, asked) = ask_in_turn(&[("off", "off"), ("ready", "ready"), ("never", "ready")]).await;
        assert_eq!(found["state"], "ready");
        assert_eq!(asked.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["off", "ready"], "no one after the first that searched");
        // Only pictures searched counts too.
        let found = first_searched(vec!["a".into()], REMOTE_WAIT, REMOTE_WAIT_TOTAL, |_, _| async { Ok(json!({ "state": "off", "image_state": "ready" })) }).await.unwrap();
        assert_eq!(found["image_state"], "ready");
        let (none, asked) = ask_in_turn(&[]).await;
        assert_eq!((none["state"].as_str(), asked.len()), (Some("no_runner"), 0));
        let (off, _) = ask_in_turn(&[("a", "off"), ("b", "loading")]).await;
        assert_eq!(off["state"], "loading", "the last answer says why nothing was found");
        let (failed, _) = ask_in_turn(&[("a", "broken")]).await;
        assert_eq!(failed["error"], "broken", "this Runner's own failure is an error");
    }

    #[tokio::test(start_paused = true)]
    async fn unreachable_runners_share_one_deadline() {
        let started = tokio::time::Instant::now();
        let (found, asked) = ask_in_turn(&[("a", "unreachable"), ("b", "unreachable"), ("c", "unreachable"), ("d", "ready")]).await;
        assert_eq!(found["state"], "unreachable");
        assert_eq!(asked, [("a".to_string(), REMOTE_WAIT), ("b".to_string(), REMOTE_WAIT)], "two full waits use up the deadline");
        assert_eq!(started.elapsed(), REMOTE_WAIT_TOTAL);
        // A later Runner gets only what is left.
        let each = Duration::from_secs(10);
        let asked = std::sync::Mutex::new(Vec::new());
        first_searched(vec!["slow".into(), "next".into()], each, Duration::from_secs(15), |target, wait| {
            asked.lock().unwrap().push((target, wait));
            async move {
                tokio::time::sleep(wait).await;
                Ok(json!({ "state": "unreachable" }))
            }
        })
        .await
        .unwrap();
        assert_eq!(asked.into_inner().unwrap(), [("slow".to_string(), each), ("next".to_string(), Duration::from_secs(5))]);
    }
}
