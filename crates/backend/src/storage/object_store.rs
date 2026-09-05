// crates/backend/src/storage/object_store.rs
//! Production `S3Client` implementation backed by aws-sdk-s3 against an
//! S3-compatible object store. AulaLite ships RustFS (https://rustfs.com) as
//! the bundled store, but any S3-compatible endpoint works. Uses path-style
//! addressing (force_path_style=true) which RustFS/MinIO require; AWS S3 itself
//! accepts both.

use super::*;
use async_trait::async_trait;
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_s3::{
    config::Builder as S3ConfigBuilder,
    presigning::PresigningConfig,
    primitives::ByteStream,
    types::{CorsConfiguration, CorsRule},
    Client,
};
use std::time::Duration;

#[derive(Clone)]
pub struct S3CompatClient {
    /// Private service-to-service client used for bucket administration and
    /// object I/O. Keeping this on the Compose network avoids making backend
    /// startup depend on public DNS, Traefik, or certificate issuance.
    pub client: Client,
    /// Browser-facing client used only to mint presigned URLs. The endpoint's
    /// host is part of the signature and therefore must be publicly routable.
    presign_client: Client,
    pub bucket: String,
    browser_origins: Vec<String>,
}

pub struct S3CompatConfig {
    /// Private endpoint for server-side object-store calls.
    pub endpoint_url: String,
    /// Public endpoint embedded in browser-direct presigned URLs.
    pub public_endpoint_url: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    /// Exact application/admin origins allowed to use browser-direct presigned
    /// PUT/GET URLs. Bucket CORS is reconciled at startup.
    pub browser_origins: Vec<String>,
}

impl S3CompatClient {
    pub fn new(cfg: S3CompatConfig) -> Self {
        let credentials = Credentials::new(
            cfg.access_key_id.clone(),
            cfg.secret_access_key.clone(),
            None,
            None,
            "static",
        );
        let s3_config = S3ConfigBuilder::new()
            .behavior_version(BehaviorVersion::latest())
            .endpoint_url(cfg.endpoint_url)
            .region(Region::new(cfg.region.clone()))
            .credentials_provider(credentials.clone())
            .force_path_style(true)
            .build();
        let presign_config = S3ConfigBuilder::new()
            .behavior_version(BehaviorVersion::latest())
            .endpoint_url(cfg.public_endpoint_url)
            .region(Region::new(cfg.region))
            .credentials_provider(credentials)
            .force_path_style(true)
            .build();
        let client = Client::from_conf(s3_config);
        let presign_client = Client::from_conf(presign_config);
        Self {
            client,
            presign_client,
            bucket: cfg.bucket,
            browser_origins: cfg.browser_origins,
        }
    }
}

fn browser_cors_configuration(origins: &[String]) -> Result<CorsConfiguration, StorageError> {
    if origins.is_empty() {
        return Err(StorageError::BucketBootstrap(
            "browser CORS requires at least one allowed origin".into(),
        ));
    }
    let rule = CorsRule::builder()
        .id("aulalite-browser-direct")
        .set_allowed_origins(Some(origins.to_vec()))
        .allowed_methods("GET")
        .allowed_methods("PUT")
        .allowed_methods("HEAD")
        .allowed_headers("*")
        .expose_headers("ETag")
        .expose_headers("Content-Length")
        .expose_headers("Content-Type")
        .max_age_seconds(3600)
        .build()
        .map_err(|error| {
            StorageError::BucketBootstrap(format!("build browser CORS rule: {error}"))
        })?;

    CorsConfiguration::builder()
        .cors_rules(rule)
        .build()
        .map_err(|error| {
            StorageError::BucketBootstrap(format!("build browser CORS configuration: {error}"))
        })
}

fn map_err<E: std::fmt::Debug>(prefix: &str, err: E) -> StorageError {
    StorageError::S3(format!("{prefix}: {err:?}"))
}

#[async_trait]
impl S3Client for S3CompatClient {
    async fn presigned_put_url(
        &self,
        key: &str,
        content_type: &str,
        content_length: i64,
        ttl: Duration,
    ) -> Result<String, StorageError> {
        let presigning =
            PresigningConfig::expires_in(ttl).map_err(|e| map_err("presigning config", e))?;
        let req = self
            .presign_client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .content_length(content_length)
            .presigned(presigning)
            .await
            .map_err(|e| map_err("presigned put", e))?;
        Ok(req.uri().to_string())
    }

