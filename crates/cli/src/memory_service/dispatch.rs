//! Runner-only authority checks and bounded dispatch. No housekeeping/background traffic.
use std::{collections::HashMap, sync::{Arc,atomic::{AtomicBool,Ordering}}, time::Duration};
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use serde_json::{json, Value};
use crate::app::App;
use super::{types::*, queue::*};

struct Registered { scope: MemoryScope, backend: Arc<dyn MemoryBackend> }
#[derive(Default)]
pub struct MemoryRuntime {
    backends: Mutex<HashMap<String, Registered>>,
    active: Mutex<HashMap<String, (String, MemoryScope, Option<Revision>, CancellationToken)>>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    pub setup: super::setup::SetupRuntime,
}
impl MemoryRuntime {
    pub fn register(&self, scope: MemoryScope, backend: Arc<dyn MemoryBackend>) {
        self.backends.lock().insert(scope.namespace.clone(), Registered { scope, backend });
    }
    pub fn backend(&self, scope: &MemoryScope) -> Result<Arc<dyn MemoryBackend>, MemoryError> {
        let registry = self.backends.lock();
        let registered = registry.get(&scope.namespace).ok_or_else(|| MemoryError::new("adapter_unavailable"))?;
        if registered.scope != *scope {
            return Err(MemoryError::new("adapter_unavailable"));
        }
        Ok(registered.backend.clone())
    }
    pub fn cancel_all(&self) {
        for (_, _, _, token) in self.active.lock().values() { token.cancel(); }
        self.backends.lock().clear();
        self.setup.clear();
    }
    pub fn cancel_invalid(&self, config: &super::config::MemoryConfig) {
        for (bot_id, scope, consent, token) in self.active.lock().values() {
            let valid = config.bots.get(bot_id).and_then(|r| r.value.as_ref()).is_some_and(|b|
                b.connection_id.as_deref() == Some(&scope.connection_id) && b.deletion_epoch == scope.deletion_epoch
                && consent.as_ref().is_none_or(|c| b.capture_conversation && &b.consent_revision == c))
                && config.connections.get(&scope.connection_id).is_some_and(|c| c.value.is_some() && c.revision == scope.connection_revision);
            if !valid { token.cancel(); }
        }
        self.backends.lock().retain(|_, r| config.connections.get(&r.scope.connection_id)
            .is_some_and(|c| c.value.is_some() && c.revision == r.scope.connection_revision));
    }
    fn lock(&self, namespace: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks.lock().entry(namespace.into()).or_default().clone()
    }
}

