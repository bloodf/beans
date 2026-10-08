//! The JSON API every app speaks: `{ method, params }` in, a result or an error out, with the
//! events on the App's bus beside it. The websocket server hands requests here; the phone
//! calls it directly.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::app::App;
use crate::model::*;
#[cfg(feature = "provider-auth")]
use crate::provider_auth;
use crate::{identity, pairing, requests, routines, runtime};

fn string(params: &Value, key: &str) -> Result<String, String> {
    match &params[key] {
        Value::String(text) if !text.is_empty() => Ok(text.clone()),
        Value::Null | Value::String(_) => Err(format!("missing {key}")),
        _ => Err(format!("{key} must be a string")),
    }
}

fn opt_string(params: &Value, key: &str) -> Option<String> {
    params[key].as_str().map(str::to_string).filter(|s| !s.is_empty())
}

/// Missing keeps the generated settings; null resets them independently of a photo.
fn look(params: &Value) -> Result<Option<Option<crate::appearance::BotLook>>, String> {
    match params.get("look") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(value) => crate::appearance::BotLook::parse(value).map(|look| Some(Some(look))),
    }
}

fn capabilities(params: &Value) -> Result<Option<Capabilities>, String> {
    params.get("capabilities").map(|value| {
        let object = value.as_object().ok_or("capabilities must be an object")?;
        for (key, value) in object {
            match key.as_str() {
                "shell" | "write" if value.is_boolean() => {},
                "plugins" if value.is_null() || value.as_array().is_some_and(|ids| ids.iter().all(Value::is_string)) => {},
                _ => return Err(format!("invalid capabilities.{key}")),
            }
        }
        let result: Capabilities = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        result.validate()?;
        Ok(result)
    }).transpose()
}

/// A Runner opens provider OAuth in its browser. A phone emits the URL to the Expo app,
/// whose in-app browser keeps the core alive for the localhost callback.
#[cfg(feature = "provider-auth")]
fn open_provider_auth(app: &Arc<App>, kind: &str, url: &str) -> Result<(), String> {
    #[cfg(feature = "runner")]
    {
        let _ = app;
        let _ = kind;
        open::that(url).map_err(|e| format!("Cannot open the browser: {e}"))
    }
    #[cfg(not(feature = "runner"))]
    {
        app.emit(crate::events::Event::ProviderAuth { kind: kind.to_string(), url: url.to_string() });
        Ok(())
    }
}

/// Parses and validates the avatar before admission, without copying or queuing any bytes.
fn avatar_file(params: &Value) -> Result<Option<Option<crate::files::OutgoingFile>>, String> {
    match params.get("avatar") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(value) => {
            let file: crate::files::OutgoingFile = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
            let source = std::path::Path::new(&file.path);
            let metadata = std::fs::metadata(source).map_err(|e| e.to_string())?;
            if !metadata.is_file() {
                return Err(format!("{} is not a file", file.path));
            }
            if metadata.len() > crate::files::MAX_ATTACHMENT_BYTES {
                return Err(format!("{} is larger than {} MB", file.path, crate::files::MAX_ATTACHMENT_BYTES / 1024 / 1024));
            }
            let name = file.name.as_deref().filter(|name| !name.trim().is_empty())
                .map(std::borrow::Cow::Borrowed)
                .or_else(|| source.file_name().map(|name| name.to_string_lossy()))
                .unwrap_or(std::borrow::Cow::Borrowed("file"));
            let mime = file.mime.as_deref().filter(|mime| mime.contains('/'))
                .unwrap_or_else(|| crate::files::mime_for(&name));
            if !mime.starts_with("image/") {
                return Err(format!("{name} is not an image"));
            }
            Ok(Some(Some(file)))
        }
    }
}

/// Stores a parsed profile image: absent leaves it, null removes it, and a file is copied
/// and queued before the roster names it.
fn store_avatar(app: &Arc<App>, avatar: Option<Option<crate::files::OutgoingFile>>) -> Result<Option<Option<Attachment>>, String> {
    match avatar {
        None => Ok(None),
        Some(None) => Ok(Some(None)),
        Some(Some(file)) => {
            let attachment = crate::files::store(app, &file).map_err(|e| e.to_string())?;
            if !attachment.is_image() {
                let _ = std::fs::remove_file(crate::files::local_path(app, &attachment.id));
                return Err(format!("{} is not an image", attachment.name));
            }
            if let Err(error) = crate::files::push_blob(app, None, &attachment) {
                let _ = std::fs::remove_file(crate::files::local_path(app, &attachment.id));
                return Err(error.to_string());
            }
            Ok(Some(Some(attachment)))
        }
    }
}