    async fn presigned_get_url(&self, key: &str, ttl: Duration) -> Result<String, StorageError> {
        let presigning =
            PresigningConfig::expires_in(ttl).map_err(|e| map_err("presigning config", e))?;
        let req = self
            .presign_client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(presigning)
            .await
            .map_err(|e| map_err("presigned get", e))?;
        Ok(req.uri().to_string())
    }

    async fn head_object(&self, key: &str) -> Result<ObjectHead, StorageError> {
        let resp = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await;
        match resp {
            Ok(o) => Ok(ObjectHead {
                size_bytes: o.content_length().unwrap_or(0),
                content_type: o.content_type().map(|s| s.to_string()),
            }),
            Err(e) => {
                let svc_err = e.into_service_error();
                if svc_err.is_not_found() {
                    Err(StorageError::NotFound(key.to_string()))
                } else {
                    Err(StorageError::S3(format!("head_object: {svc_err:?}")))
                }
            }
        }
    }

    async fn delete_object(&self, key: &str) -> Result<(), StorageError> {
        let _ = ByteStream::from_static(b""); // ensure import path is valid
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| map_err("delete_object", e))?;
        Ok(())
    }

    async fn put_object(
        &self,
        key: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> Result<(), StorageError> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(body))
            .content_type(content_type)
            .send()
            .await
            .map_err(|e| map_err("put_object", e))?;
        Ok(())
    }

    async fn ensure_bucket(&self, name: &str) -> Result<(), StorageError> {
        // HEAD bucket; if 404, create. Do not return early for an existing
        // bucket: its browser CORS policy is part of the deploy contract and
        // must converge after an APP_ORIGIN change as well as on first boot.
        let head = self.client.head_bucket().bucket(name).send().await;
        if let Err(error) = head {
            let service_error = error.into_service_error();
            if !service_error.is_not_found() {
                return Err(StorageError::BucketBootstrap(format!(
                    "head_bucket failed (non-404): {service_error:?}"
                )));
            }
            self.client
                .create_bucket()
                .bucket(name)
                .send()
                .await
                .map_err(|error| {
                    StorageError::BucketBootstrap(format!("create_bucket: {error:?}"))
                })?;
        }

        let cors_configuration = browser_cors_configuration(&self.browser_origins)?;
        self.client
            .put_bucket_cors()
            .bucket(name)
            .cors_configuration(cors_configuration)
            .send()
            .await
            .map_err(|error| {
                StorageError::BucketBootstrap(format!("put_bucket_cors: {error:?}"))
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{browser_cors_configuration, S3CompatClient, S3CompatConfig};
    use crate::storage::S3Client;
    use std::time::Duration;

    #[test]
    fn browser_cors_is_limited_to_the_configured_browser_origins() {
        let configuration = browser_cors_configuration(&[
            "https://aula.elementors.guru".into(),
            "https://admin.elementors.guru".into(),
        ])
        .unwrap();
        let rule = &configuration.cors_rules()[0];

        assert_eq!(
            rule.allowed_origins(),
            [
                "https://aula.elementors.guru",
                "https://admin.elementors.guru"
            ]
        );
        assert_eq!(rule.allowed_methods(), ["GET", "PUT", "HEAD"]);
        assert_eq!(rule.allowed_headers(), ["*"]);
        assert_eq!(rule.max_age_seconds(), Some(3600));
    }

    #[tokio::test]
    async fn presigned_urls_use_the_public_endpoint() {
        let storage = S3CompatClient::new(S3CompatConfig {
            endpoint_url: "http://rustfs:9000".into(),
            public_endpoint_url: "https://storage.example.test".into(),
            region: "us-east-1".into(),
            bucket: "aulalite".into(),
            access_key_id: "test-access-key".into(),
            secret_access_key: "test-secret-key".into(),
            browser_origins: vec!["https://app.example.test".into()],
        });

        let url = storage
            .presigned_get_url("lessons/asset.pdf", Duration::from_secs(60))
            .await
            .unwrap();
        let parsed = reqwest::Url::parse(&url).unwrap();

        assert_eq!(parsed.scheme(), "https");
        assert_eq!(parsed.host_str(), Some("storage.example.test"));
        assert_eq!(parsed.path(), "/aulalite/lessons/asset.pdf");
    }
}
