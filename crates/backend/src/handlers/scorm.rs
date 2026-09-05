// crates/backend/src/handlers/scorm.rs
//! SCORM 1.2 + 2004 runtime (Tool side).
//!
//! Flow:
//!   1. A SCORM `.zip` is uploaded via the standard presigned-upload path
//!      (`/v1/uploads/begin` with `purpose: "scorm"`, `linked_entity_type:
//!      "course"`) → produces an `available` `file_assets` row.
//!   2. Staff REGISTER it against the course (`POST /v1/courses/{cid}/scorm`),
//!      supplying the package title + the `imsmanifest.xml` text so we can parse
//!      the SCORM version + launch href. We store the metadata in
//!      `scorm_packages`.
//!   3. The player resolves the launch URL (`GET /v1/scorm/{id}/launch`) — a
//!      presigned base + the manifest launch href — and loads it in an iframe.
//!      The JS bridge (`/assets/scorm-bridge.js`) exposes `window.API` /
//!      `window.API_1484_11` and round-trips CMI through
//!      `GET/PUT /v1/scorm/{id}/cmi`.
//!
//! Manifest parsing is a HAND-ROLLED minimal extractor (no XML/zip dep): we read
//! the SCORM version off the `schemaversion`/`<metadata>` and the launch href off
//! the first `<resource ... href="...">`. This keeps the dependency tree light;
//! a full pull-parser is a clean follow-up if richer manifests need it.
//!
//! All endpoints are AUTHED (require_auth router). Staff gate on register/delete;
//! read gate on launch + CMI load/save (the learner must be able to read the
//! course). CMI is per `(package, caller)`.
use axum::extract::{Extension, Path, State};
use axum::{routing, Json, Router};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

/// Presigned launch URL TTL — long enough for a learner to work through the
/// activity, short enough to stay a transient credential.
const LAUNCH_TTL: Duration = Duration::from_secs(60 * 60);
const MAX_TITLE_LEN: usize = 200;
const MAX_MANIFEST_LEN: usize = 1024 * 1024; // 1 MiB guard on the pasted manifest

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/courses/{cid}/scorm",
            routing::get(list_packages).post(register_package),
        )
        .route("/v1/scorm/{id}", routing::delete(delete_package))
        .route("/v1/scorm/{id}/launch", routing::get(launch))
        .route("/v1/scorm/{id}/cmi", routing::get(load_cmi).put(save_cmi))
}

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

fn can_author_scorm(ctx: &RequestContext) -> bool {
    ctx.can_teach()
}

