//! Turning text into sentence vectors on the CPU, with an installed catalog model: a static
//! table (tokenize, look each token up, average) or an ONNX model through the Runner's local
//! embedding runtime. Both read only files whose size and SHA-256 were checked again at load.

use std::sync::Arc;

use tokenizers::Tokenizer;
use tokio_util::sync::CancellationToken;

use super::{CatalogModel, OnnxSpec, StaticSpec};
use crate::embeddings::assets::{verified_bytes, AssetKind, BoundAsset, InstalledAssets, LocalBinding};
use crate::embeddings::{Embedding, EmbeddingError, EmbeddingLimits, EmbeddingPurpose};

/// The most bytes a static table or tokenizer may be, past the catalog's own limit.
const MAX_FILE_BYTES: u64 = super::MAX_MODEL_BYTES;

pub enum Encoder {
    Static(Box<StaticEncoder>),
    Onnx(Arc<crate::embeddings::local::LocalEmbedding>),
}

impl Encoder {
    /// Loads an installed model, checking every file again.
    pub async fn load(model: &CatalogModel, installed: &InstalledAssets, cancel: CancellationToken) -> Result<Encoder, EmbeddingError> {
        let asset = |kind| installed.get(kind).cloned().ok_or(EmbeddingError::SetupRequired);
        match (&model.static_spec, &model.onnx) {
            (Some(spec), None) => {
                let weights = verified_bytes(&asset(AssetKind::Weights)?, MAX_FILE_BYTES).await?;
                let tokenizer = verified_bytes(&asset(AssetKind::Tokenizer)?, MAX_FILE_BYTES).await?;
                let spec = spec.clone();
                let encoder = tokio::task::spawn_blocking(move || StaticEncoder::from_bytes(&weights, &tokenizer, &spec))
                    .await
                    .map_err(|_| EmbeddingError::ModelContract)??;
                Ok(Encoder::Static(Box::new(encoder)))
            }
            (None, Some(spec)) => {
                let binding = LocalBinding {
                    runtime: asset(AssetKind::Runtime)?,
                    model: asset(AssetKind::Model)?,
                    tokenizer: asset(AssetKind::Tokenizer)?,
                };
                let profile = onnx_profile(model, spec, &binding.model, &binding.tokenizer)?;
                let embedding = crate::embeddings::local::LocalEmbedding::load(&profile, binding, EmbeddingLimits::default(), cancel).await?;
                Ok(Encoder::Onnx(Arc::new(embedding)))
            }
            _ => Err(EmbeddingError::InvalidProfile),
        }
    }

    /// One unit vector per text, in order.
    pub async fn encode(&self, texts: &[String], cancel: &CancellationToken) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        match self {
            Encoder::Static(encoder) => Ok(texts.iter().map(|text| encoder.encode(text)).collect()),
            Encoder::Onnx(embedding) => {
                let mut vectors = Vec::with_capacity(texts.len());
                for batch in texts.chunks(EmbeddingLimits::default().max_batch) {
                    // An empty text has no tokens for the model; a space reads as nothing much.
                    let batch: Vec<String> = batch.iter().map(|t| if t.trim().is_empty() { " ".to_string() } else { t.clone() }).collect();
                    vectors.extend(embedding.embed(EmbeddingPurpose::Query, &batch, cancel.clone()).await?.vectors);
                }
                Ok(vectors)
            }
        }
    }
}

/// The Runner's pinned local embedding profile for a catalog ONNX model, with the hashes of the
/// files actually installed.
pub fn onnx_profile(model: &CatalogModel, spec: &OnnxSpec, onnx: &BoundAsset, tokenizer: &BoundAsset) -> Result<crate::memory_service::types::EmbeddingProfile, EmbeddingError> {
    serde_json::from_value(serde_json::json!({
        "mode": "local_cpu",
        "model": model.repo,
        "revision": model.revision,
        "dimensions": spec.dimensions,
        "normalization": "l2",
        "distance": "cosine",
        "document_prefix": spec.prefix,
        "query_prefix": spec.prefix,
        "local": {
            "model_sha256": onnx.sha256,
            "tokenizer_sha256": tokenizer.sha256,
            "max_tokens": spec.max_tokens,
            "pooling": spec.pooling,
            "tensors": spec.tensors,
            "add_special_tokens": spec.add_special_tokens,
            "pad_id": spec.pad_id,
            "pad_type_id": spec.pad_type_id,
            "pad_token": spec.pad_token,
        },
    }))
    .map_err(|_| EmbeddingError::InvalidProfile)
}