#[derive(Clone)]
pub struct TurnAccess {
    pub bot_id: String, pub scope: MemoryScope, pub consent_revision: Option<Revision>,
    pub group_capture: bool, pub capture_cap: u32,
    calls: Arc<Mutex<(u32, u32)>>,
    pub(crate) turn_id: Option<String>,
}
impl TurnAccess {
    pub fn charge(&self, retain: bool) -> Result<(), MemoryError> {
        let mut counts = self.calls.lock();
        if counts.0 >= 8 || (retain && counts.1 >= 4) { return Err(MemoryError::new("turn_call_limit")); }
        counts.0 += 1; if retain { counts.1 += 1; } Ok(())
    }
}
/// Snapshot before any turn work/await. Enabling capture later cannot manufacture admission.
pub fn admit(app: &App, bot_id: &str, group: bool, cancel: &CancellationToken) -> Result<TurnAccess, MemoryError> {
    check_bot(app, bot_id, cancel)?;
    let machine = app.machine_file().ok_or_else(|| MemoryError::new("identity_required"))?;
    let config = app.memory_config.lock();
    let preferences = config.bots.get(bot_id).and_then(|r| r.value.as_ref()).ok_or_else(|| MemoryError::new("memory_not_configured"))?;
    let id = preferences.connection_id.as_ref().ok_or_else(|| MemoryError::new("memory_not_configured"))?;
    let connection = config.connections.get(id).filter(|c| c.value.is_some()).ok_or_else(|| MemoryError::new("connection_disconnected"))?;
    Ok(TurnAccess { bot_id: bot_id.into(), scope: MemoryScope { namespace: namespace(&machine.identity_pubkey, bot_id)?,
        connection_id: id.clone(), connection_revision: connection.revision.clone(), deletion_epoch: preferences.deletion_epoch },
        consent_revision: (preferences.capture_conversation && (!group || preferences.capture_group_text)).then(|| preferences.consent_revision.clone()),
        group_capture: group, capture_cap: preferences.max_capture_deliveries_per_turn, calls: Arc::new(Mutex::new((0, 0))), turn_id: None })
}
fn check_bot(app: &App, bot_id: &str, cancel: &CancellationToken) -> Result<(), MemoryError> {
    if cancel.is_cancelled() { return Err(MemoryError::new("cancelled")); }
    if app.is_paused() { return Err(MemoryError::new("account_paused")); }
    let bot = app.bot(bot_id).ok_or_else(|| MemoryError::new("bot_deleted"))?;
    if app.this_device_id().as_deref() != Some(&bot.runner_id) { return Err(MemoryError::new("runner_changed")); }
    Ok(())
}
pub fn recheck(app: &App, access: &TurnAccess, capture: bool, cancel: &CancellationToken) -> Result<(), MemoryError> {
    check_bot(app, &access.bot_id, cancel)?;
    let account = app.machine_file().ok_or_else(|| MemoryError::new("identity_required"))?;
    if namespace(&account.identity_pubkey, &access.bot_id)? != access.scope.namespace { return Err(MemoryError::new("identity_changed")); }
    let config = app.memory_config.lock();
    let b = config.bots.get(&access.bot_id).and_then(|r| r.value.as_ref()).ok_or_else(|| MemoryError::new("memory_not_configured"))?;
    let c = config.connections.get(&access.scope.connection_id).filter(|r| r.value.is_some()).ok_or_else(|| MemoryError::new("connection_disconnected"))?;
    if c.value.as_ref().is_some_and(|c|c.backend==BackendKind::LanceDb&&c.endpoint.is_some()) {
        return Err(MemoryError::new("lancedb_cloud_transport_unavailable"));
    }
    if b.connection_id.as_deref() != Some(&access.scope.connection_id) || c.revision != access.scope.connection_revision
        || b.deletion_epoch != access.scope.deletion_epoch { return Err(MemoryError::new("authority_changed")); }
    if capture && (access.consent_revision.is_none() || access.consent_revision.as_ref() != Some(&b.consent_revision)
        || !b.capture_conversation || (access.group_capture && !b.capture_group_text)) {
        return Err(MemoryError::new("consent_changed"));
    }
    Ok(())
}

