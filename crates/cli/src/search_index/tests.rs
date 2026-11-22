//! Semantic search end to end on a Runner, with fixture models that behave like real ones where
//! it matters: synonyms land together, colours are what an image tower sees, and a scripted
//! vision model describes what it is shown.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::runner::{ImageEncoder, Model};
use super::*;
use crate::config::Config;
use crate::embeddings::{Embedding, EmbeddingBatch, EmbeddingError, EmbeddingFingerprint, EmbeddingPurpose};
use crate::model::{Attachment, Author, Body, Message};

/// Words that mean the same thing share a dimension; anything else reads as a small constant.
const CONCEPTS: &[&[&str]] = &[
    &["dog", "puppy", "canine", "hound"],
    &["invoice", "receipt", "bill", "payment"],
    &["deploy", "release", "ship", "rollout"],
    &["上海", "shanghai"],
    &["beach", "ocean", "sea", "shore"],
    &["red", "crimson"],
    &["green"],
    &["blue", "navy"],
];
const RED: usize = 5;

fn concept_vector(text: &str) -> Vec<f32> {
    let text = text.to_lowercase();
    let mut vector: Vec<f32> = CONCEPTS.iter().map(|words| words.iter().filter(|w| text.contains(*w)).count() as f32).collect();
    vector.push(0.05);
    vector
}

/// A text model over `CONCEPTS`, counting what it embeds for indexing and for queries; it fails
/// while `fail` is set, as a provider that is down.
struct Concepts {
    fingerprint: EmbeddingFingerprint,
    documents: AtomicUsize,
    queries: AtomicUsize,
    fail: std::sync::atomic::AtomicBool,
    longest_query: AtomicUsize,
}

#[async_trait::async_trait]
impl Embedding for Concepts {
    fn fingerprint(&self) -> &EmbeddingFingerprint {
        &self.fingerprint
    }
    async fn embed(&self, purpose: EmbeddingPurpose, texts: &[String], _cancel: CancellationToken) -> Result<EmbeddingBatch, EmbeddingError> {
        let counter = if purpose == EmbeddingPurpose::Query { &self.queries } else { &self.documents };
        counter.fetch_add(texts.len(), Ordering::Relaxed);
        if self.fail.load(Ordering::Relaxed) {
            return Err(EmbeddingError::Transport);
        }
        if purpose == EmbeddingPurpose::Query {
            self.longest_query.fetch_max(texts.iter().map(|t| t.chars().count()).max().unwrap_or(0), Ordering::Relaxed);
        }
        Ok(EmbeddingBatch { fingerprint: self.fingerprint.clone(), vectors: texts.iter().map(|t| concept_vector(t)).collect() })
    }
}

/// An image tower that sees a picture's average colour, in the colour concepts' dimensions.
struct Colours(AtomicUsize);

#[async_trait::async_trait]
impl ImageEncoder for Colours {
    async fn embed(&self, images: Vec<Vec<u8>>, _cancel: CancellationToken) -> Result<Vec<Vec<f32>>, String> {
        self.0.fetch_add(images.len(), Ordering::Relaxed);
        images
            .iter()
            .map(|bytes| {
                let image = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgb8();
                let count = (image.width() * image.height()) as f32;
                let mut vector = vec![0f32; CONCEPTS.len() + 1];
                for pixel in image.pixels() {
                    for channel in 0..3 {
                        vector[RED + channel] += f32::from(pixel[channel]) / 255.0 / count;
                    }
                }
                vector[CONCEPTS.len()] = 0.05;
                Ok(vector)
            })
            .collect()
    }
}

/// A vision model that says the same thing about every image, counting the calls; its first
/// `failures` calls fail, as a provider that is down.
struct Describer {
    answer: &'static str,
    calls: AtomicUsize,
    failures: AtomicUsize,
}

#[async_trait::async_trait]
impl beans_agent::Provider for Describer {
    fn provider_id(&self) -> &str {
        "fixture"
    }
    fn model_id(&self) -> &str {
        "vision"
    }
    async fn stream(&self, request: beans_agent::ModelRequest, _cancel: CancellationToken) -> beans_agent::AssistantEventStream {
        use beans_agent::{types::StopReason, AssistantEvent};
        let beans_agent::types::LlmMessage::User(message) = &request.messages[0] else { panic!("a user message") };
        assert!(message.content.iter().any(|part| matches!(part, beans_agent::types::ContentPart::Image { .. })), "the image is sent");
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.failures.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1)).is_ok() {
            return Box::pin(futures::stream::iter(vec![AssistantEvent::Start, AssistantEvent::Error { message: "overloaded".into(), aborted: false }]));
        }
        let events = vec![
            AssistantEvent::Start,
            AssistantEvent::TextStart { index: 0 },
            AssistantEvent::TextDelta { index: 0, delta: self.answer.into() },
            AssistantEvent::TextEnd { index: 0 },
            AssistantEvent::Done { stop_reason: StopReason::Stop, usage: Default::default() },
        ];
        Box::pin(futures::stream::iter(events))
    }
}

