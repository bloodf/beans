//! Validated local and sealed-Runner memory boundary; secrets never enter status replies.
use std::{collections::BTreeMap, sync::Arc};
use serde::Deserialize;
use serde_json::{json, Value};
use crate::app::App;
use super::{config::{Record, preferences_summary}, types::*};

fn decode<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|_| "invalid_memory_request".into())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionSet {
    id: String, backend: BackendKind, name: String, #[serde(default)] endpoint: Option<String>,
    secret: SecretPatch, #[serde(default)] embedding_profile: Option<String>,
    #[serde(default)] allow_insecure_http: bool, #[serde(default)] options: Option<OptionsPatch>,
}
#[derive(Deserialize)]
#[serde(tag = "backend", rename_all = "snake_case", deny_unknown_fields)]
enum OptionsPatch {
    Hindsight {},
    OpenViking {
        bindings: BTreeMap<String, Option<BindingPatch>>,
        #[serde(default)] replace_all: bool,
        #[serde(default)] confirm_replace_all: bool,
    },
    Pgvector { schema: String, #[serde(default)] role: Option<String> }, LanceDb { region: Option<String> },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BindingPatch { mode: OpenVikingAuthMode, account_id: String, user_id: String, secret: SecretPatch }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Id { id: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbeddingSet { id: String, profile: EmbeddingProfile, secret: SecretPatch }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BotId { bot_id: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreferencesSet {
    bot_id: String,
    // Value distinguishes omitted from explicit null, without accepting client revisions.
    #[serde(default)] connection_id: Option<Value>,
    auto_recall: bool, capture_conversation: bool, capture_group_text: bool,
    #[serde(default)] unattended_capture: bool,
    #[serde(default)] max_capture_deliveries_per_turn: Option<u32>,
    #[serde(default)] recall_budget: Option<RecallBudget>,
}

fn options(app: &App, patch: OptionsPatch, old: Option<&BackendOptions>) -> Result<BackendOptions, MemoryError> {
    Ok(match patch {
        OptionsPatch::Hindsight {} => BackendOptions::Hindsight {},
        OptionsPatch::Pgvector { schema, role } => BackendOptions::Pgvector { schema, role },
        OptionsPatch::LanceDb { region } => BackendOptions::LanceDb { region },
        OptionsPatch::OpenViking { bindings, replace_all, confirm_replace_all } => {
            if replace_all && !confirm_replace_all { return Err(MemoryError::new("confirmation_required")); }
            let account = app.machine_file().ok_or_else(|| MemoryError::new("identity_required"))?;
            let mut resolved = if replace_all { BTreeMap::new() } else { match old {
                Some(BackendOptions::OpenViking { bindings }) => bindings.clone(), _ => BTreeMap::new(),
            }};
            for (bot_id, patch) in bindings {
                if app.bot(&bot_id).is_none() { return Err(MemoryError::new("bot_deleted")); }
                let namespace = namespace(&account.identity_pubkey, &bot_id)?;
                let Some(patch) = patch else { resolved.remove(&namespace); continue; };
                let previous = match old { Some(BackendOptions::OpenViking { bindings }) => bindings.get(&namespace), _ => None };
                let same = previous.is_some_and(|b| b.mode == patch.mode && b.account_id == patch.account_id && b.user_id == patch.user_id);
                if !same && matches!(patch.secret, SecretPatch::Keep) { return Err(MemoryError::new("binding_secret_required")); }
                let mut secret = previous.filter(|_| same).map(|b| b.api_key.clone());
                patch.secret.apply(&mut secret)?;
                resolved.insert(namespace, OpenVikingBinding { mode: patch.mode, account_id: patch.account_id, user_id: patch.user_id,
                    api_key: secret.unwrap_or_default() });
            }
            BackendOptions::OpenViking { bindings: resolved }
        },
    })
}

pub async fn dispatch(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    if method == "memory.connections.list" { return Ok(app.memory_summary()); }
    if is_setup_method(method) {
        let runner_id = if method.starts_with("memory.embeddings.local.") {
            params["runner_id"].as_str().ok_or("runner_id_required")?.to_owned()
        } else {
            app.bot(params["bot_id"].as_str().ok_or("missing_bot_id")?).ok_or("bot_deleted")?.runner_id
        };
        let runner = app.device(&runner_id).or_else(|| app.local_device().filter(|d|d.id==runner_id)).ok_or("runner_unknown")?;
        if !runner.is_runner() { return Err("runner_required".into()); }
        let mut body = params;
        body.as_object_mut().ok_or("invalid_setup_request")?.remove("runner_id");
        if app.this_device_id().as_deref() != Some(&runner_id) { return crate::requests::ask_within(app,&runner_id,method,body,std::time::Duration::from_secs(330)).await; }
        return serve_as(app,method,body,&app.this_device_id().ok_or("identity_required")?).await;
    }
    if method.starts_with("memory.service.") || method.starts_with("memory.operations.") {
        let bot_id = params.get("bot_id").and_then(Value::as_str).ok_or("missing_bot_id")?;
        let bot = app.bot(bot_id).ok_or("bot_deleted")?;
        if app.this_device_id().as_deref() != Some(&bot.runner_id) {
            return crate::requests::ask(app, &bot.runner_id, method, params).await;
        }
        return serve(app, method, params).await;
    }
    let result: Result<Value, MemoryError> = match method {
        "memory.connections.set" => {
            let patch: ConnectionSet = decode(params.clone())?;
            let _edit = app.roster_edit.lock().unwrap();
            let mut current = app.memory_config.lock(); let mut next = current.clone();
            let old = next.connections.get(&patch.id).and_then(|r| r.value.as_ref());
            let mut secret = old.and_then(|c| c.secret.clone()); patch.secret.apply(&mut secret).map_err(|e| e.code)?;
            let backend_options = match patch.options {
                Some(p) => Some(options(app, p, old.and_then(|c| c.options.as_ref())).map_err(|e| e.code)?),
                None => old.and_then(|c| c.options.clone()),
            };
            let endpoint = if params.get("endpoint").is_some() { patch.endpoint } else { old.and_then(|c| c.endpoint.clone()) };
            let embedding_profile = if params.get("embedding_profile").is_some() { patch.embedding_profile } else { old.and_then(|c| c.embedding_profile.clone()) };
            let allow_insecure_http = if params.get("allow_insecure_http").is_some() { patch.allow_insecure_http } else { old.is_some_and(|c| c.allow_insecure_http) };
            let c = Connection { backend: patch.backend, name: patch.name, endpoint, secret,
                embedding_profile, allow_insecure_http,
                options: backend_options, extra: old.map(|c| c.extra.clone()).unwrap_or_default() };
            next.set_connection(&patch.id, Some(c), &app.this_device_id().ok_or("identity_required")?).map_err(|e| e.code)?;
            app.persist_memory(&next, true, None).map_err(|e| e.code)?; *current = next;
            Ok(json!({"saved":true,"setup_required":true}))
        },
        "memory.connections.disconnect" => {
            let Id { id } = decode(params)?;
            let _edit = app.roster_edit.lock().unwrap();
            let mut current = app.memory_config.lock(); let mut next = current.clone();
            next.set_connection(&id, None, &app.this_device_id().ok_or("identity_required")?).map_err(|e| e.code)?;
            app.persist_memory(&next, true, None).map_err(|e| e.code)?; *current = next; Ok(json!({"saved":true}))
        },
        "memory.embeddings.set" => {
            let mut patch: EmbeddingSet = decode(params)?;
            if patch.profile.secret.is_some() { return Err("secret_patch_required".into()); }
            let _edit = app.roster_edit.lock().unwrap();
            let mut current = app.memory_config.lock(); let mut next = current.clone();
            super::config::valid_id(&patch.id).map_err(|e| e.code)?;
            let previous = next.embeddings.get(&patch.id);
            patch.profile.secret = previous.and_then(|r| r.value.as_ref()).and_then(|p| p.secret.clone());
            if let Some(old) = previous.and_then(|r| r.value.as_ref()) {
                for (key, value) in &old.extra { patch.profile.extra.entry(key.clone()).or_insert_with(|| value.clone()); }
            }
            patch.secret.apply(&mut patch.profile.secret).map_err(|e| e.code)?;
            let extra = previous.map(|r| r.extra.clone()).unwrap_or_default();
            let revision = next.next_revision(&app.this_device_id().ok_or("identity_required")?).map_err(|e| e.code)?;
            next.touch_embedding_connections(&patch.id,&app.this_device_id().ok_or("identity_required")?).map_err(|e|e.code)?;
            next.embeddings.insert(patch.id, Record { revision, value: Some(patch.profile), extra });
            app.persist_memory(&next, true, None).map_err(|e| e.code)?; *current = next; Ok(json!({"saved":true}))
        },
        "memory.embeddings.remove" => {
            let Id { id } = decode(params)?;
            let _edit = app.roster_edit.lock().unwrap();
            let mut current = app.memory_config.lock(); let mut next = current.clone();
            let revision = next.next_revision(&app.this_device_id().ok_or("identity_required")?).map_err(|e| e.code)?;
            let extra = next.embeddings.get(&id).map(|r| r.extra.clone()).unwrap_or_default();
            next.touch_embedding_connections(&id,&app.this_device_id().ok_or("identity_required")?).map_err(|e|e.code)?;
            next.embeddings.insert(id, Record { revision, value: None, extra });
            app.persist_memory(&next, true, None).map_err(|e| e.code)?; *current = next; Ok(json!({"saved":true}))
        },
        "memory.preferences.get" => {
            let BotId { bot_id } = decode(params)?;
            if app.bot(&bot_id).is_none() { return Err("bot_deleted".into()); }
            let b = app.memory_config.lock().bots.get(&bot_id).and_then(|r| r.value.clone()).unwrap_or_default();
            Ok(preferences_summary(&b))
        },
        "memory.preferences.set" => {
            let patch: PreferencesSet = decode(params.clone())?;
            let _edit = app.roster_edit.lock().unwrap();
            if app.bot(&patch.bot_id).is_none() { return Err("bot_deleted".into()); }
            let mut current = app.memory_config.lock(); let mut next = current.clone();
            let mut b = next.bots.get(&patch.bot_id).and_then(|r| r.value.clone()).unwrap_or_default();
            // serde Option<Value> maps null to None; presence is tested on the original object.
            if params.get("connection_id").is_some() {
                b.connection_id = match patch.connection_id { None | Some(Value::Null) => None,
                    Some(Value::String(id)) => Some(id), _ => return Err("invalid_connection_id".into()) };
            }
            if b.connection_id.as_ref().is_some_and(|id| next.connections.get(id).and_then(|r| r.value.as_ref()).is_none()) {
                return Err("connection_disconnected".into());
            }
            if b.connection_id.as_ref().and_then(|id|next.connections.get(id)).and_then(|r|r.value.as_ref()).is_some_and(super::config::cloud_blocked) {
                return Err("lancedb_cloud_transport_unavailable".into());
            }
            b.auto_recall = patch.auto_recall; b.capture_conversation = patch.capture_conversation;
            b.capture_group_text = patch.capture_group_text; b.unattended_capture = patch.unattended_capture;
            if let Some(cap) = patch.max_capture_deliveries_per_turn { b.max_capture_deliveries_per_turn = cap; }
            if let Some(budget) = patch.recall_budget { b.recall_budget = budget; }
            next.set_bot(&patch.bot_id, b, &app.this_device_id().ok_or("identity_required")?).map_err(|e| e.code)?;
            app.persist_memory(&next, true, None).map_err(|e| e.code)?;
            let summary = preferences_summary(next.bots[&patch.bot_id].value.as_ref().unwrap()); *current = next; Ok(summary)
        },
        _ => Err(MemoryError::new("unknown_memory_method")),
    };
    let value = result.map_err(|e| e.code)?;
    app.memory_changed(None, "config_changed");
    Ok(value)
}

pub fn is_setup_method(method: &str) -> bool {
    matches!(method, "memory.embeddings.local.preview" | "memory.embeddings.local.apply" | "memory.embeddings.local.status"
        | "memory.pgvector.initialize.preview" | "memory.pgvector.initialize.apply"
        | "memory.lance.binding.preview" | "memory.lance.binding.apply"
        | "memory.lance.export.preview" | "memory.lance.export.apply" | "memory.lance.import.preview" | "memory.lance.import.apply")
}
pub async fn serve_as(app: &Arc<App>, method: &str, params: Value, _actor: &str) -> Result<Value, String> {
    if is_setup_method(method) {
        #[cfg(feature="runner")]
        return super::setup::serve(app, _actor, method, params).await.map_err(|e|e.code);
        #[cfg(not(feature="runner"))]
        return Err("runner_required".into());
    }
    serve(app,method,params).await
}

#[cfg(not(feature = "runner"))]
pub async fn serve(_app: &Arc<App>, _method: &str, _params: Value) -> Result<Value, String> { Err("runner_required".into()) }

#[cfg(feature = "runner")]
pub async fn serve(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    use super::dispatch as d;
    use tokio_util::sync::CancellationToken;
    #[derive(Deserialize)] #[serde(deny_unknown_fields)]
    struct Query { bot_id: String, query: String }
    #[derive(Deserialize)] #[serde(deny_unknown_fields)]
    struct Retain { bot_id: String, text: String, request_id: String }
    #[derive(Deserialize)] #[serde(deny_unknown_fields)]
    struct Inspect { bot_id: String, document_id: String }
    #[derive(Deserialize)] #[serde(deny_unknown_fields)]
    struct Operation { bot_id: String, id: String }
    #[derive(Deserialize)] #[serde(deny_unknown_fields)]
    struct Delete { bot_id: String, document_id: Option<String>, confirm: bool, connection_revision: Revision, deletion_epoch: u64 }
    #[derive(Deserialize)] #[serde(deny_unknown_fields)]
    struct Advanced { bot_id: String, feature: AdvancedFeature, action: String, body: Value }
    let _admission = app.update.try_admit().ok_or("runner_draining")?;
    let cancel = CancellationToken::new();
    let bot_id = params["bot_id"].as_str().ok_or("missing_bot_id")?.to_owned();
    if method == "memory.operations.list" {
        let bot=app.bot(&bot_id).ok_or("bot_deleted")?;
        if app.this_device_id().as_deref()!=Some(&bot.runner_id) { return Err("runner_changed".into()); }
        let _: BotId = decode(params)?;
        let operations = app.store.memory_deliveries(&bot_id).map_err(|e| e.code)?.iter().map(d::delivery_summary).collect::<Vec<_>>();
        let fence = app.store.memory_fence(&bot_id).map_err(|e| e.code)?;
        return Ok(json!({"operations":operations,"deletion":fence.map(|f|json!({"pending":f.pending,"operation_id":f.operation_id,"deletion_epoch":f.scope.deletion_epoch}))}));
    }
    let access = d::admit(app, &bot_id, false, &cancel).map_err(|e| e.code)?;
    if app.memory_runtime.backend(&access.scope).is_err() {
        let initialized = tokio::time::timeout(std::time::Duration::from_secs(5),
            super::setup::initialize(app,&access,cancel.clone())).await;
        if !matches!(initialized,Ok(Ok(()))) {
            if method == "memory.service.health" {
                let reason=match initialized {Ok(Err(error))=>error.code,_=>"service_timeout".into()};
                return Ok(json!({"status":"setup_required","reason":reason,"capabilities":Capabilities::default(),
                    "deletion_pending":app.store.memory_fence(&bot_id).map_err(|e|e.code)?.is_some_and(|f|f.pending)}));
            }
            return Err("adapter_setup_failed".into());
        }
    }
    if method == "memory.service.health" {
        let _: BotId = decode(params)?;
        let pending = app.store.memory_fence(&bot_id).map_err(|e| e.code)?.is_some_and(|f|f.pending);
        let backend = app.memory_runtime.backend(&access.scope);
        let (status, capabilities) = match backend {
            Ok(b) => {
                let caps = d::effective_capabilities(b.capabilities());
                let status = if d::execute(app, &access, BackendRequest::Health, cancel, false).await.is_ok() { "ready" } else { "degraded" };
                (status, caps)
            },
            Err(_) => ("setup_required", Capabilities::default()),
        };
        return Ok(json!({"status":status,"capabilities":capabilities,"deletion_pending":pending}));
    }
    let request = match method {
        "memory.service.recall" | "memory.service.reflect" => {
            let p: Query = decode(params)?;
            debug_assert_eq!(p.bot_id, access.bot_id);
            let budget = app.memory_config.lock().bots[&access.bot_id].value.as_ref().unwrap().recall_budget.clone();
            if method.ends_with("recall") { BackendRequest::Recall { query: p.query, budget } } else { BackendRequest::Reflect { query: p.query, budget } }
        },
        "memory.service.retain" => {
            let p: Retain = decode(params)?;
            let delivery = d::enqueue(app, &access, &p.text, Vec::new(), false, Some(&p.request_id), &cancel).map_err(|e|e.code)?;
            debug_assert_eq!(p.bot_id, access.bot_id);
            return d::deliver(app, &access, &delivery.id, false, cancel).await.map_err(|e|e.code);
        },
        "memory.service.inspect" => { let p: Inspect = decode(params)?; debug_assert_eq!(p.bot_id, access.bot_id); BackendRequest::Inspect { document_id:p.document_id } },
        "memory.service.advanced" => { let p: Advanced = decode(params)?; debug_assert_eq!(p.bot_id, access.bot_id); BackendRequest::Advanced { feature:p.feature, action:p.action, body:p.body } },
        "memory.operations.retry" | "memory.operations.status" | "memory.operations.cancel" => {
            let p: Operation = decode(params)?; debug_assert_eq!(p.bot_id, access.bot_id);
            return if method.ends_with("retry") { d::deliver(app,&access,&p.id,true,cancel).await.map_err(|e|e.code) }
                else { d::poll(app,&access,&p.id,method.ends_with("cancel"),cancel).await.map_err(|e|e.code) };
        },
        "memory.service.delete" => {
            let p: Delete = decode(params)?;
            if !p.confirm { return Err("confirmation_required".into()); }
            return d::delete(app,&p.bot_id,p.document_id,p.connection_revision,p.deletion_epoch,cancel).await.map_err(|e|e.code);
        },
        _ => return Err("unknown_memory_method".into()),
    };
    let result = d::execute(app, &access, request, cancel, false).await.map_err(|e|e.code)?;
    serde_json::to_value(result).map_err(|_| "invalid_response".into())
}