fn validate_request(request: &BackendRequest) -> Result<(), MemoryError> {
    match request {
        BackendRequest::Recall { query, budget } | BackendRequest::Reflect { query, budget } => {
            budget.validate()?;
            if query.is_empty() || query.chars().count() > 4096 { return Err(MemoryError::new("invalid_query")); }
        },
        BackendRequest::Advanced { body, action, feature } => {
            if !advanced_action(feature, action) || serde_json::to_vec(body).map_err(|_| MemoryError::new("invalid_request"))?.len() > 32768
                || has_selector(body) { return Err(MemoryError::new("untrusted_selector")); }
        },
        BackendRequest::Inspect { document_id } | BackendRequest::DeleteDocument { document_id, .. } => super::config::valid_id(document_id)?,
        _ => {},
    }
    Ok(())
}
fn advanced_action(feature: &AdvancedFeature, action: &str) -> bool {
    match feature {
        AdvancedFeature::BankProfile | AdvancedFeature::BankConfig => matches!(action, "get" | "update" | "reset"),
        AdvancedFeature::Directives | AdvancedFeature::MentalModels => matches!(action, "list" | "create" | "get" | "update" | "delete" | "refresh"),
        AdvancedFeature::MentalModelHistory => matches!(action, "list" | "get" | "history"),
        AdvancedFeature::Observations => matches!(action, "list" | "scopes"),
        AdvancedFeature::MemoryEdit => action == "edit",
        AdvancedFeature::MemoryInvalidate => action == "invalidate",
        AdvancedFeature::MemoryRestore => action == "restore",
        AdvancedFeature::Documents => matches!(action, "list" | "get" | "chunks"),
        AdvancedFeature::Sessions => matches!(action, "create" | "get" | "messages" | "commit" | "delete"),
        AdvancedFeature::Resources => matches!(action, "list" | "read" | "create" | "delete" | "ingest"),
        AdvancedFeature::Tasks => matches!(action, "list" | "get" | "status" | "cancel" | "delete_record"),
    }
}
pub fn effective_capabilities(mut capabilities:Capabilities)->Capabilities {
    capabilities.advanced_actions.retain(|feature,actions| {
        actions.retain(|action|advanced_action(feature,action));
        capabilities.advanced.contains(feature) && !actions.is_empty()
    });
    capabilities.advanced.retain(|feature|capabilities.advanced_actions.contains_key(feature));
    capabilities
}
fn has_selector(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(key, value)| {
            let key = key.to_ascii_lowercase();
            ["bank", "bank_id", "namespace", "tenant", "tenant_id", "account", "account_id", "user_id", "uri", "target_uri", "table", "schema", "connection", "connection_id", "key", "api_key", "secret", "sql", "endpoint"].contains(&key.as_str()) || has_selector(value)
        }),
        Value::Array(values) => values.iter().any(has_selector), _ => false,
    }
}
struct Active<'a> { runtime: &'a MemoryRuntime, id: String }
impl Drop for Active<'_> { fn drop(&mut self) { self.runtime.active.lock().remove(&self.id); } }

pub async fn execute(app: &Arc<App>, access: &TurnAccess, request: BackendRequest, cancel: CancellationToken,
    capture: bool) -> Result<BackendResponse, MemoryError> {
    access.charge(matches!(request, BackendRequest::Retain { .. }))?;
    execute_charged(app, access, request, cancel, capture, None).await
}

async fn execute_charged(app: &Arc<App>, access: &TurnAccess, request: BackendRequest,
    cancel: CancellationToken, capture: bool, started: Option<&AtomicBool>) -> Result<BackendResponse, MemoryError> {
    validate_request(&request)?;
    recheck(app, access, capture, &cancel)?;
    let backend = app.memory_runtime.backend(&access.scope)?;
    if !backend.capabilities().supports(&request) { return Err(MemoryError::new("unsupported_capability")); }
    if matches!(request, BackendRequest::Retain { .. } | BackendRequest::Advanced { .. })
        && app.store.memory_fence(&access.bot_id)?.is_some_and(|f| f.pending) {
        return Err(MemoryError::new("deletion_pending"));
    }
    let timeout_ms = match &request { BackendRequest::Recall { budget, .. } | BackendRequest::Reflect { budget, .. } => budget.timeout_ms, _ => 5000 };
    let max_bytes = match &request { BackendRequest::Recall { budget, .. } | BackendRequest::Reflect { budget, .. } => budget.max_bytes, _ => 65536 };
    let child = cancel.child_token();
    let id = uuid::Uuid::new_v4().to_string();
    app.memory_runtime.active.lock().insert(id.clone(), (access.bot_id.clone(), access.scope.clone(),
        capture.then(|| access.consent_revision.clone()).flatten(), child.clone()));
    let _active = Active { runtime: &app.memory_runtime, id };
    let future = async {
        if let Some(started) = started { started.store(true,Ordering::Release); }
        backend.execute(&access.scope, request, child.clone()).await
    };
    tokio::pin!(future);
    let deadline = tokio::time::sleep(Duration::from_millis(timeout_ms)); tokio::pin!(deadline);
    let mut authority_tick = tokio::time::interval(Duration::from_millis(25));
    let response = loop {
        tokio::select! {
            biased;
            _ = child.cancelled() => break Err(MemoryError::new("cancelled")),
            _ = &mut deadline => { child.cancel(); break Err(MemoryError::new("service_timeout")); },
            _ = authority_tick.tick() => if let Err(e) = recheck(app, access, capture, &child) { child.cancel(); break Err(e); },
            result = &mut future => break result.map_err(|_| MemoryError::new("service_failed")),
        }
    };
    let response = response?;
    recheck(app, access, capture, &cancel)?;
    if serde_json::to_vec(&response).map_err(|_| MemoryError::new("invalid_response"))?.len() > max_bytes {
        return Err(MemoryError::new("response_too_large"));
    }
    app.memory_changed(Some(&access.bot_id), "ready");
    Ok(response)
}

