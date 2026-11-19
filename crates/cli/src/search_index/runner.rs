//! A Runner's side of semantic search: what it embeds with, loading it, the background indexer,
//! and answering queries.
//!
//! Three choices, per Runner and account, each made in Settings › Search:
//! - the text model: an installed local catalog model, or an embedding provider the user
//!   configured (`memory.embeddings.set`), which receives every indexed message;
//! - the image model: an installed local image model, whose vision tower embeds images and
//!   whose text tower embeds queries into the same space;
//! - the describer: a vision-capable chat model that writes what each image shows, which the
//!   text model then embeds. It sends images to that provider and spends tokens.
//!
//! Sending anything off the Runner needs `confirm_external` when chosen. Models are installed
//! through `schedule_models.*` with its consent-bound previews; nothing here downloads.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::{images, related, space_id, text_chunks, Query};
use crate::app::App;
use crate::embeddings::{Embedding, EmbeddingPurpose};
use crate::local_store::vectors::{ChunkVector, Embedded, Pending, Quantized, Queue, Role};
use crate::runner_state::{account, AccountFile, Loader};
use crate::schedule_models::runner::Authority;

/// After a queue fails (a provider down, a model that stopped loading), it waits this long
/// before trying again, doubling up to the longest, so a lasting failure costs a call every few
/// minutes, not every few seconds. The other queues go on.
const RETRY_FIRST: Duration = Duration::from_secs(5);
const RETRY_LONGEST: Duration = Duration::from_secs(600);

/// Messages embedded per pass, and images per pass (each costs a model call of its own size).
const TEXT_BATCH: usize = 32;
const IMAGE_BATCH: usize = 8;
/// Texts per embedding call, within every provider's batch limit.
const ENCODE_BATCH: usize = 64;
/// How often an idle indexer looks for new messages, and its pause while a backlog lasts.
const IDLE: Duration = Duration::from_secs(5);
const BUSY: Duration = Duration::from_millis(25);

/// What this Runner embeds with, for one account.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Settings {
    text: Option<TextSource>,
    image: Option<String>,
    describe: Option<Describer>,
}

impl crate::runner_state::Settings for Settings {
    const PATH: &'static str = "search-index/state.json";
}

/// Where text vectors come from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum TextSource {
    /// An installed catalog model, on this Runner's CPU.
    Local { model_id: String },
    /// An embedding profile in API mode: the provider receives every indexed message.
    Provider { profile_id: String },
}

/// The chat model that describes images: a provider kind and one of its models.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Describer {
    pub provider: String,
    pub model: String,
}

/// A loaded model for one space: the text space, or the image space with the text tower that
/// puts queries into it.
pub(crate) struct Model {
    space: String,
    text: Arc<dyn Embedding>,
    images: Option<Arc<dyn ImageEncoder>>,
}

/// A vision tower: encoded images to unit vectors.
#[async_trait::async_trait]
pub(crate) trait ImageEncoder: Send + Sync {
    async fn embed(&self, images: Vec<Vec<u8>>, cancel: CancellationToken) -> Result<Vec<Vec<f32>>, String>;
}

#[cfg(feature = "embedding-local")]
#[async_trait::async_trait]
impl ImageEncoder for crate::embeddings::image::LocalImageEmbedding {
    async fn embed(&self, images: Vec<Vec<u8>>, cancel: CancellationToken) -> Result<Vec<Vec<f32>>, String> {
        crate::embeddings::image::LocalImageEmbedding::embed(self, images, cancel).await.map_err(|e| e.to_string())
    }
}

/// The Runner's semantic-search state in memory.
#[derive(Default)]
pub struct Runtime {
    settings: AccountFile<Settings>,
    text: Arc<Loader<Model>>,
    image: Arc<Loader<Model>>,
    /// Cancels indexing passes when the account pauses or is forgotten.
    cancel: Mutex<CancellationToken>,
    wake: tokio::sync::Notify,
    /// Per queue: when it may try again after failing, and the wait after the next failure.
    backoff: Mutex<std::collections::HashMap<Queue, (tokio::time::Instant, Duration)>>,
    /// Models tests choose by id, in place of installed catalog models.
    #[cfg(test)]
    pub(crate) fixtures: Mutex<std::collections::HashMap<String, Arc<Model>>>,
    /// The describer tests choose, in place of a configured provider.
    #[cfg(test)]
    pub(crate) describer_fixture: Mutex<Option<Arc<dyn beans_agent::Provider>>>,
}

