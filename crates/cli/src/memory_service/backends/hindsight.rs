use super::http::*;
use crate::memory_service::types::*;
use reqwest::{header::HeaderMap, Method};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

const BANK: &str = "/v1/default/banks/{bank_id}";
const BASELINE: &str = "0.10.2";

/// Bound to one core-created namespace and immutable connection revision.
/// Construction performs read-only contract negotiation, never bank creation.
pub struct Hindsight {
    http: MemoryHttp,
    scope: MemoryScope,
    schema: Value,
    capabilities: Capabilities,
    config_writes: bool,
    worker: bool,
}
impl Hindsight {
    pub async fn connect(
        connection: &Connection,
        scope: &MemoryScope,
        cancel: CancellationToken,
    ) -> Result<Self, MemoryError> {
        let work = async {
            validate_settings(connection)?;
            bind(scope)?;
            let mut headers = HeaderMap::new();
            if let Some(key) = connection.secret.as_deref() {
                headers.insert("Authorization", header(&format!("Bearer {key}"))?);
            }
            let http = MemoryHttp::new(connection, headers)?;
            let version = http
                .request(
                    Method::GET,
                    "/version",
                    &[],
                    None,
                    BODY_LIMIT,
                    DEADLINE,
                    &cancel,
                )
                .await?;
            if version.get("api_version").and_then(Value::as_str) != Some(BASELINE) {
                return Err(error("unsupported_server_version"));
            }
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
            if schema.pointer("/info/version").and_then(Value::as_str) != Some(BASELINE) {
                return Err(error("unsupported_server_schema"));
            }
            let config_writes = version
                .pointer("/features/bank_config_api")
                .and_then(Value::as_bool)
                == Some(true);
            let worker = version.pointer("/features/worker").and_then(Value::as_bool) == Some(true);
            let observations = version
                .pointer("/features/observations")
                .and_then(Value::as_bool)
                == Some(true);
            let has =
                |suffix: &str, method: &str| route(&schema, &format!("{BANK}{suffix}"), method);
            let mut capabilities = Capabilities {
                retain: has("/memories", "post"),
                recall: has("/memories/recall", "post"),
                inspect: has("/documents/{document_id}", "get"),
                delete_document: has("/documents/{document_id}", "delete"),
                clear: has("", "delete"),
                operation_status: has("/operations/{operation_id}", "get"),
                cancel_operation: has("/operations/{operation_id}", "delete"),
                reflect: has("/reflect", "post"),
                // Advertise lost-response dedup only when this server's request schema supports it.
                idempotent_retain: worker
                    && has("/operations/{operation_id}", "get")
                    && request_property(
                        &schema,
                        &format!("{BANK}/memories"),
                        "post",
                        "operation_id",
                    ),
                ..Default::default()
            };
            for (feature, suffix, method, enabled) in [
                (AdvancedFeature::BankProfile, "/config", "get", true),
                (AdvancedFeature::BankConfig, "/config", "get", true),
                (AdvancedFeature::Directives, "/directives", "get", true),
                (AdvancedFeature::MentalModels, "/mental-models", "get", true),
                (
                    AdvancedFeature::MentalModelHistory,
                    "/mental-models/{mental_model_id}/history",
                    "get",
                    true,
                ),
                (
                    AdvancedFeature::Observations,
                    "/memories/list",
                    "get",
                    observations,
                ),
                (
                    AdvancedFeature::MemoryEdit,
                    "/memories/{memory_id}",
                    "patch",
                    true,
                ),
                (
                    AdvancedFeature::MemoryInvalidate,
                    "/memories/{memory_id}",
                    "patch",
                    true,
                ),
                (
                    AdvancedFeature::MemoryRestore,
                    "/memories/{memory_id}",
                    "patch",
                    true,
                ),
                (AdvancedFeature::Documents, "/documents", "get", true),
                (AdvancedFeature::Tasks, "/operations", "get", true),
            ] {
                if enabled && has(suffix, method) {
                    capabilities.advanced.insert(feature);
                }
            }
            use AdvancedFeature::*;
            let async_actions = worker && capabilities.operation_status;
            for (feature, action, suffix, method, enabled) in [
                (BankProfile, "get", "/config", "get", true),
                (BankProfile, "update", "/config", "patch", config_writes),
                (BankProfile, "reset", "/config", "delete", config_writes),
                (BankConfig, "get", "/config", "get", true),
                (BankConfig, "update", "/config", "patch", config_writes),
                (BankConfig, "reset", "/config", "delete", config_writes),
                (Directives, "list", "/directives", "get", true),
                (Directives, "create", "/directives", "post", true),
                (Directives, "get", "/directives/{directive_id}", "get", true),
                (
                    Directives,
                    "update",
                    "/directives/{directive_id}",
                    "patch",
                    true,
                ),
                (
                    Directives,
                    "delete",
                    "/directives/{directive_id}",
                    "delete",
                    true,
                ),
                (MentalModels, "list", "/mental-models", "get", true),
                (
                    MentalModels,
                    "create",
                    "/mental-models",
                    "post",
                    async_actions,
                ),
                (
                    MentalModels,
                    "get",
                    "/mental-models/{mental_model_id}",
                    "get",
                    true,
                ),
                (
                    MentalModels,
                    "update",
                    "/mental-models/{mental_model_id}",
                    "patch",
                    true,
                ),
                (
                    MentalModels,
                    "delete",
                    "/mental-models/{mental_model_id}",
                    "delete",
                    true,
                ),
                (
                    MentalModels,
                    "refresh",
                    "/mental-models/{mental_model_id}/refresh",
                    "post",
                    async_actions,
                ),
                (
                    MentalModels,
                    "history",
                    "/mental-models/{mental_model_id}/history",
                    "get",
                    true,
                ),
                (
                    MentalModelHistory,
                    "list",
                    "/mental-models/{mental_model_id}/history",
                    "get",
                    true,
                ),
                (
                    MentalModelHistory,
                    "get",
                    "/mental-models/{mental_model_id}/history",
                    "get",
                    true,
                ),
                (Observations, "list", "/memories/list", "get", true),
                (Observations, "scopes", "/observations/scopes", "get", true),
                (Observations, "clear", "/observations", "delete", true),
                (
                    Observations,
                    "clear_derived",
                    "/memories/{memory_id}/observations",
                    "delete",
                    true,
                ),
                (MemoryEdit, "get", "/memories/{memory_id}", "get", true),
                (
                    MemoryEdit,
                    "history",
                    "/memories/{memory_id}/history",
                    "get",
                    true,
                ),
                (MemoryEdit, "update", "/memories/{memory_id}", "patch", true),
                (
                    MemoryInvalidate,
                    "get",
                    "/memories/{memory_id}",
                    "get",
                    true,
                ),
                (
                    MemoryInvalidate,
                    "history",
                    "/memories/{memory_id}/history",
                    "get",
                    true,
                ),
                (
                    MemoryInvalidate,
                    "update",
                    "/memories/{memory_id}",
                    "patch",
                    true,
                ),
                (MemoryRestore, "get", "/memories/{memory_id}", "get", true),
                (
                    MemoryRestore,
                    "history",
                    "/memories/{memory_id}/history",
                    "get",
                    true,
                ),
                (
                    MemoryRestore,
                    "update",
                    "/memories/{memory_id}",
                    "patch",
                    true,
                ),
                (Documents, "list", "/documents", "get", true),
                (Documents, "get", "/documents/{document_id}", "get", true),
                (
                    Documents,
                    "delete",
                    "/documents/{document_id}",
                    "delete",
                    true,
                ),
                (
                    Documents,
                    "chunks",
                    "/documents/{document_id}/chunks",
                    "get",
                    true,
                ),
                (Tasks, "list", "/operations", "get", true),
                (Tasks, "get", "/operations/{operation_id}", "get", true),
                (
                    Tasks,
                    "cancel",
                    "/operations/{operation_id}",
                    "delete",
                    true,
                ),
                (
                    Tasks,
                    "delete_record",
                    "/operations/{operation_id}/delete",
                    "delete",
                    true,
                ),
            ] {
                if enabled && capabilities.advanced.contains(&feature) && has(suffix, method) {
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
                schema,
                capabilities,
                config_writes,
                worker,
            })
        };
        bounded(work, DEADLINE, &cancel).await
    }