pub async fn automatic_recall(app: &Arc<App>, access: &TurnAccess, query: &str, cancel: CancellationToken) -> Option<String> {
    let preferences = app.memory_config.lock().bots.get(&access.bot_id).and_then(|r| r.value.clone())?;
    if !preferences.auto_recall || query.trim().is_empty() { return None; }
    let query = query.chars().take(4096).collect();
    match execute(app, access, BackendRequest::Recall { query, budget: preferences.recall_budget.clone() }, cancel, false).await {
        Ok(response) => evidence_notes(&response.evidence, &preferences.recall_budget),
        Err(_) => { app.memory_changed(Some(&access.bot_id), "degraded"); None },
    }
}

pub fn enqueue(app: &App, access: &TurnAccess, text: &str, sources: Vec<Source>, capture: bool,
    request_id: Option<&str>, cancel: &CancellationToken) -> Result<Delivery, MemoryError> {
    let _edit = app.roster_edit.lock().unwrap();
    recheck(app, access, capture, cancel)?;
    let config = app.memory_config.lock();
    // Keep config lock through enqueue: consent/disconnect and frozen intent are serialized.
    let mut document = freeze_document(&access.scope.namespace, text, sources)?;
    if let Some(request_id) = request_id {
        super::config::valid_id(request_id)?;
        document.request_id = digest(format!("beans.memory.explicit.v1\0{}\0{request_id}", access.scope.namespace).as_bytes());
    }
    let delivery = Delivery { id: document.request_id.clone(), bot_id: access.bot_id.clone(), scope: access.scope.clone(),
        consent_revision: capture.then(|| access.consent_revision.clone()).flatten(), document,
        state: OperationState::Queued, operation_id: None, error_code: None };
    if !super::delivery_valid(&config, &delivery) { return Err(MemoryError::new("authority_changed")); }
    if app.store.memory_fence(&access.bot_id)?.is_some_and(|f| f.pending) { return Err(MemoryError::new("deletion_pending")); }
    if capture {
        let id=access.turn_id.as_deref().ok_or_else(||MemoryError::new("turn_admission_required"))?;
        app.store.enqueue_admitted_memory(&delivery,id)?;
    } else { app.store.enqueue_memory(&delivery)?; }
    Ok(delivery)
}
fn access_for_delivery(access: &TurnAccess, d: &Delivery) -> TurnAccess {
    TurnAccess { bot_id: d.bot_id.clone(), scope: d.scope.clone(), consent_revision: d.consent_revision.clone(),
        group_capture: access.group_capture, capture_cap: access.capture_cap, calls: access.calls.clone(), turn_id:access.turn_id.clone() }
}
pub async fn deliver(app: &Arc<App>, access: &TurnAccess, id: &str, retry: bool, cancel: CancellationToken) -> Result<Value, MemoryError> {
    let lock = app.memory_runtime.lock(&access.scope.namespace); let _serial = lock.lock().await;
    let before = app.store.memory_delivery(&access.bot_id, id)?;
    let scoped = access_for_delivery(access, &before);
    let capture = before.consent_revision.is_some();
    recheck(app, &scoped, capture, &cancel)?;
    let capabilities = app.memory_runtime.backend(&before.scope)?.capabilities();
    if before.state == OperationState::Completed { return Ok(delivery_summary(&before)); }
    if before.state == OperationState::Processing { return Err(MemoryError::new("operation_processing")); }
    if before.error_code.as_deref() == Some("authority_changed") { return Err(MemoryError::new("authority_changed")); }
    if before.state == OperationState::Submitted { return Err(MemoryError::new("operation_busy")); }
    if before.state != OperationState::Queued && !retry { return Ok(delivery_summary(&before)); }
    if matches!(before.state, OperationState::Failed | OperationState::DeliveryUnknown) && !capabilities.idempotent_retain {
        return Err(MemoryError::new("uncertain_delivery_not_retryable"));
    }
    if !capabilities.retain { return Err(MemoryError::new("unsupported_capability")); }
    scoped.charge(true)?;
    if app.store.memory_fence(&access.bot_id)?.is_some_and(|f| f.pending) { return Err(MemoryError::new("deletion_pending")); }
    recheck(app, &scoped, capture, &cancel)?;
    let mut submitted = before.clone(); submitted.state = OperationState::Submitted; submitted.error_code = None;
    if !app.store.transition_memory(&before, &submitted)? { return Err(MemoryError::new("operation_busy")); }
    let started=AtomicBool::new(false);
    let response = execute_charged(app, &scoped, BackendRequest::Retain { document: before.document.clone() }, cancel, capture, Some(&started)).await;
    let mut after = submitted.clone();
    match response {
        Ok(response) => {
            if let Some(operation) = response.operation {
                after.operation_id = Some(operation.id); after.state = operation.state;
                if matches!(after.state, OperationState::Queued | OperationState::Submitted) { after.state = OperationState::Processing; }
            } else { after.state = OperationState::Completed; }
        },
        Err(error) => {
            after.state = if started.load(Ordering::Acquire) { OperationState::DeliveryUnknown } else { OperationState::Failed };
            after.error_code = Some(error.code);
        },
    }
    if !app.store.transition_memory(&submitted, &after)? {
        app.memory_changed(Some(&access.bot_id), "cleanup_pending");
        let current=app.store.memory_delivery(&access.bot_id,id)?;
        if !started.load(Ordering::Acquire) && current.state==OperationState::DeliveryUnknown && current.operation_id.is_none() {
            let mut unsent=current.clone();unsent.state=OperationState::Failed;
            if app.store.transition_memory(&current,&unsent)? { return Ok(delivery_summary(&unsent)); }
        }
        return Ok(delivery_summary(&app.store.memory_delivery(&access.bot_id, id)?));
    }
    app.memory_changed(Some(&access.bot_id), if after.state == OperationState::Completed { "stored" } else { "pending" });
    Ok(delivery_summary(&after))
}

