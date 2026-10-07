//! Real Runner adapters; transport dependencies never enter phone builds.
pub mod http;
pub mod hindsight;
pub mod openviking;
#[cfg(feature = "memory-pgvector")]
pub mod pgvector;
#[cfg(all(feature = "memory-pgvector", feature = "memory-lance"))]
pub mod lance;
