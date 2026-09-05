// crates/backend/src/services/mediamtx.rs
//! MediaMTX integration: path naming, JWT minting, HTTP API client.
//! Pure helpers live here; the trait + production impl land in later tasks.

use uuid::Uuid;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PathParseError {
    #[error("path missing 'aula/' prefix: {0}")]
    MissingPrefix(String),
    #[error("path has wrong number of segments: {0}")]
    WrongShape(String),
    #[error("invalid uuid in path segment {0}: {1}")]
    InvalidUuid(usize, String),
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParsedPath {
    pub tenant_id: Uuid,
    pub course_id: Uuid,
    pub session_id: Uuid,
    pub is_screen: bool,
}

const PATH_PREFIX: &str = "aula";

pub fn path_for_session(tenant_id: Uuid, course_id: Uuid, session_id: Uuid) -> String {
    format!(
        "{PATH_PREFIX}/{}/{}/{}",
        tenant_id.simple(),
        course_id.simple(),
        session_id.simple(),
    )
}

pub fn screen_path_for_session(tenant_id: Uuid, course_id: Uuid, session_id: Uuid) -> String {
    format!(
        "{}/screen",
        path_for_session(tenant_id, course_id, session_id)
    )
}

pub fn parse_path(path: &str) -> Result<ParsedPath, PathParseError> {
    let segs: Vec<&str> = path.split('/').collect();
    if segs.is_empty() || segs[0] != PATH_PREFIX {
        return Err(PathParseError::MissingPrefix(path.to_string()));
    }
    let (is_screen, body) = match segs.len() {
        4 => (false, &segs[1..4]),
        5 if segs[4] == "screen" => (true, &segs[1..4]),
        _ => return Err(PathParseError::WrongShape(path.to_string())),
    };
    let parse = |i: usize, s: &str| -> Result<Uuid, PathParseError> {
        Uuid::parse_str(s).map_err(|_| PathParseError::InvalidUuid(i, s.to_string()))
    };
    Ok(ParsedPath {
        tenant_id: parse(1, body[0])?,
        course_id: parse(2, body[1])?,
        session_id: parse(3, body[2])?,
        is_screen,
    })
}

use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rsa::{
    pkcs1::{DecodeRsaPrivateKey, EncodeRsaPrivateKey, EncodeRsaPublicKey},
    rand_core::OsRng,
    traits::PublicKeyParts,
    RsaPrivateKey,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct MediaMtxPermission {
    pub action: String,
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ViewerClaims {
    pub iss: String,
    pub sub: String,
    pub tnt: String,
    pub mediamtx_permissions: Vec<MediaMtxPermission>,
    pub exp: i64,
}

#[derive(Clone)]
pub struct JwtSigner {
    inner: Arc<JwtSignerInner>,
}

struct JwtSignerInner {
    encoding: EncodingKey,
    decoding: DecodingKey,
    jwks_json: String,
    kid: String,
}

impl JwtSigner {
    pub fn new_ephemeral() -> Self {
        let mut rng = OsRng;
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("rsa keygen");
        Self::from_private(priv_key)
    }

    pub fn from_pem(pem: &str) -> anyhow::Result<Self> {
        let priv_key = RsaPrivateKey::from_pkcs1_pem(pem)
            .map_err(|e| anyhow::anyhow!("rsa pem parse: {e}"))?;
        Ok(Self::from_private(priv_key))
    }

    fn from_private(priv_key: RsaPrivateKey) -> Self {
        let pub_key = priv_key.to_public_key();
        let priv_pem = priv_key
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .expect("pem encode");
        let pub_pem = pub_key
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .expect("pub pem encode");
        let encoding = EncodingKey::from_rsa_pem(priv_pem.as_bytes()).expect("encoding key");
        let decoding = DecodingKey::from_rsa_pem(pub_pem.as_bytes()).expect("decoding key");
        let kid = format!("aulalite-{}", uuid::Uuid::new_v4().simple());
        let jwks_json = build_jwks_json(&pub_key, &kid);
        Self {
            inner: Arc::new(JwtSignerInner {
                encoding,
                decoding,
                jwks_json,
                kid,
            }),
        }
    }

    pub fn mint_viewer_jwt(&self, mut claims: ViewerClaims, ttl: Duration) -> String {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        claims.exp = now + ttl.as_secs() as i64;
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.inner.kid.clone());
        encode(&header, &claims, &self.inner.encoding).expect("jwt encode")
    }

    pub fn verify_viewer_jwt(&self, token: &str) -> Result<ViewerClaims, String> {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&["aulalite"]);
        decode::<ViewerClaims>(token, &self.inner.decoding, &validation)
            .map(|d| d.claims)
            .map_err(|e| e.to_string())
    }

    pub fn public_jwks(&self) -> &str {
        &self.inner.jwks_json
    }
}

fn build_jwks_json(pub_key: &rsa::RsaPublicKey, kid: &str) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let n_b64 = URL_SAFE_NO_PAD.encode(pub_key.n().to_bytes_be());
    let e_b64 = URL_SAFE_NO_PAD.encode(pub_key.e().to_bytes_be());
    serde_json::json!({
        "keys": [{
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": kid,
            "n": n_b64,
            "e": e_b64,
        }]
    })
    .to_string()
}

