// crates/backend/src/main.rs
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use backend::auth::jwks::JwksCache;
use backend::auth::local_login::LocalLoginConfig;
use backend::auth::super_admin::SuperAdminConfig;
use backend::auth::verify::Verifier;
use backend::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Local development reads the repository's ignored `.env`; pre-existing
    // process/container variables keep precedence. Production images exclude
    // `.env` and receive secrets from the deployment environment instead.
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    // Resolve the runtime environment up front (fail-safe allowlist: anything
    // not explicitly local/dev/test/ci is treated as production). Used by the
    // migration credential gate below, the RLS gate, and the secret/mock
    // fallbacks further down.
    let app_env = std::env::var("APP_ENV").unwrap_or_else(|_| "production".into());
    let is_production = backend::auth::local_login::is_production(&app_env);
    tracing::info!(%app_env, is_production, "resolved runtime environment");

    if is_production
        && !std::env::var("AULALITE_MFA_ENFORCE")
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false)
    {
        anyhow::bail!(
            "AULALITE_MFA_ENFORCE must be true in production; enrolled users must not be shown a decorative second factor"
        );
    }

    let data_encryption_enabled = if is_production {
        let session_secret = std::env::var("SSO_SESSION_SECRET")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| value.len() >= 32);
        if session_secret.is_none() {
            anyhow::bail!(
                "SSO_SESSION_SECRET must contain at least 32 characters in production; it signs SSO, LTI, and MFA step-up sessions"
            );
        }

        backend::services::secret_box::validate_required_key().map_err(|error| {
            anyhow::anyhow!("AULALITE_DATA_ENCRYPTION_KEY configuration is invalid: {error}")
        })?;
        true
    } else {
        backend::services::secret_box::current_key_configured().map_err(|error| {
            anyhow::anyhow!("AULALITE_DATA_ENCRYPTION_KEY configuration is invalid: {error}")
        })?
    };
    if !data_encryption_enabled {
        tracing::warn!(
            "application data encryption is disabled; TOTP and tenant OIDC secrets will be stored as plaintext (non-production only)"
        );
    }

    // Run schema migrations through a separate owner-level connection. The
    // runtime DATABASE_URL should use the NOBYPASSRLS `aulalite_app` role in
    // production; that role intentionally has DML privileges only and cannot
    // safely own schema changes. Local/dev may omit MIGRATION_DATABASE_URL and
    // reuse DATABASE_URL for convenience, but production fails closed.
    let migration_url = std::env::var("MIGRATION_DATABASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let pool = match migration_url {
        Some(url) => {
            let migration_pool = backend::db::pool_from_url(&url).await?;
            backend::db::run_migrations(&migration_pool).await?;
            if data_encryption_enabled {
                let report =
                    backend::db::secret_migration::rewrap_stored_secrets(&migration_pool).await?;
                tracing::info!(
                    sso_client_secrets_scanned = report.sso_client_secrets_scanned,
                    sso_client_secrets = report.sso_client_secrets,
                    mfa_totp_secrets_scanned = report.mfa_totp_secrets_scanned,
                    mfa_totp_secrets = report.mfa_totp_secrets,
                    "application secrets rewrapped under the current data key"
                );
            }
            migration_pool.close().await;
            backend::db::pool_from_env().await?
        }
        None if is_production => {
            anyhow::bail!(
                "MIGRATION_DATABASE_URL must be set in production to an owner-level Postgres \
                 role; DATABASE_URL is reserved for the least-privileged runtime role"
            );
        }
        None => {
            tracing::warn!(
                "MIGRATION_DATABASE_URL unset outside production; running migrations through \
                 DATABASE_URL. Set a separate owner URL to mirror production locally."
            );
            let pool = backend::db::pool_from_env().await?;
            backend::db::run_migrations(&pool).await?;
            if data_encryption_enabled {
                let report = backend::db::secret_migration::rewrap_stored_secrets(&pool).await?;
                tracing::info!(
                    sso_client_secrets_scanned = report.sso_client_secrets_scanned,
                    sso_client_secrets = report.sso_client_secrets,
                    mfa_totp_secrets_scanned = report.mfa_totp_secrets_scanned,
                    mfa_totp_secrets = report.mfa_totp_secrets,
                    "application secrets rewrapped under the current data key"
                );
            }
            pool
        }
    };

    // Production must use the dedicated runtime role. Besides RLS, privileged
    // ownership triggers distinguish the table owner from the runtime invoker;
    // accepting an owner-level connection here would therefore bypass both
    // tenant isolation and those mutation boundaries.
    let (runtime_db_role, runtime_session_role): (String, String) =
        sqlx::query_as("SELECT current_user::text, session_user::text")
            .fetch_one(&pool)
            .await?;
    let rls_bypassed = backend::db::warn_if_bypassing_rls(&pool).await?;
    if is_production
        && (runtime_db_role != "aulalite_app" || runtime_session_role != "aulalite_app")
    {
        anyhow::bail!(
            "Refusing to start in production: DATABASE_URL has current_user \
             `{runtime_db_role}` and session_user `{runtime_session_role}`; both must be the \
             exact `aulalite_app` runtime role so SET ROLE cannot hide owner privileges and \
             schema-owner access cannot reach request traffic."
        );
    }
    if is_production && rls_bypassed {
        anyhow::bail!(
            "Refusing to start in production: the `aulalite_app` role has SUPERUSER or \
             BYPASSRLS privileges. Restore it to NOSUPERUSER NOBYPASSRLS before starting."
        );
    }

    let project_id = std::env::var("FIREBASE_PROJECT_ID")?;
    let issuer = std::env::var("FIREBASE_TOKEN_ISSUER")?;
    let jwks_url = std::env::var("FIREBASE_JWKS_URL")?;
    let jwks = JwksCache::new(jwks_url, Duration::from_secs(3600));
    let verifier = Arc::new(Verifier::new(jwks, project_id, issuer));

    let firebase_web_api_key = std::env::var("FIREBASE_WEB_API_KEY")?;
    let app_origin = std::env::var("APP_ORIGIN")?;
    let api_origin = std::env::var("API_ORIGIN").unwrap_or_else(|_| app_origin.clone());
    let admin_origin = std::env::var("AULALITE_ADMIN_ORIGIN")
        .ok()
        .filter(|origin| !origin.trim().is_empty());
    let browser_origins = backend::parse_browser_origins(
        &std::env::var("AULALITE_BROWSER_ORIGINS").unwrap_or_else(|_| app_origin.clone()),
    )
    .map_err(anyhow::Error::msg)?;
    let super_admins =
        SuperAdminConfig::parse(&std::env::var("AULALITE_SUPER_ADMIN_EMAILS").unwrap_or_default())
            .map_err(anyhow::Error::msg)?;
    if is_production {
        let admin_origin = admin_origin
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("AULALITE_ADMIN_ORIGIN is required in production"))?;
        for (name, origin) in [
            ("APP_ORIGIN", &app_origin),
            ("API_ORIGIN", &api_origin),
            ("AULALITE_ADMIN_ORIGIN", admin_origin),
        ] {
            let parsed = reqwest::Url::parse(origin).map_err(|error| {
                anyhow::anyhow!("{name} must be an absolute HTTPS origin: {error}")
            })?;
            let canonical_origin = parsed.scheme() == "https"
                && parsed.host_str().is_some()
                && parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed.path() == "/"
                && parsed.query().is_none()
                && parsed.fragment().is_none();
            if !canonical_origin {
                anyhow::bail!(
                    "{name} must be a canonical HTTPS origin with no credentials, path, query, or fragment in production"
                );
            }
        }
        if browser_origins
            .iter()
            .any(|origin| !origin.starts_with("https://"))
        {
            anyhow::bail!("AULALITE_BROWSER_ORIGINS must contain only HTTPS origins in production");
        }
        if !browser_origins.contains(&app_origin) {
            anyhow::bail!("AULALITE_BROWSER_ORIGINS must include APP_ORIGIN");
        }
        if !browser_origins.contains(admin_origin) {
            anyhow::bail!("AULALITE_BROWSER_ORIGINS must include AULALITE_ADMIN_ORIGIN");
        }
        if super_admins.is_empty() {
            anyhow::bail!(
                "AULALITE_SUPER_ADMIN_EMAILS must name at least one verified global identity in production"
            );
        }
    }
    let promoted_super_admins = super_admins.promote_existing(&pool).await?;
    tracing::info!(
        configured = super_admins.len(),
        promoted = promoted_super_admins,
        "environment super-admin bootstrap reconciled"
    );
    let email_link_sender: Arc<dyn backend::services::invitations::EmailLinkSender> = Arc::new(
        backend::services::invitations::FirebaseEmailLinkSender::new(firebase_web_api_key),
    );

    // Use separate object-store endpoints: server-side traffic stays on the
    // private Compose network, while presigned URLs must contain the public
    // browser-reachable host because that host is covered by the signature.
    let s3_endpoint_url = std::env::var("S3_ENDPOINT_URL")?;
    let s3_internal_endpoint_url =
        std::env::var("S3_INTERNAL_ENDPOINT_URL").unwrap_or_else(|_| s3_endpoint_url.clone());
    let s3_region = std::env::var("S3_REGION")?;
    let bucket_name = std::env::var("S3_BUCKET")?;
    let aws_access_key_id = std::env::var("AWS_ACCESS_KEY_ID")?;
    let aws_secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY")?;

    // In production, refuse to boot with the shipped default object-store
    // credentials. AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY double as RustFS's
    // root credentials (see docker-compose.yml), so leaving the .env.example
    // defaults (`aulalite` / `changeme123`) in place would expose the object
    // store. Mirrors the JWT / MediaMTX / Stripe production guards.
    if is_production
        && (aws_access_key_id.trim().is_empty()
            || aws_access_key_id.trim() == "aulalite"
            || aws_secret_access_key.trim().is_empty()
            || aws_secret_access_key.trim() == "changeme123")
    {
        anyhow::bail!(
            "AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY must be set to non-default \
             values in production (these are also RustFS's root credentials)"
        );
    }
    if is_production {
        let parsed = reqwest::Url::parse(&s3_endpoint_url).map_err(|error| {
            anyhow::anyhow!("S3_ENDPOINT_URL must be an absolute public HTTPS origin: {error}")
        })?;
        let public_storage_origin = parsed.scheme() == "https"
            && parsed.host_str().is_some()
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.path() == "/"
            && parsed.query().is_none()
            && parsed.fragment().is_none();
        if !public_storage_origin {
            anyhow::bail!(
                "S3_ENDPOINT_URL must be a canonical public HTTPS origin with no credentials, path, query, or fragment in production"
            );
        }
    }

    let storage: Arc<dyn backend::storage::S3Client> =
        Arc::new(backend::storage::object_store::S3CompatClient::new(
            backend::storage::object_store::S3CompatConfig {
                endpoint_url: s3_internal_endpoint_url,
                public_endpoint_url: s3_endpoint_url,
                region: s3_region,
                bucket: bucket_name.clone(),
                access_key_id: aws_access_key_id,
                secret_access_key: aws_secret_access_key,
                browser_origins: browser_origins.clone(),
            },
        ));
    // The object store (RustFS) may not accept connections the instant the
    // backend starts — `docker compose` gates this service on `service_started`,
    // not a healthcheck — so retry bucket bootstrap with a fixed backoff rather
    // than crashing on a cold `up`. ~60s total (30 × 2s) before giving up.
    {
        let mut attempt: u32 = 0;
        loop {
            match storage.ensure_bucket(&bucket_name).await {
                Ok(()) => break,
                Err(e) if attempt < 30 => {
                    attempt += 1;
                    tracing::warn!(
                        ?e,
                        attempt,
                        "object storage not ready; retrying bucket bootstrap in 2s"
                    );
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
    tracing::info!(%bucket_name, "object storage bucket ready");

    let mediamtx_http_url =
        std::env::var("MEDIAMTX_HTTP_URL").unwrap_or_else(|_| "http://mediamtx:9997".into());
    let mediamtx_public_webrtc_url = std::env::var("MEDIAMTX_PUBLIC_WEBRTC_URL")
        .unwrap_or_else(|_| "http://localhost:8889".into());
    let mediamtx_public_hls_url =
        std::env::var("MEDIAMTX_PUBLIC_HLS_URL").unwrap_or_else(|_| "http://localhost:8888".into());
    if is_production {
        for (name, value) in [
            ("MEDIAMTX_PUBLIC_WEBRTC_URL", &mediamtx_public_webrtc_url),
            ("MEDIAMTX_PUBLIC_HLS_URL", &mediamtx_public_hls_url),
        ] {
            let parsed = reqwest::Url::parse(value).map_err(|error| {
                anyhow::anyhow!("{name} must be an absolute HTTPS URL: {error}")
            })?;
            if parsed.scheme() != "https"
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
            {
                anyhow::bail!(
                    "{name} must use HTTPS with a host and no credentials, query, or fragment in production"
                );
            }
        }
    }
    let mediamtx: Arc<dyn backend::services::mediamtx::MediaMtxClient> = Arc::new(
        backend::services::mediamtx::HttpMediaMtxClient::new(mediamtx_http_url),
    );
    let jwt_signer = match std::env::var("JWT_RS256_PRIVATE_KEY_PEM") {
        Ok(pem) if !pem.trim().is_empty() => {
            Arc::new(backend::services::mediamtx::JwtSigner::from_pem(&pem)?)
        }
        _ if is_production => {
            anyhow::bail!(
                "JWT_RS256_PRIVATE_KEY_PEM must be set in production; refusing to start with ephemeral viewer-JWT keypair"
            );
        }
        _ => {
            tracing::warn!(
                "JWT_RS256_PRIVATE_KEY_PEM unset; generating ephemeral keypair. \
                 Viewer JWTs (live + recorded video) become invalid on next restart, \
                 so any in-flight playback session will fail. NON-PRODUCTION ONLY — \
                 set a persistent PEM in any environment that needs token continuity \
                 across restarts."
            );
            Arc::new(backend::services::mediamtx::JwtSigner::new_ephemeral())
        }
    };

    // MediaMTX → backend shared-secret. The auth-publish callback always
    // requires the header `X-MediaMTX-Auth-Shared` or query `?shared=` to
    // match this secret. Reject the well-known default `changeme-…` value.
    //
    // - In production: missing/default secret is a hard error.
    // - Outside production: missing/default secret triggers an in-process
    //   random secret. The auth check is then enforced as in prod, but
    //   the secret rotates on every restart, so operators must keep
    //   MediaMTX's `serverApiKey` in sync (or set MEDIAMTX_AUTH_SHARED_HEADER
    //   explicitly for stable local runs).
    let mediamtx_auth_shared_secret: Option<String> =
        match std::env::var("MEDIAMTX_AUTH_SHARED_HEADER")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != "changeme-mediamtx-auth-shared-secret")
        {
            Some(s) => Some(s),
            None if is_production => {
                anyhow::bail!(
                    "MEDIAMTX_AUTH_SHARED_HEADER must be set to a non-default value in production"
                );
            }
            None => {
                use base64::Engine;
                use rand::RngCore;
                let mut buf = [0u8; 24];
                rand::rng().fill_bytes(&mut buf);
                let generated = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf);
                tracing::warn!(
                    "MEDIAMTX_AUTH_SHARED_HEADER unset/default; generated an ephemeral \
                 random secret for this process. MediaMTX must be configured with \
                 the same value (set MEDIAMTX_AUTH_SHARED_HEADER for a stable secret). \
                 NON-PRODUCTION ONLY."
                );
                Some(generated)
            }
        };

    // Stripe billing. Mirrors the JWT_RS256 / MEDIAMTX_AUTH_SHARED_HEADER
    // idiom: in production a missing secret key is a hard error; outside
    // production we fall back to the MockStripeClient so billing compiles and
    // runs without live Stripe keys. The webhook signing secret is likewise
    // required in production and left None (with a warn) otherwise.
    let stripe: Arc<dyn backend::services::billing::StripeClient> = match std::env::var(
        "STRIPE_SECRET_KEY",
    ) {
        Ok(key) if !key.trim().is_empty() => {
            Arc::new(backend::services::billing::HttpStripeClient::new(key))
        }
        _ if is_production => {
            anyhow::bail!(
                    "STRIPE_SECRET_KEY must be set in production; refusing to start without a live Stripe client"
                );
        }
        _ => {
            tracing::warn!(
                "STRIPE_SECRET_KEY unset; using MockStripeClient. Checkout/customer \
                     calls return deterministic fakes. NON-PRODUCTION ONLY."
            );
            Arc::new(backend::services::billing::MockStripeClient::new())
        }
    };

    let stripe_webhook_secret: Option<String> = match std::env::var("STRIPE_WEBHOOK_SECRET")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        Some(s) => Some(s),
        None if is_production => {
            anyhow::bail!(
                "STRIPE_WEBHOOK_SECRET must be set in production; refusing to start without a webhook signing secret"
            );
        }
        None => {
            tracing::warn!(
                "STRIPE_WEBHOOK_SECRET unset; webhook signature verification is unconfigured. \
                 NON-PRODUCTION ONLY."
            );
            None
        }
    };

    // Checkout success/cancel URLs. Prefer explicit env; otherwise derive from
    // APP_ORIGIN so a single origin var is enough for local/dev runs.
    let stripe_success_url = std::env::var("STRIPE_SUCCESS_URL").unwrap_or_else(|_| {
        format!(
            "{}/admin/billing?status=success",
            app_origin.trim_end_matches('/')
        )
    });
    let stripe_cancel_url = std::env::var("STRIPE_CANCEL_URL").unwrap_or_else(|_| {
        format!(
            "{}/admin/billing?status=cancel",
            app_origin.trim_end_matches('/')
        )
    });
    if is_production {
        let app_url = reqwest::Url::parse(&app_origin)?;
        for (name, value) in [
            ("STRIPE_SUCCESS_URL", &stripe_success_url),
            ("STRIPE_CANCEL_URL", &stripe_cancel_url),
        ] {
            let parsed = reqwest::Url::parse(value)
                .map_err(|error| anyhow::anyhow!("{name} must be an absolute URL: {error}"))?;
            if parsed.origin() != app_url.origin()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.fragment().is_some()
            {
                anyhow::bail!(
                    "{name} must be a same-origin APP_ORIGIN URL with no credentials or fragment in production"
                );
            }
        }
    }
    tracing::debug!(%stripe_success_url, %stripe_cancel_url, "stripe checkout return urls configured");

    // Transactional email is part of the account/invitation lifecycle, not an
    // optional cosmetic channel. Production therefore fails closed instead of
    // acknowledging invitations that can never reach their recipient.
    let email_notifier: Arc<dyn backend::services::notifications::EmailNotifier> =
        match std::env::var("RESEND_API_KEY") {
            Ok(key) if !key.trim().is_empty() => {
                let from = std::env::var("RESEND_FROM")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());
                let from = match from {
                    Some(from) => from,
                    None if is_production => anyhow::bail!(
                        "RESEND_FROM must be a verified sender identity in production"
                    ),
                    None => "AulaLite <notifications@aulalite.app>".into(),
                };
                Arc::new(backend::services::notifications::ResendEmailNotifier::new(
                    key, from,
                ))
            }
            _ if is_production => {
                anyhow::bail!(
                    "RESEND_API_KEY must be set in production; transactional invitations and account email cannot use a log-only mock"
                );
            }
            _ => {
                tracing::warn!(
                    "RESEND_API_KEY unset; using MockEmailNotifier. Email notifications \
                     are logged-only. NON-PRODUCTION ONLY."
                );
                Arc::new(backend::services::notifications::mock::MockEmailNotifier::new())
            }
        };

    // Notification push sender (FCM HTTP v1). Missing or invalid provider
    // config does not fail boot; attempts are recorded as skipped.
    let push_sender: Arc<dyn backend::services::notifications::PushSender> =
        match backend::services::notifications::FcmProviderConfig::from_env() {
            Ok(Some(cfg)) => {
                tracing::info!(project_id = %cfg.project_id, "FCM push sender configured");
                Arc::new(backend::services::notifications::FcmPushSender::from_config(cfg))
            }
            Ok(None) => {
                tracing::warn!(
                    "FCM push is not fully configured; using DisabledPushSender. \
                     Push notification attempts will be recorded as skipped."
                );
                Arc::new(backend::services::notifications::DisabledPushSender)
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "FCM push configuration is invalid; using DisabledPushSender. \
                     Push notification attempts will be recorded as skipped."
                );
                Arc::new(backend::services::notifications::DisabledPushSender)
            }
        };

    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://redis:6379".into());
    let live_room: Arc<dyn backend::services::live_room::LiveRoomBroker> = Arc::new(
        backend::services::live_room_redis::RedisLiveRoomBroker::connect(&redis_url)
            .await
            .map_err(|e| anyhow::anyhow!("live_room broker connect: {e}"))?,
    );

    let recorder: Arc<dyn backend::services::recording::RecorderTool> =
        Arc::new(backend::services::recording::RealFfmpegRecorder::new());
    let recordings_dir = std::env::var("RECORDINGS_DIR").unwrap_or_else(|_| "/recordings".into());
    let local_login = LocalLoginConfig::from_env();
    if local_login.is_enabled() {
        tracing::warn!(
            email = %local_login.email,
            "local login bypass enabled; do not enable this in production"
        );
    }

    // Auto-end overdue live sessions every 60s.
    let pool_for_sweep = pool.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            match backend::db::live_sessions::sweep_auto_end(&pool_for_sweep).await {
                Ok(ended) if !ended.is_empty() => {
                    tracing::info!(count = ended.len(), "auto-ended overdue sessions");
                    // Reconcile attendance for each auto-ended session: close any
                    // rows still open (dropped sockets) so their final segment is
                    // counted. `sweep_auto_end` sets actual_ended_at = now(), so we
                    // finalize up to now() (passing None). Best-effort per session.
                    for (session_id, _tenant_id) in ended {
                        if let Err(e) = backend::db::attendance::finalize_open_for_session(
                            &pool_for_sweep,
                            session_id,
                            None,
                        )
                        .await
                        {
                            tracing::warn!(?e, %session_id, "attendance finalize on sweep failed");
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(?e, "sweep_auto_end failed"),
            }
        }
    });

    // Daily live-room chat prune (default 90 days).
    let pool_for_prune = pool.clone();
    let retention_days: i64 = std::env::var("LIVE_ROOM_CHAT_RETENTION_DAYS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(90);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 3600));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            match backend::db::live_room::prune_older_than(&pool_for_prune, retention_days).await {
                Ok(n) if n > 0 => {
                    tracing::info!(rows = n, days = retention_days, "live_room chat prune")
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(?e, "live_room chat prune failed"),
            }
        }
    });

    // Recording sweep: every 60s, find ended sessions needing recording and
    // process a small batch sequentially per tick. The batch (vs the old limit
    // of 1) drains a burst of simultaneously-ended classes far faster — N
    // sessions clear in ~N/batch minutes instead of ~N minutes — while staying
    // sequential so we never run several ffmpeg remuxes concurrently and starve
    // the box. (For higher throughput, switch to a bounded JoinSet here.)
    // Also reset rows that have been stuck in `remuxing`/`uploading` past the
    // grace period — process_one_session never reaches its terminal state
    // when killed mid-pipeline (OOM/restart), and without this they'd sit
    // forever.
    const RECORDING_SWEEP_BATCH: i64 = 3;
    let pool_for_recording = pool.clone();
    let storage_for_recording = storage.clone();
    let bucket_for_recording = bucket_name.clone();
    let recorder_for_recording = recorder.clone();
    let recordings_dir_for_recording = recordings_dir.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;

            // Reclaim orphaned in-flight rows (stuck > 15 min). These get
            // marked `failed` so the operator/UI can see them — manually
            // retried via POST /v1/sessions/:id/recording/retry.
            let stuck_after = chrono::Duration::minutes(15);
            match backend::db::recordings::list_orphaned_in_progress(
                &pool_for_recording,
                stuck_after,
                10,
            )
            .await
            {
                Ok(orphans) if !orphans.is_empty() => {
                    tracing::warn!(count = orphans.len(), "reclaiming stuck recordings");
                    for row in orphans {
                        backend::services::recording::mark_orphan_failed(
                            &pool_for_recording,
                            row.id,
                            row.tenant_id,
                            &format!(
                                "stuck in {} for > {} min; reclaimed by sweep",
                                row.processing_status,
                                stuck_after.num_minutes()
                            ),
                        )
                        .await;
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(?e, "list_orphaned_in_progress failed"),
            }

            match backend::db::recordings::list_ended_sessions_needing_recording(
                &pool_for_recording,
                RECORDING_SWEEP_BATCH,
            )
            .await
            {
                Ok(rows) => {
                    for (session_id, tenant_id, started, ended) in rows {
                        let recordings_dir_buf =
                            std::path::PathBuf::from(&recordings_dir_for_recording);
                        match backend::services::recording::process_one_session(
                            &pool_for_recording,
                            storage_for_recording.as_ref(),
                            &bucket_for_recording,
                            recorder_for_recording.as_ref(),
                            &recordings_dir_buf,
                            tenant_id,
                            session_id,
                            started,
                            ended,
                        )
                        .await
                        {
                            Ok(Some(rec_id)) => {
                                tracing::info!(?rec_id, ?session_id, "recording processed")
                            }
                            Ok(None) => {}
                            Err(e) => {
                                tracing::warn!(?e, ?session_id, "recording processing failed")
                            }
                        }
                    }
                }
                Err(e) => tracing::warn!(?e, "list_ended_sessions_needing_recording failed"),
            }
        }
    });

    // Recording retention janitor — every 24h.
    let pool_for_retention = pool.clone();
    let storage_for_retention = storage.clone();
    let retention_days: i64 = std::env::var("LIVE_ROOM_RECORDING_RETENTION_DAYS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(365);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 3600));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            match backend::services::recording::run_retention_janitor(
                &pool_for_retention,
                storage_for_retention.as_ref(),
                retention_days,
                100,
            )
            .await
            {
                Ok(n) if n > 0 => {
                    tracing::info!(rows = n, days = retention_days, "recording retention prune")
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(?e, "recording retention janitor failed"),
            }
        }
    });

    // Calendar reminders: every ~15 min, notify enrolled students about
    // sessions / assignments due in the next ~24h (deduped via reminders_sent).
    {
        let pool_for_reminders = pool.clone();
        let email_for_reminders = email_notifier.clone();
        let push_for_reminders = push_sender.clone();
        let origin_for_reminders = app_origin.clone();
        tokio::spawn(async move {
            backend::handlers::calendar::run_reminder_loop(
                pool_for_reminders,
                email_for_reminders,
                push_for_reminders,
                origin_for_reminders,
                std::time::Duration::from_secs(15 * 60),
            )
            .await;
        });
    }

    // Outbound webhook delivery worker: every 15s, claim due deliveries
    // (cross-tenant via system context), sign + POST them, record attempts with
    // capped exponential backoff (1m/5m/15m/1h/6h, 6 attempts).
    {
        let pool_for_webhooks = pool.clone();
        tokio::spawn(async move {
            backend::services::webhook_delivery::run_delivery_worker(
                pool_for_webhooks,
                std::time::Duration::from_secs(15),
            )
            .await;
        });
    }

    // Opt-in API rate limiter. Constructed only when AULALITE_RATE_LIMIT=on;
    // otherwise None and no middleware is installed (dev/tests unaffected). If
    // the operator opted in but Redis can't be reached, refuse to run
    // unprotected rather than silently disable it.
    let rate_limit_config = backend::services::rate_limit::RateLimitConfig::from_env();
    if is_production && rate_limit_config.is_none() {
        anyhow::bail!(
            "AULALITE_RATE_LIMIT must be enabled in production; authentication and public callback edges cannot run without abuse controls"
        );
    }
    let rate_limit = match rate_limit_config {
        Some(cfg) => {
            match backend::services::rate_limit::RateLimitState::connect(&redis_url, cfg).await {
                Ok(rl) => {
                    tracing::info!("API rate limiting ENABLED (AULALITE_RATE_LIMIT=on)");
                    Some(rl)
                }
                Err(e) => return Err(anyhow::anyhow!("rate limiter connect failed: {e}")),
            }
        }
        None => None,
    };

    let app = backend::router(AppState {
        pool,
        verifier,
        email_link_sender,
        app_origin,
        api_origin,
        browser_origins,
        super_admins,
        storage,
        bucket_name,
        mediamtx,
        jwt_signer,
        mediamtx_public_webrtc_url,
        mediamtx_public_hls_url,
        live_room,
        recorder,
        recordings_dir,
        local_login,
        mediamtx_auth_shared_secret,
        stripe,
        stripe_webhook_secret,
        email_notifier,
        push_sender,
        rate_limit,
    });
    let addr: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "backend listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Resolves when the process receives SIGTERM (container stop / Dokploy redeploy)
/// or Ctrl-C, so axum stops accepting connections and drains in-flight HTTP
/// before exit instead of relying on the 10s SIGKILL fallback. Detached WS tasks
/// and the recording sweep are not tracked by axum's drain — they are torn down
/// on runtime drop — which is acceptable (clients reconnect; the orphan sweep
/// reclaims interrupted remuxes).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                tracing::warn!(?e, "failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!("shutdown signal received; draining in-flight requests");
}