/// A static embedding model: one vector per token, averaged over a text's tokens.
pub struct StaticEncoder {
    tokenizer: Tokenizer,
    /// `vocabulary × dimensions`, row by row.
    table: Vec<f32>,
    vocabulary: usize,
    dimensions: usize,
    add_special_tokens: bool,
}

impl StaticEncoder {
    pub fn from_bytes(weights: &[u8], tokenizer: &[u8], spec: &StaticSpec) -> Result<StaticEncoder, EmbeddingError> {
        let tokenizer = Tokenizer::from_bytes(tokenizer).map_err(|_| EmbeddingError::ModelContract)?;
        let (table, vocabulary, dimensions) = read_table(weights, &spec.tensor)?;
        if dimensions != spec.dimensions || vocabulary == 0 {
            return Err(EmbeddingError::ModelContract);
        }
        Ok(StaticEncoder {
            tokenizer,
            table,
            vocabulary,
            dimensions,
            add_special_tokens: spec.add_special_tokens,
        })
    }

    /// The unit-length mean of the text's token vectors; all zeros for a text with no tokens.
    pub fn encode(&self, text: &str) -> Vec<f32> {
        let mut sum = vec![0f32; self.dimensions];
        let ids = self.tokenizer.encode(text, self.add_special_tokens).map(|e| e.get_ids().to_vec()).unwrap_or_default();
        for id in &ids {
            // Out-of-range ids read as the last row, as WordLlama clamps them.
            let row = (*id as usize).min(self.vocabulary - 1) * self.dimensions;
            for (s, v) in sum.iter_mut().zip(&self.table[row..row + self.dimensions]) {
                *s += v;
            }
        }
        let norm = sum.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 && norm.is_finite() {
            sum.iter_mut().for_each(|v| *v /= norm);
        } else {
            sum.iter_mut().for_each(|v| *v = 0.0);
        }
        sum
    }
}