pub async fn poll(app: &Arc<App>, access: &TurnAccess, id: &str, cancel_operation: bool,
    cancel: CancellationToken) -> Result<Value, MemoryError> {
    let before = app.store.memory_delivery(&access.bot_id, id)?;
    if before.scope != access.scope { return Err(MemoryError::new("operation_scope_changed")); }
    if matches!(before.state, OperationState::Queued | OperationState::Failed) && cancel_operation {
        let mut after = before.clone(); after.state = OperationState::Failed; after.error_code = Some("cancelled".into());
        app.store.transition_memory(&before, &after)?; return Ok(delivery_summary(&after));
    }
    let operation_id = before.operation_id.clone().ok_or_else(|| MemoryError::new("operation_status_unavailable"))?;
    // Fenced old work may only be inspected/cancelled using its old bound scope, not retained again.
    let request = if cancel_operation { BackendRequest::CancelOperation { operation_id } } else { BackendRequest::OperationStatus { operation_id } };
    let response = execute(app, access, request, cancel, false).await?;
    let mut after = before.clone();
    if let Some(operation) = response.operation {
        after.state = operation.state;
        after.operation_id = Some(operation.id);
        if before.error_code.as_deref() == Some("authority_changed") && after.state != OperationState::Failed {
            after.state = OperationState::DeliveryUnknown; // cleanup still must verify bank/document deletion.
        }
        app.store.transition_memory(&before, &after)?;
    }
    Ok(delivery_summary(&after))
}

