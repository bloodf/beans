//! Official LanceDB 0.40 SDK local/Cloud adapter. Local directories are Runner resources,
//! not portable memory_config. Moving a database requires explicit validated export/import.
#[path = "lance_cloud.rs"]
mod cloud;
use super::pgvector::{vectors::*, VectorDistance, VectorSpace};
use crate::{
    embeddings::{Embedding, EmbeddingPurpose},
    memory_service::types::*,
};
use arrow_array::{
    types::Float32Type, Array, FixedSizeListArray, Float32Array, RecordBatch, StringArray,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use futures::TryStreamExt;
use lancedb::{
    query::{ExecutableQuery, QueryBase},
    remote::{ClientConfig, RetryConfig, TimeoutConfig},
    Table,
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
const TABLE: &str = "beans_memory_v1";
const TRANSFER_LIMIT: usize = 8 * 1024 * 1024;
/// The official SDK has no safe direct-client/proxy hook for the private local leg.
/// Core/UI must keep Cloud disabled with this reason; the local SDK remains available.
pub const CLOUD_TRANSPORT_UNAVAILABLE: &str = "lancedb_cloud_transport_unavailable";

pub enum LanceBinding {
    Local {
        directory: PathBuf,
        runner_id: String,
    },
    Cloud {
        uri: String,
        region: String,
        api_key: String,
    },
}
impl std::fmt::Debug for LanceBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local { runner_id, .. } => f
                .debug_struct("Local")
                .field("runner_id", runner_id)
                .finish_non_exhaustive(),
            Self::Cloud { .. } => f.write_str("Cloud { credentials: [redacted] }"),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferRow {
    pub document: FrozenDocument,
    pub vector: Vec<f32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanceTransfer {
    pub version: u32,
    pub namespace: String,
    pub space: VectorSpace,
    pub rows: Vec<TransferRow>,
}
pub struct LanceBackend {
    bound: MemoryScope,
    space: VectorSpace,
    connection: lancedb::Connection,
    embedding: Option<Arc<dyn Embedding>>,
    write: Mutex<()>,
    local: bool,
    _bridge: Option<cloud::CloudBridge>,
}
impl LanceBackend {
    /// Trusted Runner setup only. `create` is explicit provisioning permission, never a health side effect.
    pub async fn open(
        binding: LanceBinding,
        bound: MemoryScope,
        space: VectorSpace,
        embedding: Option<Arc<dyn Embedding>>,
        create: bool,
        cancel: CancellationToken,
    ) -> Result<Self, MemoryError> {
        if matches!(&binding, LanceBinding::Cloud { .. }) {
            return Err(MemoryError::new(CLOUD_TRANSPORT_UNAVAILABLE));
        }
        bind(&bound, &bound)?;
        space.validate()?;
        if embedding
            .as_ref()
            .is_some_and(|e| e.fingerprint().as_str() != space.fingerprint)
        {
            return Err(MemoryError::new("embedding_model_mismatch"));
        }
        bounded(cancel, 5000, create, async {
            let (connection, local, bridge) = match binding {
                LanceBinding::Local {
                    directory,
                    runner_id,
                } => {
                    if !directory.is_absolute() || runner_id.is_empty() {
                        return Err(MemoryError::new("invalid_lance_local_binding"));
                    }
                    if !create && !directory.is_dir() {
                        return Err(MemoryError::new(
                            "local_memory_unavailable_export_import_required",
                        ));
                    }
                    let path = directory
                        .to_str()
                        .ok_or_else(|| MemoryError::new("invalid_lance_local_binding"))?;
                    let connection = lancedb::connect(path)
                        .read_consistency_interval(Duration::ZERO)
                        .execute()
                        .await
                        .map_err(|_| MemoryError::new("lance_local_unavailable"))?;
                    (connection, true, None)
                }
                LanceBinding::Cloud { .. } => return Err(MemoryError::new(CLOUD_TRANSPORT_UNAVAILABLE)),
            };
            let table = match connection.open_table(TABLE).execute().await {
                Ok(table) => table,
                Err(lancedb::Error::TableNotFound { .. }) if create => connection
                    .create_empty_table(TABLE, schema(&space))
                    .execute()
                    .await
                    .map_err(|_| MemoryError::new("lance_initialization_failed"))?,
                Err(_) => return Err(MemoryError::new("lance_schema_not_ready")),
            };
            let actual = table.schema().await.map_err(|_| MemoryError::new("lance_schema_not_ready"))?;
            validate_table_schema(&space, actual.as_ref())?;
            let backend = Self {
                bound,
                space,
                connection,
                embedding,
                write: Mutex::new(()),
                local,
                _bridge: bridge,
            };
            Ok(backend)
        })
        .await
    }
    async fn ready(&self) -> Result<Table, MemoryError> {
        // Each SDK open owns a new schema cache. A cached Table::schema() can be
        // stale for 30 seconds, even when read_consistency_interval is zero.
        let table = self.connection.open_table(TABLE).execute().await
            .map_err(|_| MemoryError::new("lance_schema_not_ready"))?;
        let actual = table.schema().await.map_err(|_| MemoryError::new("lance_schema_not_ready"))?;
        validate_table_schema(&self.space, actual.as_ref())?;
        Ok(table)
    }
    fn predicate(&self, scope: &MemoryScope, id: Option<&str>) -> Result<String, MemoryError> {
        bind(&self.bound, scope)?;
        let base = format!(
            "namespace = '{}' AND fingerprint = '{}'",
            scope.namespace, self.space.fingerprint
        );
        Ok(match id {
            Some(id) => format!("{base} AND document_id = {}", literal(id)?),
            None => base,
        })
    }
    pub async fn retain_vector(
        &self,
        scope: &MemoryScope,
        document: FrozenDocument,
        fingerprint: &str,
        vector: Vec<f32>,
        cancel: CancellationToken,
    ) -> Result<BackendResponse, MemoryError> {
        bind(&self.bound, scope)?;
        document_valid(&document)?;
        self.space.validate_raw(fingerprint, &vector)?;
        bounded(cancel, 5000, true, async {
            let _write = self.write.lock().await;
            let table = self.ready().await?;
            self.merge(&table, scope, vec![TransferRow { document, vector }]).await?;
            Ok(BackendResponse {
                data: serde_json::json!({"stored":true}),
                ..Default::default()
            })
        })
        .await
    }
    async fn merge(&self, table: &Table, scope: &MemoryScope, rows: Vec<TransferRow>) -> Result<(), MemoryError> {
        let batch = records(&self.space, &scope.namespace, &rows)?;
        let mut merge = table.merge_insert(&["namespace", "document_id", "chunk_id", "fingerprint"]);
        merge
            .when_matched_update_all(Some(format!(
                "target.namespace = '{}' AND target.fingerprint = '{}'",
                scope.namespace, self.space.fingerprint
            )))
            .when_not_matched_insert_all()
            .timeout(Duration::from_secs(5));
        let schema = batch.schema();
        let reader = arrow_array::RecordBatchIterator::new(std::iter::once(Ok(batch)), schema);
        merge_result(merge.execute(Box::new(reader)).await)?;
        Ok(())
    }
    pub async fn recall_vector(
        &self,
        scope: &MemoryScope,
        fingerprint: &str,
        vector: Vec<f32>,
        budget: RecallBudget,
        cancel: CancellationToken,
    ) -> Result<BackendResponse, MemoryError> {
        let predicate = self.predicate(scope, None)?;
        budget.validate()?;
        self.space.validate_raw(fingerprint, &vector)?;
        bounded(cancel, budget.timeout_ms, false, async {
            let table = self.ready().await?;
            let distance = match self.space.distance {
                VectorDistance::Cosine => lancedb::DistanceType::Cosine,
                VectorDistance::Euclidean => lancedb::DistanceType::L2,
                VectorDistance::Dot => lancedb::DistanceType::Dot,
            };
            let batches = table.query().only_if(predicate).limit(budget.max_results)
                .nearest_to(vector).map_err(|_| MemoryError::new("invalid_embedding_vector"))?
                .distance_type(distance).bypass_vector_index().execute().await
                .map_err(|_| MemoryError::new("lance_query_failed"))?
                .try_collect::<Vec<_>>().await.map_err(|_| MemoryError::new("lance_query_failed"))?;
            let rows = decode(&self.space, &scope.namespace, &batches, budget.max_results)?;
            Ok(BackendResponse {
                evidence: bounded_evidence(rows.into_iter().map(|r| r.document), &budget),
                ..Default::default()
            })
        })
        .await
    }
    async fn inspect(
        &self,
        scope: &MemoryScope,
        id: &str,
        cancel: CancellationToken,
    ) -> Result<BackendResponse, MemoryError> {
        let predicate = self.predicate(scope, Some(id))?;
        bounded(cancel, 5000, false, async {
            let table = self.ready().await?;
            let batches = table.query().only_if(predicate).limit(2).execute().await
                .map_err(|_| MemoryError::new("lance_query_failed"))?
                .try_collect::<Vec<_>>().await.map_err(|_| MemoryError::new("lance_query_failed"))?;
            let mut rows = decode(&self.space, &scope.namespace, &batches, 1)?;
            let document = rows.pop().map(|r| r.document);
            if document.as_ref().is_some_and(|doc| doc.id != id) {
                return Err(MemoryError::new("invalid_memory_response"));
            }
            Ok(BackendResponse {
                document,
                ..Default::default()
            })
        })
        .await
    }
    async fn delete(
        &self,
        scope: &MemoryScope,
        id: Option<&str>,
        cancel: CancellationToken,
    ) -> Result<BackendResponse, MemoryError> {
        let predicate = self.predicate(scope, id)?;
        bounded(cancel, 5000, true, async {
            let _write = self.write.lock().await;
            let table = self.ready().await?;
            table.delete(predicate.as_str()).await.map_err(|_| MemoryError::new("delivery_unknown"))?;
            Ok(BackendResponse {
                data: serde_json::json!({"deleted":true,"history_may_retain_data":true}),
                ..Default::default()
            })
        })
        .await
    }
    /// Explicit local transfer only. Export contains plaintext provenance/vectors; UI must handle privately.
    pub async fn export(
        &self,
        scope: &MemoryScope,
        cancel: CancellationToken,
    ) -> Result<LanceTransfer, MemoryError> {
        let predicate = self.predicate(scope, None)?;
        if !self.local {
            return Err(MemoryError::new("lance_local_transfer_only"));
        }
        bounded(cancel, 5000, false, async {
            let table = self.ready().await?;
            let batches = table.query().only_if(predicate).limit(1001).execute().await
                .map_err(|_| MemoryError::new("lance_export_failed"))?
                .try_collect::<Vec<_>>().await.map_err(|_| MemoryError::new("lance_export_failed"))?;
            let rows = decode(&self.space, &scope.namespace, &batches, 1000)?;
            let transfer = LanceTransfer {
                version: 1,
                namespace: scope.namespace.clone(),
                space: self.space.clone(),
                rows,
            };
            if serde_json::to_vec(&transfer)
                .map_err(|_| MemoryError::new("lance_export_failed"))?
                .len()
                > TRANSFER_LIMIT
            {
                return Err(MemoryError::new("lance_transfer_limit_exceeded"));
            }
            Ok(transfer)
        })
        .await
    }
    /// Validates the complete export before one atomic SDK merge. No raw DB-file synchronization.
    pub async fn import(
        &self,
        scope: &MemoryScope,
        transfer: LanceTransfer,
        approved: bool,
        cancel: CancellationToken,
    ) -> Result<(), MemoryError> {
        bind(&self.bound, scope)?;
        if !approved {
            return Err(MemoryError::new("lance_import_approval_required"));
        }
        if !self.local {
            return Err(MemoryError::new("lance_local_transfer_only"));
        }
        if transfer.version != 1
            || transfer.namespace != scope.namespace
            || transfer.space != self.space
        {
            return Err(MemoryError::new("lance_transfer_scope_or_space_mismatch"));
        }
        if transfer.rows.len() > 1000
            || serde_json::to_vec(&transfer)
                .map_err(|_| MemoryError::new("invalid_lance_transfer"))?
                .len()
                > TRANSFER_LIMIT
        {
            return Err(MemoryError::new("lance_transfer_limit_exceeded"));
        }
        let mut ids = std::collections::BTreeSet::new();
        for row in &transfer.rows {
            document_valid(&row.document)?;
            self.space
                .validate_raw(&transfer.space.fingerprint, &row.vector)?;
            if !ids.insert(&row.document.id) {
                return Err(MemoryError::new("invalid_lance_transfer"));
            }
        }
        bounded(cancel, 5000, true, async {
            let _write = self.write.lock().await;
            let table = self.ready().await?;
            if !transfer.rows.is_empty() {
                self.merge(&table, scope, transfer.rows).await?;
            }
            Ok(())
        })
        .await
    }
    fn provider(&self) -> Result<&dyn Embedding, MemoryError> {
        self.embedding
            .as_deref()
            .ok_or_else(|| MemoryError::new("embedding_unavailable"))
    }
}
#[async_trait::async_trait]
impl MemoryBackend for LanceBackend {
    fn capabilities(&self) -> Capabilities {
        capabilities()
    }
    async fn execute(
        &self,
        scope: &MemoryScope,
        request: BackendRequest,
        cancel: CancellationToken,
    ) -> Result<BackendResponse, MemoryError> {
        bind(&self.bound, scope)?;
        match request {
            BackendRequest::Health=>bounded(cancel,5000,false,async{self.ready().await?;Ok(BackendResponse{data:serde_json::json!({"ready":true,"local":self.local,"exact_search":true,"history_may_retain_data":true}),..Default::default()})}).await,
            BackendRequest::Retain{document}=>{document_valid(&document)?;let vector=self.space.embed(self.provider()?,EmbeddingPurpose::Document,document.text.clone(),cancel.clone()).await?;self.retain_vector(scope,document,&self.space.fingerprint,vector,cancel).await},
            BackendRequest::Recall{query,budget}=>{budget.validate()?;if query.chars().count()>4096{return Err(MemoryError::new("invalid_memory_query"));}let vector=self.space.embed(self.provider()?,EmbeddingPurpose::Query,query,cancel.clone()).await?;self.recall_vector(scope,&self.space.fingerprint,vector,budget,cancel).await},
            BackendRequest::Inspect{document_id}=>self.inspect(scope,&document_id,cancel).await,
            BackendRequest::DeleteDocument{document_id,..}=>self.delete(scope,Some(&document_id),cancel).await,
            BackendRequest::Clear{..}=>self.delete(scope,None,cancel).await,
            BackendRequest::Advanced{feature:AdvancedFeature::MemoryEdit,action,body} if action=="edit"=>{
                let edit:EditDocument=serde_json::from_value(body).map_err(|_|MemoryError::new("invalid_memory_edit"))?;
                let mut doc=self.inspect(scope,&edit.document_id,cancel.clone()).await?.document.ok_or_else(||MemoryError::new("memory_document_not_found"))?;
                doc.text=edit.text;doc.content_hash=digest(doc.text.as_bytes());document_valid(&doc)?;
                let vector=self.space.embed(self.provider()?,EmbeddingPurpose::Document,doc.text.clone(),cancel.clone()).await?;
                self.retain_vector(scope,doc,&self.space.fingerprint,vector,cancel).await
            },
            _=>Err(MemoryError::new("memory_capability_unsupported"))
        }
    }
}
fn cloud_config() -> ClientConfig {
    ClientConfig {
        timeout_config: TimeoutConfig {
            timeout: Some(Duration::from_secs(5)),
            connect_timeout: Some(Duration::from_secs(2)),
            read_timeout: Some(Duration::from_secs(5)),
            ..Default::default()
        },
        retry_config: RetryConfig {
            retries: Some(0),
            connect_retries: Some(0),
            read_retries: Some(0),
            ..Default::default()
        },
        ..Default::default()
    }
}
fn cloud_target(uri: &str, region: &str, key: &str) -> Result<String, MemoryError> {
    let database = uri
        .strip_prefix("db://")
        .ok_or_else(|| MemoryError::new("invalid_lance_cloud_target"))?;
    let label = |v: &str| {
        !v.is_empty()
            && v.len() <= 63
            && v.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !v.starts_with('-')
            && !v.ends_with('-')
    };
    if !label(database)
        || !label(region)
        || key.is_empty()
        || key.len() > 4096
        || key.chars().any(char::is_control)
    {
        return Err(MemoryError::new("invalid_lance_cloud_target"));
    }
    Ok(database.into())
}
fn literal(id: &str) -> Result<String, MemoryError> {
    if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
        return Err(MemoryError::new("invalid_memory_document_id"));
    }
    Ok(format!("'{}'", id.replace('\'', "''")))
}
fn schema(space: &VectorSpace) -> SchemaRef {
    Arc::new(
        Schema::new(vec![
            Field::new("namespace", DataType::Utf8, false),
            Field::new("document_id", DataType::Utf8, false),
            Field::new("chunk_id", DataType::Utf8, false),
            Field::new("text", DataType::Utf8, false),
            Field::new("provenance", DataType::Utf8, false),
            Field::new("fingerprint", DataType::Utf8, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    space.dimensions as i32,
                ),
                false,
            ),
        ])
        .with_metadata(std::collections::HashMap::from([
            ("beans.schema_version".into(), "1".into()),
            (
                "beans.embedding_fingerprint".into(),
                space.fingerprint.clone(),
            ),
            (
                "beans.vector_distance".into(),
                format!("{:?}", space.distance),
            ),
            (
                "beans.index_generation".into(),
                space.generation.to_string(),
            ),
        ])),
    )
}
fn records(
    space: &VectorSpace,
    namespace: &str,
    rows: &[TransferRow],
) -> Result<RecordBatch, MemoryError> {
    let provenance: Vec<String> = rows
        .iter()
        .map(|r| {
            serde_json::to_string(&r.document)
                .map_err(|_| MemoryError::new("invalid_memory_document"))
        })
        .collect::<Result<_, _>>()?;
    let vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        rows.iter().map(|r| Some(r.vector.iter().copied().map(Some))),
        space.dimensions as i32,
    );
    RecordBatch::try_new(
        schema(space),
        vec![
            Arc::new(StringArray::from_iter_values(std::iter::repeat_n(namespace, rows.len()))),
            Arc::new(StringArray::from_iter_values(rows.iter().map(|r| r.document.id.as_str()))),
            Arc::new(StringArray::from_iter_values(std::iter::repeat_n("0", rows.len()))),
            Arc::new(StringArray::from_iter_values(rows.iter().map(|r| r.document.text.as_str()))),
            Arc::new(StringArray::from_iter_values(provenance.iter().map(String::as_str))),
            Arc::new(StringArray::from_iter_values(std::iter::repeat_n(space.fingerprint.as_str(), rows.len()))),
            Arc::new(vectors),
        ],
    )
    .map_err(|_| MemoryError::new("invalid_memory_document"))
}
fn generation_matches(space: &VectorSpace, actual: &Schema) -> bool {
    let metadata = actual.metadata();
    let distance = match space.distance {
        VectorDistance::Cosine => "Cosine", VectorDistance::Euclidean => "Euclidean", VectorDistance::Dot => "Dot",
    };
    metadata.get("beans.schema_version").map(String::as_str) == Some("1")
        && metadata.get("beans.embedding_fingerprint").map(String::as_str) == Some(space.fingerprint.as_str())
        && metadata.get("beans.vector_distance").map(String::as_str) == Some(distance)
        && metadata.get("beans.index_generation").and_then(|value| value.parse::<u64>().ok()) == Some(space.generation)
}
fn validate_table_schema(space: &VectorSpace, actual: &Schema) -> Result<(), MemoryError> {
    if actual.fields() != schema(space).fields() || !generation_matches(space, actual) {
        return Err(MemoryError::new("embedding_space_or_schema_mismatch"));
    }
    Ok(())
}
fn decode(
    space: &VectorSpace,
    namespace: &str,
    batches: &[RecordBatch],
    limit: usize,
) -> Result<Vec<TransferRow>, MemoryError> {
    let mut rows = Vec::new();
    let mut bytes = 0;
    for batch in batches {
        if !generation_matches(space, batch.schema().as_ref()) {
            return Err(MemoryError::new("embedding_generation_mismatch"));
        }
        if rows.len() + batch.num_rows() > limit {
            return Err(MemoryError::new("invalid_memory_response"));
        }
        let string = |name: &str| {
            batch
                .column_by_name(name)
                .and_then(|v| v.as_any().downcast_ref::<StringArray>())
                .ok_or_else(|| MemoryError::new("invalid_memory_response"))
        };
        let ns = string("namespace")?;
        let ids = string("document_id")?;
        let chunks = string("chunk_id")?;
        let text = string("text")?;
        let provenance = string("provenance")?;
        let fp = string("fingerprint")?;
        let vectors = batch
            .column_by_name("vector")
            .and_then(|v| v.as_any().downcast_ref::<FixedSizeListArray>())
            .ok_or_else(|| MemoryError::new("invalid_memory_response"))?;
        for i in 0..batch.num_rows() {
            if [ns, ids, chunks, text, provenance, fp]
                .iter()
                .any(|v| v.is_null(i))
                || vectors.is_null(i)
                || ns.value(i) != namespace
                || fp.value(i) != space.fingerprint
                || chunks.value(i) != "0"
            {
                return Err(MemoryError::new("invalid_memory_response"));
            }
            bytes += text.value(i).len() + provenance.value(i).len() + space.dimensions * 4;
            if bytes > TRANSFER_LIMIT {
                return Err(MemoryError::new("memory_response_limit_exceeded"));
            }
            let document: FrozenDocument = serde_json::from_str(provenance.value(i))
                .map_err(|_| MemoryError::new("invalid_memory_response"))?;
            document_valid(&document)?;
            if document.id != ids.value(i) || document.text != text.value(i) {
                return Err(MemoryError::new("invalid_memory_response"));
            }
            let vector = vectors.value(i);
            let vector = vector
                .as_any()
                .downcast_ref::<Float32Array>()
                .ok_or_else(|| MemoryError::new("invalid_memory_response"))?;
            if vector.null_count() != 0 {
                return Err(MemoryError::new("invalid_memory_response"));
            }
            let vector = vector.values().to_vec();
            space.validate_raw(fp.value(i), &vector)?;
            rows.push(TransferRow { document, vector });
        }
    }
    Ok(rows)
}
fn merge_result<T, E>(result: Result<T, E>) -> Result<T, MemoryError> {
    result.map_err(|_| MemoryError::new("delivery_unknown"))
}

#[cfg(test)]
#[path = "lance_tests.rs"]
mod tests;