/// A two-dimensional tensor named `name` from safetensors bytes, as f32: the 8-byte header
/// length, the JSON header, then the data at the header's offsets.
fn read_table(bytes: &[u8], name: &str) -> Result<(Vec<f32>, usize, usize), EmbeddingError> {
    let contract = || EmbeddingError::ModelContract;
    let size = u64::from_le_bytes(bytes.get(..8).ok_or_else(contract)?.try_into().map_err(|_| contract())?);
    let size = usize::try_from(size).map_err(|_| contract())?;
    let header_end = 8usize.checked_add(size).filter(|end| *end <= bytes.len() && size <= 100 * 1024 * 1024).ok_or_else(contract)?;
    let header: serde_json::Value = serde_json::from_slice(&bytes[8..header_end]).map_err(|_| contract())?;
    let tensor = header.get(name).ok_or_else(contract)?;
    let shape: Vec<usize> = serde_json::from_value(tensor["shape"].clone()).map_err(|_| contract())?;
    let [vocabulary, dimensions] = shape[..] else { return Err(contract()) };
    let offsets: [usize; 2] = serde_json::from_value(tensor["data_offsets"].clone()).map_err(|_| contract())?;
    let (width, convert): (usize, fn(&[u8]) -> f32) = match tensor["dtype"].as_str() {
        Some("F32") => (4, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        Some("F16") => (2, |b| f16_to_f32(u16::from_le_bytes([b[0], b[1]]))),
        Some("BF16") => (2, |b| f32::from_bits((u16::from_le_bytes([b[0], b[1]]) as u32) << 16)),
        _ => return Err(contract()),
    };
    let count = vocabulary.checked_mul(dimensions).ok_or_else(contract)?;
    let start = header_end.checked_add(offsets[0]).ok_or_else(contract)?;
    let end = header_end.checked_add(offsets[1]).ok_or_else(contract)?;
    if dimensions == 0 || end > bytes.len() || end.checked_sub(start) != count.checked_mul(width) {
        return Err(contract());
    }
    let table: Vec<f32> = bytes[start..end].chunks_exact(width).map(convert).collect();
    if table.iter().any(|v| !v.is_finite()) {
        return Err(contract());
    }
    Ok((table, vocabulary, dimensions))
}

/// IEEE 754 half precision to single.
fn f16_to_f32(half: u16) -> f32 {
    let sign = ((half >> 15) as u32) << 31;
    let exponent = ((half >> 10) & 0x1f) as u32;
    let mantissa = (half & 0x3ff) as u32;
    let bits = match (exponent, mantissa) {
        (0, 0) => sign,
        (0, _) => {
            // Subnormal: shift the mantissa up until its leading bit is the implicit one.
            let mut e = 127 - 15 + 1;
            let mut m = mantissa;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | ((e as u32) << 23) | ((m & 0x3ff) << 13)
        }
        (0x1f, 0) => sign | 0x7f80_0000,
        (0x1f, _) => sign | 0x7fc0_0000,
        _ => sign | ((exponent + 127 - 15) << 23) | (mantissa << 13),
    };
    f32::from_bits(bits)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Safetensors bytes holding one `rows × columns` F16 tensor.
    pub(crate) fn safetensors(name: &str, rows: usize, columns: usize, value: impl Fn(usize, usize) -> f32) -> Vec<u8> {
        let mut data = Vec::new();
        for r in 0..rows {
            for c in 0..columns {
                data.extend_from_slice(&f32_to_f16(value(r, c)).to_le_bytes());
            }
        }
        let header = serde_json::json!({ name: { "dtype": "F16", "shape": [rows, columns], "data_offsets": [0, data.len()] } }).to_string();
        let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(header.as_bytes());
        bytes.extend_from_slice(&data);
        bytes
    }

    fn f32_to_f16(value: f32) -> u16 {
        // Enough for the small, exactly representable values these fixtures use.
        let bits = value.to_bits();
        let sign = ((bits >> 16) & 0x8000) as u16;
        if value == 0.0 {
            return sign;
        }
        let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
        sign | ((exponent as u16) << 10) | ((bits >> 13) & 0x3ff) as u16
    }

    #[test]
    fn half_precision_reads_exactly() {
        for value in [0.0f32, 1.0, -2.5, 0.125, 65504.0, -0.000061035156] {
            assert_eq!(f16_to_f32(f32_to_f16(value)), value, "{value}");
        }
        assert_eq!(f16_to_f32(0x0001), 2f32.powi(-24), "smallest subnormal");
        assert!(f16_to_f32(0x7c00).is_infinite() && f16_to_f32(0x7e00).is_nan());
    }

    /// The fixture tokenizer's words a, b, c, d are ids 2 to 5; each row is its id in one column.
    #[test]
    fn a_static_model_averages_its_token_vectors() {
        let tokenizer = include_bytes!("../embeddings/fixtures/tiny-tokenizer.json");
        let weights = safetensors("embedding.weight", 8, 4, |row, column| if column == row % 4 { 1.0 } else { 0.0 });
        let spec = StaticSpec {
            tensor: "embedding.weight".into(),
            dimensions: 4,
            add_special_tokens: false,
        };
        let encoder = StaticEncoder::from_bytes(&weights, tokenizer, &spec).unwrap();
        let ids = encoder.tokenizer.encode("a b", false).unwrap().get_ids().to_vec();
        assert_eq!(ids, [2, 3]);
        let vector = encoder.encode("a b");
        let half = std::f32::consts::FRAC_1_SQRT_2;
        assert_eq!(vector, [0.0, 0.0, half, half], "rows 2 and 3, averaged and made unit length");
        assert_eq!(encoder.encode(""), [0.0; 4]);

        // A contract the file does not meet is refused, never guessed at.
        let wrong_width = StaticSpec { dimensions: 8, ..spec.clone() };
        assert!(matches!(StaticEncoder::from_bytes(&weights, tokenizer, &wrong_width), Err(EmbeddingError::ModelContract)));
        let wrong_name = StaticSpec {
            tensor: "embeddings".into(),
            ..spec.clone()
        };
        assert!(matches!(StaticEncoder::from_bytes(&weights, tokenizer, &wrong_name), Err(EmbeddingError::ModelContract)));
        let mut truncated = weights.clone();
        truncated.truncate(weights.len() - 2);
        assert!(matches!(StaticEncoder::from_bytes(&truncated, tokenizer, &spec), Err(EmbeddingError::ModelContract)));
        let mut huge_header = weights.clone();
        huge_header[..8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(matches!(StaticEncoder::from_bytes(&huge_header, tokenizer, &spec), Err(EmbeddingError::ModelContract)));
        assert!(matches!(StaticEncoder::from_bytes(&weights, b"not a tokenizer", &spec), Err(EmbeddingError::ModelContract)));
    }
}