async fn require_course_staff(
    s: &AppState,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    // Package registration/deletion is course authoring. A TA may assist and
    // grade, but does not hold the Teach capability needed to alter content.
    if !can_author_scorm(ctx) {
        return Err(ApiError::Forbidden);
    }
    if ctx.can_manage_organization() {
        return Ok(());
    }
    if !db::courses::caller_can_staff_course(
        &s.pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

async fn require_course_read(
    s: &AppState,
    ctx: &RequestContext,
    course_id: Uuid,
) -> Result<(), ApiError> {
    if ctx.can_manage_organization() {
        return Ok(());
    }
    if !db::courses::caller_can_read_course(
        &s.pool,
        course_id,
        ctx.user_id,
        ctx.tenant_id,
        is_org_admin(ctx),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct ScormPackageDto {
    pub id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub asset_id: Uuid,
    pub scorm_version: String,
    pub launch_href: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::scorm::ScormPackageRow> for ScormPackageDto {
    fn from(r: db::scorm::ScormPackageRow) -> Self {
        Self {
            id: r.id,
            course_id: r.course_id,
            title: r.title,
            asset_id: r.asset_id,
            scorm_version: r.scorm_version,
            launch_href: r.launch_href,
            created_at: r.created_at,
        }
    }
}

#[derive(Deserialize)]
pub struct RegisterPackage {
    pub title: String,
    /// The `file_assets.id` of the uploaded `.zip` (purpose "scorm").
    pub asset_id: Uuid,
    /// The `imsmanifest.xml` text, read from the package root. We parse the
    /// SCORM version + launch href from it.
    pub manifest_xml: String,
    /// Optional explicit launch href override (when the uploader already
    /// resolved it client-side); falls back to the manifest's first resource.
    #[serde(default)]
    pub launch_href: Option<String>,
}

#[derive(Serialize)]
pub struct LaunchDto {
    /// Presigned base URL for the extracted package root (the player resolves
    /// `launch_href` against this).
    pub base_url: String,
    pub launch_href: String,
    pub scorm_version: String,
    /// Full launch URL convenience = base + href when both are simple.
    pub launch_url: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_packages(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
) -> Result<Json<Vec<ScormPackageDto>>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    require_course_read(&s, &ctx, cid).await?;
    let rows = db::scorm::list_for_course(&s.pool, tenant, cid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(rows.into_iter().map(ScormPackageDto::from).collect()))
}

async fn register_package(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(cid): Path<Uuid>,
    Json(body): Json<RegisterPackage>,
) -> Result<Json<ScormPackageDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    require_course_staff(&s, &ctx, cid).await?;

    let title = body.title.trim();
    if title.is_empty() {
        return Err(ApiError::Validation("title_required".into()));
    }
    if title.chars().count() > MAX_TITLE_LEN {
        return Err(ApiError::Validation("title_too_long".into()));
    }
    if body.manifest_xml.len() > MAX_MANIFEST_LEN {
        return Err(ApiError::Validation("manifest_too_large".into()));
    }

    // The uploaded asset must exist, be available, belong to this tenant, and be
    // linked to this course (it was uploaded with purpose "scorm" against the
    // course). We verify via the tenant-scoped fetch under RLS.
    let mut tx = db::begin_with_context(&s.pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let asset = db::file_assets::fetch(&mut *tx, body.asset_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::FileAssetNotFound)?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if asset.tenant_id != tenant || asset.status != "available" {
        return Err(ApiError::FileAssetNotFound);
    }
    if asset.linked_entity_type.as_deref() != Some("course") || asset.linked_entity_id != Some(cid)
    {
        return Err(ApiError::BadRequest(
            "asset is not a SCORM package for this course".into(),
        ));
    }

    let parsed = parse_manifest(&body.manifest_xml)
        .ok_or_else(|| ApiError::BadRequest("could not parse imsmanifest.xml".into()))?;
    let launch_href = body
        .launch_href
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or(parsed.launch_href);

    let row = db::scorm::insert(
        &s.pool,
        tenant,
        cid,
        title,
        body.asset_id,
        &parsed.scorm_version,
        &launch_href,
        ctx.user_id,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(ScormPackageDto::from(row)))
}

async fn delete_package(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let pkg = db::scorm::fetch(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    require_course_staff(&s, &ctx, pkg.course_id).await?;
    let deleted = db::scorm::delete(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !deleted {
        return Err(ApiError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn launch(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<LaunchDto>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let pkg = db::scorm::fetch(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    require_course_read(&s, &ctx, pkg.course_id).await?;

    // Resolve the uploaded package object key, then presign a GET base. The
    // extracted-package serving model: the object key is the package root prefix;
    // we presign the launch object directly (base = object key dir).
    let mut tx = db::begin_with_context(&s.pool, ctx.user_id, ctx.tenant_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let asset = db::file_assets::fetch(&mut *tx, pkg.asset_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::FileAssetNotFound)?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Base = the package's extracted prefix. We derive it from the asset object
    // key (strip the .zip filename, append the package id as the extraction dir).
    let base_key = extracted_base_key(&asset.object_key, pkg.id);
    let launch_object = join_key(&base_key, &pkg.launch_href);
    let launch_url = s
        .storage
        .presigned_get_url(&launch_object, LAUNCH_TTL)
        .await
        .map_err(|e| ApiError::Internal(format!("presign launch failed: {e}")))?;
    let base_url = s
        .storage
        .presigned_get_url(&base_key, LAUNCH_TTL)
        .await
        .map_err(|e| ApiError::Internal(format!("presign base failed: {e}")))?;

    Ok(Json(LaunchDto {
        base_url,
        launch_href: pkg.launch_href,
        scorm_version: pkg.scorm_version,
        launch_url,
    }))
}

async fn load_cmi(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    let pkg = db::scorm::fetch(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    require_course_read(&s, &ctx, pkg.course_id).await?;
    let blob = db::scorm::load_cmi(&s.pool, tenant, id, ctx.user_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| serde_json::json!({}));
    Ok(Json(blob))
}

async fn save_cmi(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Result<axum::http::StatusCode, ApiError> {
    let tenant = ctx.tenant_id.ok_or(ApiError::Forbidden)?;
    if !body.is_object() {
        return Err(ApiError::BadRequest("cmi must be a JSON object".into()));
    }
    let pkg = db::scorm::fetch(&s.pool, tenant, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or(ApiError::NotFound)?;
    require_course_read(&s, &ctx, pkg.course_id).await?;
    db::scorm::save_cmi(&s.pool, tenant, id, ctx.user_id, &body)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Manifest parsing (hand-rolled, no XML/zip dep)
// ---------------------------------------------------------------------------

/// What we extract from `imsmanifest.xml`.
struct ParsedManifest {
    /// "1.2" or "2004".
    scorm_version: String,
    /// Launch href relative to the package root.
    launch_href: String,
}

/// Parse the SCORM version + launch href out of an `imsmanifest.xml`. Returns
/// None when no resource href can be found (a manifest with no launchable
/// resource is not registrable).
fn parse_manifest(xml: &str) -> Option<ParsedManifest> {
    let scorm_version = detect_version(xml);
    let launch_href = first_resource_href(xml)?;
    Some(ParsedManifest {
        scorm_version,
        launch_href,
    })
}

/// SCORM 2004 manifests carry `schemaversion` values like "2004 3rd Edition" /
/// "CAM 1.3"; SCORM 1.2 carries `schemaversion="1.2"`. Default to "1.2" when
/// ambiguous (the most permissive runtime for legacy content).
fn detect_version(xml: &str) -> String {
    let lower = xml.to_ascii_lowercase();
    // Explicit `schemaversion` is authoritative when present.
    if let Some(v) = attr_value(&lower, "schemaversion") {
        if v.contains("2004") || v.contains("1.3") {
            return "2004".to_string();
        }
        if v.contains("1.2") {
            return "1.2".to_string();
        }
    }
    // Otherwise sniff well-known 2004 markers; default to 1.2 (most permissive).
    if lower.contains("2004") || lower.contains("cam 1.3") {
        return "2004".to_string();
    }
    "1.2".to_string()
}

/// Find the `href` of the first `<resource ...>` element (the launch resource).
/// Prefers a resource whose `adlcp:scormtype`/`scormType` is "sco", falling back
/// to the first resource with any href.
fn first_resource_href(xml: &str) -> Option<String> {
    let mut fallback: Option<String> = None;
    let mut search_from = 0usize;
    while let Some(rel) = xml[search_from..].find("<resource") {
        let start = search_from + rel;
        // Slice the opening tag up to the next '>'.
        let tag_end = xml[start..].find('>').map(|e| start + e)?;
        let tag = &xml[start..=tag_end];
        let href = attr_value(tag, "href");
        let is_sco = attr_value(&tag.to_ascii_lowercase(), "scormtype")
            .map(|v| v == "sco")
            .unwrap_or(false);
        if let Some(h) = href {
            let decoded = xml_unescape(&h);
            if is_sco {
                return Some(decoded);
            }
            if fallback.is_none() {
                fallback = Some(decoded);
            }
        }
        search_from = tag_end + 1;
    }
    fallback
}

/// Extract `name="value"` (single or double quotes) from a tag/string. Returns
/// the FIRST occurrence. Case-sensitive on `name`; callers lower-case as needed.
fn attr_value(haystack: &str, name: &str) -> Option<String> {
    let key_pos = haystack.find(name)?;
    let after = &haystack[key_pos + name.len()..];
    let after = after.trim_start();
    let after = after.strip_prefix('=')?;
    let after = after.trim_start();
    let quote = after.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &after[1..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

/// Minimal XML entity unescape for href values.
fn xml_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

/// Derive the extracted-package base object key from the uploaded `.zip` key and
/// the package id. The extraction worker (out of scope here) is expected to
/// unzip the package into `<zip-dir>/scorm-<pkgid>/`; we presign GETs under that
/// prefix. We keep this pure + deterministic so the player and the (future)
/// extraction worker agree on the layout.
fn extracted_base_key(zip_object_key: &str, package_id: Uuid) -> String {
    let dir = match zip_object_key.rfind('/') {
        Some(idx) => &zip_object_key[..idx],
        None => "",
    };
    if dir.is_empty() {
        format!("scorm-{}", package_id.simple())
    } else {
        format!("{dir}/scorm-{}", package_id.simple())
    }
}

/// Join a base key and a relative href into a single object key, normalizing
/// slashes and stripping any leading "./" or "/".
fn join_key(base: &str, href: &str) -> String {
    let href = href.trim_start_matches("./").trim_start_matches('/');
    // Drop any query/fragment for the object lookup (the player re-applies them
    // against base_url client-side).
    let href = href.split(['?', '#']).next().unwrap_or(href);
    if base.is_empty() {
        href.to_string()
    } else {
        format!("{}/{}", base.trim_end_matches('/'), href)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role_context(role: core_types::TenantRole) -> RequestContext {
        RequestContext {
            user_id: Uuid::nil(),
            firebase_uid: "test".into(),
            email: "test@example.test".into(),
            display_name: None,
            tenant_id: Some(Uuid::nil()),
            tenant_role: Some(role),
            is_platform_admin: false,
            identity_scope: crate::auth::identity::IdentityScope::Global,
        }
    }

    const SCORM_12: &str = r#"<?xml version="1.0"?>
        <manifest identifier="M1">
          <metadata><schema>ADL SCORM</schema><schemaversion>1.2</schemaversion></metadata>
          <organizations default="O"><organization identifier="O"><title>T</title></organization></organizations>
          <resources>
            <resource identifier="R1" type="webcontent" adlcp:scormtype="sco" href="index.html">
              <file href="index.html"/>
            </resource>
          </resources>
        </manifest>"#;

    const SCORM_2004: &str = r#"<?xml version="1.0"?>
        <manifest xmlns="http://www.imsglobal.org/xsd/imscp_v1p1">
          <metadata><schemaversion>2004 3rd Edition</schemaversion></metadata>
          <resources>
            <resource identifier="R" type="webcontent" adlcp:scormType="sco" href="shared/launch.html">
            </resource>
          </resources>
        </manifest>"#;

    #[test]
    fn parses_scorm_12() {
        let p = parse_manifest(SCORM_12).unwrap();
        assert_eq!(p.scorm_version, "1.2");
        assert_eq!(p.launch_href, "index.html");
    }

    #[test]
    fn parses_scorm_2004() {
        let p = parse_manifest(SCORM_2004).unwrap();
        assert_eq!(p.scorm_version, "2004");
        assert_eq!(p.launch_href, "shared/launch.html");
    }

    #[test]
    fn prefers_sco_resource_over_asset() {
        let xml = r#"<resources>
            <resource identifier="A" type="webcontent" href="common.js"></resource>
            <resource identifier="B" type="webcontent" adlcp:scormtype="sco" href="start.html"></resource>
          </resources>"#;
        assert_eq!(first_resource_href(xml).as_deref(), Some("start.html"));
    }

    #[test]
    fn falls_back_to_first_href_when_no_sco() {
        let xml = r#"<resource href="only.html"></resource>"#;
        assert_eq!(first_resource_href(xml).as_deref(), Some("only.html"));
    }

    #[test]
    fn returns_none_without_any_href() {
        assert!(parse_manifest("<manifest></manifest>").is_none());
    }

    #[test]
    fn attr_value_handles_both_quote_styles() {
        assert_eq!(attr_value(r#"a x="y""#, "x").as_deref(), Some("y"));
        assert_eq!(attr_value("a x='y'", "x").as_deref(), Some("y"));
    }

    #[test]
    fn unescapes_href_entities() {
        let xml = r#"<resource adlcp:scormtype="sco" href="a.html?x=1&amp;y=2"></resource>"#;
        assert_eq!(first_resource_href(xml).as_deref(), Some("a.html?x=1&y=2"));
    }

    #[test]
    fn join_key_normalizes() {
        assert_eq!(
            join_key("t/scorm-1", "./index.html"),
            "t/scorm-1/index.html"
        );
        assert_eq!(join_key("t/scorm-1", "/index.html"), "t/scorm-1/index.html");
        assert_eq!(
            join_key("t/scorm-1", "page.html?x=1#a"),
            "t/scorm-1/page.html"
        );
    }

    #[test]
    fn extracted_base_key_derives_from_zip() {
        let id = Uuid::nil();
        let key = extracted_base_key("tenant/2026/06/asset/pkg.zip", id);
        assert_eq!(key, format!("tenant/2026/06/asset/scorm-{}", id.simple()));
        let rootless = extracted_base_key("pkg.zip", id);
        assert_eq!(rootless, format!("scorm-{}", id.simple()));
    }

    #[test]
    fn scorm_authoring_requires_teach_not_assist() {
        assert!(can_author_scorm(&role_context(
            core_types::TenantRole::Teacher
        )));
        assert!(!can_author_scorm(&role_context(core_types::TenantRole::Ta)));
    }
}
