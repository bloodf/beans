//! Shared vector-store invariants; no SQL selectors or filesystem paths come from model input.
use crate::{
    embeddings::{Embedding, EmbeddingPurpose, IndexGeneration},
    memory_service::types::*,
};
use serde::{Deserialize, Serialize};
use std::{future::Future, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VectorDistance {
    Cosine,
    Euclidean,
    Dot,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VectorSpace {
    pub fingerprint: String,
    pub dimensions: usize,
    pub distance: VectorDistance,
    pub generation: u64,
}
impl VectorSpace {
    pub fn new(
        fingerprint: String,
        dimensions: usize,
        distance: VectorDistance,
    ) -> Result<Self, MemoryError> {
        if !hex_digest(&fingerprint) || !(1..=16000).contains(&dimensions) {
            return Err(MemoryError::new("invalid_vector_space"));
        }
        Ok(Self {
            fingerprint,
            dimensions,
            distance,
            generation: 1,
        })
    }
    pub fn from_generation(
        index: &IndexGeneration,
        distance: VectorDistance,
    ) -> Result<Self, MemoryError> {
        if index.generation == 0 || index.generation > i64::MAX as u64 {
            return Err(MemoryError::new("invalid_index_generation"));
        }
        let mut space = Self::new(
            index.fingerprint.as_str().into(),
            index.dimensions,
            distance,
        )?;
        space.generation = index.generation;
        Ok(space)
    }
    pub fn validate(&self) -> Result<(), MemoryError> {
        if !hex_digest(&self.fingerprint) || !(1..=16000).contains(&self.dimensions) {
            return Err(MemoryError::new("invalid_vector_space"));
        }
        if self.generation == 0 || self.generation > i64::MAX as u64 {
            return Err(MemoryError::new("invalid_index_generation"));
        }
        Ok(())
    }
    pub fn validate_raw(&self, fingerprint: &str, vector: &[f32]) -> Result<(), MemoryError> {
        if fingerprint != self.fingerprint {
            return Err(MemoryError::new("embedding_model_mismatch"));
        }
        if vector.len() != self.dimensions {
            return Err(MemoryError::new("embedding_dimension_mismatch"));
        }
        let norm = vector.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
        if vector.iter().any(|v| !v.is_finite()) || !norm.is_finite() || norm == 0. {
            return Err(MemoryError::new("invalid_embedding_vector"));
        }
        Ok(())
    }
    pub async fn embed(
        &self,
        provider: &dyn Embedding,
        purpose: EmbeddingPurpose,
        text: String,
        cancel: CancellationToken,
    ) -> Result<Vec<f32>, MemoryError> {
        if provider.fingerprint().as_str() != self.fingerprint {
            return Err(MemoryError::new("embedding_model_mismatch"));
        }
        let batch = provider
            .embed(purpose, &[text], cancel)
            .await
            .map_err(|_| MemoryError::new("embedding_failed"))?;
        if batch.vectors.len() != 1 {
            return Err(MemoryError::new("embedding_count_mismatch"));
        }
        let vector = batch
            .vectors
            .into_iter()
            .next()
            .ok_or_else(|| MemoryError::new("embedding_count_mismatch"))?;
        self.validate_raw(batch.fingerprint.as_str(), &vector)?;
        Ok(vector)
    }
}
pub(crate) fn hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn bind(bound: &MemoryScope, current: &MemoryScope) -> Result<(), MemoryError> {
    if !hex_digest(&bound.namespace)
        || bound.namespace != current.namespace
        || bound.connection_id != current.connection_id
        || bound.connection_revision != current.connection_revision
        || bound.deletion_epoch != current.deletion_epoch
    {
        return Err(MemoryError::new("memory_scope_mismatch"));
    }
    Ok(())
}
pub(crate) fn document_valid(doc: &FrozenDocument) -> Result<(), MemoryError> {
    if doc.id.is_empty()
        || doc.id.len() > 256
        || doc.request_id.is_empty()
        || doc.request_id.len() > 256
        || doc.text.is_empty()
        || doc.text.len() > 32768
        || doc.sources.len() > 256
        || doc
            .sources
            .iter()
            .any(|s| s.chat_id.len() > 256 || s.message_id.len() > 256)
        || doc.content_hash != digest(doc.text.as_bytes())
    {
        return Err(MemoryError::new("invalid_memory_document"));
    }
    Ok(())
}
pub(crate) fn capabilities() -> Capabilities {
    Capabilities {
        retain: true,
        recall: true,
        inspect: true,
        delete_document: true,
        clear: true,
        idempotent_retain: true,
        advanced: std::collections::BTreeSet::from([AdvancedFeature::MemoryEdit]),
        advanced_actions: std::collections::BTreeMap::from([(
            AdvancedFeature::MemoryEdit,
            std::collections::BTreeSet::from(["edit".into()]),
        )]),
        ..Default::default()
    }
}
pub(crate) fn evidence(doc: FrozenDocument) -> Evidence {
    Evidence {
        id: format!("{}:0", doc.id),
        text: doc.text,
        document_id: Some(doc.id),
    }
}
pub(crate) fn bounded_evidence(
    docs: impl IntoIterator<Item = FrozenDocument>,
    budget: &RecallBudget,
) -> Vec<Evidence> {
    let mut bytes = 0;
    let mut chars = 0;
    let mut result = Vec::new();
    for doc in docs {
        let item = evidence(doc);
        let size = serde_json::to_vec(&item).map_or(usize::MAX, |v| v.len());
        let count = item.text.chars().count();
        if result.len() >= budget.max_results
            || bytes + size > budget.max_bytes
            || chars + count > budget.max_context_chars
        {
            break;
        }
        bytes += size;
        chars += count;
        result.push(item);
    }
    result
}
pub(crate) async fn bounded<T>(
    cancel: CancellationToken,
    timeout_ms: u64,
    write: bool,
    future: impl Future<Output = Result<T, MemoryError>>,
) -> Result<T, MemoryError> {
    if cancel.is_cancelled() {
        return Err(MemoryError::new("cancelled"));
    }
    tokio::select! {biased; _=cancel.cancelled()=>Err(MemoryError::new(if write{"delivery_unknown"}else{"cancelled"})),
    result=tokio::time::timeout(Duration::from_millis(timeout_ms),future)=>result.unwrap_or_else(|_|Err(MemoryError::new(if write{"delivery_unknown"}else{"memory_timeout"}))) }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditDocument {
    pub document_id: String,
    pub text: String,
}
