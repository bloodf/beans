//! Official OpenViking v0.4.23 REST. Native USER identity, not spoofed key-mode headers.
use super::http::*;
use crate::memory_service::types::*;
use reqwest::{header::HeaderMap, Method, Url};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

const BASELINE: &str = "0.4.23";
fn authorization_values(binding: &OpenVikingBinding) -> (&str, &str, &str) {
    (&binding.account_id, &binding.user_id, &binding.api_key)
}
fn authorization_mode(binding: &OpenVikingBinding) -> &'static str {
    match binding.mode {
        OpenVikingAuthMode::UserKey => "api_key",
        OpenVikingAuthMode::TrustedGateway => "trusted",
    }
}

pub struct OpenViking {
    http: MemoryHttp,
    scope: MemoryScope,
    authorization: OpenVikingBinding,
    home: String,
    schema: Value,
    capabilities: Capabilities,
}
impl OpenViking {
    /// Configuration lives in account-encrypted, typed Connection.options.
    /// The binding is selected by trusted core namespace, never operation arguments.
    pub async fn connect(
        connection: &Connection,
        scope: &MemoryScope,
        cancel: CancellationToken,
    ) -> Result<Self, MemoryError> {
        let work = async {
            validate_settings(connection)?;
            bind(scope)?;
            let authorization = configuration(connection)?
                .get(&scope.namespace)
                .ok_or_else(|| error("bot_authorization_missing"))?
                .clone();
            let (account, user, key) = authorization_values(&authorization);
            let home = format!("viking://user/{user}");
            let mut headers = HeaderMap::new();
            headers.insert("X-API-Key", header(key)?);
            if authorization.mode == OpenVikingAuthMode::TrustedGateway {
                headers.insert("X-OpenViking-Account", header(account)?);
                headers.insert("X-OpenViking-User", header(user)?);
            }
            let http = MemoryHttp::new(connection, headers)?;
            let health = http
                .request(
                    Method::GET,
                    "/health",
                    &[],
                    None,
                    BODY_LIMIT,
                    DEADLINE,
                    &cancel,
                )
                .await?;
            verify_health(&health, &authorization)?;
            let schema = http
                .request(
                    Method::GET,
                    "/openapi.json",
                    &[],
                    None,
                    SCHEMA_LIMIT,
                    DEADLINE,
                    &cancel,
                )
                .await?;
            // The official server's OpenAPI version is 0.1.0, independently of package version.
            if schema.pointer("/info/title").and_then(Value::as_str) != Some("OpenViking API")
                || schema.pointer("/info/version").and_then(Value::as_str) != Some("0.1.0")
            {
                return Err(error("unsupported_server_schema"));
            }
            let has = |path: &str, method: &str| route(&schema, path, method);
            if !has("/api/v1/system/status", "get") {
                return Err(error("unsupported_server_schema"));
            }
            let status = http
                .request(
                    Method::GET,
                    "/api/v1/system/status",
                    &[],
                    None,
                    BODY_LIMIT,
                    DEADLINE,
                    &cancel,
                )
                .await?;
            verify_status(&status, &authorization)?;
            let sessions = has("/api/v1/sessions", "post")
                && has("/api/v1/sessions/{session_id}/messages", "post")
                && has("/api/v1/sessions/{session_id}/commit", "post")
                && has("/api/v1/tasks/{task_id}", "get")
                && request_property(&schema, "/api/v1/sessions", "post", "auto_commit_policy");
            let mut capabilities = Capabilities {
                retain: sessions,
                recall: has("/api/v1/search/find", "post")
                    && request_property(&schema, "/api/v1/search/find", "post", "target_uri"),
                inspect: has("/api/v1/content/read", "get"),
                delete_document: has("/api/v1/fs", "delete"),
                operation_status: has("/api/v1/tasks/{task_id}", "get"),
                cancel_operation: has("/api/v1/tasks/{task_id}/cancel", "post"),
                // A successful USER file delete does not establish whole-home erasure/fencing.
                clear: false,
                reflect: false,
                idempotent_retain: false,
                write_fence: false,
                ..Default::default()
            };
            if sessions {
                capabilities.advanced.insert(AdvancedFeature::Sessions);
            }
            if has("/api/v1/fs/ls", "get") || has("/api/v1/resources", "post") {
                capabilities.advanced.insert(AdvancedFeature::Resources);
            }
            if capabilities.operation_status {
                capabilities.advanced.insert(AdvancedFeature::Tasks);
            }
            use AdvancedFeature::*;
            for (feature, action, path, method) in [
                (Sessions, "list", "/api/v1/sessions", "get"),
                (Sessions, "create", "/api/v1/sessions", "post"),
                (Sessions, "get", "/api/v1/sessions/{session_id}", "get"),
                (
                    Sessions,
                    "delete",
                    "/api/v1/sessions/{session_id}",
                    "delete",
                ),
                (
                    Sessions,
                    "commit",
                    "/api/v1/sessions/{session_id}/commit",
                    "post",
                ),
                (
                    Sessions,
                    "add_message",
                    "/api/v1/sessions/{session_id}/messages",
                    "post",
                ),
                (Resources, "list", "/api/v1/fs/ls", "get"),
                (Resources, "add", "/api/v1/resources", "post"),
                (Resources, "read", "/api/v1/content/read", "get"),
                (Resources, "delete", "/api/v1/fs", "delete"),
                (Tasks, "list", "/api/v1/tasks", "get"),
                (Tasks, "get", "/api/v1/tasks/{task_id}", "get"),
                (Tasks, "cancel", "/api/v1/tasks/{task_id}/cancel", "post"),
            ] {
                if capabilities.advanced.contains(&feature) && has(path, method) {
                    capabilities
                        .advanced_actions
                        .entry(feature)
                        .or_default()
                        .insert(action.into());
                }
            }
            Ok(Self {
                http,
                scope: scope.clone(),
                authorization,
                home,
                schema,
                capabilities,
            })
        };
        bounded(work, DEADLINE, &cancel).await
    }
    async fn verify_identity(&self, cancel: &CancellationToken) -> Result<(), MemoryError> {
        let health = self
            .http
            .request(
                Method::GET,
                "/health",
                &[],
                None,
                BODY_LIMIT,
                DEADLINE,
                cancel,
            )
            .await?;
        verify_health(&health, &self.authorization)
    }
    async fn call(
        &self,
        method: Method,
        path: &str,
        template: &str,
        query: &[(&str, String)],
        body: Option<Value>,
        max_bytes: usize,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Value, MemoryError> {
        if !route(&self.schema, template, method_name(&method)) {
            return Err(error("unsupported_operation"));
        }
        let mut value = self
            .http
            .request(method, path, query, body, max_bytes, timeout, cancel)
            .await?;
        let result = result(&value)?;
        validate_result_scope(result, &self.home)?;
        Ok(value.get_mut("result").unwrap().take())
    }
    fn uri(&self, relative: &str) -> Result<String, MemoryError> {
        relative_path(relative)?;
        Ok(format!("{}/{relative}", self.home))
    }
    async fn create_session(
        &self,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<Value, MemoryError> {
        let value = self
            .call(
                Method::POST,
                "/api/v1/sessions",
                "/api/v1/sessions",
                &[],
                Some(json!({"session_id":id,"auto_commit_policy":null})),
                BODY_LIMIT,
                DEADLINE,
                cancel,
            )
            .await?;
        if string(&value, "session_id")? != id {
            return Err(error("session_mismatch"));
        }
        Ok(value)
    }
    async fn commit(
        &self,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<BackendResponse, MemoryError> {
        let value = self
            .call(
                Method::POST,
                &format!("/api/v1/sessions/{id}/commit"),
                "/api/v1/sessions/{session_id}/commit",
                &[],
                Some(json!({"keep_recent_count":0})),
                BODY_LIMIT,
                DEADLINE,
                cancel,
            )
            .await?;
        // Archive success is not completed memory extraction.
        let task = string(&value, "task_id")?;
        identifier(task)?;
        Ok(BackendResponse {
            operation: Some(ServiceOperation {
                id: task.into(),
                state: OperationState::Submitted,
            }),
            data: self.http.public_data(value),
            ..Default::default()
        })
    }
    async fn task(
        &self,
        id: &str,
        cancel_task: bool,
        cancel: &CancellationToken,
    ) -> Result<BackendResponse, MemoryError> {
        let (method, path, template) = if cancel_task {
            (
                Method::POST,
                format!("/api/v1/tasks/{id}/cancel"),
                "/api/v1/tasks/{task_id}/cancel",
            )
        } else {
            (
                Method::GET,
                format!("/api/v1/tasks/{id}"),
                "/api/v1/tasks/{task_id}",
            )
        };
        let value = self
            .call(
                method,
                &path,
                template,
                &[],
                None,
                BODY_LIMIT,
                DEADLINE,
                cancel,
            )
            .await?;
        let operation = parse_task(&value)?;
        if operation.id != id {
            return Err(error("operation_mismatch"));
        }
        Ok(BackendResponse {
            operation: Some(operation),
            data: task_summary(&value),
            ..Default::default()
        })
    }
    async fn advanced(
        &self,
        feature: AdvancedFeature,
        action: &str,
        body: Value,
        cancel: &CancellationToken,
    ) -> Result<BackendResponse, MemoryError> {
        // Build and validate the entire operation before even the identity preflight.
        let mut query = vec![];
        let (method, path, template, payload) = match feature {
            AdvancedFeature::Sessions => match action {
                "list" => {
                    fields(&body, &[])?;
                    (
                        Method::GET,
                        "/api/v1/sessions".into(),
                        "/api/v1/sessions",
                        None,
                    )
                }
                "create" => {
                    fields(&body, &["id"])?;
                    let id = action_id(&body)?;
                    (
                        Method::POST,
                        "/api/v1/sessions".into(),
                        "/api/v1/sessions",
                        Some(json!({"session_id":id,"auto_commit_policy":null})),
                    )
                }
                "get" | "delete" | "commit" => {
                    fields(&body, &["id"])?;
                    let id = action_id(&body)?;
                    let (method, tail, template) = match action {
                        "get" => (Method::GET, "", "/api/v1/sessions/{session_id}"),
                        "delete" => (Method::DELETE, "", "/api/v1/sessions/{session_id}"),
                        _ => (
                            Method::POST,
                            "/commit",
                            "/api/v1/sessions/{session_id}/commit",
                        ),
                    };
                    (
                        method,
                        format!("/api/v1/sessions/{id}{tail}"),
                        template,
                        if action == "commit" {
                            Some(json!({"keep_recent_count":0}))
                        } else {
                            None
                        },
                    )
                }
                "add_message" => {
                    fields(&body, &["id", "role", "content"])?;
                    let id = action_id(&body)?;
                    let role = body
                        .get("role")
                        .and_then(Value::as_str)
                        .ok_or_else(|| error("invalid_request"))?;
                    if !["user", "assistant"].contains(&role) {
                        return Err(error("invalid_request"));
                    }
                    let content = body
                        .get("content")
                        .and_then(Value::as_str)
                        .ok_or_else(|| error("invalid_request"))?;
                    if content.is_empty() || content.len() > 32768 {
                        return Err(error("invalid_request"));
                    }
                    (
                        Method::POST,
                        format!("/api/v1/sessions/{id}/messages"),
                        "/api/v1/sessions/{session_id}/messages",
                        Some(json!({"role":role,"content":crate::memory::scrub(content)})),
                    )
                }
                _ => return Err(error("unsupported_action")),
            },
            AdvancedFeature::Resources => match action {
                "list" => {
                    fields(&body, &["path", "offset"])?;
                    let relative = body
                        .get("path")
                        .and_then(Value::as_str)
                        .unwrap_or("resources");
                    if relative != "resources" && !relative.starts_with("resources/") {
                        return Err(error("invalid_request"));
                    }
                    let uri = self.uri(relative)?;
                    let offset = body
                        .get("offset")
                        .map(|v| v.as_u64().ok_or_else(|| error("invalid_request")))
                        .transpose()?
                        .unwrap_or(0);
                    if offset > 100_000 {
                        return Err(error("invalid_request"));
                    }
                    query = vec![
                        ("uri", uri),
                        ("node_limit", "20".into()),
                        ("offset", offset.to_string()),
                        ("output", "original".into()),
                    ];
                    (Method::GET, "/api/v1/fs/ls".into(), "/api/v1/fs/ls", None)
                }
                "add" => {
                    fields(&body, &["id", "source_url"])?;
                    let id = action_id(&body)?;
                    let source = body
                        .get("source_url")
                        .and_then(Value::as_str)
                        .ok_or_else(|| error("invalid_request"))?;
                    let source = Url::parse(source).map_err(|_| error("invalid_request"))?;
                    if !["http", "https"].contains(&source.scheme())
                        || source.host_str().is_none()
                        || !source.username().is_empty()
                        || source.password().is_some()
                        || source.query().is_some()
                        || source.fragment().is_some()
                    {
                        return Err(error("invalid_request"));
                    }
                    let target = self.uri(&format!("resources/beans/{id}"))?;
                    (
                        Method::POST,
                        "/api/v1/resources".into(),
                        "/api/v1/resources",
                        Some(
                            json!({"path":source.as_str(),"to":target,"create_parent":true,"wait":false,"watch_interval":0,"strict":true}),
                        ),
                    )
                }
                "read" | "delete" => {
                    fields(&body, &["path"])?;
                    let relative = body
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or_else(|| error("invalid_request"))?;
                    if !relative.starts_with("resources/") {
                        return Err(error("invalid_request"));
                    }
                    query.push(("uri", self.uri(relative)?));
                    if action == "delete" {
                        query.push(("recursive", "true".into()));
                    }
                    if action == "read" {
                        (
                            Method::GET,
                            "/api/v1/content/read".into(),
                            "/api/v1/content/read",
                            None,
                        )
                    } else {
                        (Method::DELETE, "/api/v1/fs".into(), "/api/v1/fs", None)
                    }
                }
                _ => return Err(error("unsupported_action")),
            },
            AdvancedFeature::Tasks => match action {
                "list" => {
                    fields(&body, &[])?;
                    query.push(("limit", "20".into()));
                    (Method::GET, "/api/v1/tasks".into(), "/api/v1/tasks", None)
                }
                "get" | "cancel" => {
                    fields(&body, &["id"])?;
                    let id = action_id(&body)?;
                    let tail = if action == "cancel" { "/cancel" } else { "" };
                    (
                        if action == "cancel" {
                            Method::POST
                        } else {
                            Method::GET
                        },
                        format!("/api/v1/tasks/{id}{tail}"),
                        if action == "cancel" {
                            "/api/v1/tasks/{task_id}/cancel"
                        } else {
                            "/api/v1/tasks/{task_id}"
                        },
                        None,
                    )
                }
                _ => return Err(error("unsupported_action")),
            },
            _ => return Err(error("unsupported_operation")),
        };
        if !route(&self.schema, template, method_name(&method)) {
            return Err(error("unsupported_operation"));
        }
        self.verify_identity(cancel).await?;
        let value = self
            .call(
                method, &path, template, &query, payload, BODY_LIMIT, DEADLINE, cancel,
            )
            .await?;
        let operation = if feature == AdvancedFeature::Tasks && action != "list" {
            Some(parse_task(&value)?)
        } else if (feature == AdvancedFeature::Sessions && action == "commit")
            || (feature == AdvancedFeature::Resources && action == "add")
        {
            let id = string(&value, "task_id")?;
            identifier(id)?;
            Some(ServiceOperation {
                id: id.into(),
                state: OperationState::Submitted,
            })
        } else {
            None
        };
        Ok(BackendResponse {
            operation,
            data: self.http.public_data(value),
            ..Default::default()
        })
    }
}

#[async_trait::async_trait]
impl MemoryBackend for OpenViking {
    fn capabilities(&self) -> Capabilities {
        self.capabilities.clone()
    }
    async fn execute(
        &self,
        scope: &MemoryScope,
        request: BackendRequest,
        cancel: CancellationToken,
    ) -> Result<BackendResponse, MemoryError> {
        let timeout = match &request {
            BackendRequest::Recall { budget, .. } => Duration::from_millis(budget.timeout_ms),
            _ => DEADLINE,
        };
        let work = async {
            check_scope(&self.scope, scope)?;
            if !self.capabilities.supports(&request) {
                return Err(error("unsupported_operation"));
            }
            // Validation precedes preflight; no attacker-selected paths reach transport.
            match &request {
                BackendRequest::Retain { document } => {
                    identifier(&document.id)?;
                    if document.id.len() > 128
                        || document.text.is_empty()
                        || document.text.len() > 32768
                    {
                        return Err(error("invalid_document"));
                    }
                }
                BackendRequest::Recall { query, budget } => {
                    budget.validate()?;
                    if query.is_empty() || query.chars().count() > 4096 {
                        return Err(error("invalid_query"));
                    }
                }
                BackendRequest::Inspect { document_id }
                | BackendRequest::DeleteDocument { document_id, .. } => {
                    relative_path(document_id)?;
                }
                BackendRequest::OperationStatus { operation_id }
                | BackendRequest::CancelOperation { operation_id } => {
                    identifier(operation_id)?;
                }
                _ => {}
            }
            if let BackendRequest::Advanced {
                feature,
                action,
                body,
            } = request
            {
                return self.advanced(feature, &action, body, &cancel).await;
            }
            self.verify_identity(&cancel).await?;
            match request {
                BackendRequest::Health => {
                    let status = self
                        .http
                        .request(
                            Method::GET,
                            "/api/v1/system/status",
                            &[],
                            None,
                            BODY_LIMIT,
                            DEADLINE,
                            &cancel,
                        )
                        .await?;
                    verify_status(&status, &self.authorization)?;
                    Ok(BackendResponse {
                        data: json!({"healthy":true,"version":BASELINE,"auth_mode":authorization_mode(&self.authorization)}),
                        ..Default::default()
                    })
                }
                BackendRequest::Retain { document } => {
                    let id = format!("beans-{}-{}", scope.namespace, document.id);
                    self.create_session(&id, &cancel).await?;
                    self.call(Method::POST,&format!("/api/v1/sessions/{id}/messages"),"/api/v1/sessions/{session_id}/messages",&[],
                    Some(json!({"role":"user","content":crate::memory::scrub(&document.text),"source_message_ids":document.sources.iter().map(|s|s.message_id.as_str()).collect::<Vec<_>>() })),BODY_LIMIT,DEADLINE,&cancel).await?;
                    self.commit(&id, &cancel).await
                }
                BackendRequest::Recall { query, budget } => {
                    let value=self.call(Method::POST,"/api/v1/search/find","/api/v1/search/find",&[],
                    Some(json!({"query":query,"target_uri":self.home,"limit":budget.max_results,"include_provenance":true,"read_content":false})),
                    budget.max_bytes,Duration::from_millis(budget.timeout_ms),&cancel).await?;
                    let value = self.http.public_data(value);
                    let mut evidence = vec![];
                    let mut remaining = budget.max_context_chars;
                    let mut seen = BTreeSet::new();
                    for category in ["memories", "resources", "skills"] {
                        let Some(rows) = value.get(category).and_then(Value::as_array) else {
                            continue;
                        };
                        for row in rows {
                            let uri = string(row, "uri")?;
                            if evidence.len() == budget.max_results
                                || remaining == 0
                                || !seen.insert(uri)
                            {
                                continue;
                            }
                            let text = row
                                .get("abstract")
                                .and_then(Value::as_str)
                                .ok_or_else(|| error("invalid_response"))?;
                            let text: String = text.chars().take(remaining).collect();
                            remaining = remaining.saturating_sub(text.chars().count());
                            if !text.is_empty() {
                                evidence.push(Evidence {
                                    id: uri.into(),
                                    text,
                                    document_id: Some(
                                        uri.strip_prefix(&format!("{}/", self.home))
                                            .ok_or_else(|| error("scope_mismatch"))?
                                            .into(),
                                    ),
                                });
                            }
                        }
                    }
                    if !value.is_object()
                        || !["memories", "resources", "skills"]
                            .iter()
                            .any(|key| value.get(*key).is_some_and(Value::is_array))
                    {
                        return Err(error("invalid_response"));
                    }
                    Ok(BackendResponse {
                        evidence,
                        ..Default::default()
                    })
                }
                BackendRequest::Inspect { document_id } => {
                    let value = self
                        .call(
                            Method::GET,
                            "/api/v1/content/read",
                            "/api/v1/content/read",
                            &[("uri", self.uri(&document_id)?)],
                            None,
                            BODY_LIMIT,
                            DEADLINE,
                            &cancel,
                        )
                        .await?;
                    if !value.is_string() {
                        return Err(error("invalid_response"));
                    }
                    Ok(BackendResponse {
                        data: json!({"document_id":document_id,"text":self.http.public_data(value)}),
                        ..Default::default()
                    })
                }
                BackendRequest::DeleteDocument { document_id, .. } => {
                    let value = self
                        .call(
                            Method::DELETE,
                            "/api/v1/fs",
                            "/api/v1/fs",
                            &[
                                ("uri", self.uri(&document_id)?),
                                ("recursive", "true".into()),
                            ],
                            None,
                            BODY_LIMIT,
                            DEADLINE,
                            &cancel,
                        )
                        .await?;
                    let operation = if let Some(id) = value.get("task_id").and_then(Value::as_str) {
                        identifier(id)?;
                        Some(ServiceOperation {
                            id: id.into(),
                            state: OperationState::Submitted,
                        })
                    } else {
                        None
                    };
                    Ok(BackendResponse {
                        operation,
                        data: self.http.public_data(value),
                        ..Default::default()
                    })
                }
                BackendRequest::OperationStatus { operation_id } => {
                    self.task(&operation_id, false, &cancel).await
                }
                BackendRequest::CancelOperation { operation_id } => {
                    self.task(&operation_id, true, &cancel).await
                }
                _ => Err(error("unsupported_operation")),
            }
        };
        bounded(work, timeout, &cancel).await
    }
}

fn configuration(
    connection: &Connection,
) -> Result<&BTreeMap<String, OpenVikingBinding>, MemoryError> {
    match &connection.options {
        Some(BackendOptions::OpenViking { bindings }) => Ok(bindings),
        _ => Err(error("bot_authorization_missing")),
    }
}
/// Pure validation; no network and no public secret-bearing data or Debug implementation.
pub fn validate_connection(connection: &Connection) -> Result<(), MemoryError> {
    validate_settings(connection)?;
    validated_endpoint(connection).map(|_| ())
}
fn validate_settings(connection: &Connection) -> Result<(), MemoryError> {
    if connection.backend != BackendKind::OpenViking {
        return Err(error("backend_mismatch"));
    }
    if connection.secret.is_some() {
        return Err(error("bot_authorization_required"));
    }
    let config = configuration(connection)?;
    if config.is_empty() || config.len() > 1024 {
        return Err(error("invalid_backend_configuration"));
    }
    let mut identities = BTreeSet::new();
    let mut user_keys = BTreeSet::new();
    for (namespace, authorization) in config {
        if namespace.len() != 64
            || !namespace
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(error("invalid_namespace"));
        }
        let (account, user, key) = authorization_values(authorization);
        identifier(account)?;
        identifier(user)?;
        if key.is_empty() || key.len() > 8192 {
            return Err(error("invalid_secret"));
        }
        header(key)?;
        if !identities.insert((account, user)) {
            return Err(error("shared_bot_identity"));
        }
        if authorization.mode == OpenVikingAuthMode::UserKey && !user_keys.insert(key) {
            return Err(error("shared_bot_key"));
        }
    }
    Ok(())
}
fn verify_health(value: &Value, authorization: &OpenVikingBinding) -> Result<(), MemoryError> {
    let (account, user, _) = authorization_values(authorization);
    if value.get("version").and_then(Value::as_str) != Some(BASELINE) {
        return Err(error("unsupported_server_version"));
    }
    if value.get("healthy").and_then(Value::as_bool) != Some(true) {
        return Err(error("service_unhealthy"));
    }
    if value.get("auth_mode").and_then(Value::as_str) != Some(authorization_mode(authorization))
        || value.get("account_id").and_then(Value::as_str) != Some(account)
        || value.get("user_id").and_then(Value::as_str) != Some(user)
        || value.get("role").and_then(Value::as_str) != Some("user")
    {
        return Err(error("authorization_binding_mismatch"));
    }
    Ok(())
}
fn verify_status(value: &Value, authorization: &OpenVikingBinding) -> Result<(), MemoryError> {
    let result = result(value)?;
    if result.get("initialized").and_then(Value::as_bool) != Some(true)
        || result.get("user").and_then(Value::as_str) != Some(authorization.user_id.as_str())
    {
        return Err(error("authorization_binding_mismatch"));
    }
    Ok(())
}
fn result(value: &Value) -> Result<&Value, MemoryError> {
    if value.get("status").and_then(Value::as_str) != Some("ok") {
        return Err(error("service_failed"));
    }
    value.get("result").ok_or_else(|| error("invalid_response"))
}
fn relative_path(path: &str) -> Result<(), MemoryError> {
    if path.is_empty() || path.len() > 2048 || path.starts_with('/') || path.ends_with('/') {
        return Err(error("scope_mismatch"));
    }
    for part in path.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.trim() != part
            || part
                .chars()
                .any(|c| c.is_control() || ['%', '?', '#', '\\', '$', '{', '}'].contains(&c))
        {
            return Err(error("scope_mismatch"));
        }
    }
    Ok(())
}
fn validate_uri(uri: &str, home: &str) -> Result<(), MemoryError> {
    if uri == home {
        return Ok(());
    }
    let tail = uri
        .strip_prefix(home)
        .and_then(|tail| tail.strip_prefix('/'))
        .ok_or_else(|| error("scope_mismatch"))?;
    relative_path(tail)
}
fn validate_result_scope(value: &Value, home: &str) -> Result<(), MemoryError> {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if key == "uri" || key.ends_with("_uri") {
                    if !value.is_null() {
                        validate_uri(value.as_str().ok_or_else(|| error("scope_mismatch"))?, home)?;
                    }
                }
                if key.ends_with("_uris") {
                    for uri in value.as_array().ok_or_else(|| error("scope_mismatch"))? {
                        validate_uri(uri.as_str().ok_or_else(|| error("scope_mismatch"))?, home)?;
                    }
                }
                if key == "resource_id" {
                    if let Some(uri) = value.as_str().filter(|v| v.starts_with("viking://")) {
                        validate_uri(uri, home)?;
                    }
                }
                validate_result_scope(value, home)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                validate_result_scope(item, home)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn action_id(body: &Value) -> Result<&str, MemoryError> {
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| error("invalid_request"))?;
    identifier(id)?;
    Ok(id)
}
fn parse_task(value: &Value) -> Result<ServiceOperation, MemoryError> {
    let id = string(value, "task_id")?;
    identifier(id)?;
    let state = match string(value, "status")? {
        "pending" => OperationState::Submitted,
        "running" | "cancelling" => OperationState::Processing,
        "completed" => OperationState::Completed,
        "failed" | "cancelled" => OperationState::Failed,
        _ => return Err(error("invalid_response")),
    };
    Ok(ServiceOperation {
        id: id.into(),
        state,
    })
}
fn task_summary(value: &Value) -> Value {
    json!({"task_id":value.get("task_id"),"status":value.get("status"),"quiescence_verified":false})
}

#[cfg(test)]
#[path = "openviking_tests.rs"]
mod tests;
