// crates/backend/src/storage/mock.rs
//! In-memory mock for testing. Captures every call and serves deterministic
//! presigned URLs and HEAD responses based on what tests "simulate".

use super::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum S3Call {
    PresignPut {
        key: String,
        content_type: String,
        content_length: i64,
        ttl_secs: u64,
    },
    PresignGet {
        key: String,
        ttl_secs: u64,
    },
    Head {
        key: String,
    },
    Delete {
        key: String,
    },
    PutObject {
        key: String,
        size: usize,
        content_type: String,
    },
    EnsureBucket {
        name: String,
    },
}

#[derive(Clone, Default)]
pub struct MockS3Client {
    pub calls: Arc<Mutex<Vec<S3Call>>>,
    /// Objects the test has "simulated" being in storage.
    /// Map key -> (size, content_type).
    pub objects: Arc<Mutex<HashMap<String, (i64, String)>>>,
}

impl MockS3Client {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test helper: declare that an object exists with the given size + type.
    /// Used to make `head_object` return success.
    pub fn simulate_object(
        &self,
        key: impl Into<String>,
        size: i64,
        content_type: impl Into<String>,
    ) {
        self.objects
            .lock()
            .unwrap()
            .insert(key.into(), (size, content_type.into()));
    }

    pub fn calls(&self) -> Vec<S3Call> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: S3Call) {
        self.calls.lock().unwrap().push(call);
    }
}

#[async_trait]
impl S3Client for MockS3Client {
    async fn presigned_put_url(
        &self,
        key: &str,
        content_type: &str,
        content_length: i64,
        ttl: Duration,
    ) -> Result<String, StorageError> {
        self.record(S3Call::PresignPut {
            key: key.to_string(),
            content_type: content_type.to_string(),
            content_length,
            ttl_secs: ttl.as_secs(),
        });
        Ok(format!("https://mock.s3/put/{key}?ttl={}", ttl.as_secs()))
    }

    async fn presigned_get_url(&self, key: &str, ttl: Duration) -> Result<String, StorageError> {
        self.record(S3Call::PresignGet {
            key: key.to_string(),
            ttl_secs: ttl.as_secs(),
        });
        Ok(format!("https://mock.s3/get/{key}?ttl={}", ttl.as_secs()))
    }

    async fn head_object(&self, key: &str) -> Result<ObjectHead, StorageError> {
        self.record(S3Call::Head {
            key: key.to_string(),
        });
        let objects = self.objects.lock().unwrap();
        match objects.get(key) {
            Some((size, ct)) => Ok(ObjectHead {
                size_bytes: *size,
                content_type: Some(ct.clone()),
            }),
            None => Err(StorageError::NotFound(key.to_string())),
        }
    }

    async fn delete_object(&self, key: &str) -> Result<(), StorageError> {
        self.record(S3Call::Delete {
            key: key.to_string(),
        });
        self.objects.lock().unwrap().remove(key);
        Ok(())
    }

    async fn put_object(
        &self,
        key: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> Result<(), StorageError> {
        self.record(S3Call::PutObject {
            key: key.to_string(),
            size: body.len(),
            content_type: content_type.to_string(),
        });
        // Mock the object existing for subsequent head_object calls.
        self.objects.lock().unwrap().insert(
            key.to_string(),
            (body.len() as i64, content_type.to_string()),
        );
        Ok(())
    }

    async fn ensure_bucket(&self, name: &str) -> Result<(), StorageError> {
        self.record(S3Call::EnsureBucket {
            name: name.to_string(),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_records_all_call_types() {
        let s3 = MockS3Client::new();
        s3.simulate_object("k", 100, "image/png");
        let _ = s3
            .presigned_put_url("k", "image/png", 100, Duration::from_secs(900))
            .await
            .unwrap();
        let _ = s3
            .presigned_get_url("k", Duration::from_secs(900))
            .await
            .unwrap();
        let head = s3.head_object("k").await.unwrap();
        s3.delete_object("k").await.unwrap();
        s3.ensure_bucket("aulalite").await.unwrap();
        let calls = s3.calls();
        assert_eq!(calls.len(), 5);
        assert_eq!(head.size_bytes, 100);
        assert!(matches!(calls[0], S3Call::PresignPut { .. }));
        assert!(matches!(calls[4], S3Call::EnsureBucket { .. }));
    }

    #[tokio::test]
    async fn head_returns_not_found_for_unsimulated_key() {
        let s3 = MockS3Client::new();
        let err = s3.head_object("missing").await.unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
    }
}
