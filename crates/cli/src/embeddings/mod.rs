//! Separate vector-space profiles: portable identity, explicit Runner transports.
mod types;
pub use types::*;
#[cfg(feature = "runner")]
pub mod api;
#[cfg(feature = "runner")]
pub mod assets;
#[cfg(feature = "embedding-local")]
pub mod local;

#[cfg(feature = "runner")]
#[async_trait::async_trait]
pub trait Embedding: Send + Sync {
    fn fingerprint(&self) -> &EmbeddingFingerprint;
    async fn embed(
        &self,
        purpose: EmbeddingPurpose,
        texts: &[String],
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<EmbeddingBatch, EmbeddingError>;
}

#[cfg(test)]
mod tests;
