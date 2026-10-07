use crate::memory_service::types::EmbeddingProfile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, time::Duration};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingMode { #[default] Api, LocalCpu }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pooling { Mean, Cls, Pooled }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TensorNames {
    pub input_ids: String,
    pub attention_mask: String,
    pub token_type_ids: Option<String>,
    pub output: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalModelConfig {
    pub model_sha256: String,
    pub tokenizer_sha256: String,
    pub max_tokens: usize,
    pub pooling: Pooling,
    pub tensors: TensorNames,
    pub add_special_tokens: bool,
    pub pad_id: u32,
    pub pad_type_id: u32,
    pub pad_token: String,
}
impl LocalModelConfig {
    pub fn validate(&self) -> Result<(), EmbeddingError> {
        EmbeddingFingerprint::parse(&self.model_sha256)?;
        EmbeddingFingerprint::parse(&self.tokenizer_sha256)?;
        let names = [&self.tensors.input_ids, &self.tensors.attention_mask, &self.tensors.output];
        if self.max_tokens == 0 || self.max_tokens > 8192 || self.pad_token.len() > 256
            || names.iter().any(|n| n.is_empty() || n.len() > 256 || n.chars().any(char::is_control))
            || self.tensors.input_ids == self.tensors.attention_mask
            || self.tensors.token_type_ids.as_ref().is_some_and(|n| n.is_empty() || n.len() > 256
                || n.chars().any(char::is_control) || n == &self.tensors.input_ids || n == &self.tensors.attention_mask)
        { return Err(EmbeddingError::InvalidProfile); }
        Ok(())
    }
}

/// Errors contain no raw request, URL, credentials, response body or native-library cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EmbeddingError {
    #[error("invalid embedding profile")] InvalidProfile,
    #[error("invalid embedding input or limits")] InvalidInput,
    #[error("invalid embedding response")] InvalidResponse,
    #[error("embedding vector-space mismatch; replacement index required")] SpaceMismatch,
    #[error("approved replacement generation required")] ReindexRequired,
    #[error("embedding request cancelled")] Cancelled,
    #[error("embedding request timed out")] Timeout,
    #[error("embedding response exceeds the approved bound")] ResponseTooLarge,
    #[error("embedding transport failed")] Transport,
    #[error("embedding service rejected the request")] Service,
    #[error("embedding endpoint redirects are forbidden")] Redirect,
    #[error("embedding response model differs from the pinned model")] ModelMismatch,
    #[error("local embedding assets require setup")] SetupRequired,
    #[error("local embedding asset checksum or size differs")] AssetMismatch,
    #[error("local embedding runtime unavailable or incompatible")] RuntimeUnavailable,
    #[error("local embedding runtime is already bound to different assets; restart required")] RuntimeConflict,
    #[error("local embedding model or tokenizer contract differs")] ModelContract,
    #[error("explicit unexpired approval of the exact asset preview is required")] ApprovalRequired,
    #[error("asset installation failed")] InstallFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct EmbeddingFingerprint(String);
impl EmbeddingFingerprint {
    pub fn parse(value: &str) -> Result<Self, EmbeddingError> {
        if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(EmbeddingError::InvalidProfile);
        }
        Ok(Self(value.into()))
    }
    pub fn as_str(&self) -> &str { &self.0 }
}
impl<'de> Deserialize<'de> for EmbeddingFingerprint {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}
impl fmt::Display for EmbeddingFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbeddingPurpose { Query, Document }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Normalization { None, L2 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Distance { Cosine, Dot, Euclidean }
#[derive(Clone, Debug)]
pub struct EmbeddingSpec {
    pub(super) mode: EmbeddingMode,
    pub(super) dimensions: usize,
    pub(super) normalization: Normalization,
    pub(super) distance: Distance,
    pub(super) local: Option<LocalModelConfig>,
    model: String,
    query_prefix: String,
    document_prefix: String,
    endpoint: Option<reqwest::Url>,
    fingerprint: EmbeddingFingerprint,
}
impl EmbeddingSpec {
    pub fn from_profile(p: &EmbeddingProfile) -> Result<Self, EmbeddingError> {
        if p.model.is_empty() || p.model.len() > 1024 || p.model.chars().any(char::is_control)
            || p.revision.is_empty() || p.revision.len() > 1024 || p.revision.chars().any(char::is_control)
            || p.dimensions == 0 || p.dimensions > 65536
            || p.query_prefix.len() > 4096 || p.document_prefix.len() > 4096
        { return Err(EmbeddingError::InvalidProfile); }
        let normalization = match p.normalization.as_str() {
            "none" => Normalization::None, "l2" => Normalization::L2,
            _ => return Err(EmbeddingError::InvalidProfile),
        };
        let distance = match p.distance.as_str() {
            "cosine" => Distance::Cosine, "dot" => Distance::Dot, "euclidean" => Distance::Euclidean,
            _ => return Err(EmbeddingError::InvalidProfile),
        };
        let endpoint = match p.mode {
            EmbeddingMode::Api => {
                if p.local.is_some() { return Err(EmbeddingError::InvalidProfile); }
                let u = validate_url(p.endpoint.as_deref().ok_or(EmbeddingError::InvalidProfile)?)?;
                if !u.path().ends_with("/embeddings") { return Err(EmbeddingError::InvalidProfile); }
                Some(u)
            },
            EmbeddingMode::LocalCpu => {
                if p.endpoint.is_some() || p.secret.is_some() { return Err(EmbeddingError::InvalidProfile); }
                p.local.as_ref().ok_or(EmbeddingError::InvalidProfile)?.validate()?;
                None
            },
        };
        // Canonical JSON uses lexically sorted object keys, recursively. Exclude only
        // credentials, not unknown semantic metadata: future preprocessing must fence indexes.
        let mut identity = serde_json::to_value(p).map_err(|_| EmbeddingError::InvalidProfile)?;
        let object = identity.as_object_mut().ok_or(EmbeddingError::InvalidProfile)?;
        object.remove("secret");
        if let Some(u) = &endpoint { object.insert("endpoint".into(), u.as_str().into()); }
        let encoded = serde_json::to_vec(&canonical(identity)).map_err(|_| EmbeddingError::InvalidProfile)?;
        if encoded.len() > 65536 { return Err(EmbeddingError::InvalidProfile); }
        let mut hash = Sha256::new();
        hash.update(b"beans.embedding.space.v1\0prefix-exact-utf8\0ort-2.0.0-rc.13/tokenizers-0.22.2/right-longest.v1\0");
        hash.update((encoded.len() as u64).to_be_bytes()); hash.update(encoded);
        let fingerprint = EmbeddingFingerprint(format!("{:x}", hash.finalize()));
        Ok(Self { mode: p.mode, dimensions: p.dimensions as usize, normalization, distance,
            local: p.local.clone(), model: p.model.clone(), query_prefix: p.query_prefix.clone(),
            document_prefix: p.document_prefix.clone(), endpoint, fingerprint })
    }
    pub fn fingerprint(&self) -> &EmbeddingFingerprint { &self.fingerprint }
    pub fn model(&self) -> &str { &self.model }
    pub fn endpoint(&self) -> Option<&reqwest::Url> { self.endpoint.as_ref() }
    pub fn mode(&self) -> EmbeddingMode { self.mode }
    pub fn dimensions(&self) -> usize { self.dimensions }
    pub fn normalization(&self) -> Normalization { self.normalization }
    pub fn distance(&self) -> Distance { self.distance }
    pub fn local_config(&self) -> Option<&LocalModelConfig> { self.local.as_ref() }
    pub fn prepare(&self, purpose: EmbeddingPurpose, texts: &[String], limits: &EmbeddingLimits)
        -> Result<Vec<String>, EmbeddingError> {
        limits.validate()?;
        if texts.is_empty() || texts.len() > limits.max_batch { return Err(EmbeddingError::InvalidInput); }
        let prefix = match purpose { EmbeddingPurpose::Query => &self.query_prefix, EmbeddingPurpose::Document => &self.document_prefix };
        let mut total = 0usize;
        let mut result = Vec::with_capacity(texts.len());
        for text in texts {
            let len = text.len().checked_add(prefix.len()).ok_or(EmbeddingError::InvalidInput)?;
            total = total.checked_add(len).ok_or(EmbeddingError::InvalidInput)?;
            if text.is_empty() || len > limits.max_input_bytes || total > limits.max_total_input_bytes {
                return Err(EmbeddingError::InvalidInput);
            }
            let mut prepared = String::with_capacity(len); prepared.push_str(prefix); prepared.push_str(text);
            result.push(prepared);
        }
        Ok(result)
    }
    pub fn validate_vectors(&self, mut vectors: Vec<Vec<f32>>, count: usize) -> Result<EmbeddingBatch, EmbeddingError> {
        if count == 0 || count > 64 || vectors.len() != count { return Err(EmbeddingError::InvalidResponse); }
        for vector in &mut vectors {
            validate_vector(vector, self.dimensions)?;
            if self.normalization == Normalization::L2 {
                let norm = vector.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>().sqrt();
                for v in vector { *v = (f64::from(*v) / norm) as f32; }
            }
        }
        Ok(EmbeddingBatch { fingerprint: self.fingerprint.clone(), vectors })
    }
}
fn canonical(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(o) => serde_json::Value::Object(o.into_iter().map(|(k,v)|(k,canonical(v))).collect::<std::collections::BTreeMap<_,_>>().into_iter().collect()),
        serde_json::Value::Array(a) => serde_json::Value::Array(a.into_iter().map(canonical).collect()),
        v => v,
    }
}
pub(crate) fn validate_url(endpoint: &str) -> Result<reqwest::Url, EmbeddingError> {
    let url = reqwest::Url::parse(endpoint).map_err(|_| EmbeddingError::InvalidProfile)?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none()
        || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some()
    { return Err(EmbeddingError::InvalidProfile); }
    Ok(url)
}
fn validate_vector(vector: &[f32], dimensions: usize) -> Result<(), EmbeddingError> {
    if vector.len() != dimensions || vector.iter().any(|v| !v.is_finite())
        || !vector.iter().any(|v| *v != 0.) { return Err(EmbeddingError::InvalidResponse); }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct EmbeddingBatch { pub fingerprint: EmbeddingFingerprint, pub vectors: Vec<Vec<f32>> }
#[derive(Clone, Debug)]
pub struct EmbeddingLimits {
    pub max_batch: usize,
    pub max_input_bytes: usize,
    pub max_total_input_bytes: usize,
    pub max_response_bytes: usize,
    pub timeout: Duration,
}
impl Default for EmbeddingLimits {
    fn default() -> Self { Self { max_batch: 64, max_input_bytes: 32768, max_total_input_bytes: 524288,
        max_response_bytes: 8 * 1024 * 1024, timeout: Duration::from_secs(5) } }
}
impl EmbeddingLimits {
    pub fn validate(&self) -> Result<(), EmbeddingError> {
        if self.max_batch == 0 || self.max_batch > 64 || self.max_input_bytes == 0 || self.max_input_bytes > 32768
            || self.max_total_input_bytes == 0 || self.max_total_input_bytes > 524288
            || self.max_response_bytes == 0 || self.max_response_bytes > 16*1024*1024
            || self.timeout.is_zero() || self.timeout > Duration::from_secs(30) { return Err(EmbeddingError::InvalidInput); }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexGeneration {
    pub generation: u64,
    pub fingerprint: EmbeddingFingerprint,
    pub dimensions: usize,
}
impl IndexGeneration {
    pub fn new(generation: u64, fingerprint: EmbeddingFingerprint, dimensions: usize) -> Result<Self, EmbeddingError> {
        if generation == 0 || dimensions == 0 || dimensions > 65536 { return Err(EmbeddingError::ReindexRequired); }
        Ok(Self { generation, fingerprint, dimensions })
    }
    pub fn validate_batch(&self, batch: &EmbeddingBatch) -> Result<(), EmbeddingError> {
        if batch.fingerprint != self.fingerprint { return Err(EmbeddingError::SpaceMismatch); }
        if batch.vectors.is_empty() { return Err(EmbeddingError::InvalidResponse); }
        for vector in &batch.vectors { validate_vector(vector, self.dimensions)?; }
        Ok(())
    }
    pub fn replacement(&self, generation: u64, fingerprint: EmbeddingFingerprint, dimensions: usize, approved: bool) -> Result<Self, EmbeddingError> {
        if !approved || generation <= self.generation { return Err(EmbeddingError::ReindexRequired); }
        Self::new(generation, fingerprint, dimensions)
    }
}

/// CPU pooling for exact exported model contracts. No guessed output/dimension selection.
pub fn pool_output(data: &[f32], shape: &[i64], mask: &[i64], batch: usize, tokens: usize,
    dimensions: usize, pooling: Pooling) -> Result<Vec<Vec<f32>>, EmbeddingError> {
    if batch == 0 || batch > 64 || tokens == 0 || tokens > 8192 || dimensions == 0 || dimensions > 65536
        || mask.len() != batch*tokens || mask.iter().any(|&m| m != 0 && m != 1)
        || data.iter().any(|v| !v.is_finite()) { return Err(EmbeddingError::ModelContract); }
    if pooling == Pooling::Pooled {
        if shape != [batch as i64, dimensions as i64] || data.len() != batch*dimensions {
            return Err(EmbeddingError::ModelContract);
        }
        return Ok(data.chunks_exact(dimensions).map(<[f32]>::to_vec).collect());
    }
    if shape != [batch as i64,tokens as i64,dimensions as i64] || data.len() != batch*tokens*dimensions {
        return Err(EmbeddingError::ModelContract);
    }
    let mut result=Vec::with_capacity(batch);
    for b in 0..batch {
        let start=b*tokens*dimensions;
        if pooling == Pooling::Cls {
            if mask[b*tokens] != 1 { return Err(EmbeddingError::ModelContract); }
            result.push(data[start..start+dimensions].to_vec()); continue;
        }
        let count=mask[b*tokens..(b+1)*tokens].iter().filter(|&&m| m==1).count();
        if count==0 { return Err(EmbeddingError::ModelContract); }
        let mut sum=vec![0f64;dimensions];
        for t in 0..tokens {
            if mask[b*tokens+t] == 0 { continue; }
            for (d,s) in sum.iter_mut().enumerate() { *s+=f64::from(data[start+t*dimensions+d]); }
        }
        result.push(sum.into_iter().map(|s|(s/count as f64) as f32).collect());
    }
    Ok(result)
}
