//! PostgreSQL adapter: verified TLS, prepared values, fixed trusted schema and exact scoped search.
//! Initialization is never implicit. A one-use approval binds the connected server and complete SQL.
#[path = "lance_vectors.rs"]
pub(crate) mod vectors;
use crate::{
    embeddings::{Embedding, EmbeddingPurpose},
    memory_service::types::*,
};
use serde::Serialize;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tokio_postgres::{config::SslMode, Client, Config};
use tokio_util::sync::CancellationToken;
use vectors::*;
pub use vectors::{VectorDistance, VectorSpace};

pub struct PgvectorConfig {
    connection: Config,
    schema: String,
    pub space: VectorSpace,
}
impl std::fmt::Debug for PgvectorConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgvectorConfig")
            .field("schema", &self.schema)
            .field("space", &self.space)
            .finish_non_exhaustive()
    }
}
impl PgvectorConfig {
    pub fn new(connection: &str, schema: &str, space: VectorSpace) -> Result<Self, MemoryError> {
        let endpoint = reqwest::Url::parse(connection)
            .map_err(|_| MemoryError::new("invalid_postgres_target"))?;
        if !matches!(endpoint.scheme(), "postgres" | "postgresql")
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.fragment().is_some()
            || endpoint
                .query_pairs()
                .any(|(key, _)| key.eq_ignore_ascii_case("password"))
        {
            return Err(MemoryError::new("postgres_credentials_must_be_separate"));
        }
        if schema.is_empty()
            || schema.len() > 63
            || !schema
                .bytes()
                .enumerate()
                .all(|(i, b)| b.is_ascii_lowercase() || b == b'_' || (i > 0 && b.is_ascii_digit()))
            || matches!(schema, "public" | "pg_catalog" | "information_schema")
            || schema.starts_with("pg_")
        {
            return Err(MemoryError::new("invalid_memory_schema"));
        }
        space.validate()?;
        // Build the wire configuration from the one validated URL authority. The
        // PostgreSQL DSN parser scans '@' outside authority, so never reparse the URI.
        let hostname = endpoint.host_str().ok_or_else(|| MemoryError::new("invalid_postgres_target"))?;
        let hostname = hostname.strip_prefix('[').and_then(|host| host.strip_suffix(']')).unwrap_or(hostname);
        let mut config = Config::new();
        config.host(hostname).port(endpoint.port().unwrap_or(5432));
        let database = percent_encoding::percent_decode_str(endpoint.path().trim_start_matches('/'))
            .decode_utf8().map_err(|_| MemoryError::new("invalid_postgres_target"))?;
        if !database.is_empty() { config.dbname(database.as_ref()); }
        for (key, value) in endpoint.query_pairs() {
            match key.as_ref() {
                "sslmode" if value == "disable" => return Err(MemoryError::new("postgres_tls_required")),
                "sslmode" if value == "require" || value == "prefer" => {},
                "user" if !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control) => { config.user(value.as_ref()); },
                "application_name" if value.len() <= 256 && !value.chars().any(char::is_control) => { config.application_name(value.as_ref()); },
                _ => return Err(MemoryError::new("invalid_postgres_option")),
            }
        }
        config.ssl_mode(SslMode::Require);
        config.connect_timeout(Duration::from_secs(5));
        Ok(Self {
            connection: config,
            schema: schema.into(),
            space,
        })
    }
    /// Credentials come from the private account secret, never an endpoint or model parameter.
    pub fn with_password(mut self, password: Option<&str>) -> Result<Self, MemoryError> {
        if let Some(password) = password {
            if password.len() > 4096 || password.contains('\0') {
                return Err(MemoryError::new("invalid_postgres_credentials"));
            }
            self.connection.password(password);
        }
        Ok(self)
    }
    pub fn with_user(mut self, user: &str) -> Result<Self, MemoryError> {
        if user.is_empty() || user.len() > 256 || user.chars().any(char::is_control) {
            return Err(MemoryError::new("invalid_postgres_credentials"));
        }
        self.connection.user(user);
        Ok(self)
    }
    fn documents(&self) -> String {
        format!("\"{}\".documents", self.schema)
    }
    fn chunks(&self) -> String {
        format!("\"{}\".chunks", self.schema)
    }
    fn ddl(&self) -> String {
        format!(
            r#"SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '5s';
CREATE EXTENSION IF NOT EXISTS vector WITH SCHEMA public;
CREATE SCHEMA IF NOT EXISTS "{schema}";
CREATE TABLE {documents} (
    namespace text NOT NULL, document_id text NOT NULL, document jsonb NOT NULL,
    PRIMARY KEY (namespace, document_id)
);
CREATE TABLE {chunks} (
    namespace text NOT NULL, document_id text NOT NULL,
    chunk_id integer NOT NULL CHECK (chunk_id = 0),
    fingerprint text NOT NULL CHECK (fingerprint = '{fingerprint}'),
    index_generation bigint NOT NULL CHECK (index_generation = {generation}),
    embedding public.vector({dimensions}) NOT NULL,
    PRIMARY KEY (namespace, document_id, chunk_id),
    FOREIGN KEY (namespace, document_id) REFERENCES {documents}(namespace, document_id) ON DELETE CASCADE
);
-- Primary-key indexes support exact scoped scans; no approximate vector index.
-- Requires CREATE on database/schema and superuser for an absent vector extension.
-- No grants are applied. Existing schemas/tables/data are not replaced.
"#,
            schema = self.schema,
            documents = self.documents(),
            chunks = self.chunks(),
            fingerprint = self.space.fingerprint,
            dimensions = self.space.dimensions,
            generation = self.space.generation
        )
    }
    fn validate_constraints(&self, constraints: &[CatalogConstraint]) -> Result<(), MemoryError> {
        let expected_fingerprint = format!("CHECK ((fingerprint = '{}'::text))", self.space.fingerprint);
        let expected_generation = format!("CHECK ((index_generation = {}))", self.space.generation);
        let key = |table: &str, columns: &[&str]| constraints.iter().any(|c| {
            c.table == table && c.kind == "p" && c.validated
                && c.columns.iter().map(String::as_str).eq(columns.iter().copied())
        });
        let check = |column: &str, definition: &str| constraints.iter().any(|c| {
            c.table == "chunks" && c.kind == "c" && c.validated
                && c.columns.iter().map(String::as_str).eq([column]) && c.definition == definition
        });
        let cascade = constraints.iter().any(|c| {
            c.table == "chunks" && c.kind == "f" && c.validated && c.delete_action == "c"
                && c.columns.iter().map(String::as_str).eq(["namespace", "document_id"])
                && c.referenced_schema.as_deref() == Some(self.schema.as_str())
                && c.referenced_table.as_deref() == Some("documents")
                && c.referenced_columns.iter().map(String::as_str).eq(["namespace", "document_id"])
        });
        if !key("documents", &["namespace", "document_id"])
            || !key("chunks", &["namespace", "document_id", "chunk_id"])
            || !check("chunk_id", "CHECK ((chunk_id = 0))")
            || !check("fingerprint", &expected_fingerprint)
            || !check("index_generation", &expected_generation)
            || !cascade
        { return Err(MemoryError::new("memory_schema_not_ready")); }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct CatalogConstraint {
    table: String,
    definition: String,
    kind: String,
    validated: bool,
    columns: Vec<String>,
    referenced_schema: Option<String>,
    referenced_table: Option<String>,
    referenced_columns: Vec<String>,
    delete_action: String,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct InitializationTarget {
    pub database: String,
    pub database_oid: u32,
    pub role: String,
    pub server_address: Option<String>,
    pub server_port: Option<i32>,
    pub server_version: String,
    pub session_pid: i32,
}
#[derive(Clone, Debug, Serialize)]
pub struct SchemaReadiness {
    pub ready: bool,
    pub extension_version: Option<String>,
    pub extension_schema: Option<String>,
    pub schema_exists: bool,
    pub can_create_schema: bool,
    pub can_create_tables: bool,
    pub can_install_extension: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct InitializePreview {
    pub target: InitializationTarget,
    pub schema: String,
    pub sql: String,
    pub approval_token: String,
    pub expires_in_seconds: u64,
    pub readiness: SchemaReadiness,
}
struct PendingApproval {
    target: InitializationTarget,
    sql: String,
    token: String,
    created: Instant,
}
pub struct PgvectorBackend {
    config: PgvectorConfig,
    bound: MemoryScope,
    client: Mutex<Client>,
    embedding: Option<Arc<dyn Embedding>>,
    approval: Mutex<Option<PendingApproval>>,
}
impl PgvectorBackend {
    pub async fn connect(
        config: PgvectorConfig,
        bound: MemoryScope,
        embedding: Option<Arc<dyn Embedding>>,
        cancel: CancellationToken,
    ) -> Result<Self, MemoryError> {
        bind(&bound, &bound)?;
        config.space.validate()?;
        if embedding
            .as_ref()
            .is_some_and(|e| e.fingerprint().as_str() != config.space.fingerprint)
        {
            return Err(MemoryError::new("embedding_model_mismatch"));
        }
        let tls = memory_postgres_rustls::MakeRustlsConnect::new(lorca_tls::client_config(&[]));
        let (client, connection) = bounded(cancel, 5000, false, async {
            config
                .connection
                .connect(tls)
                .await
                .map_err(|_| MemoryError::new("postgres_connection_failed"))
        })
        .await?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        Ok(Self {
            config,
            bound,
            client: Mutex::new(client),
            embedding,
            approval: Mutex::new(None),
        })
    }
    #[cfg(test)]
    async fn sandbox(config: PgvectorConfig, bound: MemoryScope) -> Result<Self, MemoryError> {
        let (client, connection) = config
            .connection
            .connect(tokio_postgres::NoTls)
            .await
            .map_err(|_| MemoryError::new("postgres_connection_failed"))?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        Ok(Self {
            config,
            bound,
            client: Mutex::new(client),
            embedding: None,
            approval: Mutex::new(None),
        })
    }
    async fn target(client: &Client) -> Result<InitializationTarget, MemoryError> {
        let row=client.query_one("SELECT current_database(), (SELECT oid FROM pg_database WHERE datname=current_database()), current_user, inet_server_addr()::text, inet_server_port(), current_setting('server_version'), pg_backend_pid()",&[]).await.map_err(|_|MemoryError::new("postgres_inspection_failed"))?;
        Ok(InitializationTarget {
            database: row.get(0),
            database_oid: row.get(1),
            role: row.get(2),
            server_address: row.get(3),
            server_port: row.get(4),
            server_version: row.get(5),
            session_pid: row.get(6),
        })
    }
    /// UI-only read-only preview. The caller must display target, schema, SQL and permissions.
    pub async fn initialize_preview(
        &self,
        cancel: CancellationToken,
    ) -> Result<InitializePreview, MemoryError> {
        bounded(cancel,5000,false,async {
            let client=self.client.lock().await;let target=Self::target(&client).await?;
            let readiness=self.status_client(&client).await?;
            let count:i64=client.query_one("SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname IN ('documents','chunks')",&[&self.config.schema]).await.map_err(|_|MemoryError::new("postgres_inspection_failed"))?.get(0);
            if count!=0{return Err(MemoryError::new("memory_schema_exists"));}
            let sql=self.config.ddl();let token=uuid::Uuid::new_v4().to_string();
            *self.approval.lock().await=Some(PendingApproval{target:target.clone(),sql:sql.clone(),token:token.clone(),created:Instant::now()});
            Ok(InitializePreview{target,schema:self.config.schema.clone(),sql,approval_token:token,expires_in_seconds:300,readiness})
        }).await
    }
    /// UI-only, explicit confirmation of the exact preview. Tokens are single-use even on failure.
    pub async fn initialize_apply(
        &self,
        approval_token: &str,
        cancel: CancellationToken,
    ) -> Result<(), MemoryError> {
        let approved = self
            .approval
            .lock()
            .await
            .take()
            .ok_or_else(|| MemoryError::new("schema_approval_required"))?;
        if approved.token != approval_token
            || approved.created.elapsed() > Duration::from_secs(300)
            || approved.sql != self.config.ddl()
        {
            return Err(MemoryError::new("invalid_schema_approval"));
        }
        bounded(cancel, 5000, true, async {
            let mut client = self.client.lock().await;
            if Self::target(&client).await? != approved.target {
                return Err(MemoryError::new("schema_target_changed"));
            }
            let tx = client
                .transaction()
                .await
                .map_err(|_| MemoryError::new("schema_initialization_failed"))?;
            tx.batch_execute(&approved.sql)
                .await
                .map_err(|_| MemoryError::new("schema_initialization_failed"))?;
            self.ready_client(&tx).await?;
            tx.commit()
                .await
                .map_err(|_| MemoryError::new("delivery_unknown"))
        })
        .await
    }
    async fn ready_client<C: tokio_postgres::GenericClient + Sync>(
        &self,
        client: &C,
    ) -> Result<(), MemoryError> {
        let sql="SELECT c.relname,a.attname,format_type(a.atttypid,a.atttypmod),a.attnotnull FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname IN ('documents','chunks') AND a.attnum>0 AND NOT a.attisdropped ORDER BY c.relname,a.attnum";
        let rows = client
            .query(sql, &[&self.config.schema])
            .await
            .map_err(|_| MemoryError::new("memory_schema_not_ready"))?;
        let actual: Vec<(String, String, String, bool)> = rows
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
            .collect();
        let vector_type = format!("vector({})", self.config.space.dimensions);
        let expected = vec![
            ("chunks", "namespace", "text"),
            ("chunks", "document_id", "text"),
            ("chunks", "chunk_id", "integer"),
            ("chunks", "fingerprint", "text"),
            ("chunks", "index_generation", "bigint"),
            ("chunks", "embedding", vector_type.as_str()),
            ("documents", "namespace", "text"),
            ("documents", "document_id", "text"),
            ("documents", "document", "jsonb"),
        ];
        if actual.len() != expected.len()
            || actual
                .iter()
                .zip(expected)
                .any(|(a, e)| a.0 != e.0 || a.1 != e.1 || a.2 != e.2 || !a.3)
        {
            return Err(MemoryError::new("memory_schema_not_ready"));
        }
        let constraints = client.query(r#"
SELECT c.relname, pg_get_constraintdef(k.oid), k.contype::text, k.convalidated,
       ARRAY(SELECT a.attname::text FROM unnest(k.conkey) WITH ORDINALITY x(attnum, ord)
             JOIN pg_attribute a ON a.attrelid=k.conrelid AND a.attnum=x.attnum ORDER BY x.ord),
       rn.nspname, rc.relname,
       ARRAY(SELECT a.attname::text FROM unnest(k.confkey) WITH ORDINALITY x(attnum, ord)
             JOIN pg_attribute a ON a.attrelid=k.confrelid AND a.attnum=x.attnum ORDER BY x.ord),
       k.confdeltype::text
FROM pg_constraint k JOIN pg_class c ON c.oid=k.conrelid
JOIN pg_namespace n ON n.oid=c.relnamespace
LEFT JOIN pg_class rc ON rc.oid=k.confrelid LEFT JOIN pg_namespace rn ON rn.oid=rc.relnamespace
WHERE n.nspname=$1 AND c.relname IN ('documents','chunks')
"#, &[&self.config.schema]).await.map_err(|_| MemoryError::new("memory_schema_not_ready"))?;
        let constraints: Vec<CatalogConstraint> = constraints.iter().map(|row| CatalogConstraint {
            table: row.get(0), definition: row.get(1), kind: row.get(2), validated: row.get(3),
            columns: row.get(4), referenced_schema: row.get(5), referenced_table: row.get(6),
            referenced_columns: row.get(7), delete_action: row.get(8),
        }).collect();
        self.config.validate_constraints(&constraints)
    }
    pub async fn readiness(&self, cancel: CancellationToken) -> Result<(), MemoryError> {
        bounded(cancel, 5000, false, async {
            self.ready_client(&*self.client.lock().await).await
        })
        .await
    }
    async fn status_client(&self, client: &Client) -> Result<SchemaReadiness, MemoryError> {
        let row=client.query_one("SELECT (SELECT extversion FROM pg_extension WHERE extname='vector'), (SELECT n.nspname FROM pg_extension e JOIN pg_namespace n ON n.oid=e.extnamespace WHERE e.extname='vector'), EXISTS(SELECT 1 FROM pg_namespace WHERE nspname=$1), has_database_privilege(current_database(),'CREATE'), COALESCE((SELECT has_schema_privilege(oid,'CREATE') FROM pg_namespace WHERE nspname=$1),true), (SELECT rolsuper FROM pg_roles WHERE rolname=current_user)",&[&self.config.schema]).await.map_err(|_|MemoryError::new("postgres_inspection_failed"))?;
        Ok(SchemaReadiness {
            ready: self.ready_client(client).await.is_ok(),
            extension_version: row.get(0),
            extension_schema: row.get(1),
            schema_exists: row.get(2),
            can_create_schema: row.get(3),
            can_create_tables: row.get(4),
            can_install_extension: row.get(5),
        })
    }
    pub async fn schema_status(
        &self,
        cancel: CancellationToken,
    ) -> Result<SchemaReadiness, MemoryError> {
        bounded(cancel, 5000, false, async {
            self.status_client(&*self.client.lock().await).await
        })
        .await
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
        self.config.space.validate_raw(fingerprint, &vector)?;
        bounded(cancel,5000,true,async{
            let mut client=self.client.lock().await;self.ready_client(&*client).await?;
            let tx=client.transaction().await.map_err(|_|MemoryError::new("postgres_write_failed"))?;
            let doc=serde_json::to_value(&document).map_err(|_|MemoryError::new("invalid_memory_document"))?;
            let stmt=tx.prepare(&format!("INSERT INTO {} (namespace,document_id,document) VALUES ($1,$2,$3) ON CONFLICT (namespace,document_id) DO UPDATE SET document=EXCLUDED.document",self.config.documents())).await.map_err(|_|MemoryError::new("postgres_write_failed"))?;
            tx.execute(&stmt,&[&scope.namespace,&document.id,&doc]).await.map_err(|_|MemoryError::new("postgres_write_failed"))?;
            let stmt=tx.prepare(&format!("INSERT INTO {} (namespace,document_id,chunk_id,fingerprint,index_generation,embedding) VALUES ($1,$2,0,$3,$4,$5) ON CONFLICT (namespace,document_id,chunk_id) DO UPDATE SET fingerprint=EXCLUDED.fingerprint,index_generation=EXCLUDED.index_generation,embedding=EXCLUDED.embedding",self.config.chunks())).await.map_err(|_|MemoryError::new("postgres_write_failed"))?;
            let vector=pgvector::Vector::from(vector);
            let generation=self.config.space.generation as i64;
            tx.execute(&stmt,&[&scope.namespace,&document.id,&fingerprint,&generation,&vector]).await.map_err(|_|MemoryError::new("postgres_write_failed"))?;
            tx.commit().await.map_err(|_|MemoryError::new("delivery_unknown"))?;
            Ok(BackendResponse{data:serde_json::json!({"stored":true}),..Default::default()})
        }).await
    }
    pub async fn recall_vector(
        &self,
        scope: &MemoryScope,
        fingerprint: &str,
        vector: Vec<f32>,
        budget: RecallBudget,
        cancel: CancellationToken,
    ) -> Result<BackendResponse, MemoryError> {
        bind(&self.bound, scope)?;
        budget.validate()?;
        self.config.space.validate_raw(fingerprint, &vector)?;
        bounded(cancel,budget.timeout_ms,false,async{
            let client=self.client.lock().await;self.ready_client(&*client).await?;
            let operator=match self.config.space.distance{VectorDistance::Cosine=>"<=>",VectorDistance::Euclidean=>"<->",VectorDistance::Dot=>"<#>"};
            // Materialize the scoped rows before ordering: exact search never uses a global ANN index.
            let sql=format!("WITH scoped AS MATERIALIZED (SELECT d.document,c.embedding FROM {} c JOIN {} d ON d.namespace=c.namespace AND d.document_id=c.document_id WHERE c.namespace=$1 AND d.namespace=$1 AND c.fingerprint=$2) SELECT document FROM scoped ORDER BY embedding {operator} $3 LIMIT $4",self.config.chunks(),self.config.documents());
            let stmt=client.prepare(&sql).await.map_err(|_|MemoryError::new("postgres_query_failed"))?;
            let vector=pgvector::Vector::from(vector);let limit=budget.max_results as i64;
            let rows=client.query(&stmt,&[&scope.namespace,&fingerprint,&vector,&limit]).await.map_err(|_|MemoryError::new("postgres_query_failed"))?;
            let mut docs=Vec::new();for row in rows {let doc:FrozenDocument=serde_json::from_value(row.get(0)).map_err(|_|MemoryError::new("invalid_memory_response"))?;document_valid(&doc)?;docs.push(doc);}
            Ok(BackendResponse{evidence:bounded_evidence(docs,&budget),..Default::default()})
        }).await
    }
    async fn inspect(
        &self,
        scope: &MemoryScope,
        id: &str,
        cancel: CancellationToken,
    ) -> Result<BackendResponse, MemoryError> {
        bind(&self.bound, scope)?;
        bounded(cancel, 5000, false, async {
            let client = self.client.lock().await;
            self.ready_client(&*client).await?;
            let stmt = client
                .prepare(&format!(
                    "SELECT document FROM {} WHERE namespace=$1 AND document_id=$2",
                    self.config.documents()
                ))
                .await
                .map_err(|_| MemoryError::new("postgres_query_failed"))?;
            let row = client
                .query_opt(&stmt, &[&scope.namespace, &id])
                .await
                .map_err(|_| MemoryError::new("postgres_query_failed"))?;
            let document = row
                .map(|r| {
                    serde_json::from_value::<FrozenDocument>(r.get(0))
                        .map_err(|_| MemoryError::new("invalid_memory_response"))
                })
                .transpose()?;
            if let Some(doc) = &document {
                document_valid(doc)?;
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
        bind(&self.bound, scope)?;
        bounded(cancel, 5000, true, async {
            let client = self.client.lock().await;
            self.ready_client(&*client).await?;
            let (sql, params): (String, Vec<&(dyn tokio_postgres::types::ToSql + Sync)>) = match &id
            {
                Some(id) => (
                    format!(
                        "DELETE FROM {} WHERE namespace=$1 AND document_id=$2",
                        self.config.documents()
                    ),
                    vec![&scope.namespace, id],
                ),
                None => (
                    format!("DELETE FROM {} WHERE namespace=$1", self.config.documents()),
                    vec![&scope.namespace],
                ),
            };
            let stmt = client
                .prepare(&sql)
                .await
                .map_err(|_| MemoryError::new("postgres_delete_failed"))?;
            client
                .execute(&stmt, &params)
                .await
                .map_err(|_| MemoryError::new("delivery_unknown"))?;
            Ok(BackendResponse {
                data: serde_json::json!({"deleted":true,"history_may_retain_data":true}),
                ..Default::default()
            })
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
impl MemoryBackend for PgvectorBackend {
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
            BackendRequest::Health => {
                self.readiness(cancel).await?;
                Ok(BackendResponse {
                    data: serde_json::json!({"ready":true,"exact_search":true}),
                    ..Default::default()
                })
            }
            BackendRequest::Retain { document } => {
                document_valid(&document)?;
                let vector = self
                    .config
                    .space
                    .embed(
                        self.provider()?,
                        EmbeddingPurpose::Document,
                        document.text.clone(),
                        cancel.clone(),
                    )
                    .await?;
                self.retain_vector(
                    scope,
                    document,
                    &self.config.space.fingerprint,
                    vector,
                    cancel,
                )
                .await
            }
            BackendRequest::Recall { query, budget } => {
                budget.validate()?;
                if query.chars().count() > 4096 {
                    return Err(MemoryError::new("invalid_memory_query"));
                }
                let vector = self
                    .config
                    .space
                    .embed(
                        self.provider()?,
                        EmbeddingPurpose::Query,
                        query,
                        cancel.clone(),
                    )
                    .await?;
                self.recall_vector(
                    scope,
                    &self.config.space.fingerprint,
                    vector,
                    budget,
                    cancel,
                )
                .await
            }
            BackendRequest::Inspect { document_id } => {
                self.inspect(scope, &document_id, cancel).await
            }
            BackendRequest::DeleteDocument { document_id, .. } => {
                self.delete(scope, Some(&document_id), cancel).await
            }
            BackendRequest::Clear { .. } => self.delete(scope, None, cancel).await,
            BackendRequest::Advanced {
                feature: AdvancedFeature::MemoryEdit,
                action,
                body,
            } if action == "edit" => {
                let edit: EditDocument = serde_json::from_value(body)
                    .map_err(|_| MemoryError::new("invalid_memory_edit"))?;
                let mut doc = self
                    .inspect(scope, &edit.document_id, cancel.clone())
                    .await?
                    .document
                    .ok_or_else(|| MemoryError::new("memory_document_not_found"))?;
                doc.text = edit.text;
                doc.content_hash = digest(doc.text.as_bytes());
                document_valid(&doc)?;
                let vector = self
                    .config
                    .space
                    .embed(
                        self.provider()?,
                        EmbeddingPurpose::Document,
                        doc.text.clone(),
                        cancel.clone(),
                    )
                    .await?;
                self.retain_vector(scope, doc, &self.config.space.fingerprint, vector, cancel)
                    .await
            }
            _ => Err(MemoryError::new("memory_capability_unsupported")),
        }
    }
}
#[cfg(test)]
#[path = "pgvector_tests.rs"]
mod tests;
