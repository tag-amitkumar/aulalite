// crates/backend/src/storage/mod.rs
//! S3-compatible blob storage abstraction. Production implementation wraps
//! aws-sdk-s3 against an S3-compatible object store (RustFS in the AulaLite
//! stack); tests inject MockS3Client.

pub mod mock;
pub mod object_store;

use async_trait::async_trait;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("s3 error: {0}")]
    S3(String),
    #[error("object not found: {0}")]
    NotFound(String),
    #[error("size mismatch: expected {expected}, observed {observed}")]
    SizeMismatch { expected: i64, observed: i64 },
    #[error("bucket bootstrap failed: {0}")]
    BucketBootstrap(String),
}

/// Object metadata returned by `head_object`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectHead {
    pub size_bytes: i64,
    pub content_type: Option<String>,
}

/// Operations the upload handlers need from blob storage.
#[async_trait]
pub trait S3Client: Send + Sync {
    /// Mint a presigned URL the client uses to PUT bytes directly.
    /// `content_type` and `content_length` are baked into the signed URL,
    /// so the client must `PUT` with matching headers.
    async fn presigned_put_url(
        &self,
        key: &str,
        content_type: &str,
        content_length: i64,
        ttl: Duration,
    ) -> Result<String, StorageError>;

    /// Mint a presigned URL the client uses to GET bytes for display/download.
    async fn presigned_get_url(&self, key: &str, ttl: Duration) -> Result<String, StorageError>;

    /// HEAD the object; used by `/uploads/:id/complete` to verify size + presence.
    async fn head_object(&self, key: &str) -> Result<ObjectHead, StorageError>;

    /// Hard delete the object. The DB row is separately marked `pruned`.
    async fn delete_object(&self, key: &str) -> Result<(), StorageError>;

    /// Server-side direct upload (bypasses presigned URLs). Used by the
    /// recording sidecar to push remuxed MP4 bytes to the bucket. Phase 1b-δ.
    async fn put_object(
        &self,
        key: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> Result<(), StorageError>;

    /// Idempotent bucket creation. Called at backend startup.
    async fn ensure_bucket(&self, name: &str) -> Result<(), StorageError>;
}