use async_trait::async_trait;
use std::collections::HashSet;

#[derive(Debug, thiserror::Error)]
pub enum MediaMtxError {
    #[error("mediamtx api error: {0}")]
    Api(String),
    #[error("transport: {0}")]
    Transport(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathStatus {
    Active,
    Inactive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaMtxCall {
    PublishStarted { path: String },
    PublishEnded { path: String },
    PathStatus { path: String },
    Healthz,
}

/// Whether a live MediaMTX session sitting on `session_path` belongs to the
/// class rooted at `root`, and should therefore be evicted when that class
/// ends.
///
/// A class owns more than its main path: the teacher's screen share publishes
/// to `<root>/screen`, promoted students to `<root>/student/<id>`, and
/// breakouts to `<root>/breakout/<id>`. `end-class` only ever passes the main
/// path, so matching on equality alone would leave every one of those
/// publishers running after the class was over.
///
/// The separator is required: `aula/t/c/s2` must NOT be swept up by a class
/// rooted at `aula/t/c/s`.
pub fn is_session_subpath(root: &str, session_path: &str) -> bool {
    session_path == root
        || session_path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[async_trait]
pub trait MediaMtxClient: Send + Sync {
    /// Idempotent: register/refresh path (typically issued on go-live).
    async fn publish_started(&self, path: &str) -> Result<(), MediaMtxError>;
    /// Idempotent: remove path after publish ends.
    async fn publish_ended(&self, path: &str) -> Result<(), MediaMtxError>;
    /// Returns current liveness of a path.
    async fn path_status(&self, path: &str) -> Result<PathStatus, MediaMtxError>;
    /// Health probe; returns Ok if MediaMTX HTTP API is reachable.
    async fn healthz(&self) -> Result<(), MediaMtxError>;
}

/// Test-only stub for `MediaMtxClient`. The struct is `pub` so integration
/// tests outside this crate can construct it, but it is `#[doc(hidden)]` to
/// keep it out of generated rustdoc and signal that production callers
/// must never inject it. Always-success semantics make it dangerous if
/// wired into prod — see the recording tests for the intended usage.
#[doc(hidden)]
#[derive(Clone, Default)]
pub struct MockMediaMtxClient {
    pub calls: Arc<Mutex<Vec<MediaMtxCall>>>,
    pub active: Arc<Mutex<HashSet<String>>>,
}

use std::sync::Mutex;

impl MockMediaMtxClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn calls(&self) -> Vec<MediaMtxCall> {
        self.calls.lock().unwrap().clone()
    }

    pub fn simulate_active(&self, path: impl Into<String>) {
        self.active.lock().unwrap().insert(path.into());
    }

    fn record(&self, call: MediaMtxCall) {
        self.calls.lock().unwrap().push(call);
    }
}

#[async_trait]
impl MediaMtxClient for MockMediaMtxClient {
    async fn publish_started(&self, path: &str) -> Result<(), MediaMtxError> {
        self.record(MediaMtxCall::PublishStarted { path: path.into() });
        self.active.lock().unwrap().insert(path.into());
        Ok(())
    }
    async fn publish_ended(&self, path: &str) -> Result<(), MediaMtxError> {
        self.record(MediaMtxCall::PublishEnded { path: path.into() });
        self.active.lock().unwrap().remove(path);
        Ok(())
    }
    async fn path_status(&self, path: &str) -> Result<PathStatus, MediaMtxError> {
        self.record(MediaMtxCall::PathStatus { path: path.into() });
        Ok(if self.active.lock().unwrap().contains(path) {
            PathStatus::Active
        } else {
            PathStatus::Inactive
        })
    }
    async fn healthz(&self) -> Result<(), MediaMtxError> {
        self.record(MediaMtxCall::Healthz);
        Ok(())
    }
}

#[derive(Clone)]
pub struct HttpMediaMtxClient {
    pub base_url: String,
    pub http: reqwest::Client,
}

impl HttpMediaMtxClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(5))
                .build()
                .expect("static MediaMTX HTTP client configuration must be valid"),
        }
    }
}