impl Runtime {
    /// Drops loaded models and cached settings, and stops loads and passes.
    pub fn clear(&self) {
        let mut cancel = self.cancel.lock();
        cancel.cancel();
        *cancel = CancellationToken::new();
        drop(cancel);
        self.settings.forget();
        self.text.reset();
        self.image.reset();
        self.backoff.lock().clear();
    }

    /// Runs one pass of `queue` unless it is waiting out a failure, and records how it went.
    async fn attempt(&self, queue: Queue, pass: impl std::future::Future<Output = Result<bool, String>>) -> bool {
        if self.backoff.lock().get(&queue).is_some_and(|(until, _)| tokio::time::Instant::now() < *until) {
            return false;
        }
        match pass.await {
            Ok(more) => {
                self.backoff.lock().remove(&queue);
                more
            }
            Err(error) => {
                let mut backoff = self.backoff.lock();
                let wait = backoff.get(&queue).map_or(RETRY_FIRST, |(_, last)| (*last * 2).min(RETRY_LONGEST));
                tracing::warn!(%error, ?queue, retry_in = ?wait, "indexing for search");
                backoff.insert(queue, (tokio::time::Instant::now() + wait, wait));
                false
            }
        }
    }

    fn cancel_token(&self) -> CancellationToken {
        self.cancel.lock().child_token()
    }

    /// Has the indexer look for work now.
    pub fn wake(&self) {
        self.wake.notify_one();
    }
}

#[cfg(test)]
impl Model {
    pub(crate) fn fixture(space: String, text: Arc<dyn Embedding>, images: Option<Arc<dyn ImageEncoder>>) -> Model {
        Model { space, text, images }
    }
}

fn settings(app: &App) -> Result<Settings, String> {
    app.search_index.settings.get(app)
}

/// A role's current choice resolved: the key its loaded model is kept under (account, choice,
/// exact files or profile revision) and its space.
struct Choice {
    key: String,
    space: String,
}

fn text_choice(app: &App, source: &TextSource) -> Result<Choice, String> {
    let account = account(app)?;
    match source {
        TextSource::Local { model_id } => {
            #[cfg(test)]
            if let Some(fixture) = app.search_index.fixtures.lock().get(model_id) {
                return Ok(Choice { key: format!("{account}\0fixture\0{model_id}"), space: fixture.space.clone() });
            }
            let (model, _, files) = crate::schedule_models::runner::installed(app, model_id)?;
            if model.kind() != "text" {
                return Err("not_a_text_model".into());
            }
            Ok(Choice { key: format!("{account}\0local\0{model_id}\0{files}"), space: space_id("text", model_id, &files) })
        }
        TextSource::Provider { profile_id } => {
            let (revision, profile) = api_profile(app, profile_id)?;
            let spec = crate::embeddings::EmbeddingSpec::from_profile(&profile).map_err(|_| "invalid_embedding_profile")?;
            let fingerprint = spec.fingerprint().as_str();
            Ok(Choice {
                key: format!("{account}\0provider\0{profile_id}\0{fingerprint}\0{}\0{}", revision.counter, revision.device_id),
                space: space_id("text", &format!("profile:{profile_id}"), fingerprint),
            })
        }
    }
}

fn image_choice(app: &App, model_id: &str) -> Result<Choice, String> {
    let account = account(app)?;
    #[cfg(test)]
    if let Some(fixture) = app.search_index.fixtures.lock().get(model_id) {
        return Ok(Choice { key: format!("{account}\0fixture\0{model_id}"), space: fixture.space.clone() });
    }
    let (model, _, files) = crate::schedule_models::runner::installed(app, model_id)?;
    if model.kind() != "image" {
        return Err("not_an_image_model".into());
    }
    Ok(Choice { key: format!("{account}\0local\0{model_id}\0{files}"), space: space_id("image", model_id, &files) })
}