    async fn bank_request(
        &self,
        method: Method,
        suffix: &str,
        template: &str,
        query: &[(&str, String)],
        body: Option<Value>,
        max_bytes: usize,
        timeout: std::time::Duration,
        cancel: &CancellationToken,
    ) -> Result<Value, MemoryError> {
        if !route(
            &self.schema,
            &format!("{BANK}{template}"),
            method_name(&method),
        ) {
            return Err(error("unsupported_operation"));
        }
        let value = self
            .http
            .request(
                method,
                &format!("/v1/default/banks/{}{suffix}", self.scope.namespace),
                query,
                body,
                max_bytes,
                timeout,
                cancel,
            )
            .await?;
        validate_bank(&value, &self.scope.namespace)?;
        Ok(value)
    }

    async fn advanced(
        &self,
        feature: AdvancedFeature,
        action: &str,
        mut body: Value,
        cancel: &CancellationToken,
    ) -> Result<BackendResponse, MemoryError> {
        reject_selectors(&body)?;
        let mut query = vec![];
        let mut submit = false;
        let (method, suffix, template, payload): (Method, String, String, Option<Value>) =
            match feature {
                AdvancedFeature::BankProfile | AdvancedFeature::BankConfig => match action {
                    "get" => {
                        fields(&body, &[])?;
                        (Method::GET, "/config".into(), "/config".into(), None)
                    }
                    "update" => {
                        if !self.config_writes {
                            return Err(error("unsupported_operation"));
                        }
                        fields(&body, &["updates"])?;
                        let updates = body
                            .get("updates")
                            .ok_or_else(|| error("invalid_request"))?;
                        fields(
                            updates,
                            &[
                                "disposition_skepticism",
                                "disposition_literalism",
                                "disposition_empathy",
                                "reflect_mission",
                                "retain_mission",
                                "retain_chunk_size",
                                "retain_extraction_mode",
                                "retain_custom_instructions",
                                "retain_extract_labels",
                                "recall_max_tokens",
                                "recall_budget",
                                "reflect_max_iterations",
                                "reflect_max_tokens",
                            ],
                        )?;
                        (
                            Method::PATCH,
                            "/config".into(),
                            "/config".into(),
                            Some(body),
                        )
                    }
                    "reset" => {
                        if !self.config_writes {
                            return Err(error("unsupported_operation"));
                        }
                        fields(&body, &[])?;
                        (Method::DELETE, "/config".into(), "/config".into(), None)
                    }
                    _ => return Err(error("unsupported_action")),
                },
                AdvancedFeature::Directives | AdvancedFeature::MentalModels => {
                    let mental = feature == AdvancedFeature::MentalModels;
                    let collection = if mental {
                        "/mental-models"
                    } else {
                        "/directives"
                    };
                    let id_template = if mental {
                        "/mental-models/{mental_model_id}"
                    } else {
                        "/directives/{directive_id}"
                    };
                    match action {
                        "list" => {
                            query = pagination(&body)?;
                            (Method::GET, collection.into(), collection.into(), None)
                        }
                        "create" => {
                            if mental {
                                if !self.worker || !self.capabilities.operation_status {
                                    return Err(error("unsupported_operation"));
                                }
                                submit = true;
                                fields(
                                    &body,
                                    &[
                                        "id",
                                        "name",
                                        "source_query",
                                        "tags",
                                        "max_tokens",
                                        "trigger",
                                    ],
                                )?;
                                required_text(&body, "name")?;
                                required_text(&body, "source_query")?;
                                no_background_refresh(&mut body)?;
                            } else {
                                fields(
                                    &body,
                                    &["name", "content", "priority", "is_active", "tags"],
                                )?;
                                required_text(&body, "name")?;
                                required_text(&body, "content")?;
                            }
                            (
                                Method::POST,
                                collection.into(),
                                collection.into(),
                                Some(body),
                            )
                        }
                        "get" | "delete" => {
                            fields(&body, &["id"])?;
                            let id = action_id(&body)?;
                            (
                                if action == "get" {
                                    Method::GET
                                } else {
                                    Method::DELETE
                                },
                                format!("{collection}/{id}"),
                                id_template.into(),
                                None,
                            )
                        }
                        "update" => {
                            if mental {
                                fields(
                                    &body,
                                    &[
                                        "id",
                                        "name",
                                        "source_query",
                                        "tags",
                                        "max_tokens",
                                        "trigger",
                                    ],
                                )?;
                                no_background_refresh(&mut body)?;
                            } else {
                                fields(
                                    &body,
                                    &["id", "name", "content", "priority", "is_active", "tags"],
                                )?;
                            }
                            let id = action_id(&body)?.to_owned();
                            body.as_object_mut().unwrap().remove("id");
                            (
                                Method::PATCH,
                                format!("{collection}/{id}"),
                                id_template.into(),
                                Some(body),
                            )
                        }
                        "refresh" if mental => {
                            if !self.worker || !self.capabilities.operation_status {
                                return Err(error("unsupported_operation"));
                            }
                            fields(&body, &["id"])?;
                            let id = action_id(&body)?;
                            submit = true;
                            (
                                Method::POST,
                                format!("{collection}/{id}/refresh"),
                                format!("{id_template}/refresh"),
                                None,
                            )
                        }
                        "history" if mental => {
                            fields(&body, &["id"])?;
                            let id = action_id(&body)?;
                            (
                                Method::GET,
                                format!("{collection}/{id}/history"),
                                format!("{id_template}/history"),
                                None,
                            )
                        }
                        _ => return Err(error("unsupported_action")),
                    }
                }
                AdvancedFeature::MentalModelHistory => {
                    if action != "list" && action != "get" {
                        return Err(error("unsupported_action"));
                    }
                    fields(&body, &["id"])?;
                    let id = action_id(&body)?;
                    (
                        Method::GET,
                        format!("/mental-models/{id}/history"),
                        "/mental-models/{mental_model_id}/history".into(),
                        None,
                    )
                }
                AdvancedFeature::Observations => match action {
                    "list" => {
                        query = pagination(&body)?;
                        query.push(("type", "observation".into()));
                        (
                            Method::GET,
                            "/memories/list".into(),
                            "/memories/list".into(),
                            None,
                        )
                    }
                    "scopes" => {
                        query = pagination(&body)?;
                        (
                            Method::GET,
                            "/observations/scopes".into(),
                            "/observations/scopes".into(),
                            None,
                        )
                    }
                    "clear" => {
                        fields(&body, &[])?;
                        (
                            Method::DELETE,
                            "/observations".into(),
                            "/observations".into(),
                            None,
                        )
                    }
                    "clear_derived" => {
                        fields(&body, &["id"])?;
                        let id = action_id(&body)?;
                        (
                            Method::DELETE,
                            format!("/memories/{id}/observations"),
                            "/memories/{memory_id}/observations".into(),
                            None,
                        )
                    }
                    _ => return Err(error("unsupported_action")),
                },
                AdvancedFeature::MemoryEdit
                | AdvancedFeature::MemoryInvalidate
                | AdvancedFeature::MemoryRestore => {
                    if action == "get" || action == "history" {
                        fields(&body, &["id"])?;
                        let id = action_id(&body)?;
                        let tail = if action == "history" { "/history" } else { "" };
                        (
                            Method::GET,
                            format!("/memories/{id}{tail}"),
                            format!("/memories/{{memory_id}}{tail}"),
                            None,
                        )
                    } else {
                        if action != "update" {
                            return Err(error("unsupported_action"));
                        }
                        if feature == AdvancedFeature::MemoryEdit {
                            fields(
                                &body,
                                &[
                                    "id",
                                    "text",
                                    "context",
                                    "occurred_start",
                                    "occurred_end",
                                    "tags",
                                ],
                            )?;
                            if object(&body)?.len() < 2 {
                                return Err(error("invalid_request"));
                            }
                        } else {
                            fields(&body, &["id", "reason"])?;
                            body["state"] =
                                json!(if feature == AdvancedFeature::MemoryInvalidate {
                                    "invalidated"
                                } else {
                                    "valid"
                                });
                        }
                        let id = action_id(&body)?.to_owned();
                        body.as_object_mut().unwrap().remove("id");
                        (
                            Method::PATCH,
                            format!("/memories/{id}"),
                            "/memories/{memory_id}".into(),
                            Some(body),
                        )
                    }
                }
                AdvancedFeature::Documents => match action {
                    "list" => {
                        query = pagination(&body)?;
                        (Method::GET, "/documents".into(), "/documents".into(), None)
                    }
                    "get" | "delete" | "chunks" => {
                        fields(&body, &["id"])?;
                        let id = action_id(&body)?;
                        let tail = if action == "chunks" { "/chunks" } else { "" };
                        if action == "chunks" {
                            query.push(("limit", "20".into()));
                        }
                        (
                            if action == "delete" {
                                Method::DELETE
                            } else {
                                Method::GET
                            },
                            format!("/documents/{id}{tail}"),
                            format!("/documents/{{document_id}}{tail}"),
                            None,
                        )
                    }
                    _ => return Err(error("unsupported_action")),
                },
                AdvancedFeature::Tasks => match action {
                    "list" => {
                        query = pagination(&body)?;
                        (
                            Method::GET,
                            "/operations".into(),
                            "/operations".into(),
                            None,
                        )
                    }
                    "get" | "cancel" | "delete_record" => {
                        fields(&body, &["id"])?;
                        let id = action_id(&body)?;
                        let tail = if action == "delete_record" {
                            "/delete"
                        } else {
                            ""
                        };
                        (
                            if action == "get" {
                                Method::GET
                            } else {
                                Method::DELETE
                            },
                            format!("/operations/{id}{tail}"),
                            format!("/operations/{{operation_id}}{tail}"),
                            None,
                        )
                    }
                    _ => return Err(error("unsupported_action")),
                },
                _ => return Err(error("unsupported_operation")),
            };
        let value = self
            .bank_request(
                method, &suffix, &template, &query, payload, BODY_LIMIT, DEADLINE, cancel,
            )
            .await?;
        let operation = if submit {
            let id = string(&value, "operation_id")?;
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

/// Pure validation for the core's encrypted connection-setting boundary.
pub fn validate_connection(connection: &Connection) -> Result<(), MemoryError> {
    validate_settings(connection)?;
    validated_endpoint(connection).map(|_| ())
}
fn validate_settings(connection: &Connection) -> Result<(), MemoryError> {
    if connection.backend != BackendKind::Hindsight {
        return Err(error("backend_mismatch"));
    }
    if connection
        .secret
        .as_ref()
        .is_some_and(|s| s.is_empty() || s.len() > 8192)
    {
        return Err(error("invalid_secret"));
    }
    if connection
        .options
        .as_ref()
        .is_some_and(|options| !matches!(options, BackendOptions::Hindsight {}))
    {
        return Err(error("backend_mismatch"));
    }
    if let Some(secret) = &connection.secret {
        header(secret)?;
    }
    Ok(())
}

#[async_trait::async_trait]
impl MemoryBackend for Hindsight {
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
            BackendRequest::Recall { budget, .. } | BackendRequest::Reflect { budget, .. } => {
                std::time::Duration::from_millis(budget.timeout_ms)
            }
            _ => DEADLINE,
        };
        let work = async {
            check_scope(&self.scope, scope)?;
            if !self.capabilities.supports(&request) {
                return Err(error("unsupported_operation"));
            }
            match request {
                BackendRequest::Health => {
                    let version = self
                        .http
                        .request(
                            Method::GET,
                            "/version",
                            &[],
                            None,
                            BODY_LIMIT,
                            DEADLINE,
                            &cancel,
                        )
                        .await?;
                    if version.get("api_version").and_then(Value::as_str) != Some(BASELINE) {
                        return Err(error("unsupported_server_version"));
                    }
                    self.http
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
                    Ok(BackendResponse {
                        data: json!({"healthy":true,"api_version":BASELINE,"bank_config_writes":self.config_writes,"worker":self.worker,"write_fence":false}),
                        ..Default::default()
                    })
                }
                BackendRequest::Retain { document } => {
                    identifier(&document.id)?;
                    if document.text.is_empty()
                        || document.text.len() > 32768
                        || document.request_id.is_empty()
                    {
                        return Err(error("invalid_document"));
                    }
                    let async_mode = self.worker && self.capabilities.operation_status;
                    let operation_id = operation_uuid(&scope.namespace, &document.request_id);
                    let mut body = json!({"items":[{"content":crate::memory::scrub(&document.text),"document_id":document.id,
                    "metadata":{"beans_content_hash":document.content_hash,"beans_request_id":document.request_id,
                    "beans_sources":serde_json::to_string(&document.sources).map_err(|_| error("invalid_document"))?}}],
                    "async":async_mode});
                    if async_mode && self.capabilities.idempotent_retain {
                        body["operation_id"] = json!(operation_id);
                    }
                    let value = self
                        .bank_request(
                            Method::POST,
                            "/memories",
                            "/memories",
                            &[],
                            Some(body),
                            BODY_LIMIT,
                            DEADLINE,
                            &cancel,
                        )
                        .await?;
                    if value.get("success").and_then(Value::as_bool) != Some(true)
                        || value.get("async").and_then(Value::as_bool) != Some(async_mode)
                        || value.get("bank_id").and_then(Value::as_str)
                            != Some(scope.namespace.as_str())
                        || value.get("items_count").and_then(Value::as_u64) != Some(1)
                    {
                        return Err(error("invalid_response"));
                    }
                    let operation = if async_mode {
                        let id = string(&value, "operation_id")?;
                        identifier(id)?;
                        if let Some(ids) = value.get("operation_ids").filter(|v| !v.is_null()) {
                            let ids = ids.as_array().ok_or_else(|| error("invalid_response"))?;
                            if ids.len() != 1 || ids[0].as_str() != Some(id) {
                                return Err(error("operation_mismatch"));
                            }
                        }
                        if self.capabilities.idempotent_retain && id != operation_id {
                            return Err(error("operation_mismatch"));
                        }
                        Some(ServiceOperation {
                            id: id.into(),
                            state: OperationState::Submitted,
                        })
                    } else {
                        Some(ServiceOperation {
                            id: document.request_id,
                            state: OperationState::Completed,
                        })
                    };
                    Ok(BackendResponse {
                        operation,
                        ..Default::default()
                    })
                }
                BackendRequest::Recall { query, budget } => {
                    self.retrieve(query, budget, false, &cancel).await
                }
                BackendRequest::Reflect { query, budget } => {
                    self.retrieve(query, budget, true, &cancel).await
                }
                BackendRequest::Inspect { document_id } => {
                    identifier(&document_id)?;
                    let value = self
                        .bank_request(
                            Method::GET,
                            &format!("/documents/{document_id}"),
                            "/documents/{document_id}",
                            &[],
                            None,
                            BODY_LIMIT,
                            DEADLINE,
                            &cancel,
                        )
                        .await?;
                    if string(&value, "id")? != document_id {
                        return Err(error("document_mismatch"));
                    }
                    Ok(BackendResponse {
                        data: self.http.public_data(value),
                        ..Default::default()
                    })
                }
                BackendRequest::DeleteDocument { document_id, .. } => {
                    identifier(&document_id)?;
                    let value = self
                        .bank_request(
                            Method::DELETE,
                            &format!("/documents/{document_id}"),
                            "/documents/{document_id}",
                            &[],
                            None,
                            BODY_LIMIT,
                            DEADLINE,
                            &cancel,
                        )
                        .await?;
                    successful_delete(&value)?;
                    if string(&value, "document_id")? != document_id {
                        return Err(error("document_mismatch"));
                    }
                    Ok(BackendResponse {
                        data: self.http.public_data(value),
                        ..Default::default()
                    })
                }
                BackendRequest::Clear { .. } => {
                    let value = self
                        .bank_request(
                            Method::DELETE,
                            "",
                            "",
                            &[],
                            None,
                            BODY_LIMIT,
                            DEADLINE,
                            &cancel,
                        )
                        .await?;
                    successful_delete(&value)?;
                    Ok(BackendResponse {
                        data: self.http.public_data(value),
                        ..Default::default()
                    })
                }
                BackendRequest::OperationStatus { operation_id } => {
                    identifier(&operation_id)?;
                    let value = self
                        .bank_request(
                            Method::GET,
                            &format!("/operations/{operation_id}"),
                            "/operations/{operation_id}",
                            &[],
                            None,
                            BODY_LIMIT,
                            DEADLINE,
                            &cancel,
                        )
                        .await?;
                    let operation = parse_operation(&value)?;
                    if operation.id != operation_id {
                        return Err(error("operation_mismatch"));
                    }
                    Ok(BackendResponse {
                        operation: Some(operation),
                        data: self.http.public_data(value),
                        ..Default::default()
                    })
                }
                BackendRequest::CancelOperation { operation_id } => {
                    identifier(&operation_id)?;
                    let value = self
                        .bank_request(
                            Method::DELETE,
                            &format!("/operations/{operation_id}"),
                            "/operations/{operation_id}",
                            &[],
                            None,
                            BODY_LIMIT,
                            DEADLINE,
                            &cancel,
                        )
                        .await?;
                    if value.get("success").and_then(Value::as_bool) != Some(true)
                        || string(&value, "operation_id")? != operation_id
                    {
                        return Err(error("invalid_response"));
                    }
                    // Cooperative cancellation is not proof that all in-flight writes stopped.
                    Ok(BackendResponse {
                        operation: Some(ServiceOperation {
                            id: operation_id,
                            state: OperationState::Failed,
                        }),
                        data: json!({"status":"cancelled","quiescence_verified":false}),
                        ..Default::default()
                    })
                }
                BackendRequest::Advanced {
                    feature,
                    action,
                    body,
                } => self.advanced(feature, &action, body, &cancel).await,
            }
        };
        bounded(work, timeout, &cancel).await
    }
}

impl Hindsight {
    async fn retrieve(
        &self,
        query: String,
        budget: RecallBudget,
        reflect: bool,
        cancel: &CancellationToken,
    ) -> Result<BackendResponse, MemoryError> {
        budget.validate()?;
        if query.is_empty() || query.chars().count() > 4096 {
            return Err(error("invalid_query"));
        }
        let suffix = if reflect {
            "/reflect"
        } else {
            "/memories/recall"
        };
        let mut body = json!({"query":query,"budget":"low","max_tokens":(budget.max_context_chars / 4).max(1)});
        if reflect {
            body["include"] = json!({"facts":{}});
        }
        let value = self
            .bank_request(
                Method::POST,
                suffix,
                suffix,
                &[],
                Some(body),
                budget.max_bytes,
                std::time::Duration::from_millis(budget.timeout_ms),
                cancel,
            )
            .await?;
        let public = self.http.public_data(value);
        let results = if reflect {
            public
                .pointer("/based_on/memories")
                .and_then(Value::as_array)
        } else {
            Some(
                public
                    .get("results")
                    .and_then(Value::as_array)
                    .ok_or_else(|| error("invalid_response"))?,
            )
        };
        let mut evidence = vec![];
        let mut remaining = budget.max_context_chars;
        if let Some(results) = results {
            for row in results.iter().take(budget.max_results) {
                let id = string(row, "id")?;
                let text = string(row, "text")?;
                let text: String = text.chars().take(remaining).collect();
                remaining = remaining.saturating_sub(text.chars().count());
                if !text.is_empty() {
                    evidence.push(Evidence {
                        id: id.into(),
                        text,
                        document_id: row
                            .get("document_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    });
                }
            }
        }
        let data = if reflect {
            let text = string(&public, "text")?;
            json!({"text":text.chars().take(budget.max_context_chars).collect::<String>()})
        } else {
            Value::Null
        };
        Ok(BackendResponse {
            evidence,
            data,
            ..Default::default()
        })
    }
}

fn validate_bank(value: &Value, namespace: &str) -> Result<(), MemoryError> {
    match value {
        Value::Object(map) => {
            if let Some(bank) = map.get("bank_id") {
                if bank.as_str() != Some(namespace) {
                    return Err(error("scope_mismatch"));
                }
            }
            for value in map.values() {
                validate_bank(value, namespace)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_bank(value, namespace)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn reject_selectors(value: &Value) -> Result<(), MemoryError> {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if [
                    "bank",
                    "bank_id",
                    "namespace",
                    "account",
                    "account_id",
                    "tenant",
                    "tenant_id",
                    "uri",
                    "target_uri",
                    "connection_id",
                    "api_key",
                    "secret",
                    "base_url",
                    "endpoint",
                ]
                .iter()
                .any(|selector| selector.eq_ignore_ascii_case(key))
                {
                    return Err(error("invalid_request"));
                }
                reject_selectors(value)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                reject_selectors(item)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn required_text(value: &Value, key: &str) -> Result<(), MemoryError> {
    let text = value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| error("invalid_request"))?;
    if text.is_empty() || text.len() > 32768 {
        return Err(error("invalid_request"));
    }
    Ok(())
}
fn action_id(value: &Value) -> Result<&str, MemoryError> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| error("invalid_request"))?;
    identifier(id)?;
    Ok(id)
}
fn pagination(value: &Value) -> Result<Vec<(&'static str, String)>, MemoryError> {
    fields(value, &["limit", "offset"])?;
    let limit = value
        .get("limit")
        .map(|v| v.as_u64().ok_or_else(|| error("invalid_request")))
        .transpose()?
        .unwrap_or(20);
    let offset = value
        .get("offset")
        .map(|v| v.as_u64().ok_or_else(|| error("invalid_request")))
        .transpose()?
        .unwrap_or(0);
    if !(1..=20).contains(&limit) || offset > 100_000 {
        return Err(error("invalid_request"));
    }
    Ok(vec![
        ("limit", limit.to_string()),
        ("offset", offset.to_string()),
    ])
}
fn no_background_refresh(value: &mut Value) -> Result<(), MemoryError> {
    if let Some(trigger) = value.get("trigger") {
        fields(
            trigger,
            &[
                "refresh_after_consolidation",
                "refresh_cron",
                "budget",
                "mode",
                "min_refresh_interval_seconds",
            ],
        )?;
        if trigger
            .get("refresh_after_consolidation")
            .is_some_and(|v| v.as_bool() != Some(false))
            || trigger.get("refresh_cron").is_some_and(|v| !v.is_null())
        {
            return Err(error("background_refresh_not_authorized"));
        }
    }
    if value.get("trigger").is_none() {
        value["trigger"] = json!({});
    }
    value["trigger"]["refresh_after_consolidation"] = json!(false);
    value["trigger"]["refresh_cron"] = Value::Null;
    Ok(())
}
fn operation_uuid(namespace: &str, request_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"beans.memory.hindsight.operation.v1\0");
    hash.update(namespace.as_bytes());
    hash.update(request_id.as_bytes());
    let bytes = hash.finalize();
    let mut id = [0; 16];
    id.copy_from_slice(&bytes[..16]);
    id[6] = (id[6] & 0x0f) | 0x50;
    id[8] = (id[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(id).to_string()
}
fn parse_operation(value: &Value) -> Result<ServiceOperation, MemoryError> {
    let id = string(value, "operation_id")?;
    identifier(id)?;
    let state = match string(value, "status")? {
        "pending" | "queued" | "accepted" => OperationState::Submitted,
        "processing" => OperationState::Processing,
        "completed" => OperationState::Completed,
        "failed" | "cancelled" => OperationState::Failed,
        "not_found" => OperationState::DeliveryUnknown,
        _ => return Err(error("invalid_response")),
    };
    Ok(ServiceOperation {
        id: id.into(),
        state,
    })
}
fn successful_delete(value: &Value) -> Result<(), MemoryError> {
    if value.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(error("invalid_response"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "hindsight_tests.rs"]
mod tests;
