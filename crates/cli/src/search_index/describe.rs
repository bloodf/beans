//! Describing images with a vision-capable chat model the user chose, so the text model finds
//! them in any language it reads ("the receipt from the Lisbon dinner"). Each image is sent to
//! that model's provider once, its description kept with the message (`image_descriptions`),
//! and the tokens counted in the chat's spending like any side call.

use std::sync::Arc;
use std::time::Duration;

use beans_agent::provider::AssistantAccumulator;
use beans_agent::types::{ContentPart, LlmMessage, StopReason, UserMessage};
use beans_agent::{ModelRequest, RequestOptions};
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use super::images;
use super::runner::{image_bytes, Describer};
use crate::app::App;
use crate::local_store::vectors::Queue;

/// Messages described per pass: each image is a paid call, so the backlog drains slowly.
const BATCH: usize = 4;
/// How long one description may take, and the longest kept.
const WAIT: Duration = Duration::from_secs(60);
const MAX_CHARS: usize = 1500;

const PROMPT: &str = "You describe images so that people can find them later by searching in their own words. \
    In two to four plain sentences, say what the image shows: the kind of image (photo, screenshot, diagram, document, \
    receipt, chart…), the main subjects, the place and what is happening, and any visible text, quoted as written. \
    Answer with the description only.";

/// The provider `describer` names, if it takes images.
fn provider(app: &Arc<App>, describer: &Describer) -> Result<Arc<dyn beans_agent::Provider>, String> {
    #[cfg(test)]
    if let Some(fixture) = app.search_index.describer_fixture.lock().clone() {
        return Ok(fixture);
    }
    let provider = crate::providers::provider_for(app, &describer.provider, Some(&describer.model), None)?;
    if !provider.supports_images() {
        return Err("model_without_images".into());
    }
    Ok(provider)
}

/// Whether `describer` can describe images: a provider this Runner can reach and a model that
/// takes images.
pub(super) fn check(app: &Arc<App>, describer: &Describer) -> Result<(), String> {
    provider(app, describer).map(|_| ())
}

/// Describes the images of up to `BATCH` waiting messages that are not described yet.
/// Returns whether there is more to do: more to describe, or descriptions to embed.
pub(super) async fn describe_once(app: &Arc<App>, describer: &Describer, cancel: &CancellationToken) -> Result<bool, String> {
    let store = app.clone();
    let pending = tokio::task::spawn_blocking(move || store.store.pending(Queue::Descriptions, BATCH))
        .await
        .map_err(|_| "describing_failed")?
        .map_err(|error| format!("{error:#}"))?;
    if pending.is_empty() {
        return Ok(false);
    }
    let provider = provider(app, describer)?;
    let model = format!("{}/{}", describer.provider, describer.model);
    let stored = |result: anyhow::Result<bool>| result.map_err(|error| format!("{error:#}"));
    for waiting in &pending {
        let described = app.store.descriptions(&waiting.message.id).map_err(|error| format!("{error:#}"))?;
        let mut descriptions = Vec::new();
        for (_, attachment) in images(&waiting.message) {
            if cancel.is_cancelled() {
                return Ok(false);
            }
            if described.iter().any(|(id, _)| id == &attachment.id) {
                continue;
            }
            // A picture not on this Runner, or not readable, is skipped for good.
            let Some(bytes) = image_bytes(app, attachment).await else { continue };
            match describe(app, &provider, &waiting.message.chat_id, &bytes, cancel).await {
                Ok(Some(text)) => descriptions.push((attachment.id.clone(), text)),
                Ok(None) => {}
                // The provider failed: keep what was described, leave the message waiting, and
                // let the queue back off before it tries again.
                Err(error) => {
                    stored(app.store.save_descriptions(&waiting.message.id, waiting.version, &model, &descriptions, false))?;
                    return Err(error);
                }
            }
        }
        stored(app.store.save_descriptions(&waiting.message.id, waiting.version, &model, &descriptions, true))?;
    }
    // New descriptions queued their messages for the text model: the next pass embeds them.
    Ok(true)
}

/// One image's description, its tokens counted in `chat_id`'s spending: `Ok(None)` when the
/// picture cannot be prepared or the model had nothing to say, `Err` when the provider failed
/// (an error, an abort, or no answer in time), which is worth trying again later.
async fn describe(app: &Arc<App>, provider: &Arc<dyn beans_agent::Provider>, chat_id: &str, bytes: &[u8], cancel: &CancellationToken) -> Result<Option<String>, String> {
    let Ok(image) = beans_agent::images::prepare(bytes) else { return Ok(None) };
    let mut content = image.into_parts();
    content.push(ContentPart::text("Describe this image."));
    let request = ModelRequest {
        system_prompt: PROMPT.into(),
        messages: vec![LlmMessage::User(UserMessage { content, timestamp: beans_agent::now_ms() })],
        tools: Vec::new(),
        cache_points: Vec::new(),
        max_tokens: Some(1024),
        options: RequestOptions::default().with_session_id(chat_id),
    };
    let read = async {
        let mut stream = provider.stream(request, cancel.clone()).await;
        let mut answer = AssistantAccumulator::new(provider.provider_id(), provider.model_id());
        while let Some(event) = stream.next().await {
            answer.apply(&event);
        }
        answer.finish(cancel.is_cancelled())
    };
    let answer = tokio::time::timeout(WAIT, read).await.map_err(|_| "describer_timed_out".to_string())?;
    app.add_side_usage(chat_id, &answer.usage);
    if matches!(answer.stop_reason, StopReason::Aborted | StopReason::Error) {
        return Err(answer.error_message.unwrap_or_else(|| "describer_failed".into()));
    }
    let text: String = answer.text().trim().chars().take(MAX_CHARS).collect();
    Ok((!text.is_empty()).then_some(text))
}