/// An embedding profile in API mode, with its secret, and its revision.
fn api_profile(app: &App, id: &str) -> Result<(crate::memory_service::types::Revision, crate::memory_service::types::EmbeddingProfile), String> {
    let config = app.memory_config.lock();
    let record = config.embeddings.get(id).ok_or("profile_not_found")?;
    let profile = record.value.clone().ok_or("profile_not_found")?;
    if profile.mode != crate::embeddings::EmbeddingMode::Api {
        return Err("profile_not_api".into());
    }
    Ok((record.revision.clone(), profile))
}

async fn load_text(app: Arc<App>, source: TextSource, space: String, cancel: CancellationToken) -> Result<Model, String> {
    let text: Arc<dyn Embedding> = match source {
        TextSource::Local { model_id } => {
            #[cfg(test)]
            if let Some(fixture) = app.search_index.fixtures.lock().get(&model_id).cloned() {
                return Ok(Model { space, text: fixture.text.clone(), images: None });
            }
            #[cfg(feature = "embedding-local")]
            {
                let (model, assets, files) = crate::schedule_models::runner::installed(&app, &model_id)?;
                Arc::new(crate::schedule_models::engine::CatalogEmbedding::load(&model, &assets, &files, cancel).await.map_err(|e| e.to_string())?)
            }
            #[cfg(not(feature = "embedding-local"))]
            return Err("runtime_unavailable".into());
        }
        TextSource::Provider { profile_id } => {
            let (_, profile) = api_profile(&app, &profile_id)?;
            Arc::new(crate::embeddings::api::ApiEmbedding::new(&profile, false, Default::default()).map_err(|e| e.to_string())?)
        }
    };
    Ok(Model { space, text, images: None })
}

async fn load_image(app: Arc<App>, model_id: String, space: String, cancel: CancellationToken) -> Result<Model, String> {
    #[cfg(test)]
    if let Some(fixture) = app.search_index.fixtures.lock().get(&model_id).cloned() {
        return Ok(Model { space, text: fixture.text.clone(), images: fixture.images.clone() });
    }
    #[cfg(feature = "embedding-local")]
    {
        let (model, assets, _) = crate::schedule_models::runner::installed(&app, &model_id)?;
        let clip = crate::schedule_models::engine::ClipModel::load(&model, &assets, cancel).await.map_err(|e| e.to_string())?;
        Ok(Model { space, text: Arc::new(clip.text), images: Some(Arc::new(clip.images)) })
    }
    #[cfg(not(feature = "embedding-local"))]
    {
        let _ = (model_id, space, cancel);
        Err("runtime_unavailable".into())
    }
}

/// A role's loaded model, or `(state, error)` saying why not, with a load started when due.
fn model(app: &Arc<App>, role: Role) -> Result<Arc<Model>, (&'static str, Option<String>)> {
    let settings = settings(app).map_err(|e| ("off", Some(e)))?;
    let (loader, choice) = match role {
        Role::Text => (&app.search_index.text, settings.text.as_ref().map(|source| text_choice(app, source))),
        Role::Image => (&app.search_index.image, settings.image.as_deref().map(|id| image_choice(app, id))),
    };
    let choice = match choice {
        None => return Err(("off", None)),
        Some(Err(error)) => return Err(("failed", Some(error))),
        Some(Ok(choice)) => choice,
    };
    if let Some(model) = loader.ready(&choice.key) {
        return Ok(model);
    }
    let (app, space) = (app.clone(), choice.space.clone());
    match role {
        Role::Text => {
            let source = settings.text.clone().expect("a text choice");
            loader.start(&choice.key, move |cancel| load_text(app, source, space, cancel));
        }
        Role::Image => {
            let id = settings.image.clone().expect("an image choice");
            loader.start(&choice.key, move |cancel| load_image(app, id, space, cancel));
        }
    }
    match loader.status(&choice.key) {
        "ready" => loader.ready(&choice.key).ok_or(("loading", None)),
        "failed" => Err(("failed", loader.error(&choice.key))),
        _ => Err(("loading", None)),
    }
}

/// Serves `search_index.*` on this Runner for `actor`.
pub async fn serve(app: &Arc<App>, method: &str, params: Value, actor: &str) -> Result<Value, String> {
    let authority = Authority::capture(app, actor)?;
    if !app.local_device().is_some_and(|d| d.is_runner()) {
        return Err("runner_required".into());
    }
    match method {
        "search_index.status" => status(app),
        "search_index.select" => select(app, &authority, &params),
        _ => Err("unknown_search_index_method".into()),
    }
}