struct Scratch(Arc<App>, std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

fn runner() -> Scratch {
    let home = std::env::temp_dir().join(format!("beans-search-index-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&home).unwrap();
    let app = App::load(Config { home: home.clone(), port: 0 }).unwrap();
    crate::identity::create(&app, Some("Runner".into())).unwrap();
    assert!(app.local_device().unwrap().is_runner());
    Scratch(app, home)
}

fn chat(app: &App) -> String {
    app.state.lock().unwrap().chats[0].meta.id.clone()
}

fn say(app: &App, chat: &str, id: &str, at: f64, body: Body) -> Message {
    let mut message = Message::new(chat, Author::You, body);
    message.id = id.into();
    message.created_at = at;
    app.store.upsert(&message).unwrap();
    message
}

fn png(colour: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(8, 8, image::Rgb(colour));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// A message with one picture, its bytes already on this Runner.
fn picture(app: &App, chat: &str, id: &str, at: f64, name: &str, colour: [u8; 3]) -> Message {
    let attachment = Attachment { id: format!("att-{id}"), name: name.into(), mime: "image/png".into(), size: 1, width: Some(8), height: Some(8) };
    std::fs::create_dir_all(app.config.files_dir()).unwrap();
    std::fs::write(crate::files::local_path(app, &attachment.id).unwrap(), png(colour)).unwrap();
    say(app, chat, id, at, Body::Text { text: "look".into(), attachments: vec![attachment], mentions: vec![], reply_to: None })
}

/// Registers fixture models under `id`: text only, or an image model with both towers.
fn fixture(app: &App, id: &str, images: bool) -> (Arc<Concepts>, Arc<Colours>) {
    let fingerprint = EmbeddingFingerprint::parse(&format!("{:x}", sha2::Sha256::digest(id.as_bytes()))).unwrap();
    let text = Arc::new(Concepts { fingerprint, documents: AtomicUsize::new(0), queries: AtomicUsize::new(0), fail: Default::default(), longest_query: AtomicUsize::new(0) });
    let colours = Arc::new(Colours(AtomicUsize::new(0)));
    let role = if images { "image" } else { "text" };
    let tower: Option<Arc<dyn ImageEncoder>> = images.then(|| colours.clone() as Arc<dyn ImageEncoder>);
    let model = Model::fixture(space_id(role, id, "fixture"), text.clone(), tower);
    app.search_index.fixtures.lock().insert(id.into(), Arc::new(model));
    (text, colours)
}

async fn call(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    crate::api::dispatch(app, method, params).await
}

/// Chooses per `params` and waits until every chosen role is ready.
async fn choose(app: &Arc<App>, params: Value) -> Value {
    call(app, "search_index.select", params).await.unwrap();
    for _ in 0..200 {
        let status = call(app, "search_index.status", json!({})).await.unwrap();
        if ["text", "image"].iter().all(|role| status[role]["state"] == "ready" || status[role]["state"] == "off") {
            return status;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!("the fixture models did not load");
}

fn local(id: &str) -> Value {
    json!({ "kind": "local", "model_id": id })
}

async fn index_all(app: &Arc<App>) {
    while runner::index_once(app).await.unwrap() {}
}

async fn search(app: &Arc<App>, query: &str) -> Value {
    call(app, "chats.search_semantic", json!({ "query": query, "limit": 5 })).await.unwrap()
}

fn ids(result: &Value, list: &str) -> Vec<String> {
    result[list].as_array().unwrap().iter().map(|hit| hit["message_id"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn text_is_found_by_meaning_within_the_roster() {
    let scratch = runner();
    let app = &scratch.0;
    let chat = chat(app);
    say(app, &chat, "puppy", 1.0, Body::text("My puppy loves running on the shore"));
    say(app, &chat, "receipt", 2.0, Body::text("Here is the receipt for March"));
    say(app, &chat, "ship", 3.0, Body::text("We ship the new build on Friday"));
    say(app, &chat, "file", 4.0, Body::Text {
        text: "see attached".into(),
        attachments: vec![Attachment { id: "att-bill".into(), name: "bill-2024.pdf".into(), mime: "application/pdf".into(), size: 1, width: None, height: None }],
        mentions: vec![], reply_to: None,
    });
    say(app, &chat, "tool", 5.0, serde_json::from_value(json!({"kind": "tool", "name": "bash", "summary": "invoice payment receipt bill", "detail": "", "is_running": false})).unwrap());
    let mut orphan = Message::new("absent-chat", Author::You, Body::text("a canine by the sea"));
    orphan.id = "orphan".into();
    app.store.upsert(&orphan).unwrap();

    assert_eq!(search(app, "dog").await["state"], "off", "nothing is embedded until a model is chosen");
    fixture(app, "concepts", false);
    let status = choose(app, json!({ "text": local("concepts") })).await;
    assert_eq!((status["text"]["pending"].as_u64(), status["image"]["state"].as_str()), (Some(5), Some("off")));
    index_all(app).await;
    let status = call(app, "search_index.status", json!({})).await.unwrap();
    assert_eq!((status["text"]["indexed"].as_u64(), status["text"]["pending"].as_u64()), (Some(5), Some(0)));

    let found = search(app, "a canine at the ocean").await;
    assert_eq!((found["state"].as_str(), found["image_state"].as_str()), (Some("ready"), Some("off")));
    assert_eq!(ids(&found, "related")[0], "puppy", "no word in common, same meaning");
    assert!(!ids(&found, "related").contains(&"orphan".to_string()), "chats outside the roster never show");
    let payment = search(app, "payment").await;
    let related = payment["related"].as_array().unwrap();
    let kinds: Vec<(&str, &str)> = related.iter().map(|h| (h["message_id"].as_str().unwrap(), h["kind"].as_str().unwrap())).collect();
    assert!(kinds.contains(&("receipt", "message")) && kinds.contains(&("file", "file")), "{kinds:?}");
    assert!(!ids(&payment, "related").contains(&"tool".to_string()), "tool rows are never embedded");
    let file = related.iter().find(|h| h["kind"] == "file").unwrap();
    assert_eq!((file["attachment_id"].as_str(), file["name"].as_str()), (Some("att-bill"), Some("bill-2024.pdf")));

    // An edit drops the old vectors at once, before the indexer runs again.
    let mut puppy = app.store.message(&chat, "puppy").unwrap().unwrap();
    puppy.body = Body::text("Rollout notes for the staging cluster");
    app.store.upsert(&puppy).unwrap();
    assert!(!ids(&search(app, "dog").await, "related").contains(&"puppy".to_string()));
    index_all(app).await;
    assert!(ids(&search(app, "release").await, "related").contains(&"puppy".to_string()));
    // So does a deletion, and the in-memory copy lets the rows go.
    let before = app.store.cached_vectors();
    app.store.remove(&chat, "ship").unwrap();
    assert!(!ids(&search(app, "deploy").await, "related").contains(&"ship".to_string()));
    assert_eq!(app.store.cached_vectors(), before - 1);
    // A far query finds nothing close, and a caller's own floor leaves out the rest.
    assert!(search(app, "dog").await["related"].as_array().unwrap().iter().all(|h| h["score"].as_f64().unwrap() < 0.5));
    let floored = call(app, "chats.search_semantic", json!({ "query": "dog", "min_score": 0.5 })).await.unwrap();
    assert!(floored["related"].as_array().unwrap().is_empty());

    // Turning text search off deletes its vectors.
    call(app, "search_index.select", json!({ "text": null })).await.unwrap();
    assert_eq!(search(app, "release").await["state"], "off");
    assert_eq!(app.store.embedding_progress(crate::local_store::vectors::Role::Text).unwrap().0, 0);
}

#[tokio::test]
async fn embedding_is_incremental_and_resumes_after_reopening() {
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let home = std::env::temp_dir().join(format!("beans-search-index-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&home).unwrap();
    let _cleanup = Cleanup(home.clone());
    let config = Config { home: home.clone(), port: 0 };
    let app = App::load(config.clone()).unwrap();
    crate::identity::create(&app, Some("Runner".into())).unwrap();
    let chat = chat(&app);
    for i in 0..70 {
        say(&app, &chat, &format!("m{i:02}"), i as f64, Body::text(format!("note {i} about the beach")));
    }
    // A reply still streaming waits; a failed one keeps its words and is embedded.
    let mut streaming = Message::new(&chat, Author::System, Body::text("a hound"));
    streaming.id = "streaming".into();
    streaming.state = crate::model::MessageState::Streaming;
    app.store.upsert(&streaming).unwrap();
    let mut failed = Message::new(&chat, Author::System, Body::text("half a reply about a hound"));
    failed.id = "failed".into();
    failed.state = crate::model::MessageState::Failed { error: "provider".into() };
    app.store.upsert(&failed).unwrap();
    let (text, _) = fixture(&app, "concepts", false);
    choose(&app, json!({ "text": local("concepts") })).await;
    assert!(runner::index_once(&app).await.unwrap(), "a full batch leaves more to do");
    index_all(&app).await;
    assert_eq!(text.documents.load(Ordering::Relaxed), 71);
    assert_eq!(ids(&search(&app, "dog").await, "related"), ["failed"]);
    index_all(&app).await;
    assert_eq!(text.documents.load(Ordering::Relaxed), 71, "nothing is embedded twice");
    streaming.state = crate::model::MessageState::Complete;
    app.store.upsert(&streaming).unwrap();
    index_all(&app).await;
    assert_eq!(text.documents.load(Ordering::Relaxed), 72, "the finished reply, once");

    // A reopened store keeps its vectors and embeds only what is new.
    drop(app);
    let app = App::load(config).unwrap();
    let (again, _) = fixture(&app, "concepts", false);
    choose(&app, json!({ "text": local("concepts") })).await;
    say(&app, &chat, "new", 100.0, Body::text("a new puppy"));
    index_all(&app).await;
    assert_eq!(again.documents.load(Ordering::Relaxed), 1);
    assert!(ids(&search(&app, "dog").await, "related").contains(&"new".to_string()));

    // Another model is another space: everything is embedded again.
    let (other, _) = fixture(&app, "other-concepts", false);
    choose(&app, json!({ "text": local("other-concepts") })).await;
    index_all(&app).await;
    assert_eq!(other.documents.load(Ordering::Relaxed), 73);
}

#[tokio::test]
async fn work_saved_after_its_message_changed_is_dropped() {
    use crate::local_store::vectors::{ChunkVector, Embedded, Quantized, Queue, Role};
    let scratch = runner();
    let app = &scratch.0;
    let chat = chat(app);
    say(app, &chat, "m", 1.0, Body::text("the puppy"));
    fixture(app, "concepts", false);
    choose(app, json!({ "text": local("concepts") })).await;
    let space = app.store.vector_space(Role::Text).unwrap();
    let space = match space {
        Some(space) => space,
        None => {
            runner::index_once(app).await.unwrap();
            app.store.vector_space(Role::Text).unwrap().unwrap()
        }
    };
    say(app, &chat, "m", 1.0, Body::text("the puppy, again"));
    let read = app.store.pending(Queue::Text, 10).unwrap();
    let vector = || vec![ChunkVector { chunk: 0, vector: Quantized::from_f32(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]).unwrap() }];
    // The message changes while the indexer embeds it: the stale work is dropped.
    say(app, &chat, "m", 1.0, Body::text("the invoice"));
    let stale = Embedded { message_id: "m".into(), version: read[0].version, chunks: vector() };
    assert_eq!(app.store.save_embeddings(Role::Text, &space, &[stale]).unwrap(), 0);
    // So is work for a space no longer chosen.
    let fresh = app.store.pending(Queue::Text, 10).unwrap();
    let elsewhere = Embedded { message_id: "m".into(), version: fresh[0].version, chunks: vector() };
    assert_eq!(app.store.save_embeddings(Role::Text, "another-space", &[elsewhere]).unwrap(), 0);
    // And a description arriving mid-embed, which changes no text, still gets embedded.
    let mid = app.store.pending(Queue::Text, 10).unwrap();
    app.store.connection.lock().unwrap().execute("INSERT INTO image_descriptions VALUES ('m', 'att', 'fixture', 'a dog')", []).unwrap();
    let early = Embedded { message_id: "m".into(), version: mid[0].version, chunks: vector() };
    assert_eq!(app.store.save_embeddings(Role::Text, &space, &[early]).unwrap(), 0, "the description bumped the version");
    index_all(app).await;
    assert_eq!(ids(&search(app, "bill").await, "related"), ["m"]);
}

#[tokio::test]
async fn images_are_found_by_what_they_show_and_by_their_descriptions() {
    let scratch = runner();
    let app = &scratch.0;
    let chat = chat(app);
    picture(app, &chat, "red", 1.0, "IMG_0001.png", [230, 10, 10]);
    picture(app, &chat, "blue", 2.0, "IMG_0002.png", [10, 10, 230]);
    let (_, colours) = fixture(app, "clip", true);
    fixture(app, "concepts", false);
    let status = choose(app, json!({ "text": local("concepts"), "image_model_id": "clip" })).await;
    assert_eq!(status["image"]["pending"], 2);
    index_all(app).await;
    assert_eq!(colours.0.load(Ordering::Relaxed), 2, "each picture is read once");
    let found = search(app, "something crimson").await;
    assert_eq!(found["image_state"], "ready");
    let first = &found["images"][0];
    assert_eq!((first["message_id"].as_str(), first["kind"].as_str(), first["attachment_id"].as_str()), (Some("red"), Some("image"), Some("att-red")));
    assert_eq!(ids(&search(app, "navy").await, "images")[0], "blue");

    // Describing images sends them to a provider: it must be confirmed, and only then runs.
    let describer = Arc::new(Describer { answer: "A restaurant receipt on a red table", calls: AtomicUsize::new(0), failures: AtomicUsize::new(0) });
    *app.search_index.describer_fixture.lock() = Some(describer.clone());
    let describe = json!({ "describe": { "provider": "fixture", "model": "vision" } });
    assert_eq!(call(app, "search_index.select", describe.clone()).await.unwrap_err(), "external_confirmation_required");
    let mut confirmed = describe;
    confirmed["confirm_external"] = true.into();
    call(app, "search_index.select", confirmed).await.unwrap();
    index_all(app).await;
    assert_eq!(describer.calls.load(Ordering::Relaxed), 2);
    let described = search(app, "the invoice from dinner").await;
    let hit = described["related"].as_array().unwrap().iter().find(|h| h["kind"] == "image").expect("a description matched");
    assert_eq!(hit["snippet"], "A restaurant receipt on a red table");
    index_all(app).await;
    assert_eq!(describer.calls.load(Ordering::Relaxed), 2, "each image is described once");

    // A message that drops its picture drops the picture's vectors and description.
    let mut red = app.store.message(&chat, "red").unwrap().unwrap();
    red.body = Body::text("never mind");
    app.store.upsert(&red).unwrap();
    assert!(!ids(&search(app, "crimson").await, "images").contains(&"red".to_string()));
    app.store.remove(&chat, "red").unwrap();
    assert!(app.store.descriptions("red").unwrap().is_empty());

    // Turning images off deletes their vectors; text search goes on.
    call(app, "search_index.select", json!({ "image_model_id": null })).await.unwrap();
    let off = search(app, "navy").await;
    assert_eq!((off["image_state"].as_str(), off["state"].as_str()), (Some("off"), Some("ready")));
}

#[tokio::test]
async fn an_embedding_provider_must_be_confirmed_and_its_outage_is_reported() {
    use crate::memory_service::{config::Record, types::Revision};
    let scratch = runner();
    let app = &scratch.0;
    let profile: crate::memory_service::types::EmbeddingProfile = serde_json::from_value(json!({
        "mode": "api", "model": "text-embedding-3-small", "revision": "1", "dimensions": 4, "normalization": "l2",
        "distance": "cosine", "document_prefix": "", "query_prefix": "", "endpoint": "https://127.0.0.1:9/v1/embeddings", "secret": "fixture-key",
    }))
    .unwrap();
    let revision = Revision { counter: 1, device_id: app.this_device_id().unwrap() };
    app.memory_config.lock().embeddings.insert("openai".into(), Record { revision, value: Some(profile), extra: Default::default() });
    let status = call(app, "search_index.status", json!({})).await.unwrap();
    assert_eq!(status["profiles"], json!([{ "id": "openai", "model": "text-embedding-3-small", "dimensions": 4, "has_secret": true }]));
    let provider = json!({ "text": { "kind": "provider", "profile_id": "openai" } });
    assert_eq!(call(app, "search_index.select", provider.clone()).await.unwrap_err(), "external_confirmation_required");
    let mut confirmed = provider;
    confirmed["confirm_external"] = true.into();
    choose(app, confirmed).await;
    // The provider cannot be reached: the query says so instead of failing the search.
    let found = search(app, "anything").await;
    assert_eq!(found["state"], "failed");
    assert!(found["error"].is_string());
    assert!(found["related"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn choices_are_checked_and_belong_to_the_requester() {
    let scratch = runner();
    let app = &scratch.0;
    let me = app.this_device_id().unwrap();
    assert_eq!(call(app, "search_index.select", json!({ "text": local("not-installed") })).await.unwrap_err(), "model_not_installed");
    assert_eq!(call(app, "search_index.select", json!({ "image_model_id": "not-installed" })).await.unwrap_err(), "model_not_installed");
    assert_eq!(call(app, "search_index.select", json!({ "text": { "kind": "provider", "profile_id": "none" } })).await.unwrap_err(), "profile_not_found");
    assert_eq!(call(app, "search_index.select", json!({ "text": "nonsense" })).await.unwrap_err(), "invalid_request");
    let status = call(app, "search_index.status", json!({})).await.unwrap();
    assert_eq!((status["text"]["state"].as_str(), status["image"]["state"].as_str()), (Some("off"), Some("off")));
    assert_eq!(status["runs_models"], cfg!(feature = "embedding-local"));
    assert_eq!(serve_as(app, "search_index.forget", json!({}), &me).await.unwrap_err(), "unknown_search_index_method");
    let stranger = crate::keys::Machine::generate().pubkey();
    assert_eq!(serve_as(app, "search_index.status", json!({}), &stranger).await.unwrap_err(), "requester_revoked");
    assert_eq!(serve_semantic_as(app, json!({ "query": "x" }), &stranger).await.unwrap_err(), "requester_revoked");
    assert_eq!(candidate_runners(app), app.this_device_id().into_iter().collect::<Vec<_>>(), "a Runner asks only itself");
}

/// The relative floor comes from the best confirmed hit: a best candidate that no longer holds
/// (its message changed) must not hide the results that do.
#[tokio::test]
async fn a_stale_best_candidate_does_not_hide_the_rest() {
    let scratch = runner();
    let app = &scratch.0;
    let chat = chat(app);
    say(app, &chat, "exact", 1.0, Body::text("dog"));
    say(app, &chat, "partly", 2.0, Body::text("a dog by the sea, on the shore, at the beach"));
    fixture(app, "concepts", false);
    choose(app, json!({ "text": local("concepts") })).await;
    index_all(app).await;
    assert_eq!(ids(&search(app, "dog").await, "related"), ["exact"], "the far one is under half the best");
    app.store.connection.lock().unwrap().execute("UPDATE message_search SET embed_pending = 1 WHERE message_id = 'exact'", []).unwrap();
    assert_eq!(ids(&search(app, "dog").await, "related"), ["partly"]);
}

/// A model that fails waits out a growing pause before its queue tries again, while the other
/// queues go on; once it works again, the queue catches up.
#[tokio::test(start_paused = true)]
async fn a_failing_model_backs_off_and_the_other_queues_go_on() {
    let scratch = runner();
    let app = &scratch.0;
    let chat = chat(app);
    say(app, &chat, "text", 1.0, Body::text("a puppy"));
    picture(app, &chat, "red", 2.0, "red.png", [230, 10, 10]);
    let (text, _) = fixture(app, "concepts", false);
    let (_, colours) = fixture(app, "clip", true);
    choose(app, json!({ "text": local("concepts"), "image_model_id": "clip" })).await;
    text.fail.store(true, Ordering::Relaxed);
    assert!(!runner::index_once(app).await.unwrap(), "a failed queue asks for no quick retry");
    assert_eq!(colours.0.load(Ordering::Relaxed), 1, "pictures were embedded anyway");
    let tried = text.documents.load(Ordering::Relaxed);
    runner::index_once(app).await.unwrap();
    assert_eq!(text.documents.load(Ordering::Relaxed), tried, "no retry before the pause ends");
    tokio::time::advance(std::time::Duration::from_secs(6)).await;
    runner::index_once(app).await.unwrap();
    assert!(text.documents.load(Ordering::Relaxed) > tried, "a retry once it ends");
    // Still failing: the next pause is longer.
    let tried = text.documents.load(Ordering::Relaxed);
    tokio::time::advance(std::time::Duration::from_secs(6)).await;
    runner::index_once(app).await.unwrap();
    assert_eq!(text.documents.load(Ordering::Relaxed), tried, "the pause doubled");
    // Choosing the model again (after fixing it, say) tries at once.
    text.fail.store(false, Ordering::Relaxed);
    choose(app, json!({ "text": local("concepts") })).await;
    index_all(app).await;
    assert!(text.documents.load(Ordering::Relaxed) > tried, "no wait after a new choice");
    assert_eq!(ids(&search(app, "dog").await, "related"), ["text"]);
}

/// Indexing reads only pictures already on this Runner; one that arrives later (fetched for a
/// turn) is read then. A picture that is not an image after all never holds up the others.
#[tokio::test]
async fn pictures_are_read_only_once_here_and_a_broken_one_stays_out_of_the_way() {
    let scratch = runner();
    let app = &scratch.0;
    let chat = chat(app);
    let elsewhere = Attachment { id: "att-elsewhere".into(), name: "beach.png".into(), mime: "image/png".into(), size: 1, width: None, height: None };
    say(app, &chat, "elsewhere", 1.0, Body::Text { text: "look".into(), attachments: vec![elsewhere.clone()], mentions: vec![], reply_to: None });
    let broken = Attachment { id: "att-broken".into(), name: "broken.png".into(), mime: "image/png".into(), size: 1, width: None, height: None };
    std::fs::create_dir_all(app.config.files_dir()).unwrap();
    std::fs::write(crate::files::local_path(app, &broken.id).unwrap(), b"not an image at all").unwrap();
    let blue = picture(app, &chat, "blue", 2.0, "blue.png", [10, 10, 230]);
    let Body::Text { attachments, .. } = &blue.body else { unreachable!() };
    let both = vec![broken, attachments[0].clone()];
    say(app, &chat, "mixed", 3.0, Body::Text { text: "two".into(), attachments: both, mentions: vec![], reply_to: None });
    let (_, colours) = fixture(app, "clip", true);
    choose(app, json!({ "image_model_id": "clip" })).await;
    index_all(app).await;
    assert_eq!(colours.0.load(Ordering::Relaxed), 2, "the two readable pictures, and no download");
    assert!(app.store.embedding_progress(crate::local_store::vectors::Role::Image).unwrap().1 == 0, "nothing is stuck");
    assert!(ids(&search(app, "navy").await, "images").contains(&"mixed".to_string()));
    // The missing picture arrives, as `files::ensure_local` brings it for a turn.
    std::fs::write(crate::files::local_path(app, &elsewhere.id).unwrap(), png([10, 10, 230])).unwrap();
    assert_eq!(app.store.requeue_picture(&elsewhere.id).unwrap(), 1);
    index_all(app).await;
    assert!(ids(&search(app, "navy").await, "images").contains(&"elsewhere".to_string()));
}

/// A describer that fails keeps what it described, leaves the picture waiting instead of
/// skipping it for good, and finishes once the provider answers again.
#[tokio::test(start_paused = true)]
async fn a_describer_failure_leaves_the_picture_waiting() {
    let scratch = runner();
    let app = &scratch.0;
    let chat = chat(app);
    picture(app, &chat, "red", 1.0, "red.png", [230, 10, 10]);
    fixture(app, "concepts", false);
    choose(app, json!({ "text": local("concepts") })).await;
    let describer = Arc::new(Describer { answer: "A receipt", calls: AtomicUsize::new(0), failures: AtomicUsize::new(1) });
    *app.search_index.describer_fixture.lock() = Some(describer.clone());
    call(app, "search_index.select", json!({ "describe": { "provider": "fixture", "model": "vision" }, "confirm_external": true })).await.unwrap();
    index_all(app).await;
    assert_eq!(describer.calls.load(Ordering::Relaxed), 1);
    assert!(app.store.descriptions("red").unwrap().is_empty());
    assert_eq!(app.store.describe_backlog().unwrap(), 1, "still waiting, not skipped");
    tokio::time::advance(std::time::Duration::from_secs(6)).await;
    index_all(app).await;
    assert_eq!(app.store.descriptions("red").unwrap().len(), 1);
    assert_eq!(app.store.describe_backlog().unwrap(), 0);
    assert!(search(app, "invoice").await["related"].as_array().unwrap().iter().any(|h| h["kind"] == "image"));
}

#[tokio::test]
async fn queries_hold_at_their_edges() {
    let scratch = runner();
    let app = &scratch.0;
    let chat = chat(app);
    for i in 0..60 {
        say(app, &chat, &format!("m{i:02}"), i as f64, Body::text(format!("note {i} about a puppy")));
    }
    let (text, _) = fixture(app, "concepts", false);
    choose(app, json!({ "text": local("concepts") })).await;
    index_all(app).await;
    let asked = |params: Value| async { call(app, "chats.search_semantic", params).await.unwrap() };
    let blank = asked(json!({ "query": "  \n\t " })).await;
    assert_eq!((blank["state"].as_str(), blank["related"].as_array().unwrap().len()), (Some("ready"), 0));
    assert_eq!(text.queries.load(Ordering::Relaxed), 0, "a blank query embeds nothing");
    assert_eq!(asked(json!({ "query": "dog", "limit": 0 })).await["related"].as_array().unwrap().len(), 1, "at least one");
    assert_eq!(asked(json!({ "query": "dog", "limit": 1000 })).await["related"].as_array().unwrap().len(), 50, "at most fifty");
    assert!(asked(json!({ "query": "dog", "min_score": 5.0 })).await["related"].as_array().unwrap().iter().all(|h| h["score"].as_f64().unwrap() >= 0.999));
    assert!(!asked(json!({ "query": "dog", "min_score": -5.0 })).await["related"].as_array().unwrap().is_empty());
    asked(json!({ "query": "dog ".repeat(2000) })).await;
    assert_eq!(text.longest_query.load(Ordering::Relaxed), 1000, "a long query is cut, not refused");
    assert_eq!(asked(json!({ "query": "🐶🐕 canine" })).await["state"], "ready");
    assert!(call(app, "chats.search_semantic", json!({ "limit": 5 })).await.is_err(), "a query is required");
    assert!(call(app, "chats.search_semantic", json!({ "query": 42 })).await.is_err());
}

#[test]
fn an_answer_counts_as_searched_when_either_space_was() {
    assert!(searched(&json!({ "state": "ready", "image_state": "off" })));
    assert!(searched(&json!({ "state": "off", "image_state": "ready" })));
    assert!(!searched(&json!({ "state": "off", "image_state": "failed" })));
    assert!(!searched(&json!({})));
}

/// A loopback embedding service that records every input it is sent and answers with unit
/// vectors of `dimensions`.
async fn recording_embeddings(model: &'static str, dimensions: usize) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/embeddings", listener.local_addr().unwrap());
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let recorded = recorded.clone();
            tokio::spawn(async move {
                let (mut bytes, mut buf) = (Vec::new(), [0u8; 8192]);
                let end = loop {
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&buf[..n]);
                    if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break at + 4;
                    }
                };
                let head = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                let length: usize = head.lines().find_map(|line| line.strip_prefix("content-length:").and_then(|v| v.trim().parse().ok())).unwrap_or(0);
                while bytes.len() < end + length {
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buf[..n]);
                }
                recorded.lock().unwrap().push(String::from_utf8_lossy(&bytes[end..]).into_owned());
                let body: Value = serde_json::from_slice(&bytes[end..]).unwrap_or(Value::Null);
                let count = body["input"].as_array().map(Vec::len).unwrap_or(0);
                let mut unit = vec![0.0f32; dimensions];
                unit[0] = 1.0;
                let data: Vec<Value> = (0..count).map(|index| json!({ "index": index, "embedding": unit })).collect();
                let answer = json!({ "model": model, "data": data }).to_string();
                let response = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}", answer.len());
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    (url, seen)
}

/// Issue #594: indexed text and queries reach an embedding provider through the secret shield.
#[tokio::test]
async fn an_embedding_provider_is_sent_placeholders_never_secret_values() {
    use base64::Engine;
    use crate::memory_service::{config::Record, types::Revision};
    const KEY: &str = "sk-proj-abcdefghijklmnopqrstuvwxyz0123";
    let scratch = runner();
    let app = &scratch.0;
    app.secrets.seed_environment([("OPENAI_API_KEY", KEY)]);
    let (endpoint, seen) = recording_embeddings("text-embedding-3-small", 4).await;
    let profile: crate::memory_service::types::EmbeddingProfile = serde_json::from_value(json!({
        "mode": "api", "model": "text-embedding-3-small", "revision": "1", "dimensions": 4, "normalization": "l2",
        "distance": "cosine", "document_prefix": "", "query_prefix": "", "endpoint": endpoint, "secret": "fixture-key",
    }))
    .unwrap();
    let revision = Revision { counter: 1, device_id: app.this_device_id().unwrap() };
    app.memory_config.lock().embeddings.insert("openai".into(), Record { revision, value: Some(profile), extra: Default::default() });
    let chat = chat(app);
    say(app, &chat, "m1", 1.0, Body::text(format!("the deploy key is {KEY} and the weather is fine")));
    choose(app, json!({ "text": { "kind": "provider", "profile_id": "openai" }, "confirm_external": true })).await;
    index_all(app).await;
    let found = search(app, &format!("where did I paste {KEY}")).await;
    assert_eq!(found["state"], "ready", "{found}");

    let sent = seen.lock().unwrap().join("\n");
    assert!(sent.contains("the deploy key is {{secret:OPENAI_API_KEY}}"), "the message was indexed with a placeholder: {sent}");
    assert!(sent.contains("where did I paste {{secret:OPENAI_API_KEY}}"), "the query is redacted the same way: {sent}");
    let escaped = serde_json::to_string(KEY).unwrap();
    let percent: String = KEY.bytes().map(|b| format!("%{b:02X}")).collect();
    let base64 = base64::engine::general_purpose::STANDARD.encode(KEY);
    for form in [KEY, escaped.trim_matches('"'), &percent, &base64, "abcdefghijklmnopqrstuvwxyz0123"] {
        assert!(!sent.contains(form), "the secret reached the provider as {form}");
    }
}