#[async_trait]
impl MediaMtxClient for HttpMediaMtxClient {
    async fn publish_started(&self, _path: &str) -> Result<(), MediaMtxError> {
        // MediaMTX auto-registers paths on first publish; nothing to do here.
        // Reserved for future explicit registration if MediaMTX config requires it.
        Ok(())
    }

    /// Evict every WebRTC session still attached to `path` when a class ends.
    ///
    /// This used to POST `/v3/paths/kick/{path}`, which MediaMTX 1.18.2 does
    /// NOT implement -- probing the running server shows gin's no-route handler
    /// (`404`, `text/plain`, body `404 page not found`), as opposed to a real
    /// route's typed `404 {"status":"error",...}` in `application/json`. Because
    /// the old code deliberately treated `NOT_FOUND` as success, every call
    /// returned `Ok(())` without evicting anything: end-class never dropped a
    /// lingering publisher, so a stale session could keep the path occupied and
    /// make the teacher's next go-live collide with it.
    ///
    /// The supported API is per-SESSION: enumerate `/v3/webrtcsessions/list`
    /// and `POST /v3/webrtcsessions/kick/{id}` for each session on the path.
    /// Still best-effort -- a session that has already gone away answers
    /// `404 {"error":"session not found"}`, which is success for our purposes.
    async fn publish_ended(&self, path: &str) -> Result<(), MediaMtxError> {
        #[derive(serde::Deserialize)]
        struct SessionItem {
            id: String,
            #[serde(default)]
            path: String,
        }
        #[derive(serde::Deserialize)]
        struct SessionList {
            #[serde(default)]
            items: Vec<SessionItem>,
            #[serde(default)]
            #[serde(rename = "pageCount")]
            page_count: u32,
        }

        // Page through the session list rather than trusting one page: a busy
        // deployment can hold more sessions than MediaMTX returns by default.
        let mut ids: Vec<String> = Vec::new();
        let mut page: u32 = 0;
        loop {
            let url = format!(
                "{}/v3/webrtcsessions/list?page={}&itemsPerPage=1000",
                self.base_url, page
            );
            let resp = self
                .http
                .get(&url)
                .send()
                .await
                .map_err(|e| MediaMtxError::Transport(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(MediaMtxError::Api(format!(
                    "webrtcsessions list: {}",
                    resp.status()
                )));
            }
            let body: SessionList = crate::services::bounded_http::json(resp, 4 * 1024 * 1024)
                .await
                .map_err(|e| MediaMtxError::Api(e.to_string()))?;
            ids.extend(
                body.items
                    .into_iter()
                    .filter(|s| is_session_subpath(path, &s.path))
                    .map(|s| s.id),
            );
            page += 1;
            if page >= body.page_count {
                break;
            }
        }

        // Kick EVERY session before reporting a problem. Returning on the first
        // failure left the remaining publishers on the path running, which is
        // the exact leak this function exists to prevent -- and the one that
        // matters most is whichever session is still publishing.
        let mut failures: Vec<String> = Vec::new();
        for id in ids {
            let url = format!("{}/v3/webrtcsessions/kick/{}", self.base_url, id);
            let resp = match self.http.post(&url).send().await {
                Ok(resp) => resp,
                Err(e) => {
                    failures.push(format!("{id}: {e}"));
                    continue;
                }
            };
            // A session that ended between the list and the kick is already
            // gone -- exactly the outcome we wanted.
            if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NOT_FOUND {
                failures.push(format!("{id}: {}", resp.status()));
            }
        }
        if !failures.is_empty() {
            return Err(MediaMtxError::Api(format!(
                "kick on {}: {}",
                path,
                failures.join(", ")
            )));
        }
        Ok(())
    }

    async fn path_status(&self, path: &str) -> Result<PathStatus, MediaMtxError> {
        let url = format!("{}/v3/paths/get/{}", self.base_url, path);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| MediaMtxError::Transport(e.to_string()))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(PathStatus::Inactive);
        }
        if !resp.status().is_success() {
            return Err(MediaMtxError::Api(format!(
                "path get {}: {}",
                path,
                resp.status()
            )));
        }
        let body: serde_json::Value = crate::services::bounded_http::json(resp, 1024 * 1024)
            .await
            .map_err(|e| MediaMtxError::Api(e.to_string()))?;
        let active = body.get("ready").and_then(|v| v.as_bool()).unwrap_or(false);
        Ok(if active {
            PathStatus::Active
        } else {
            PathStatus::Inactive
        })
    }

    async fn healthz(&self) -> Result<(), MediaMtxError> {
        let url = format!("{}/v3/config/global/get", self.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| MediaMtxError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(MediaMtxError::Api(format!("healthz {}", resp.status())));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Uuid {
        Uuid::parse_str("9c2f4a8e-7b13-4f7c-91d2-b6a8e5c0d3e1").unwrap()
    }
    fn c() -> Uuid {
        Uuid::parse_str("4f1a2c8b-9d6e-4a7f-8c5b-3d2e1f9a0c8e").unwrap()
    }
    fn s() -> Uuid {
        Uuid::parse_str("a1b2c3d4-e5f6-4789-abcd-ef0123456789").unwrap()
    }

    #[test]
    fn path_for_session_uses_simple_uuids_with_aula_prefix() {
        let path = path_for_session(t(), c(), s());
        assert_eq!(
            path,
            "aula/9c2f4a8e7b134f7c91d2b6a8e5c0d3e1/4f1a2c8b9d6e4a7f8c5b3d2e1f9a0c8e/a1b2c3d4e5f64789abcdef0123456789"
        );
    }

    #[test]
    fn screen_path_appends_slash_screen() {
        let main = path_for_session(t(), c(), s());
        let screen = screen_path_for_session(t(), c(), s());
        assert_eq!(screen, format!("{main}/screen"));
    }

    #[test]
    fn parse_path_round_trips_main() {
        let path = path_for_session(t(), c(), s());
        let parsed = parse_path(&path).unwrap();
        assert_eq!(
            parsed,
            ParsedPath {
                tenant_id: t(),
                course_id: c(),
                session_id: s(),
                is_screen: false
            }
        );
    }

    #[test]
    fn parse_path_round_trips_screen() {
        let path = screen_path_for_session(t(), c(), s());
        let parsed = parse_path(&path).unwrap();
        assert_eq!(
            parsed,
            ParsedPath {
                tenant_id: t(),
                course_id: c(),
                session_id: s(),
                is_screen: true
            }
        );
    }

    #[test]
    fn parse_path_rejects_missing_prefix() {
        let err = parse_path("foo/bar/baz/qux").unwrap_err();
        assert!(matches!(err, PathParseError::MissingPrefix(_)));
    }

    #[test]
    fn parse_path_rejects_wrong_shape() {
        let err = parse_path("aula/foo/bar").unwrap_err();
        assert!(matches!(err, PathParseError::WrongShape(_)));
    }

    #[test]
    fn parse_path_rejects_bad_uuid() {
        let err = parse_path("aula/notauuid/notauuid/notauuid").unwrap_err();
        assert!(matches!(err, PathParseError::InvalidUuid(_, _)));
    }

    #[test]
    fn jwt_signer_mints_and_verifies_round_trip() {
        let signer = JwtSigner::new_ephemeral();
        let claims = ViewerClaims {
            iss: "aulalite".into(),
            sub: "user-uuid".into(),
            tnt: "tenant-uuid".into(),
            mediamtx_permissions: vec![MediaMtxPermission {
                action: "read".into(),
                path: "aula/x/y/z".into(),
            }],
            exp: 0,
        };
        let token = signer.mint_viewer_jwt(claims, std::time::Duration::from_secs(900));
        let decoded = signer.verify_viewer_jwt(&token).unwrap();
        assert_eq!(decoded.iss, "aulalite");
        assert_eq!(decoded.mediamtx_permissions.len(), 1);
        assert!(decoded.exp > 0);
    }

    #[test]
    fn jwt_signer_rejects_tampered_token() {
        let signer = JwtSigner::new_ephemeral();
        let claims = ViewerClaims {
            iss: "aulalite".into(),
            sub: "u".into(),
            tnt: "t".into(),
            mediamtx_permissions: vec![],
            exp: 0,
        };
        let token = signer.mint_viewer_jwt(claims, std::time::Duration::from_secs(900));
        let mut bad = token.clone();
        let last = bad.pop().unwrap();
        let flipped = if last == 'A' { 'B' } else { 'A' };
        bad.push(flipped);
        assert!(signer.verify_viewer_jwt(&bad).is_err());
    }

    #[test]
    fn jwt_signer_jwks_round_trip() {
        let signer = JwtSigner::new_ephemeral();
        let jwks = signer.public_jwks();
        assert!(jwks.contains("\"kty\":\"RSA\""), "got: {jwks}");
        assert!(jwks.contains("\"n\":"), "got: {jwks}");
    }

    #[test]
    fn session_subpath_matches_the_whole_class_tree() {
        let root = "aula/tenant/course/session";
        // The main path itself.
        assert!(is_session_subpath(root, root));
        // Every sub-surface a class publishes to.
        assert!(is_session_subpath(root, "aula/tenant/course/session/screen"));
        assert!(is_session_subpath(
            root,
            "aula/tenant/course/session/student/abc"
        ));
        assert!(is_session_subpath(
            root,
            "aula/tenant/course/session/breakout/1"
        ));
    }

    #[test]
    fn session_subpath_requires_a_separator() {
        let root = "aula/tenant/course/session";
        // A DIFFERENT session whose id merely starts with ours must survive:
        // ending one class must never kick another class's publisher.
        assert!(!is_session_subpath(root, "aula/tenant/course/session2"));
        assert!(!is_session_subpath(root, "aula/tenant/course/sessionX/screen"));
    }

    #[test]
    fn session_subpath_rejects_unrelated_and_parent_paths() {
        let root = "aula/tenant/course/session";
        assert!(!is_session_subpath(root, "aula/other/course/session"));
        assert!(!is_session_subpath(root, "aula/tenant/course"));
        assert!(!is_session_subpath(root, ""));
    }

    #[tokio::test]
    async fn mock_records_publish_started_and_ended() {
        let client = MockMediaMtxClient::new();
        client.publish_started("aula/x/y/z").await.unwrap();
        client.publish_ended("aula/x/y/z").await.unwrap();
        let calls = client.calls();
        assert_eq!(calls.len(), 2);
        assert!(matches!(calls[0], MediaMtxCall::PublishStarted { .. }));
        assert!(matches!(calls[1], MediaMtxCall::PublishEnded { .. }));
    }

    #[tokio::test]
    async fn mock_path_status_defaults_to_inactive() {
        let client = MockMediaMtxClient::new();
        let status = client.path_status("aula/x/y/z").await.unwrap();
        assert_eq!(status, PathStatus::Inactive);
    }

    #[tokio::test]
    async fn mock_simulate_active_returns_active() {
        let client = MockMediaMtxClient::new();
        client.simulate_active("aula/x/y/z");
        let status = client.path_status("aula/x/y/z").await.unwrap();
        assert_eq!(status, PathStatus::Active);
    }

    #[tokio::test]
    async fn mock_healthz_ok() {
        let client = MockMediaMtxClient::new();
        client.healthz().await.unwrap();
    }
}