/// Each role's choice, whether it is ready, and how far its queue has come, plus what can be
/// chosen: the API-mode embedding profiles. Installable models come from `schedule_models.list`.
pub fn status(app: &Arc<App>) -> Result<Value, String> {
    let settings = settings(app)?;
    let progress = |role| app.store.embedding_progress(role).map_err(|_| "storage_failed".to_string());
    let role = |role: Role, choice: Value| -> Result<Value, String> {
        let (state, error) = match model(app, role) {
            Ok(_) => ("ready", None),
            Err(why) => why,
        };
        let (indexed, pending) = progress(role)?;
        Ok(json!({ "choice": choice, "state": state, "error": error, "indexed": indexed, "pending": if state == "off" { 0 } else { pending } }))
    };
    let profiles: Vec<Value> = app
        .memory_config
        .lock()
        .embeddings
        .iter()
        .filter_map(|(id, record)| record.value.as_ref().filter(|p| p.mode == crate::embeddings::EmbeddingMode::Api).map(|p| (id, p)))
        .map(|(id, profile)| json!({ "id": id, "model": profile.model, "dimensions": profile.dimensions, "has_secret": profile.secret.is_some() }))
        .collect();
    Ok(json!({
        "text": role(Role::Text, json!(settings.text))?,
        "image": role(Role::Image, json!(settings.image))?,
        "describe": { "choice": settings.describe, "pending": app.store.describe_backlog().map_err(|_| "storage_failed")? },
        "profiles": profiles,
        "runs_models": cfg!(feature = "embedding-local"),
    }))
}

/// Changes any of `text` (a `TextSource` or null), `image_model_id` (or null), and `describe` (a
/// `Describer` or null); an omitted key keeps its choice. Choosing a provider or a describer
/// sends data off this Runner and needs `confirm_external: true`. A role turned off deletes its
/// vectors; a new choice embeds everything again in the background.
fn select(app: &Arc<App>, authority: &Authority, params: &Value) -> Result<Value, String> {
    fn read<T: for<'de> Deserialize<'de>>(value: &Value) -> Result<Option<T>, String> {
        serde_json::from_value(value.clone()).map_err(|_| "invalid_request".to_string())
    }
    let confirmed = params["confirm_external"].as_bool().unwrap_or(false);
    let external = |changed: bool| if changed && !confirmed { Err("external_confirmation_required".to_string()) } else { Ok(()) };
    let (_, after) = app.search_index.settings.edit(
        app,
        || authority.check(app),
        |settings| {
            let before = settings.clone();
            if let Some(value) = params.get("text") {
                let text: Option<TextSource> = read(value)?;
                if let Some(source) = &text {
                    text_choice(app, source)?;
                    external(matches!(source, TextSource::Provider { .. }) && settings.text.as_ref() != Some(source))?;
                }
                settings.text = text;
            }
            if let Some(value) = params.get("image_model_id") {
                let id: Option<String> = read(value)?;
                if let Some(id) = &id {
                    image_choice(app, id)?;
                }
                settings.image = id;
            }
            if let Some(value) = params.get("describe") {
                let describe: Option<Describer> = read(value)?;
                if let Some(describer) = &describe {
                    super::describe::check(app, describer)?;
                    external(settings.describe.as_ref() != Some(describer))?;
                }
                settings.describe = describe;
            }
            Ok((before, settings.clone()))
        },
    )?;
    // A role named in the request starts over, even unchanged: choosing a model again is how the
    // user retries one that failed to load (after reinstalling it, say).
    for (role, named, off) in [
        (Role::Text, params.get("text").is_some(), after.text.is_none()),
        (Role::Image, params.get("image_model_id").is_some(), after.image.is_none()),
    ] {
        if named {
            if role == Role::Text { &app.search_index.text } else { &app.search_index.image }.reset();
        }
        if off {
            app.store.set_vector_space(role, None).map_err(|_| "storage_failed")?;
        }
    }
    // A queue whose choice was named tries at once: its pause was for the model it had.
    {
        let mut backoff = app.search_index.backoff.lock();
        for (queue, key) in [(Queue::Text, "text"), (Queue::Images, "image_model_id"), (Queue::Descriptions, "describe")] {
            if params.get(key).is_some() {
                backoff.remove(&queue);
            }
        }
    }
    app.search_index.wake();
    status(app)
}