pub async fn delete(app: &Arc<App>, bot_id: &str, document_id: Option<String>, expected: Revision,
    epoch: u64, cancel: CancellationToken) -> Result<Value, MemoryError> {
    let old = admit(app, bot_id, false, &cancel)?;
    if old.scope.connection_revision != expected || old.scope.deletion_epoch != epoch { return Err(MemoryError::new("stale_confirmation")); }
    if let Some(fence) = app.store.memory_fence(bot_id)?.filter(|f|f.pending) {
        if fence.scope != old.scope || fence.document_id != document_id { return Err(MemoryError::new("deletion_pending")); }
        return finish_delete(app,&fence,cancel).await;
    }
    let capabilities = app.memory_runtime.backend(&old.scope)?.capabilities();
    if (document_id.is_some() && !capabilities.delete_document) || (document_id.is_none() && !capabilities.clear) {
        return Err(MemoryError::new("unsupported_capability"));
    }
    let fence = {
        let _edit = app.roster_edit.lock().unwrap();
        recheck(app, &old, false, &cancel)?;
        let mut current = app.memory_config.lock(); let mut next = current.clone();
        let mut b = next.bots.get(bot_id).and_then(|r| r.value.clone()).ok_or_else(|| MemoryError::new("memory_not_configured"))?;
        b.deletion_epoch = b.deletion_epoch.checked_add(1).ok_or_else(|| MemoryError::new("epoch_exhausted"))?;
        let scope = MemoryScope { deletion_epoch: b.deletion_epoch, ..old.scope.clone() };
        next.set_bot(bot_id, b, &app.this_device_id().ok_or_else(|| MemoryError::new("identity_required"))?)?;
        let fence = DeletionFence { bot_id: bot_id.into(), scope, document_id, request_id: uuid::Uuid::new_v4().to_string(), pending: true, operation_id: None };
        app.persist_memory(&next, true, Some(&fence))?; *current = next; fence
    };
    finish_delete(app, &fence, cancel).await
}
pub async fn finish_delete(app: &Arc<App>, fence: &DeletionFence, cancel: CancellationToken) -> Result<Value, MemoryError> {
    let access = admit(app, &fence.bot_id, false, &cancel)?;
    if access.scope != fence.scope { return Err(MemoryError::new("authority_changed")); }
    let lock = app.memory_runtime.lock(&fence.scope.namespace); let _serial = lock.lock().await;
    // Unknown/lost or async submissions cannot be proved quiescent by a local mutex.
    let uncertain = app.store.memory_has_uncertain(&fence.scope)?;
    if uncertain { return Ok(json!({"pending":true,"reason":"remote_quiescence_unverified","deletion_epoch":fence.scope.deletion_epoch})); }
    if app.memory_runtime.backend(&access.scope).is_err() {
        tokio::time::timeout(Duration::from_secs(5),super::setup::initialize(app,&access,cancel.clone())).await
            .map_err(|_|MemoryError::new("service_timeout"))??;
    }
    if let Some(operation_id) = &fence.operation_id {
        let response = execute(app,&access,BackendRequest::OperationStatus{operation_id:operation_id.clone()},cancel,false).await?;
        let completed = response.operation.as_ref().is_some_and(|o|o.state==OperationState::Completed);
        let mut after=fence.clone();after.pending=!completed;
        app.store.update_memory_fence(fence,&after)?;
        return Ok(json!({"pending":after.pending,"operation_id":after.operation_id,"deletion_epoch":after.scope.deletion_epoch}));
    }
    let request = match &fence.document_id {
        Some(id) => BackendRequest::DeleteDocument { document_id: id.clone(), request_id: fence.request_id.clone() },
        None => BackendRequest::Clear { request_id: fence.request_id.clone() },
    };
    let response = execute(app, &access, request, cancel, false).await?;
    let mut after = fence.clone();
    after.pending = response.operation.as_ref().is_some_and(|o| o.state != OperationState::Completed);
    after.operation_id = response.operation.map(|o| o.id);
    app.store.update_memory_fence(fence, &after)?;
    Ok(json!({"pending":after.pending,"operation_id":after.operation_id,"deletion_epoch":after.scope.deletion_epoch}))
}
pub fn delivery_summary(d: &Delivery) -> Value {
    json!({"id":d.id,"document_id":d.document.id,"state":d.state,"operation_id":d.operation_id,"error_code":d.error_code})
}
