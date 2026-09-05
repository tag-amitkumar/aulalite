use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::DecodingKey;
use tokio::sync::{Mutex, RwLock};

#[derive(Debug, thiserror::Error)]
pub enum JwksError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("response: {0}")]
    Response(#[from] crate::services::bounded_http::BoundedJsonError),
    #[error("invalid key data: {0}")]
    KeyData(String),
    #[error("kid not found")]
    KidNotFound,
}

#[derive(Clone)]
pub struct JwksCache {
    inner: Arc<RwLock<Inner>>,
    refresh_lock: Arc<Mutex<()>>,
    url: String,
    ttl: Duration,
    http: reqwest::Client,
}

struct Inner {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Option<Instant>,
}

enum CacheLookup {
    Hit(DecodingKey),
    FreshMiss,
    Stale,
}

impl JwksCache {
    pub fn new(url: impl Into<String>, ttl: Duration) -> Self {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .expect("static Firebase JWKS HTTP client configuration must be valid");
        Self::with_client(url, ttl, http)
    }

    /// Construct a cache with a caller-supplied client. Tenant-configurable
    /// OIDC endpoints use this to inject a DNS-pinned, private-network-safe
    /// client while Firebase uses the bounded default above.
    pub fn with_client(url: impl Into<String>, ttl: Duration, http: reqwest::Client) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner {
                keys: HashMap::new(),
                fetched_at: None,
            })),
            refresh_lock: Arc::new(Mutex::new(())),
            url: url.into(),
            ttl,
            http,
        }
    }

    pub async fn key_for_kid(&self, kid: &str) -> Result<DecodingKey, JwksError> {
        match self.lookup(kid).await {
            CacheLookup::Hit(key) => return Ok(key),
            CacheLookup::FreshMiss => return Err(JwksError::KidNotFound),
            CacheLookup::Stale => {}
        }

        // One upstream refresh per cache expiry. This also prevents an
        // attacker-controlled random `kid` from turning every token attempt
        // into a JWKS request while a fresh key set is already cached.
        let _refresh_guard = self.refresh_lock.lock().await;
        match self.lookup(kid).await {
            CacheLookup::Hit(key) => return Ok(key),
            CacheLookup::FreshMiss => return Err(JwksError::KidNotFound),
            CacheLookup::Stale => {}
        }
        self.refresh().await?;
        match self.lookup(kid).await {
            CacheLookup::Hit(key) => Ok(key),
            CacheLookup::FreshMiss | CacheLookup::Stale => Err(JwksError::KidNotFound),
        }
    }

    async fn lookup(&self, kid: &str) -> CacheLookup {
        let cache = self.inner.read().await;
        if cache.fetched_at.is_none_or(|t| t.elapsed() > self.ttl) {
            return CacheLookup::Stale;
        }

        cache
            .keys
            .get(kid)
            .cloned()
            .map(CacheLookup::Hit)
            .unwrap_or(CacheLookup::FreshMiss)
    }

    async fn refresh(&self) -> Result<(), JwksError> {
        let response = self.http.get(&self.url).send().await?.error_for_status()?;
        let response: JwkSet = crate::services::bounded_http::json(response, 1024 * 1024).await?;
        let mut keys = HashMap::new();

        for jwk in response.keys {
            if jwk.kty != "RSA" {
                continue;
            }
            let n = jwk
                .n
                .ok_or_else(|| JwksError::KeyData("missing n".into()))?;
            let e = jwk
                .e
                .ok_or_else(|| JwksError::KeyData("missing e".into()))?;
            let key = DecodingKey::from_rsa_components(&n, &e)
                .map_err(|err| JwksError::KeyData(err.to_string()))?;
            keys.insert(jwk.kid, key);
        }

        let mut cache = self.inner.write().await;
        cache.keys = keys;
        cache.fetched_at = Some(Instant::now());
        Ok(())
    }
}

#[derive(serde::Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}

#[derive(serde::Deserialize)]
struct Jwk {
    kid: String,
    kty: String,
    n: Option<String>,
    e: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{JwksCache, JwksError};
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn empty_cache_returns_error_when_refresh_fails() {
        let cache = JwksCache::new("http://127.0.0.1:1/jwks", Duration::from_secs(60));
        let result = cache.key_for_kid("nope").await;

        assert!(
            result.is_err(),
            "expected error from unreachable JWKS endpoint"
        );
    }

    #[tokio::test]
    async fn fresh_unknown_kid_does_not_hit_the_network() {
        let cache = JwksCache::new("http://127.0.0.1:1/jwks", Duration::from_secs(60));
        cache.inner.write().await.fetched_at = Some(Instant::now());

        let result = cache.key_for_kid("attacker-random-kid").await;
        assert!(matches!(result, Err(JwksError::KidNotFound)));
    }
}