/// `chats.search_semantic` on this Runner: the text space and the image space, each when ready.
pub async fn semantic(app: &Arc<App>, query: &Query) -> Result<Value, String> {
    let search = |role| async move {
        let model = match model(app, role) {
            Ok(model) => model,
            Err((state, error)) => return (state, Vec::new(), error),
        };
        match search(app, &model, query).await {
            Ok(hits) => ("ready", hits, None),
            Err(error) => ("failed", Vec::new(), Some(error)),
        }
    };
    let (state, related, error) = search(Role::Text).await;
    let (image_state, images, image_error) = search(Role::Image).await;
    Ok(json!({
        "state": state, "related": related, "image_state": image_state, "images": images,
        "runner_id": app.this_device_id(), "error": error.or(image_error),
    }))
}

async fn search(app: &Arc<App>, model: &Model, query: &Query) -> Result<Vec<Value>, String> {
    if query.text.is_empty() {
        return Ok(Vec::new());
    }
    let batch = model.text.embed(EmbeddingPurpose::Query, std::slice::from_ref(&query.text), app.search_index.cancel_token()).await.map_err(|e| e.to_string())?;
    let vector = batch.vectors.into_iter().next().ok_or("embedding_failed")?;
    let (app, space, limit, min_score) = (app.clone(), model.space.clone(), query.limit, query.min_score);
    tokio::task::spawn_blocking(move || related(&app, &space, &vector, limit, min_score)).await.map_err(|_| "search_failed".to_string())?
}

/// The indexer: works through the queues while there is work, then looks again every few
/// seconds or when woken. Runs for the process's life on a Runner.
pub async fn run(app: Arc<App>) {
    loop {
        let more = index_once(&app).await.unwrap_or_else(|error| {
            tracing::warn!(%error, "reading search settings");
            false
        });
        tokio::select! {
            _ = app.search_index.wake.notified() => {}
            _ = tokio::time::sleep(if more { BUSY } else { IDLE }) => {}
        }
    }
}

/// One pass over each queue with a model ready. A queue that fails waits out its backoff
/// while the others go on. Returns whether more may be waiting.
pub(crate) async fn index_once(app: &Arc<App>) -> Result<bool, String> {
    if !app.has_identity() || app.state.lock().unwrap().paused {
        return Ok(false);
    }
    let runtime = &app.search_index;
    let mut more = false;
    if let Ok(model) = model(app, Role::Text) {
        more |= runtime.attempt(Queue::Text, embed_text(app, &model)).await;
    }
    if let Ok(model) = model(app, Role::Image) {
        more |= runtime.attempt(Queue::Images, embed_images(app, &model)).await;
    }
    if let Some(describer) = settings(app)?.describe {
        more |= runtime.attempt(Queue::Descriptions, super::describe::describe_once(app, &describer, &runtime.cancel_token())).await;
    }
    Ok(more)
}

/// Runs `work` on the store off the async threads.
async fn blocking<T: Send + 'static>(app: &Arc<App>, work: impl FnOnce(&App) -> anyhow::Result<T> + Send + 'static) -> Result<T, String> {
    let app = app.clone();
    tokio::task::spawn_blocking(move || work(&app)).await.map_err(|_| "indexing_failed".to_string())?.map_err(|error| format!("{error:#}"))
}

/// The queue of `role`, after making sure the store keeps vectors for `space`.
async fn pending(app: &Arc<App>, role: Role, space: &str, queue: Queue, limit: usize) -> Result<Vec<Pending>, String> {
    let space = space.to_string();
    blocking(app, move |app| {
        if app.store.vector_space(role)?.as_deref() != Some(space.as_str()) {
            app.store.set_vector_space(role, Some(&space))?;
        }
        app.store.pending(queue, limit)
    })
    .await
}