pub async fn dispatch(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    match method {
        method if method.starts_with("memory.") && !matches!(method, "memory.read" | "memory.write") =>
            crate::memory_service::api::dispatch(app, method, params).await,
        "hello" => Ok(json!({
            "version": crate::config::VERSION,
            "has_identity": app.has_identity(),
            "is_identity_device": app.is_identity_device(),
            "device_id": app.this_device_id(),
            "relay_url": app.relay_url(),
            "relay_connected": app.relay_connected.load(std::sync::atomic::Ordering::Relaxed),
            "relay_update_required": app.relay_update_required.load(std::sync::atomic::Ordering::Relaxed),
            "relay_error": app.relay_problem.lock().unwrap().clone(),
        })),
        "diagnostics.report" => Ok(crate::diagnostics::report(app).await),
        // An app connecting checks for newer public catalogs, unless checked within the hour.
        "bootstrap" => {
            crate::catalog::check_in_background(app);
            crate::marketplace::check_in_background(app);
            Ok(app.snapshot())
        },

        "identity.create" => {
            let phrase = identity::create(app, opt_string(&params, "device_name")).map_err(|e| e.to_string())?;
            Ok(json!({ "phrase": phrase, "device_id": app.this_device_id() }))
        }
        "identity.restore" => {
            let phrase = string(&params, "phrase")?;
            identity::restore(app, &phrase, opt_string(&params, "device_name")).await.map_err(|e| e.to_string())?;
            Ok(json!({ "device_id": app.this_device_id() }))
        }

        "pair.start" => {
            let (nonce, pairing_string) = pairing::start(app.clone()).await.map_err(|e| e.to_string())?;
            Ok(json!({ "nonce": nonce, "pairing_string": pairing_string }))
        }
        "pair.status" => Ok(pairing::status(app, &string(&params, "nonce")?)),
        "pair.cancel" => {
            pairing::cancel(app, &string(&params, "nonce")?);
            Ok(Value::Null)
        }
        "pair.accept" => {
            let text = string(&params, "pairing_string")?;
            pairing::accept(app.clone(), &text, opt_string(&params, "device_name")).await.map_err(|e| e.to_string())
        }
        "pair.abort" => {
            pairing::abort(app);
            Ok(Value::Null)
        }

        "device.rename" => {
            let name = string(&params, "name")?;
            app.rename_device(&name).map_err(|e| e.to_string())?;
            Ok(json!({ "name": name }))
        }
        // Another Device by id, or this one: the latter is `identity.forget`.
        "device.unpair" => {
            let id = string(&params, "id")?;
            if app.this_device_id().as_deref() == Some(id.as_str()) {
                crate::sync::revoke_self(app).await;
                app.forget_identity().map_err(|e| e.to_string())?;
            } else {
                crate::sync::unpair_device(app, &id).await?;
            }
            Ok(Value::Null)
        }
        // Beans never asks another Device to install from the unadapted upstream feed.
        "device.update" | "device.auto_update" => Err(crate::config::SELF_UPDATE_UNAVAILABLE.into()),
        "identity.forget" => {
            crate::sync::revoke_self(app).await;
            app.forget_identity().map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        // The account goes from the relay first; a relay that cannot be told leaves it in place.
        "identity.delete" => {
            crate::sync::delete_identity(app).await?;
            app.forget_identity().map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        // Onboarding after a pair or a restore: the account's providers, once the first pull
        // has brought its credentials.
        "sync.account" => {
            crate::sync::wait_for_account(app, crate::sync::ACCOUNT_WAIT).await;
            Ok(json!({ "providers": app.credentials.lock().unwrap().statuses() }))
        }
        // The app came back to the foreground: ask the relay again now, not after the backoff.
        "sync.wake" => {
            app.wake_sync();
            Ok(Value::Null)
        }

        // The desktop app names the chat on screen while it is frontmost, null otherwise; a
        // reply the user is watching arrive is not pushed to their phone.
        "ui.watching" => {
            app.set_watched_chat(opt_string(&params, "chat_id"));
            Ok(Value::Null)
        }
        // A phone's APNs or FCM device token, registered with the relay under this machine.
        "push.register" => {
            let (platform, device_token) = (string(&params, "platform")?, string(&params, "token")?);
            let url = app.relay_url().ok_or("No relay configured")?;
            let machine = app.machine_file().and_then(|m| m.machine().ok()).ok_or("Not paired")?;
            let token = crate::sync::token_or_register(app, &url, &machine).await.map_err(|e| e.to_string())?;
            app.relay.put_push_token(&url, &token, &platform, &device_token, params["environment"].as_str()).await.map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "push.unregister" => {
            let url = app.relay_url().ok_or("No relay configured")?;
            let machine = app.machine_file().and_then(|m| m.machine().ok()).ok_or("Not paired")?;
            let token = crate::sync::token_or_register(app, &url, &machine).await.map_err(|e| e.to_string())?;
            app.relay.delete_push_token(&url, &token).await.map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }

        "config.set" => {
            if params.get("relay_url").is_some() {
                let url = params["relay_url"].as_str().map(str::to_string);
                app.set_relay_url(url).map_err(|e| e.to_string())?;
            }
            Ok(json!({ "relay_url": app.relay_url() }))
        }

        "account.pause" => {
            let paused = params.get("paused").and_then(Value::as_bool).ok_or("paused must be a boolean")?;
            app.set_paused(paused);
            Ok(json!({ "paused": paused }))
        }
        // Local update control (`update_control`): status for anyone on this Device, prepare
        // and cancel only with the operator's token.
        "update.status" | "update.prepare" | "update.cancel" => crate::update_control::dispatch(app, method, &params),

        "bots.create" => {
            let look = look(&params)?.flatten();
            // A bot from the marketplace starts from its template's profile, with the routines
            // and the first turn `marketplace::welcome` gives it.
            let template = match opt_string(&params, "template_id") {
                Some(id) => Some(crate::marketplace::template(app, &id).await?),
                None => None,
            };
            // The first turn of a template is new work, admitted below once the fields validate.
            let from_template = |pick: fn(&crate::marketplace::BotTemplate) -> &String| template.as_ref().map(|(t, _)| pick(t).clone());
            let capabilities = capabilities(&params)?.unwrap_or_default();
            let name = opt_string(&params, "name").or_else(|| from_template(|t| &t.name)).ok_or("missing name")?;
            let runner_id = string(&params, "runner_id")?;
            let avatar = avatar_file(&params)?;
            let mut admission = None;
            let bot = Bot {
                id: opt_string(&params, "id").unwrap_or_default(),
                name,
                description: opt_string(&params, "description").or_else(|| from_template(|t| &t.description)).unwrap_or_default(),
                symbol_name: opt_string(&params, "symbol_name").or_else(|| from_template(|t| &t.symbol_name)).unwrap_or_else(|| "sparkles".into()),
                accent: opt_string(&params, "accent").or_else(|| from_template(|t| &t.accent)).unwrap_or_else(|| "indigo".into()),
                avatar: None,
                look,
                runner_id,
                provider: opt_string(&params, "provider").unwrap_or_else(|| "deepseek".into()),
                model: opt_string(&params, "model"),
                thinking: opt_string(&params, "thinking"),
                // Older apps sent a second behavioral field. `insert_bot` folds it into the
                // description, then clears this rolling-upgrade slot.
                legacy_instructions: opt_string(&params, "instructions").unwrap_or_default(),
                workdir: opt_string(&params, "workdir"),
                created_at: 0.0,
                capabilities,
            };
            // Every bot has one direct chat; both land in a single roster change.
            let (bot, chat) = app.create_bot_with_dm_prepared(bot, opt_string(&params, "chat_id"), |bot| {
                // The profile, Runner and ids validate before admission; a refusal stores no avatar.
                if template.is_some() {
                    admission = Some(app.update.try_admit().ok_or_else(|| anyhow::anyhow!(crate::update_control::UPDATING))?);
                }
                bot.avatar = store_avatar(app, avatar).map_err(anyhow::Error::msg)?.flatten();
                Ok(())
            }).map_err(|e| e.to_string())?;
            if let (Some((template, plugins)), Some(admission)) = (template, admission) {
                crate::marketplace::welcome(app, &bot, &chat.meta.id, &template, plugins, opt_string(&params, "greeting"), admission);
            }
            Ok(json!({ "bot": bot, "chat_id": chat.meta.id }))
        }
        "bots.update" => {
            let id = string(&params, "id")?;
            // The image is copied and queued before the roster names it, so every Device can
            // fetch the blob by the time it reads the profile.
            let capabilities = capabilities(&params)?;
            let look = look(&params)?;
            app.bot(&id).ok_or("Unknown bot")?;
            let avatar = store_avatar(app, avatar_file(&params)?)?;
            let bot = app.update_bot(&id, |bot| {
                if let Some(v) = opt_string(&params, "name") { bot.name = v; }
                if let Some(v) = opt_string(&params, "symbol_name") { bot.symbol_name = v; }
                if let Some(v) = opt_string(&params, "accent") { bot.accent = v; }
                if let Some(v) = avatar { bot.avatar = v; }
                if let Some(v) = look { bot.look = v; }
                if let Some(v) = params["description"].as_str() {
                    bot.description = v.trim().to_string();
                } else if let Some(v) = params["instructions"].as_str() {
                    // Compatibility for an older app updating the retired field.
                    bot.description = v.trim().to_string();
                }
                bot.legacy_instructions.clear();
                if let Some(v) = opt_string(&params, "provider") { bot.provider = v; }
                if let Some(v) = params["model"].as_str() { bot.model = Some(v).filter(|m| !m.trim().is_empty()).map(str::to_string); }
                if let Some(v) = params["thinking"].as_str() { bot.thinking = Some(v.trim().to_string()).filter(|t| !t.is_empty()); }
                if let Some(v) = opt_string(&params, "runner_id") { bot.runner_id = v; }
                if let Some(v) = params["workdir"].as_str() { bot.workdir = Some(v.to_string()).filter(|w| !w.trim().is_empty()); }
                if let Some(v) = capabilities { bot.capabilities = v; }
            })
            .map_err(|e| e.to_string())?;
            Ok(json!({ "bot": bot }))
        }
        "bots.delete" => {
            app.delete_bot(&string(&params, "id")?).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }

        "chats.create" => {
            let bot_ids: Vec<String> = serde_json::from_value(params["bot_ids"].clone()).unwrap_or_default();
            let kind = opt_string(&params, "kind").unwrap_or_else(|| "group".into());
            let chat = if kind == "dm" {
                let bot_id = bot_ids.first().cloned().ok_or("missing bot_ids")?;
                app.dm_with(&bot_id, opt_string(&params, "id")).map_err(|e| e.to_string())?
            } else {
                app.create_chat(ChatMeta {
                    id: opt_string(&params, "id").unwrap_or_default(),
                    kind,
                    title: opt_string(&params, "title"),
                    owner_bot_id: opt_string(&params, "owner_bot_id").or_else(|| bot_ids.first().cloned()),
                    description: opt_string(&params, "description").map(|text| text.trim().to_string()),
                    bot_ids,
                    is_pinned: false,
                    created_at: 0.0,
                })
                .map_err(|e| e.to_string())?
            };
            Ok(json!({ "chat": chat }))
        }
        "chats.dm" => {
            let chat = app.dm_with(&string(&params, "bot_id")?, opt_string(&params, "id")).map_err(|e| e.to_string())?;
            Ok(json!({ "chat": chat }))
        }
        "chats.send" => {
            // Every field is checked before the first file is copied. Absent and null mean "not given".
            let chat_id = string(&params, "chat_id")?;
            app.chat(&chat_id).ok_or("Unknown chat")?;
            let optional = |key: &str| match params.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(text)) => Ok(Some(text.clone())),
                Some(_) => Err(format!("{key} must be a string")),
            };
            let text = optional("text")?;
            let (message_id, reply_to) = (optional("message_id")?, optional("reply_to")?);
            // Fixed messages: a serde error could echo the offending value.
            let files: Vec<crate::files::OutgoingFile> = match params.get("attachments") {
                None | Some(Value::Null) => Vec::new(),
                Some(value) => serde_json::from_value(value.clone()).map_err(|_| "attachments must be an array of files")?,
            };
            let mentions: Vec<String> = match params.get("mentions") {
                None | Some(Value::Null) => Vec::new(),
                Some(value) => serde_json::from_value(value.clone()).map_err(|_| "mentions must be an array of strings")?,
            };
            if text.is_none() && files.is_empty() {
                return Err("missing text".into());
            }
            let message = runtime::send_user_message(app.clone(), &chat_id, &text.unwrap_or_default(), message_id, files, mentions, reply_to)
                .map_err(|e| e.to_string())?;
            Ok(json!({ "message": message }))
        }
        "files.path" => {
            // Where the attachment's bytes are on this machine, fetched from the relay first
            // when another Device sent it.
            let attachment: Attachment = serde_json::from_value(params["attachment"].clone()).map_err(|e| e.to_string())?;
            let path = crate::files::ensure_local(app, &attachment).await.map_err(|e| e.to_string())?;
            Ok(json!({ "path": path }))
        }
        "chats.stop" => {
            runtime::cancel_chat(app, &string(&params, "chat_id")?);
            Ok(Value::Null)
        }
        // A message the direct chat's turn holds for its next step: read now. Here when the bot
        // runs here, else sealed to its Runner.
        "chats.send_now" => {
            let chat_id = string(&params, "chat_id")?;
            let message_id = string(&params, "message_id")?;
            let chat = app.chat(&chat_id).ok_or("Unknown chat")?;
            if chat.meta.is_group() {
                return Err("Send now is for a direct chat".into());
            }
            let bot = chat.meta.bot_ids.first().and_then(|id| app.bot(id)).ok_or("The chat has no bot")?;
            if app.this_device_id().as_deref() == Some(bot.runner_id.as_str()) {
                #[cfg(feature = "runner")]
                return crate::turns::send_now(app, &chat_id, &message_id).map(|sent| json!({ "sent": sent }));
            }
            requests::ask(app, &bot.runner_id, "chats.send_now", json!({ "chat_id": chat_id, "message_id": message_id })).await
        }
        #[cfg(feature = "runner")]
        "chats.compact" => {
            let chat_id = string(&params, "chat_id")?;
            app.chat(&chat_id).ok_or("Unknown chat")?;
            // A model call of its own: new work, held back while an update drains this Runner.
            let _running = app.update.try_admit().ok_or(crate::update_control::UPDATING)?;
            let tokens_before = crate::turns::compact_now(app, &chat_id, opt_string(&params, "bot_id").as_deref()).await?;
            Ok(json!({ "tokens_before": tokens_before }))
        }
        // A bot's memory lives on its Runner: read and written here for the bots this Device
        // runs, and through a sealed request to the Runner for the others.
        "bots.memory" => {
            let bot = app.bot(&string(&params, "bot_id")?).ok_or("Unknown bot")?;
            let runner = app.device(&bot.runner_id).map(|d| d.name).unwrap_or_else(|| "its Runner".into());
            let here = app.this_device_id().as_deref() == Some(bot.runner_id.as_str());
            let mut overview = if here {
                requests::memory_read(app, &bot.id)?
            } else {
                requests::ask(app, &bot.runner_id, "memory.read", json!({ "bot_id": bot.id })).await?
            };
            overview["bot_id"] = json!(bot.id);
            overview["here"] = json!(here);
            overview["runner"] = json!(runner);
            Ok(overview)
        }
        "bots.memory.write" => {
            let bot = app.bot(&string(&params, "bot_id")?).ok_or("Unknown bot")?;
            let text = params["text"].as_str().ok_or("missing text")?;
            let expected_hash = opt_string(&params, "expected_hash");
            if app.this_device_id().as_deref() == Some(bot.runner_id.as_str()) {
                requests::memory_write(app, &bot.id, text, expected_hash.as_deref())
            } else {
                requests::ask(app, &bot.runner_id, "memory.write", json!({ "bot_id": bot.id, "text": text, "expected_hash": expected_hash })).await
            }
        }
        "chats.delete" => {
            app.delete_chat(&string(&params, "chat_id")?);
            Ok(Value::Null)
        }
        "chats.rename" => {
            let title = params["title"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            app.rename_chat(&string(&params, "chat_id")?, title).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "chats.set_description" => {
            let description = params["description"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            app.describe_chat(&string(&params, "chat_id")?, description).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "chats.pin" => {
            let pinned = params["pinned"].as_bool();
            app.update_chat_meta(&string(&params, "chat_id")?, |meta| meta.is_pinned = pinned.unwrap_or(!meta.is_pinned)).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "chats.add_bot" => {
            let chat_id = string(&params, "chat_id")?;
            let bot_id = string(&params, "bot_id")?;
            let bot = app.bot(&bot_id).ok_or("Unknown bot")?;
            app.update_chat_meta(&chat_id, |meta| {
                if meta.is_group() && meta.bot_ids.len() < MAX_GROUP_BOTS && !meta.bot_ids.contains(&bot_id) {
                    meta.bot_ids.push(bot_id.clone());
                }
            })
            .map_err(|e| e.to_string())?;
            app.notice(&chat_id, format!("{} joined the chat.", bot.name));
            Ok(Value::Null)
        }
        "chats.remove_bot" => {
            let chat_id = string(&params, "chat_id")?;
            let bot_id = string(&params, "bot_id")?;
            app.update_chat_meta(&chat_id, |meta| {
                if meta.is_group() && meta.bot_ids.len() > 1 {
                    meta.bot_ids.retain(|id| id != &bot_id);
                    if meta.owner_bot_id.as_deref() == Some(bot_id.as_str()) {
                        meta.owner_bot_id = meta.bot_ids.first().cloned();
                    }
                }
            })
            .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "chats.set_owner" => {
            let chat_id = string(&params, "chat_id")?;
            let bot_id = string(&params, "bot_id")?;
            let chat = app.chat(&chat_id).ok_or("Unknown chat")?;
            if !chat.meta.is_group() {
                return Err("Only a group has an owner".into());
            }
            if !chat.meta.bot_ids.contains(&bot_id) {
                return Err(format!("{} is not in {}", crate::runtime::name_of(app, &bot_id), app.chat_title(&chat.meta)));
            }
            app.update_chat_meta(&chat_id, |meta| meta.owner_bot_id = Some(bot_id.clone())).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "chats.search" => {
            let query = string(&params, "query")?;
            let limit = params["limit"].as_u64().unwrap_or(20).clamp(1, 50) as usize;
            let terms = crate::local_store::search_terms(&query);
            if terms.is_empty() {
                return Ok(json!({ "chats": [], "messages": [] }));
            }
            let (chat_ids, chats) = {
                let state = app.state.lock().unwrap();
                let bots: std::collections::HashMap<&str, &Bot> =
                    state.bots.iter().map(|bot| (bot.id.as_str(), bot)).collect();
                let ids: std::collections::HashSet<String> =
                    state.chats.iter().map(|chat| chat.meta.id.clone()).collect();
                let chats = state
                    .chats
                    .iter()
                    .filter_map(|chat| {
                        let mut parts = chat.meta.title.clone().into_iter().collect::<Vec<_>>();
                        for id in &chat.meta.bot_ids {
                            if let Some(bot) = bots.get(id.as_str()) {
                                parts.extend([bot.name.clone(), bot.description.clone()]);
                            }
                        }
                        let text = parts.join(" ");
                        crate::local_store::search_matches(&text, &terms).then(|| {
                            json!({
                                "chat_id": chat.meta.id,
                                "snippet": crate::local_store::search_snippet(&text, &terms),
                            })
                        })
                    })
                    .take(limit)
                    .collect::<Vec<_>>();
                (ids, chats)
            };
            let messages: Vec<Value> = app
                .store
                .search_messages(&query, limit)
                .map_err(|error| error.to_string())?
                .into_iter()
                .filter(|hit| chat_ids.contains(&hit.chat_id))
                .map(|hit| {
                    json!({
                        "chat_id": hit.chat_id,
                        "message_id": hit.message_id,
                        "snippet": hit.snippet,
                        "author": hit.author,
                        "created_at": hit.created_at,
                    })
                })
                .collect();
            Ok(json!({ "chats": chats, "messages": messages }))
        }
        // Older messages than the snapshot carried, a page at a time, oldest first.
        "chats.messages" => {
            let chat_id = string(&params, "chat_id")?;
            if app.chat(&chat_id).is_none() {
                return Err("No such chat".into());
            }
            let limit = params["limit"].as_u64().unwrap_or(SNAPSHOT_MESSAGES as u64).clamp(1, 200) as usize;
            let before = params["before"].as_str();
            let mut page = app.message_page(&chat_id, before, limit);
            // The end of what is here, with more on the relay: read a page back and look again.
            // A relay that cannot be reached ends the chat here for now.
            if page.0.len() < limit && app.history_is_partial(&chat_id) {
                match crate::sync::older_messages(app, &chat_id).await {
                    Ok(_) => page = app.message_page(&chat_id, before, limit),
                    Err(error) => {
                        tracing::warn!(%error, %chat_id, "fetching older messages");
                        page.1 = false;
                    }
                }
            }
            let (messages, has_more) = page;
            Ok(json!({ "messages": messages, "has_more": has_more }))
        }
        "chats.mark_read" => {
            app.mark_read(&string(&params, "chat_id")?, true);
            Ok(Value::Null)
        }

        // Routines live in the roster; any Device edits them, the bot's Runner runs them. A
        // routine's check is the bot's to write, on its Runner, with the routines tool.
        "routines.create" => {
            let routine = routines::create(
                app,
                &string(&params, "bot_id")?,
                &string(&params, "name")?,
                &string(&params, "schedule")?,
                params["prompt"].as_str().unwrap_or(""),
                None,
                params["enabled"].as_bool().unwrap_or(true),
            )?;
            Ok(json!({ "routine": app.routine_out(&routine) }))
        }
        "routines.update" => {
            let id = string(&params, "id")?;
            let mut routine = app.routine(&id).ok_or("Unknown routine")?;
            if params.get("name").is_some() || params.get("schedule").is_some() || params.get("prompt").is_some() {
                routine = routines::edit(app, &id, opt_string(&params, "name").as_deref(), opt_string(&params, "schedule").as_deref(), params["prompt"].as_str(), None)?;
            }
            if let Some(enabled) = params["enabled"].as_bool() {
                routine = routines::set_enabled(app, &id, enabled)?;
            }
            Ok(json!({ "routine": app.routine_out(&routine) }))
        }
        "routines.delete" => {
            routines::delete(app, &string(&params, "id")?)?;
            Ok(Value::Null)
        }
        "routines.run" => {
            routines::run_now(app, &string(&params, "id")?)?;
            Ok(Value::Null)
        }
        "routines.describe" => routines::describe(&string(&params, "schedule")?),

        // The marketplace: plugins, each with the Runners that have it, and bots to add from a
        // template (`bots.create { template_id }`).
        "marketplace" => {
            let query = opt_string(&params, "query").unwrap_or_default();
            let mut index = crate::marketplace::index(app).await;
            if !query.trim().is_empty()
                && crate::marketplace::search_plugins(&index.plugins, &query).is_empty()
                && crate::marketplace::search_bots(&index.bots, &query).is_empty()
                && crate::marketplace::check_for_missing(app).await {
                index = crate::marketplace::current(app);
            }
            let installed_on: Vec<(String, Vec<crate::model::PluginStatus>)> =
                app.state.lock().unwrap().devices.iter().map(|d| (d.id.clone(), d.plugins.clone())).collect();
            let plugins: Vec<Value> = crate::marketplace::search_plugins(&index.plugins, &query)
                .into_iter()
                .map(|m| {
                    let mut out = serde_json::to_value(m).unwrap_or_default();
                    // A server from a Runner's mcp.json that happens to share the id is not this plugin.
                    out["installed_on"] = json!(installed_on.iter().filter(|(_, p)| p.iter().any(|s| s.id == m.id && s.source != crate::plugins::mcp_json::SOURCE)).map(|(id, _)| id.clone()).collect::<Vec<_>>());
                    out
                })
                .collect();
            let bots: Vec<Value> = crate::marketplace::search_bots(&index.bots, &query)
                .into_iter()
                .map(|t| {
                    let mut out = serde_json::to_value(t).unwrap_or_default();
                    // The apps word a routine's schedule from the CLI's sentence, as a bot's own.
                    for routine in out["routines"].as_array_mut().into_iter().flatten() {
                        let text = routine["schedule"].as_str().and_then(|s| crate::schedule::parse(s).ok()).map(|s| s.describe());
                        routine["schedule_text"] = json!(text);
                    }
                    out
                })
                .collect();
            Ok(json!({ "plugins": plugins, "bots": bots }))
        }
        // Checks the configured marketplace feed now, even within the last check's hour.
        "marketplace.reload" => {
            let changed = crate::marketplace::check(app, true).await?;
            Ok(json!({ "updated": crate::marketplace::current(app).updated, "changed": changed }))
        }
        "mcp.parse" => crate::plugins::mcp_json::parse_reply(&string(&params, "text")?),
        verb if verb.starts_with("mcp.") => {
            let runner_id = opt_string(&params, "runner_id");
            Box::pin(crate::plugins::mcp_json::on_runner(app, runner_id.as_deref(), verb, params)).await
        }
        // Plugins are installed per Runner, here or through a sealed request to that Runner.
        "plugins.install" => {
            let runner_id = string(&params, "runner_id")?;
            let manifest = match params.get("manifest") {
                Some(value) => crate::plugins::Manifest::parse(value)?,
                None => {
                    let id = string(&params, "plugin_id")?;
                    let mut index = crate::marketplace::index(app).await;
                    if index.plugin(&id).is_none() && crate::marketplace::check_for_missing(app).await {
                        index = crate::marketplace::current(app);
                    }
                    index.plugin(&id).cloned().ok_or_else(|| format!("No plugin {id} in the marketplace"))?
                }
            };
            let source = if params.get("plugin_id").is_some() { "marketplace" } else { "inline" };
            let body = json!({ "manifest": manifest, "source": source });
            let status = crate::plugins::on_runner(app, &runner_id, "plugins.install", body).await?;
            Ok(json!({ "status": status }))
        }
        "plugins.uninstall" => {
            let runner_id = string(&params, "runner_id")?;
            crate::plugins::on_runner(app, &runner_id, "plugins.uninstall", json!({ "plugin_id": string(&params, "plugin_id")? })).await
        }
        "plugins.set_variables" => {
            let runner_id = string(&params, "runner_id")?;
            let body = json!({ "plugin_id": string(&params, "plugin_id")?, "variables": params["variables"] });
            let status = crate::plugins::on_runner(app, &runner_id, "plugins.variables", body).await?;
            Ok(json!({ "status": status }))
        }
        // On another Runner, the sign-in page opens here (`plugins::sign_in`).
        "plugins.connect" => {
            let runner_id = string(&params, "runner_id")?;
            let plugin_id = string(&params, "plugin_id")?;
            let body = json!({ "plugin_id": plugin_id, "server": opt_string(&params, "server") });
            if app.this_device_id().as_deref() != Some(runner_id.as_str()) {
                let plugin = app.device(&runner_id).and_then(|device| device.plugins.into_iter().find(|p| p.id == plugin_id));
                let name = plugin.map(|p| p.name).unwrap_or_else(|| plugin_id.clone());
                return crate::plugins::sign_in::from_here(app, &runner_id, "plugins.connect", body, &plugin_id, &name).await;
            }
            crate::plugins::on_runner(app, &runner_id, "plugins.connect", body).await
        }
        // The phone's page for sign-in `sign_in` closed before the browser came back.
        "plugins.auth.cancel" => {
            app.cancel_plugin_sign_in(opt_string(&params, "sign_in").as_deref());
            Ok(Value::Null)
        }
        "plugins.sign_out" => {
            let runner_id = string(&params, "runner_id")?;
            crate::plugins::on_runner(app, &runner_id, "plugins.sign_out", json!({ "plugin_id": string(&params, "plugin_id")?, "server": opt_string(&params, "server") })).await
        }
        "plugins.detail" => {
            let runner_id = string(&params, "runner_id")?;
            crate::plugins::on_runner(app, &runner_id, "plugins.detail", json!({ "plugin_id": string(&params, "plugin_id")? })).await
        }
        // Auto-review: the check on plugin and shell actions, shared through the roster.
        // `rules` replaces the list; a rule without an id gets one.
        "auto_review.set" => {
            let mut auto_review = app.auto_review();
            if let Some(enabled) = params["is_enabled"].as_bool() {
                auto_review.is_enabled = enabled;
            }
            if let Some(rules) = params["rules"].as_array() {
                auto_review.rules = rules
                    .iter()
                    .filter_map(|r| {
                        let text = r["text"].as_str()?.trim().to_string();
                        if text.is_empty() {
                            return None;
                        }
                        let behavior = if r["behavior"].as_str() == Some("ask") { "ask" } else { "allow" };
                        Some(AutoReviewRule {
                            id: r["id"].as_str().filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                            text,
                            behavior: behavior.into(),
                            tool: r["tool"].as_str().map(str::to_string),
                        })
                    })
                    .collect();
            }
            app.set_auto_review(auto_review.clone());
            Ok(json!({ "auto_review": auto_review }))
        }
        // The user answered a permission card: here when the bot runs here, else sealed to
        // its Runner.
        "chats.permission" => {
            let chat_id = string(&params, "chat_id")?;
            let message_id = string(&params, "message_id")?;
            let decision = string(&params, "decision")?;
            let message = app.message(&chat_id, &message_id).ok_or("Unknown message")?;
            if matches!(&message.body, Body::Permission { tool, .. } if tool == "propose") && matches!(decision.as_str(), "always") {
                return Err("Always allow is unavailable for proposals".into());
            }
            let Author::Bot { bot_id } = &message.author else { return Err("Not a permission request".into()) };
            let bot = app.bot(bot_id).ok_or("Unknown bot")?;
            let body = json!({ "chat_id": chat_id, "message_id": message_id, "decision": decision });
            // Sign in on a card for a bot on another Runner: the sign-in page opens here.
            if let Body::Permission { tool, plugin_id, plugin_name, decision: current, .. } = &message.body {
                let signs_in = tool == "connect" && current == "pending" && decision != "deny";
                if signs_in && app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
                    return crate::plugins::sign_in::from_here(app, &bot.runner_id, "permission.answer", body, plugin_id, plugin_name).await;
                }
            }
            crate::plugins::on_runner(app, &bot.runner_id, "permission.answer", body).await
        }

        // A command running in its terminal: the user's answer goes to it from its card, or it
        // stops. Here when the bot runs here, else sealed to its Runner. The text is never kept.
        "bash.stdin" | "bash.stop" | "bash.background" => {
            let chat_id = string(&params, "chat_id")?;
            let message_id = string(&params, "message_id")?;
            let message = app.message(&chat_id, &message_id).ok_or("Unknown message")?;
            let Author::Bot { bot_id } = &message.author else { return Err("That row has no command waiting".into()) };
            let bot = app.bot(bot_id).ok_or("Unknown bot")?;
            let body = json!({ "chat_id": chat_id, "message_id": message_id, "text": params["text"], "enter": params["enter"] });
            if app.this_device_id().as_deref() == Some(bot.runner_id.as_str()) {
                #[cfg(feature = "runner")]
                return crate::shell::serve(app, method, &body).await;
            }
            requests::ask(app, &bot.runner_id, method, body).await
        }

        // Read one API key on demand for the local settings editor. Snapshots and events
        // continue to carry masked provider statuses.
        "providers.api_key" => {
            let kind = string(&params, "kind")?;
            let credentials = app.credentials.lock().unwrap();
            if crate::credentials::is_custom(&kind) {
                let provider = credentials.custom.get(&kind).ok_or("Unknown provider")?;
                return Ok(json!({ "api_key": provider.api_key, "base_url": provider.base_url }));
            }
            if !matches!(kind.as_str(), "deepseek" | "anthropic" | "opencode" | "opencode-go") {
                return Err("Not an API-key provider".into());
            }
            let credential = credentials.api_key(&kind);
            Ok(json!({
                "api_key": credential.map(|c| &c.api_key),
                "base_url": credential.and_then(|c| c.base_url.as_ref()),
            }))
        }
        #[cfg(feature = "provider-auth")]
        "providers.connect_deepseek" => {
            provider_auth::connect_deepseek(app, &string(&params, "api_key")?, opt_string(&params, "base_url").as_deref()).await?;
            Ok(json!({ "providers": app.credentials.lock().unwrap().statuses() }))
        }
        #[cfg(feature = "provider-auth")]
        "providers.connect_anthropic" => {
            provider_auth::connect_anthropic(app, &string(&params, "api_key")?, opt_string(&params, "base_url").as_deref()).await?;
            Ok(json!({ "providers": app.credentials.lock().unwrap().statuses() }))
        }
        #[cfg(feature = "provider-auth")]
        "providers.connect_opencode" => {
            provider_auth::connect_opencode(app, &string(&params, "api_key")?, opt_string(&params, "base_url").as_deref()).await?;
            Ok(json!({ "providers": app.credentials.lock().unwrap().statuses() }))
        }
        #[cfg(feature = "provider-auth")]
        "providers.connect_opencode_go" => {
            provider_auth::connect_opencode_go(app, &string(&params, "api_key")?, opt_string(&params, "base_url").as_deref()).await?;
            Ok(json!({ "providers": app.credentials.lock().unwrap().statuses() }))
        }
        // The chat models a custom provider's server lists, for the model picker.
        #[cfg(feature = "provider-auth")]
        "providers.refresh" => {
            if params.get("runner_id").is_some() { return provider_auth::guided_request(app, "providers.custom.refresh", params).await; }
            let updated = provider_auth::refresh_custom_models(app).await?;
            Ok(json!({ "updated": updated }))
        }
        #[cfg(feature = "provider-auth")]
        "providers.list_models" => {
            if params.get("runner_id").is_some() { return provider_auth::guided_request(app, "providers.custom.preview", params).await; }
            provider_auth::capability_input(&params)?;
            let str_param = |key: &str| params[key].as_str().unwrap_or_default().to_string();
            let listed = provider_auth::list_custom_models(app, &str_param("name"), &str_param("api"), &str_param("base_url"), &str_param("api_key"), opt_string(&params, "integration").as_deref()).await?;
            Ok(json!({ "listed": listed.is_some(), "models": listed.unwrap_or_default() }))
        }
        // Adds a custom provider, or saves one with `kind`, once its server answers.
        #[cfg(feature = "provider-auth")]
        "providers.connect_custom" => {
            if params.get("runner_id").is_some() { return provider_auth::guided_request(app, "providers.custom.save", params).await; }
            let input = provider_auth::CustomInput {
                kind: opt_string(&params, "kind"),
                integration: opt_string(&params, "integration"),
                name: params["name"].as_str().unwrap_or_default().to_string(),
                api: opt_string(&params, "api").unwrap_or_else(|| "chat-completions".into()),
                base_url: params["base_url"].as_str().unwrap_or_default().to_string(),
                api_key: params["api_key"].as_str().unwrap_or_default().to_string(),
                models: params["models"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect(),
                capabilities: provider_auth::capability_input(&params)?,
            };
            let kind = provider_auth::connect_custom(app, input).await?;
            Ok(json!({ "kind": kind, "providers": app.credentials.lock().unwrap().statuses() }))
        }
        #[cfg(feature = "provider-auth")]
        "providers.connect_chatgpt" => {
            let cancel = app.begin_provider_auth();
            let opener = app.clone();
            let login = provider_auth::connect_chatgpt(app, move |url| open_provider_auth(&opener, "chatgpt", url));
            let tokens = tokio::select! {
                result = login => result?,
                _ = cancel.cancelled() => return Err("Sign-in cancelled".into()),
            };
            Ok(json!({ "email": tokens.email, "providers": app.credentials.lock().unwrap().statuses() }))
        }
        #[cfg(feature = "provider-auth")]
        "providers.connect_grok" => {
            let cancel = app.begin_provider_auth();
            let opener = app.clone();
            let login = provider_auth::connect_grok(app, move |url| open_provider_auth(&opener, "grok", url));
            let tokens = tokio::select! {
                result = login => result?,
                _ = cancel.cancelled() => return Err("Sign-in cancelled".into()),
            };
            Ok(json!({ "email": tokens.email, "providers": app.credentials.lock().unwrap().statuses() }))
        }
        #[cfg(feature = "provider-auth")]
        "providers.auth.cancel" => {
            app.cancel_provider_auth();
            Ok(Value::Null)
        }
        #[cfg(feature = "provider-auth")]
        "providers.disconnect" => {
            app.cancel_provider_auth();
            provider_auth::disconnect(app, &string(&params, "kind")?)?;
            Ok(json!({ "providers": app.credentials.lock().unwrap().statuses() }))
        }
        // Checks the configured model catalog now, even within the last check's hour.
        "models.reload" => {
            let changed = crate::catalog::check(app, true).await?;
            Ok(json!({ "updated": beans_models::updated(), "changed": changed }))
        }

        other => Err(format!("unknown method {other}")),
    }
}