/// Vectors for `chunks[i]` (owned by `owners[i]`, a pending index), grouped per message.
fn group(pending: &[Pending], owners: Vec<(usize, u32)>, vectors: Vec<Vec<f32>>) -> Vec<Embedded> {
    let mut embedded: Vec<Embedded> = pending.iter().map(|p| Embedded { message_id: p.message.id.clone(), version: p.version, chunks: Vec::new() }).collect();
    for ((index, chunk), vector) in owners.into_iter().zip(vectors) {
        // A chunk the model maps to nothing is left out; its message still counts as done.
        if let Some(vector) = Quantized::unit(&vector) {
            embedded[index].chunks.push(ChunkVector { chunk, vector });
        }
    }
    embedded
}

async fn save(app: &Arc<App>, role: Role, space: &str, embedded: Vec<Embedded>) -> Result<(), String> {
    let space = space.to_string();
    blocking(app, move |app| app.store.save_embeddings(role, &space, &embedded)).await.map(|_| ())
}

/// Embeds up to `TEXT_BATCH` waiting messages' text, names, and image descriptions.
async fn embed_text(app: &Arc<App>, model: &Model) -> Result<bool, String> {
    let pending = pending(app, Role::Text, &model.space, Queue::Text, TEXT_BATCH).await?;
    let (mut owners, mut texts) = (Vec::new(), Vec::new());
    for (index, waiting) in pending.iter().enumerate() {
        for (chunk, text) in text_chunks(&waiting.message, &waiting.descriptions) {
            owners.push((index, chunk));
            texts.push(text);
        }
    }
    let cancel = app.search_index.cancel_token();
    let mut vectors = Vec::with_capacity(texts.len());
    for batch in texts.chunks(ENCODE_BATCH) {
        vectors.extend(model.text.embed(EmbeddingPurpose::Document, batch, cancel.clone()).await.map_err(|e| e.to_string())?.vectors);
    }
    if vectors.len() != texts.len() || cancel.is_cancelled() {
        return Err("embedding_failed".into());
    }
    let more = pending.len() == TEXT_BATCH;
    save(app, Role::Text, &model.space, group(&pending, owners, vectors)).await?;
    Ok(more)
}

/// Embeds the images of up to `IMAGE_BATCH` waiting messages. An image that cannot be read
/// (missing, or not an image after all) is left out.
async fn embed_images(app: &Arc<App>, model: &Model) -> Result<bool, String> {
    let Some(encoder) = &model.images else { return Ok(false) };
    let pending = pending(app, Role::Image, &model.space, Queue::Images, IMAGE_BATCH).await?;
    let (mut owners, mut pictures) = (Vec::new(), Vec::new());
    for (index, waiting) in pending.iter().enumerate() {
        for (chunk, attachment) in images(&waiting.message) {
            if let Some(bytes) = image_bytes(app, attachment).await {
                owners.push((index, chunk));
                pictures.push(bytes);
            }
        }
    }
    let cancel = app.search_index.cancel_token();
    let mut vectors = Vec::with_capacity(pictures.len());
    for batch in pictures.chunks(IMAGE_BATCH) {
        vectors.extend(encoder.embed(batch.to_vec(), cancel.clone()).await?);
    }
    if vectors.len() != pictures.len() || cancel.is_cancelled() {
        return Err("embedding_failed".into());
    }
    let more = pending.len() == IMAGE_BATCH;
    save(app, Role::Image, &model.space, group(&pending, owners, vectors)).await?;
    Ok(more)
}

/// An image attachment as a model takes it: converted and turned upright as the agent shows
/// images (`files::inline_image`, kept per attachment), as encoded bytes. Only pictures already
/// on this Runner are read: indexing never downloads a chat's history from the relay. `None`
/// when the picture is not here or is not an image after all.
pub(super) async fn image_bytes(app: &Arc<App>, attachment: &crate::model::Attachment) -> Option<Vec<u8>> {
    use base64::Engine as _;
    if !crate::files::is_local(app, &attachment.id) {
        return None;
    }
    let (app, attachment) = (app.clone(), attachment.clone());
    tokio::task::spawn_blocking(move || {
        let inline = crate::files::inline_image(&app, &attachment).ok()?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(inline.data).ok()?;
        // One undecodable picture must not fail its whole batch, again and again.
        #[cfg(feature = "embedding-local")]
        image::load_from_memory(&bytes).ok()?;
        Some(bytes)
    })
    .await
    .ok()?
}
