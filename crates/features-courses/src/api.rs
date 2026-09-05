// crates/features-courses/src/api.rs
//! Tiny HTTP helper. The Firebase ID token is read from a context provider
//! that the auth flow sets at sign-in. The base URL defaults to "" so
//! paths like "/v1/courses" hit same-origin (works for `dx serve` and Dokploy).

use serde::{de::DeserializeOwned, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub struct ApiContext {
    pub base_url: String,
    pub id_token: String,
}

pub const TENANT_HEADER: &str = "x-aulalite-tenant";
pub const WORKSPACE_STORAGE_KEY: &str = "aulalite.selected_workspace_id";

fn normalized_workspace_id(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() > 64 {
        return None;
    }
    uuid::Uuid::parse_str(value)
        .ok()
        .map(|id| id.hyphenated().to_string())
}

#[cfg(target_arch = "wasm32")]
pub fn selected_workspace_id() -> Option<String> {
    web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(WORKSPACE_STORAGE_KEY).ok().flatten())
        .and_then(|value| normalized_workspace_id(&value))
}

#[cfg(target_arch = "wasm32")]
pub fn set_selected_workspace_id(tenant_id: Option<&str>) {
    let Some(storage) = web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    else {
        return;
    };
    match tenant_id.and_then(normalized_workspace_id) {
        Some(value) => {
            let _ = storage.set_item(WORKSPACE_STORAGE_KEY, &value);
        }
        None => {
            let _ = storage.remove_item(WORKSPACE_STORAGE_KEY);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn selected_workspace_id() -> Option<String> {
    platform_bridge::native_preferences::selected_workspace_id()
        .ok()
        .flatten()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn set_selected_workspace_id(tenant_id: Option<&str>) {
    let _ = platform_bridge::native_preferences::set_selected_workspace_id(tenant_id);
}

#[cfg(target_arch = "wasm32")]
pub fn apply_workspace_header(headers: &web_sys::Headers) {
    if let Some(tenant_id) = selected_workspace_id() {
        let _ = headers.set(TENANT_HEADER, &tenant_id);
    }
}

#[derive(Debug)]
pub enum ApiError {
    Network(String),
    Status(u16, String),
    Decode(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Network(msg) => write!(f, "network: {msg}"),
            ApiError::Status(code, body) => write!(f, "status {code}: {body}"),
            ApiError::Decode(msg) => write!(f, "decode: {msg}"),
        }
    }
}

/// Callback the shell registers at boot. On 401, fetch_json calls this to
/// get a fresh token. If this returns Err the wrapper signs the user out.
pub type RefreshFn = std::sync::Arc<
    dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>>>>,
>;

thread_local! {
    static REFRESH: std::cell::RefCell<Option<RefreshFn>> = std::cell::RefCell::new(None);
}

pub fn set_refresh_fn(f: RefreshFn) {
    REFRESH.with(|r| *r.borrow_mut() = Some(f));
}

fn try_refresh() -> Option<RefreshFn> {
    REFRESH.with(|r| r.borrow().clone())
}

/// Acquire a fresh access token via the registered refresher. Used by the
/// live-room socket reconnect loop when the server closes the WebSocket with
/// `4001 AUTH_EXPIRED`. Returns `None` if no refresher is registered (e.g.
/// in tests/non-wasm) or the refresh failed.
#[cfg(target_arch = "wasm32")]
pub async fn refresh_access_token() -> Option<String> {
    let f = try_refresh()?;
    f().await.ok()
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn refresh_access_token() -> Option<String> {
    let f = try_refresh()?;
    f().await.ok()
}

/// Base URL for the backend on native (desktop/mobile) targets. The web build
/// uses `""` (same-origin), but native shells must hit an absolute URL. Runtime
/// environment wins over release-build configuration; debug Android defaults
/// to the emulator host alias while other debug targets use localhost.
#[cfg(not(target_arch = "wasm32"))]
pub fn native_api_base_url() -> String {
    platform_bridge::native::api_base_url()
}

#[cfg(target_arch = "wasm32")]
fn runtime_config_value(name: &str) -> String {
    js_sys::Reflect::get(&js_sys::global(), &wasm_bindgen::JsValue::from_str(name))
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_string()
}

/// Browser API origin generated into `runtime-config.js` by the web container.
/// Empty preserves same-origin behavior for `dx serve` and local tests.
#[cfg(target_arch = "wasm32")]
pub fn web_api_base_url() -> String {
    runtime_config_value("__AULALITE_API_BASE_URL__")
}

#[cfg(not(target_arch = "wasm32"))]
pub fn web_api_base_url() -> String {
    String::new()
}

/// True only when the current browser origin exactly matches the configured
/// admin control-plane origin.
#[cfg(target_arch = "wasm32")]
pub fn is_admin_host() -> bool {
    let configured = runtime_config_value("__AULALITE_ADMIN_ORIGIN__");
    !configured.is_empty()
        && web_sys::window()
            .and_then(|window| window.location().origin().ok())
            .is_some_and(|origin| origin.trim_end_matches('/') == configured)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn is_admin_host() -> bool {
    false
}

#[cfg(target_arch = "wasm32")]
mod web_impl {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    async fn do_fetch<T: DeserializeOwned>(
        cx: &ApiContext,
        method: &str,
        path: &str,
        body_str: Option<String>,
    ) -> Result<T, ApiError> {
        let window = web_sys::window().ok_or_else(|| ApiError::Network("no window".into()))?;
        let opts = web_sys::RequestInit::new();
        opts.set_method(method);
        if let Some(b) = body_str {
            opts.set_body(&b.into());
        }
        let url = format!("{}{}", cx.base_url, path);
        let req = web_sys::Request::new_with_str_and_init(&url, &opts)
            .map_err(|e| ApiError::Network(format!("{e:?}")))?;
        let headers = req.headers();
        let _ = headers.set("content-type", "application/json");
        if !cx.id_token.is_empty() {
            let _ = headers.set("authorization", &format!("Bearer {}", cx.id_token));
        }
        apply_workspace_header(&headers);
        let resp_value = JsFuture::from(window.fetch_with_request(&req))
            .await
            .map_err(|e| ApiError::Network(format!("{e:?}")))?;
        let resp: web_sys::Response = resp_value
            .dyn_into()
            .map_err(|_| ApiError::Network("not a Response".into()))?;
        let status = resp.status();
        let text = JsFuture::from(
            resp.text()
                .map_err(|e| ApiError::Network(format!("{e:?}")))?,
        )
        .await
        .map_err(|e| ApiError::Network(format!("{e:?}")))?
        .as_string()
        .unwrap_or_default();

        if !(200..300).contains(&status) {
            return Err(ApiError::Status(status as u16, text));
        }
        if text.is_empty() {
            serde_json::from_str("null").map_err(|e| ApiError::Decode(e.to_string()))
        } else {
            serde_json::from_str(&text).map_err(|e| ApiError::Decode(e.to_string()))
        }
    }

    pub async fn fetch_json<T: DeserializeOwned>(
        cx: &ApiContext,
        method: &str,
        path: &str,
        body: Option<&(impl Serialize + ?Sized)>,
    ) -> Result<T, ApiError> {
        let body_str = match body {
            Some(b) => Some(serde_json::to_string(b).map_err(|e| ApiError::Decode(e.to_string()))?),
            None => None,
        };

        // First attempt with whatever token ApiContext currently holds.
        match do_fetch::<T>(cx, method, path, body_str.clone()).await {
            Err(ApiError::Status(401, _)) => {
                // Try once to refresh the token and retry. If no refresher is
                // registered, surface the 401 as-is.
                let refresher = match super::try_refresh() {
                    Some(f) => f,
                    None => return Err(ApiError::Status(401, "unauthorized".into())),
                };
                let new_token = refresher().await.map_err(|e| ApiError::Status(401, e))?;
                let retried_cx = ApiContext {
                    base_url: cx.base_url.clone(),
                    id_token: new_token,
                };
                do_fetch::<T>(&retried_cx, method, path, body_str).await
            }
            other => other,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native_impl {
    use super::*;
    use std::sync::OnceLock;
    use std::time::Duration;

    const MAX_API_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

    fn api_client() -> Result<&'static reqwest::Client, ApiError> {
        static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
        CLIENT
            .get_or_init(|| {
                reqwest::Client::builder()
                    .connect_timeout(Duration::from_secs(8))
                    .timeout(Duration::from_secs(30))
                    .redirect(reqwest::redirect::Policy::none())
                    .user_agent(concat!("AulaLite/", env!("CARGO_PKG_VERSION")))
                    .build()
                    .map_err(|_| "native HTTP client could not be initialized".to_string())
            })
            .as_ref()
            .map_err(|message| ApiError::Network(message.clone()))
    }

    fn method_for(method: &str) -> reqwest::Method {
        match method {
            "GET" => reqwest::Method::GET,
            "POST" => reqwest::Method::POST,
            "PATCH" => reqwest::Method::PATCH,
            "PUT" => reqwest::Method::PUT,
            "DELETE" => reqwest::Method::DELETE,
            other => reqwest::Method::from_bytes(other.as_bytes()).unwrap_or(reqwest::Method::GET),
        }
    }

    fn cache_descriptor(method: &str, path: &str) -> Option<(String, u64, u64)> {
        let platform_bridge::offline::SyncPolicy::ReadThrough {
            fresh_for_secs,
            stale_if_offline_secs,
        } = platform_bridge::offline::sync_policy(method, path)
        else {
            return None;
        };
        let principal = platform_bridge::offline::current_principal_partition()?;
        let key = platform_bridge::offline::cache_key(
            selected_workspace_id().as_deref(),
            &principal,
            path,
        );
        Some((key, fresh_for_secs, stale_if_offline_secs))
    }

    fn decode_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
        if body.is_empty() {
            serde_json::from_str("null").map_err(|e| ApiError::Decode(e.to_string()))
        } else {
            serde_json::from_slice(body).map_err(|e| ApiError::Decode(e.to_string()))
        }
    }

    async fn bounded_body(mut response: reqwest::Response) -> Result<Vec<u8>, ApiError> {
        if response
            .content_length()
            .is_some_and(|length| length > MAX_API_RESPONSE_BYTES as u64)
        {
            return Err(ApiError::Network(
                "response exceeded the safe size limit".into(),
            ));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ApiError::Network("response could not be read".into()))?
        {
            if body.len().saturating_add(chunk.len()) > MAX_API_RESPONSE_BYTES {
                return Err(ApiError::Network(
                    "response exceeded the safe size limit".into(),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    async fn do_fetch<T: DeserializeOwned>(
        cx: &ApiContext,
        method: &str,
        path: &str,
        body_str: Option<String>,
    ) -> Result<T, ApiError> {
        let cache = cache_descriptor(method, path);
        if let Some((key, _, _)) = cache.as_ref() {
            if let Ok(Some(hit)) = platform_bridge::offline::get(key) {
                if hit.freshness == platform_bridge::offline::CacheFreshness::Fresh {
                    if let Ok(value) = decode_body(&hit.bytes) {
                        return Ok(value);
                    }
                }
            }
        }
        let url = format!("{}{}", cx.base_url, path);
        let mut req = api_client()?
            .request(method_for(method), &url)
            .header("content-type", "application/json");
        if !cx.id_token.is_empty() {
            req = req.header("authorization", format!("Bearer {}", cx.id_token));
        }
        if let Some(tenant_id) = selected_workspace_id() {
            req = req.header(TENANT_HEADER, tenant_id);
        }
        if let Some(b) = body_str {
            req = req.body(b);
        }

        let resp = match req.send().await {
            Ok(response) => response,
            Err(_) => {
                if let Some((key, _, _)) = cache.as_ref() {
                    if let Ok(Some(hit)) = platform_bridge::offline::get(key) {
                        return decode_body(&hit.bytes);
                    }
                }
                return Err(ApiError::Network(
                    "The service could not be reached. Check your connection and try again.".into(),
                ));
            }
        };
        let status = resp.status().as_u16();
        let body = bounded_body(resp).await?;

        if !(200..300).contains(&status) {
            if status >= 500 {
                if let Some((key, _, _)) = cache.as_ref() {
                    if let Ok(Some(hit)) = platform_bridge::offline::get(key) {
                        return decode_body(&hit.bytes);
                    }
                }
            }
            return Err(ApiError::Status(
                status,
                String::from_utf8_lossy(&body).into_owned(),
            ));
        }
        if let Some((key, fresh_for_secs, stale_if_offline_secs)) = cache {
            let _ =
                platform_bridge::offline::put(&key, &body, fresh_for_secs, stale_if_offline_secs);
        }
        decode_body(&body)
    }

    pub async fn fetch_json<T: DeserializeOwned>(
        cx: &ApiContext,
        method: &str,
        path: &str,
        body: Option<&(impl Serialize + ?Sized)>,
    ) -> Result<T, ApiError> {
        let body_str = match body {
            Some(b) => Some(serde_json::to_string(b).map_err(|e| ApiError::Decode(e.to_string()))?),
            None => None,
        };

        // First attempt with whatever token ApiContext currently holds. On 401,
        // refresh once via the registered RefreshFn and retry — same logic as
        // the wasm path.
        match do_fetch::<T>(cx, method, path, body_str.clone()).await {
            Err(ApiError::Status(401, _)) => {
                let refresher = match super::try_refresh() {
                    Some(f) => f,
                    None => return Err(ApiError::Status(401, "unauthorized".into())),
                };
                let new_token = refresher().await.map_err(|e| ApiError::Status(401, e))?;
                let retried_cx = ApiContext {
                    base_url: cx.base_url.clone(),
                    id_token: new_token,
                };
                do_fetch::<T>(&retried_cx, method, path, body_str).await
            }
            other => other,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native_impl::fetch_json;
#[cfg(target_arch = "wasm32")]
pub use web_impl::fetch_json;

// --- Phase 1c: assignments + submissions ---

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct AssignmentDto {
    pub id: String,
    pub course_id: String,
    pub lesson_id: Option<String>,
    pub title: String,
    pub instructions_md: String,
    pub grading_mode: String,
    pub max_points: Option<i32>,
    pub allow_late: bool,
    pub lock_on_submit: bool,
    pub accepts_text: bool,
    pub accepts_files: bool,
    pub release_mode: String,
    /// Percentage docked from a numeric grade for late submissions. `0` = off.
    #[serde(default)]
    pub late_penalty_percent: i32,
    /// Extra attempts allowed after a return-for-resubmit. `0` = no resubmits.
    #[serde(default)]
    pub max_resubmissions: i32,
    pub attachment_asset_ids: Vec<String>,
    pub due_at: Option<String>,
    pub status: String,
    pub published_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct SubmissionDto {
    pub id: String,
    pub assignment_id: String,
    pub course_id: String,
    pub student_user_id: String,
    pub status: String,
    pub text_answer: Option<String>,
    pub attachment_asset_ids: Vec<String>,
    pub submitted_at: Option<String>,
    pub is_late: bool,
    /// Attempt counter; `1` for the first submission, incremented on each
    /// return-for-resubmit.
    #[serde(default = "default_attempt_number")]
    pub attempt_number: i32,
    /// Late-penalty percentage actually deducted at grade time. `None` until
    /// graded (and hidden from the student until release).
    #[serde(default)]
    pub applied_late_penalty_percent: Option<i32>,
    pub numeric_grade: Option<f64>,
    pub letter_grade: Option<String>,
    pub passed: Option<bool>,
    pub student_visible_feedback: Option<String>,
    pub graded_at: Option<String>,
    pub released_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

fn default_attempt_number() -> i32 {
    1
}

pub async fn list_course_assignments(
    ctx: &ApiContext,
    course_id: &str,
    include_drafts: bool,
) -> Result<Vec<AssignmentDto>, ApiError> {
    let path = format!("/v1/courses/{course_id}/assignments?include_drafts={include_drafts}");
    fetch_json(ctx, "GET", &path, None::<&()>).await
}

pub async fn list_lesson_assignments(
    ctx: &ApiContext,
    lesson_id: &str,
) -> Result<Vec<AssignmentDto>, ApiError> {
    let path = format!("/v1/lessons/{lesson_id}/assignments");
    fetch_json(ctx, "GET", &path, None::<&()>).await
}

pub async fn get_assignment(ctx: &ApiContext, id: &str) -> Result<AssignmentDto, ApiError> {
    fetch_json(ctx, "GET", &format!("/v1/assignments/{id}"), None::<&()>).await
}

#[derive(serde::Serialize)]
pub struct CreateAssignmentBody<'a> {
    pub title: &'a str,
    pub instructions_md: &'a str,
    pub grading_mode: &'a str,
    pub max_points: Option<i32>,
    pub lesson_id: Option<&'a str>,
    pub allow_late: bool,
    pub lock_on_submit: bool,
    pub accepts_text: bool,
    pub accepts_files: bool,
    pub release_mode: &'a str,
    pub late_penalty_percent: i32,
    pub max_resubmissions: i32,
    pub due_at: Option<&'a str>,
}

pub async fn create_assignment(
    ctx: &ApiContext,
    course_id: &str,
    body: &CreateAssignmentBody<'_>,
) -> Result<AssignmentDto, ApiError> {
    let path = format!("/v1/courses/{course_id}/assignments");
    fetch_json(ctx, "POST", &path, Some(body)).await
}

pub async fn publish_assignment(ctx: &ApiContext, id: &str) -> Result<AssignmentDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/assignments/{id}/publish"),
        None::<&()>,
    )
    .await
}

/// Body for `PATCH /v1/assignments/:id`. Every editable field is sent with a
/// definite value so the backend's `double_option` deserializer sees
/// `Some(Some(v))` (set) or `Some(None)` (clear) rather than "leave alone".
///
/// - `max_points`: numeric mode → `Some(n)` (JSON number); pass/fail → `None`
///   (JSON `null`, clears the value).
/// - `due_at`: RFC3339 string → `Some(s)`; cleared → `None` (JSON `null`).
/// - `lesson_id`: `Some(id)` to attach; `None` to detach.
#[derive(serde::Serialize)]
pub struct UpdateAssignmentBody<'a> {
    pub title: &'a str,
    pub instructions_md: &'a str,
    pub grading_mode: &'a str,
    pub max_points: Option<i32>,
    pub allow_late: bool,
    pub lock_on_submit: bool,
    pub accepts_text: bool,
    pub accepts_files: bool,
    pub release_mode: &'a str,
    pub late_penalty_percent: i32,
    pub max_resubmissions: i32,
    pub due_at: Option<String>,
    pub lesson_id: Option<&'a str>,
}

pub async fn update_assignment(
    ctx: &ApiContext,
    id: &str,
    body: &UpdateAssignmentBody<'_>,
) -> Result<AssignmentDto, ApiError> {
    fetch_json(ctx, "PATCH", &format!("/v1/assignments/{id}"), Some(body)).await
}

pub async fn create_or_get_submission(
    ctx: &ApiContext,
    assignment_id: &str,
) -> Result<SubmissionDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/assignments/{assignment_id}/submissions"),
        None::<&()>,
    )
    .await
}

#[derive(serde::Serialize)]
pub struct PatchSubmissionBody<'a> {
    pub text_answer: Option<&'a str>,
    pub attachment_asset_ids: Option<Vec<&'a str>>,
}

pub async fn patch_submission(
    ctx: &ApiContext,
    id: &str,
    body: &PatchSubmissionBody<'_>,
) -> Result<SubmissionDto, ApiError> {
    fetch_json(ctx, "PATCH", &format!("/v1/submissions/{id}"), Some(body)).await
}

pub async fn submit_submission(ctx: &ApiContext, id: &str) -> Result<SubmissionDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/submissions/{id}/submit"),
        None::<&()>,
    )
    .await
}

pub async fn list_assignment_submissions(
    ctx: &ApiContext,
    assignment_id: &str,
) -> Result<Vec<SubmissionDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/assignments/{assignment_id}/submissions"),
        None::<&()>,
    )
    .await
}

/// One per-criterion score sent with a rubric-based grade. The backend sums the
/// `points` into the numeric grade and persists each pair.
#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct CriterionScoreInput {
    pub criterion_id: String,
    pub points: f64,
}

#[derive(serde::Serialize)]
pub struct GradeBody<'a> {
    pub numeric_grade: Option<f64>,
    pub letter_grade: Option<&'a str>,
    pub passed: Option<bool>,
    pub student_visible_feedback: Option<&'a str>,
    pub teacher_only_notes: Option<&'a str>,
    /// Optional rubric per-criterion scores. Omitted when empty so the backend
    /// stays on the legacy numeric/letter/pass path (the field is
    /// `#[serde(default)]` server-side, so a missing key means "no rubric").
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub criteria: Vec<CriterionScoreInput>,
}

// --- Rubric-based grading ---

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct RubricCriterionDto {
    pub id: String,
    pub label: String,
    pub max_points: i32,
    pub sort_order: i32,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct RubricDto {
    pub id: String,
    pub course_id: String,
    pub assignment_id: String,
    pub title: String,
    pub criteria: Vec<RubricCriterionDto>,
}

#[derive(serde::Serialize)]
pub struct CriterionInput<'a> {
    pub label: &'a str,
    pub max_points: i32,
}

#[derive(serde::Serialize)]
pub struct UpsertRubricBody<'a> {
    pub title: &'a str,
    pub criteria: Vec<CriterionInput<'a>>,
}

/// GET the rubric attached to an assignment. The endpoint returns `null` (=>
/// `None`) when the assignment has no rubric. Staff-only on the backend.
pub async fn get_assignment_rubric(
    ctx: &ApiContext,
    assignment_id: &str,
) -> Result<Option<RubricDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/assignments/{assignment_id}/rubric"),
        None::<&()>,
    )
    .await
}

/// Create or replace the rubric for an assignment (title + ordered criteria).
pub async fn upsert_assignment_rubric(
    ctx: &ApiContext,
    assignment_id: &str,
    body: &UpsertRubricBody<'_>,
) -> Result<RubricDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/assignments/{assignment_id}/rubric"),
        Some(body),
    )
    .await
}

/// Delete the rubric attached to an assignment. 204 on success → `()`.
pub async fn delete_assignment_rubric(
    ctx: &ApiContext,
    assignment_id: &str,
) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/assignments/{assignment_id}/rubric"),
        None::<&()>,
    )
    .await
}

pub async fn grade_submission(
    ctx: &ApiContext,
    id: &str,
    body: &GradeBody<'_>,
) -> Result<SubmissionDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/submissions/{id}/grade"),
        Some(body),
    )
    .await
}

pub async fn release_submission(ctx: &ApiContext, id: &str) -> Result<SubmissionDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/submissions/{id}/release"),
        None::<&()>,
    )
    .await
}

pub async fn return_submission(ctx: &ApiContext, id: &str) -> Result<SubmissionDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/submissions/{id}/return"),
        None::<&()>,
    )
    .await
}

// --- Phase 1.5: /v1/me ---

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct UserDto {
    pub user_id: String,
    pub firebase_uid: String,
    pub email: String,
    pub display_name: Option<String>,
    pub tenant_id: Option<String>,
    pub tenant_role: Option<String>,
    pub is_platform_admin: bool,
}

pub async fn get_me(ctx: &ApiContext) -> Result<UserDto, ApiError> {
    let mut result: Result<UserDto, ApiError> = fetch_json(ctx, "GET", "/v1/me", None::<&()>).await;
    if selected_workspace_id().is_some() && matches!(&result, Err(ApiError::Status(400 | 403, _))) {
        // Memberships can be suspended or removed while a device still has
        // the old choice. Recover through the deterministic server default
        // instead of locking the account out during bootstrap.
        set_selected_workspace_id(None);
        result = fetch_json(ctx, "GET", "/v1/me", None::<&()>).await;
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(user) = &result {
        let _ = platform_bridge::offline::set_authenticated_principal(&user.user_id);
    }
    result
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct WorkspaceDto {
    pub tenant_id: String,
    pub name: String,
    pub slug: String,
    pub role: String,
    pub current: bool,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct WorkspacesDto {
    pub current_tenant_id: Option<String>,
    pub workspaces: Vec<WorkspaceDto>,
}

pub async fn get_my_workspaces(ctx: &ApiContext) -> Result<WorkspacesDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/me/workspaces", None::<&()>).await
}

#[derive(Clone, Debug, serde::Serialize, PartialEq, Default)]
pub struct MfaChallengeBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trusted_device_token: Option<String>,
    #[serde(default)]
    pub remember_device: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_label: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct MfaChallengeResponseDto {
    pub stepped_up: bool,
    pub stepup_token: String,
    pub used_recovery_code: bool,
    pub trusted_device_token: Option<String>,
    pub trusted_device_expires_at: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct TrustedDeviceDto {
    pub id: String,
    pub label: String,
    pub user_agent: Option<String>,
    pub last_used_at: Option<String>,
    pub expires_at: String,
    pub created_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct TrustedDeviceListResponseDto {
    pub devices: Vec<TrustedDeviceDto>,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct ResetMfaRecoveryCodesResponseDto {
    pub recovery_codes: Vec<String>,
}

pub async fn challenge_mfa(
    ctx: &ApiContext,
    body: &MfaChallengeBody,
) -> Result<MfaChallengeResponseDto, ApiError> {
    fetch_json(ctx, "POST", "/v1/auth/mfa/challenge", Some(body)).await
}

pub async fn list_mfa_trusted_devices(
    ctx: &ApiContext,
) -> Result<TrustedDeviceListResponseDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/me/mfa/trusted-devices", None::<&()>).await
}

pub async fn revoke_mfa_trusted_device(ctx: &ApiContext, id: &str) -> Result<(), ApiError> {
    let _: serde_json::Value = fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/me/mfa/trusted-devices/{id}"),
        None::<&()>,
    )
    .await?;
    Ok(())
}

pub async fn admin_reset_mfa_recovery_codes(
    ctx: &ApiContext,
    user_id: &str,
) -> Result<ResetMfaRecoveryCodesResponseDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/admin/tenant/memberships/{user_id}/mfa/recovery-codes"),
        None::<&()>,
    )
    .await
}

pub fn api_error_body_contains(err: &ApiError, needle: &str) -> bool {
    matches!(err, ApiError::Status(_, body) if body.contains(needle))
}

// --- Local-only login bypass for browser checklist testing ---

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct DevLoginProfileDto {
    pub name: String,
    pub email: String,
    pub display_name: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct DevLoginConfigDto {
    pub enabled: bool,
    pub profiles: Vec<DevLoginProfileDto>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct DevLoginResponseDto {
    pub id_token: String,
}

#[derive(serde::Serialize)]
pub struct DevLoginRequest<'a> {
    pub profile: &'a str,
}

pub async fn get_dev_login_config(ctx: &ApiContext) -> Result<DevLoginConfigDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/dev/login/config", None::<&()>).await
}

pub async fn dev_login(ctx: &ApiContext, profile: &str) -> Result<DevLoginResponseDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        "/v1/dev/login",
        Some(&DevLoginRequest { profile }),
    )
    .await
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct LocalLoginResponse {
    pub id_token: String,
}

#[derive(serde::Serialize)]
struct LocalLoginRequest<'a> {
    email: &'a str,
    password: &'a str,
}

pub async fn local_login(
    ctx: &ApiContext,
    email: &str,
    password: &str,
) -> Result<LocalLoginResponse, String> {
    fetch_json::<LocalLoginResponse>(
        ctx,
        "POST",
        "/v1/auth/local-login",
        Some(&LocalLoginRequest { email, password }),
    )
    .await
    .map_err(|e| e.to_string())
}

/// Dioxus hook that reads the Signal-wrapped ApiContext from context and
/// returns a fresh clone. The call is repeated each render so it stays
/// reactive inside Dioxus components.
pub fn use_api() -> ApiContext {
    use dioxus::prelude::*;
    use_context::<Signal<ApiContext>>().read().clone()
}

// --- Phase 1.5: /v1/me/courses + /v1/me/schedule ---

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct MyCourseDto {
    pub course_id: String,
    pub slug: String,
    pub title: String,
    pub status: String,
    pub role: String,
    pub next_session_at: Option<String>,
}

pub async fn list_my_courses(ctx: &ApiContext) -> Result<Vec<MyCourseDto>, ApiError> {
    fetch_json(ctx, "GET", "/v1/me/courses", None::<&()>).await
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ScheduleEntryDto {
    pub session_id: String,
    pub course_id: String,
    pub course_title: String,
    pub title: String,
    pub starts_at: String,
    pub duration_minutes: i32,
    pub status: String,
}

pub async fn list_my_schedule(ctx: &ApiContext) -> Result<Vec<ScheduleEntryDto>, ApiError> {
    fetch_json(ctx, "GET", "/v1/me/schedule", None::<&()>).await
}

// --- Phase 1.5: /v1/courses CRUD ---

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct CourseDto {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub cover_asset_id: Option<String>,
    pub owner_user_id: String,
    #[serde(default)]
    pub caller_course_role: Option<String>,
    // Default so existing struct-literal construction sites (and older API
    // responses) keep working; the backend now always returns these.
    #[serde(default)]
    pub syllabus_md: Option<String>,
    #[serde(default)]
    pub grading_policy_md: Option<String>,
    /// Catalog self-enrollment toggle; default false for older payloads.
    #[serde(default)]
    pub self_enrollment_enabled: bool,
    pub created_at: String,
}

/// Mirrors the backend `CourseSyllabusDto` from
/// `GET /v1/courses/:id/syllabus` — the student-readable syllabus surface.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct CourseSyllabusDto {
    pub course_id: String,
    pub title: String,
    pub syllabus_md: Option<String>,
    pub grading_policy_md: Option<String>,
}

/// Mirrors the backend `DuplicateCourseDto` from
/// `POST /v1/courses/:id/duplicate`.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct DuplicateCourseDto {
    pub id: String,
    pub slug: String,
    pub title: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct LessonSummaryDto {
    pub id: String,
    pub course_id: String,
    pub module_id: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub title: String,
    pub body_md: Option<String>,
    pub video_asset_id: Option<String>,
    pub live_session_id: Option<String>,
    pub sort_order: i32,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ModuleWithLessonsDto {
    pub id: String,
    pub course_id: String,
    pub title: String,
    pub sort_order: i32,
    pub lessons: Vec<LessonSummaryDto>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ModuleDto {
    pub id: String,
    pub course_id: String,
    pub title: String,
    pub sort_order: i32,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct CourseMemberDto {
    pub user_id: String,
    pub display_name: Option<String>,
    pub email: String,
    pub role: String,
    pub status: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct CourseSessionDto {
    pub session_id: String,
    pub course_id: String,
    pub course_title: String,
    pub course_slug: String,
    pub title: String,
    pub starts_at: String,
    pub duration_minutes: i32,
    pub status: String,
    pub diverged: bool,
}

pub async fn list_courses(ctx: &ApiContext) -> Result<Vec<CourseDto>, ApiError> {
    fetch_json(ctx, "GET", "/v1/courses", None::<&()>).await
}

#[derive(serde::Serialize)]
pub struct CreateCourseBody<'a> {
    pub title: &'a str,
    pub description: Option<&'a str>,
}

#[derive(serde::Serialize, Default)]
pub struct PatchCourseBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_asset_id: Option<Option<&'a str>>,
    // Double-Option: omitted = leave alone, Some(None) = clear, Some(Some) = set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub syllabus_md: Option<Option<&'a str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grading_policy_md: Option<Option<&'a str>>,
    /// Catalog self-enrollment toggle (teacher/org-admin only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_enrollment_enabled: Option<bool>,
}

#[derive(serde::Serialize)]
pub struct CreateModuleBody<'a> {
    pub title: &'a str,
}

#[derive(serde::Serialize)]
pub struct ReorderModulesBody {
    pub module_ids: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct CreateLessonBody<'a> {
    #[serde(rename = "type")]
    pub r#type: &'a str,
    pub title: &'a str,
    pub body_md: Option<&'a str>,
    pub live_session_id: Option<&'a str>,
}

#[derive(serde::Serialize)]
pub struct ReorderLessonsBody {
    pub lesson_ids: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct PatchAssignmentBody {
    pub attachment_asset_ids: Option<Vec<String>>,
}

#[derive(serde::Serialize)]
pub struct PatchModuleBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<&'a str>,
}

#[derive(serde::Serialize, Default)]
pub struct PatchLessonBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_md: Option<&'a str>,
    /// Double-Option so callers can distinguish "leave alone" (None) from
    /// "set to NULL" (Some(None)) — matches the backend's PatchLesson DTO.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_session_id: Option<Option<&'a str>>,
}

#[derive(serde::Serialize)]
pub struct CreateInvitationBody<'a> {
    pub email: &'a str,
    pub role: &'a str,
}

#[derive(serde::Serialize)]
pub struct CreateCodeBody {
    pub max_uses: Option<i32>,
    pub expires_at: Option<String>,
}

pub async fn create_course(
    ctx: &ApiContext,
    body: &CreateCourseBody<'_>,
) -> Result<CourseDto, ApiError> {
    fetch_json(ctx, "POST", "/v1/courses", Some(body)).await
}

pub async fn patch_course(
    ctx: &ApiContext,
    course_id: &str,
    body: &PatchCourseBody<'_>,
) -> Result<CourseDto, ApiError> {
    fetch_json(
        ctx,
        "PATCH",
        &format!("/v1/courses/{course_id}"),
        Some(body),
    )
    .await
}

/// `GET /v1/courses/{id}/syllabus` — readable by anyone who can read the
/// course (students included).
pub async fn get_course_syllabus(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<CourseSyllabusDto, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/syllabus"),
        None::<&()>,
    )
    .await
}

/// `POST /v1/courses/{id}/duplicate` — staff/org-admin deep-copy into a new
/// draft. Returns the new course id/slug/title for navigation.
pub async fn duplicate_course(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<DuplicateCourseDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/duplicate"),
        None::<&()>,
    )
    .await
}

pub async fn get_course_outline(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<Vec<ModuleWithLessonsDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/modules-with-lessons"),
        None::<&()>,
    )
    .await
}

pub async fn list_course_members(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<Vec<CourseMemberDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/members"),
        None::<&()>,
    )
    .await
}

pub async fn list_course_sessions(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<Vec<CourseSessionDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/sessions"),
        None::<&()>,
    )
    .await
}

pub async fn create_module(
    ctx: &ApiContext,
    course_id: &str,
    body: &CreateModuleBody<'_>,
) -> Result<ModuleDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/modules"),
        Some(body),
    )
    .await
}

pub async fn reorder_modules(
    ctx: &ApiContext,
    course_id: &str,
    body: &ReorderModulesBody,
) -> Result<serde_json::Value, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/modules/reorder"),
        Some(body),
    )
    .await
}

pub async fn create_lesson(
    ctx: &ApiContext,
    course_id: &str,
    module_id: &str,
    body: &CreateLessonBody<'_>,
) -> Result<LessonSummaryDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/modules/{module_id}/lessons"),
        Some(body),
    )
    .await
}

pub async fn reorder_lessons(
    ctx: &ApiContext,
    course_id: &str,
    module_id: &str,
    body: &ReorderLessonsBody,
) -> Result<serde_json::Value, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/modules/{module_id}/lessons/reorder"),
        Some(body),
    )
    .await
}

pub async fn patch_module(
    ctx: &ApiContext,
    course_id: &str,
    module_id: &str,
    body: &PatchModuleBody<'_>,
) -> Result<ModuleDto, ApiError> {
    fetch_json(
        ctx,
        "PATCH",
        &format!("/v1/courses/{course_id}/modules/{module_id}"),
        Some(body),
    )
    .await
}

pub async fn patch_lesson(
    ctx: &ApiContext,
    course_id: &str,
    module_id: &str,
    lesson_id: &str,
    body: &PatchLessonBody<'_>,
) -> Result<LessonSummaryDto, ApiError> {
    fetch_json(
        ctx,
        "PATCH",
        &format!("/v1/courses/{course_id}/modules/{module_id}/lessons/{lesson_id}"),
        Some(body),
    )
    .await
}

pub async fn patch_assignment(
    ctx: &ApiContext,
    assignment_id: &str,
    body: &PatchAssignmentBody,
) -> Result<AssignmentDto, ApiError> {
    fetch_json(
        ctx,
        "PATCH",
        &format!("/v1/assignments/{assignment_id}"),
        Some(body),
    )
    .await
}

// --- Destructive operations + assignment-unpublish ---

pub async fn delete_course(ctx: &ApiContext, course_id: &str) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/courses/{course_id}"),
        None::<&()>,
    )
    .await
}

pub async fn delete_module(
    ctx: &ApiContext,
    course_id: &str,
    module_id: &str,
) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/courses/{course_id}/modules/{module_id}"),
        None::<&()>,
    )
    .await
}

pub async fn delete_lesson(
    ctx: &ApiContext,
    course_id: &str,
    module_id: &str,
    lesson_id: &str,
) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/courses/{course_id}/modules/{module_id}/lessons/{lesson_id}"),
        None::<&()>,
    )
    .await
}

pub async fn delete_assignment(ctx: &ApiContext, assignment_id: &str) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/assignments/{assignment_id}"),
        None::<&()>,
    )
    .await
}

pub async fn unpublish_assignment(
    ctx: &ApiContext,
    assignment_id: &str,
) -> Result<AssignmentDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/assignments/{assignment_id}/unpublish"),
        Some(&serde_json::json!({})),
    )
    .await
}

// --- Phase 1.5: invitations + redeem ---

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct CourseInvitationDto {
    pub id: String,
    pub email: String,
    pub role: String,
    pub status: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct CodeSummaryDto {
    pub id: String,
    pub last4: String,
    pub max_uses: Option<i32>,
    pub uses: i32,
    pub expires_at: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct RedeemedDto {
    pub course_id: String,
    pub course_title: String,
}

pub async fn list_course_invitations(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<Vec<CourseInvitationDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/invitations"),
        None::<&()>,
    )
    .await
}

pub async fn create_course_invitation(
    ctx: &ApiContext,
    course_id: &str,
    body: &CreateInvitationBody<'_>,
) -> Result<serde_json::Value, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/invitations"),
        Some(body),
    )
    .await
}

pub async fn revoke_course_invitation(
    ctx: &ApiContext,
    course_id: &str,
    invitation_id: &str,
) -> Result<serde_json::Value, ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/courses/{course_id}/invitations/{invitation_id}"),
        None::<&()>,
    )
    .await
}

pub async fn list_enrollment_codes(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<Vec<CodeSummaryDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/codes"),
        None::<&()>,
    )
    .await
}

pub async fn create_enrollment_code(
    ctx: &ApiContext,
    course_id: &str,
    body: &CreateCodeBody,
) -> Result<serde_json::Value, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/codes"),
        Some(body),
    )
    .await
}

pub async fn revoke_enrollment_code(
    ctx: &ApiContext,
    course_id: &str,
    code_id: &str,
) -> Result<serde_json::Value, ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/courses/{course_id}/codes/{code_id}"),
        None::<&()>,
    )
    .await
}

pub async fn accept_invitation(ctx: &ApiContext, token: &str) -> Result<RedeemedDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/invitations/{token}/accept"),
        Some(&serde_json::json!({})),
    )
    .await
}

pub async fn redeem_enrollment_code(ctx: &ApiContext, code: &str) -> Result<RedeemedDto, ApiError> {
    #[derive(serde::Serialize)]
    struct Body<'a> {
        code: &'a str,
    }
    fetch_json(ctx, "POST", "/v1/codes/redeem", Some(&Body { code })).await
}

// --- Transcript + catalog / self-enrollment ---

/// Mirrors the backend `TranscriptCertificateDto`.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct TranscriptCertificateDto {
    pub credential_id: String,
    pub status: String,
}

/// One course row of `GET /v1/me/transcript`.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct TranscriptCourseDto {
    pub course_id: String,
    pub slug: String,
    pub title: String,
    pub status: String,
    pub role: String,
    pub membership_status: String,
    pub joined_at: String,
    pub lessons_total: i64,
    pub lessons_completed: i64,
    pub graded_released_count: i64,
    pub weighted_total: Option<f64>,
    pub letter_grade: Option<String>,
    pub certificate: Option<TranscriptCertificateDto>,
}

/// Mirrors the backend `TranscriptDto` (student + generated_at are not needed
/// by the view; the courses list is the payload).
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct TranscriptResponseDto {
    pub student_user_id: String,
    pub generated_at: String,
    pub courses: Vec<TranscriptCourseDto>,
}

pub async fn fetch_my_transcript(ctx: &ApiContext) -> Result<TranscriptResponseDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/me/transcript", None::<&()>).await
}

/// One open course in the workspace catalog.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct CatalogCourseDto {
    pub course_id: String,
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub cover_asset_id: Option<String>,
    #[serde(default)]
    pub owner_name: Option<String>,
    pub enrolled: bool,
}

pub async fn list_catalog(ctx: &ApiContext) -> Result<Vec<CatalogCourseDto>, ApiError> {
    fetch_json(ctx, "GET", "/v1/catalog", None::<&()>).await
}

pub async fn catalog_self_enroll(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<RedeemedDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/catalog/{course_id}/enroll"),
        Some(&serde_json::json!({})),
    )
    .await
}

// --- File uploads (begin + complete + get-url) ---

#[derive(Clone, Debug, serde::Serialize, PartialEq)]
pub struct UploadBeginBody<'a> {
    pub filename: &'a str,
    pub content_type: &'a str,
    pub size_bytes: i64,
    pub linked_entity_type: &'a str,
    pub linked_entity_id: String,
    pub purpose: &'a str,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct UploadBeginResponse {
    pub asset_id: String,
    pub presigned_put_url: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct UploadCompleteResponse {
    pub asset_id: String,
    pub status: String,
    pub size_bytes: i64,
    pub content_type: String,
}

pub async fn upload_begin(
    ctx: &ApiContext,
    body: &UploadBeginBody<'_>,
) -> Result<UploadBeginResponse, ApiError> {
    fetch_json(ctx, "POST", "/v1/uploads/begin", Some(body)).await
}

pub async fn upload_complete(
    ctx: &ApiContext,
    asset_id: &str,
) -> Result<UploadCompleteResponse, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/uploads/{asset_id}/complete"),
        Some(&serde_json::json!({})),
    )
    .await
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct FileAssetUrlResponse {
    pub url: String,
}

pub async fn file_asset_get_url(
    ctx: &ApiContext,
    asset_id: &str,
) -> Result<FileAssetUrlResponse, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/file-assets/{asset_id}/url"),
        None::<&()>,
    )
    .await
}

pub async fn delete_file_asset(ctx: &ApiContext, asset_id: &str) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/file-assets/{asset_id}"),
        None::<&()>,
    )
    .await
}

// --- Admin: audit events ---

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct AuditEventDto {
    pub id: String,
    pub actor_user_id: String,
    pub actor_email: Option<String>,
    pub actor_display_name: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub metadata: Option<serde_json::Value>,
    pub occurred_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct AuditListResponseDto {
    pub events: Vec<AuditEventDto>,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct TenantDto {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub status: String,
    pub recording_default: bool,
    pub recording_retention_days: i32,
    pub created_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct MembershipDto {
    pub user_id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub role: String,
    pub status: String,
    pub joined_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct MembershipListResponseDto {
    pub memberships: Vec<MembershipDto>,
}

pub async fn get_my_tenant(ctx: &ApiContext) -> Result<TenantDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/admin/tenant", None::<&()>).await
}

#[derive(Clone, Debug, serde::Serialize, Default, PartialEq)]
pub struct PatchTenantBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_default: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_retention_days: Option<i32>,
}

pub async fn patch_my_tenant(
    ctx: &ApiContext,
    body: &PatchTenantBody,
) -> Result<TenantDto, ApiError> {
    fetch_json(ctx, "PATCH", "/v1/admin/tenant", Some(body)).await
}

#[derive(Clone, Debug, serde::Serialize, Default, PartialEq)]
pub struct PatchMembershipBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

pub async fn patch_tenant_membership(
    ctx: &ApiContext,
    user_id: &str,
    body: &PatchMembershipBody,
) -> Result<MembershipDto, ApiError> {
    fetch_json(
        ctx,
        "PATCH",
        &format!("/v1/admin/tenant/memberships/{user_id}"),
        Some(body),
    )
    .await
}

pub async fn list_tenant_memberships(
    ctx: &ApiContext,
) -> Result<MembershipListResponseDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/admin/tenant/memberships", None::<&()>).await
}

/// Deliberately verbose confirmation required by the ownership-transfer API.
/// Keeping the phrase shared with the UI prevents a visually valid dialog
/// from sending a subtly different value to the server.
pub const OWNERSHIP_TRANSFER_CONFIRMATION: &str = "TRANSFER OWNERSHIP";

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct TransferOwnershipBody {
    pub new_owner_user_id: String,
    pub confirmation: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq, Eq)]
pub struct TransferOwnershipResponseDto {
    pub previous_owner_user_id: String,
    pub new_owner_user_id: String,
    pub transferred_at: String,
}

/// Atomically promote an active member to organization owner and demote the
/// caller to organization admin. The backend remains authoritative for owner
/// identity, target status, and the exact confirmation phrase.
pub async fn transfer_tenant_ownership(
    ctx: &ApiContext,
    new_owner_user_id: &str,
    confirmation: &str,
) -> Result<TransferOwnershipResponseDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        "/v1/admin/tenant/transfer-ownership",
        Some(&TransferOwnershipBody {
            new_owner_user_id: new_owner_user_id.to_string(),
            confirmation: confirmation.to_string(),
        }),
    )
    .await
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct FileAssetSummaryDto {
    pub asset_id: String,
    pub owner_user_id: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub linked_entity_type: Option<String>,
    pub linked_entity_id: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct FileAssetListResponseDto {
    pub assets: Vec<FileAssetSummaryDto>,
}

pub async fn list_tenant_file_assets(
    ctx: &ApiContext,
) -> Result<FileAssetListResponseDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/admin/file-assets", None::<&()>).await
}

pub async fn list_audit_events(
    ctx: &ApiContext,
    limit: Option<i64>,
    before: Option<&str>,
) -> Result<AuditListResponseDto, ApiError> {
    let mut path = String::from("/v1/admin/audit");
    let mut parts: Vec<String> = Vec::new();
    if let Some(l) = limit {
        parts.push(format!("limit={l}"));
    }
    if let Some(b) = before {
        parts.push(format!("before={}", urlencode(b)));
    }
    if !parts.is_empty() {
        path.push('?');
        path.push_str(&parts.join("&"));
    }
    fetch_json(ctx, "GET", &path, None::<&()>).await
}

fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            other => {
                let mut buf = [0u8; 4];
                let bytes = other.encode_utf8(&mut buf).as_bytes();
                let mut out = String::with_capacity(bytes.len() * 3);
                for b in bytes {
                    out.push_str(&format!("%{:02X}", b));
                }
                out
            }
        })
        .collect()
}

// --- Course recordings list ---

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct CourseRecordingListItemDto {
    pub recording_id: String,
    pub session_id: String,
    pub session_title: String,
    pub starts_at: String,
    pub started_at: String,
    pub ended_at: String,
    pub duration_seconds: i32,
    pub processing_status: String,
    pub has_playback: bool,
}

pub async fn list_course_recordings(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<Vec<CourseRecordingListItemDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/recordings"),
        None::<&()>,
    )
    .await
}

// --- Live session series (recurring) ---

#[derive(Clone, Debug, serde::Serialize, PartialEq)]
pub struct CreateSeriesBody {
    pub title: String,
    pub starts_at: String,
    pub duration_minutes: i32,
    pub frequency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byweekday: Option<Vec<String>>,
    pub end_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_teacher_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_enabled: Option<bool>,
    pub transport_mode: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct SeriesDto {
    pub id: String,
    pub course_id: String,
    pub title: String,
    pub frequency: String,
    pub end_kind: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct OccurrenceDto {
    pub id: String,
    pub series_id: String,
    pub occurrence_index: i32,
    pub title: String,
    pub status: String,
    pub starts_at: String,
    pub duration_minutes: i32,
    pub diverged: bool,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct SeriesCreatedDto {
    pub series: SeriesDto,
    pub occurrences: Vec<OccurrenceDto>,
}

pub async fn create_series(
    ctx: &ApiContext,
    course_id: &str,
    body: &CreateSeriesBody,
) -> Result<SeriesCreatedDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/sessions"),
        Some(body),
    )
    .await
}

pub async fn delete_series(ctx: &ApiContext, series_id: &str) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/series/{series_id}"),
        None::<&()>,
    )
    .await
}

#[derive(Clone, Debug, serde::Serialize, Default, PartialEq)]
pub struct PatchOccurrenceBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starts_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_minutes: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

pub async fn patch_occurrence(
    ctx: &ApiContext,
    session_id: &str,
    body: &PatchOccurrenceBody,
) -> Result<OccurrenceDto, ApiError> {
    fetch_json(
        ctx,
        "PATCH",
        &format!("/v1/sessions/{session_id}"),
        Some(body),
    )
    .await
}

// --- Instant-Start Session: start-now + active-session ---

#[derive(Clone, Debug, serde::Serialize, Default, PartialEq)]
pub struct StartNowBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_minutes: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_enabled: Option<bool>,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct StartNowResponseDto {
    pub session_id: String,
    pub series_id: String,
    pub title: String,
    pub starts_at: String,
    pub duration_minutes: i32,
    pub recording_enabled: bool,
    pub transport_mode: String,
    pub status: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct StartNowConflictDto {
    pub error: String,
    pub active_session_id: String,
    pub title: String,
    pub starts_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct ActiveSessionInfoDto {
    pub session_id: String,
    pub title: String,
    pub starts_at: String,
    pub transport_mode: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct ActiveSessionResponseDto {
    pub active: Option<ActiveSessionInfoDto>,
}

/// Result type for start-now: distinguishes 409-conflict (carries the
/// active session info) from other API errors.
pub enum StartNowOutcome {
    Created(StartNowResponseDto),
    Conflict(StartNowConflictDto),
    Failed(ApiError),
}

pub async fn start_session_now(
    ctx: &ApiContext,
    course_id: &str,
    body: &StartNowBody,
) -> StartNowOutcome {
    let path = format!("/v1/courses/{course_id}/sessions/start-now");
    match fetch_json::<StartNowResponseDto>(ctx, "POST", &path, Some(body)).await {
        Ok(dto) => StartNowOutcome::Created(dto),
        Err(ApiError::Status(409, body_str)) => {
            match serde_json::from_str::<StartNowConflictDto>(&body_str) {
                Ok(conflict) => StartNowOutcome::Conflict(conflict),
                Err(_) => StartNowOutcome::Failed(ApiError::Status(409, body_str)),
            }
        }
        Err(e) => StartNowOutcome::Failed(e),
    }
}

pub async fn get_active_session(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<ActiveSessionResponseDto, ApiError> {
    let path = format!("/v1/courses/{course_id}/active-session");
    fetch_json(ctx, "GET", &path, None::<&()>).await
}

// --- Phase 2: session attendance report (staff-only) ---

/// Mirrors the backend `AttendanceDto` in
/// `crates/backend/src/handlers/live_sessions.rs`. The backend serializes
/// `user_id` (Uuid) and the timestamps (`DateTime<Utc>`) as JSON strings, so
/// they're decoded here as `String` / `Option<String>`. Field names use serde
/// defaults and match the backend struct exactly.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct AttendanceDto {
    pub user_id: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub first_joined_at: String,
    pub last_left_at: Option<String>,
    pub total_seconds: i32,
    pub reconnect_count: i32,
}

/// `GET /v1/sessions/{session_id}/attendance` — durable attendance report.
/// The endpoint authorizes course-staff only and returns 403 to others, so
/// callers should treat `ApiError::Status(403, _)` as "not visible" rather
/// than a hard error.
pub async fn list_session_attendance(
    ctx: &ApiContext,
    session_id: &str,
) -> Result<Vec<AttendanceDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/sessions/{session_id}/attendance"),
        None::<&()>,
    )
    .await
}

// --- Analytics dashboards (org overview + per-course) ---

/// Mirrors the backend `OverviewDto`. Uuid/count fields serialize as JSON
/// numbers, so every field is decoded as `i64`. Authorized for org-admin /
/// platform-admin only; the endpoint returns 403 to everyone else.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct OverviewDto {
    pub courses_total: i64,
    pub courses_published: i64,
    pub members_students: i64,
    pub members_teachers: i64,
    pub members_tas: i64,
    pub members_parents: i64,
    pub sessions_last_30d: i64,
    pub sessions_ended_total: i64,
    pub recordings_available: i64,
    pub assignments_total: i64,
    pub submissions_total: i64,
    pub submissions_graded: i64,
}

/// Mirrors the backend `AssignmentProgressDto`. `avg_numeric_grade` is `None`
/// when no submissions have been graded numerically.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct AssignmentProgressDto {
    pub assignment_id: String,
    pub title: String,
    pub status: String,
    pub submitted_count: i64,
    pub graded_count: i64,
    pub avg_numeric_grade: Option<f64>,
}

/// Mirrors the backend `CourseAnalyticsDto`. `course_id` is a Uuid serialized
/// as a JSON string. Authorized for course-staff / platform-admin; the
/// endpoint returns 403 to non-staff, so callers should treat
/// `ApiError::Status(403, _)` as "not visible" rather than a hard error.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct CourseAnalyticsDto {
    pub course_id: String,
    pub enrolled_students: i64,
    pub sessions_total: i64,
    pub sessions_ended: i64,
    pub unique_attendees: i64,
    pub avg_attendance_seconds: f64,
    pub assignments: Vec<AssignmentProgressDto>,
    #[serde(default)]
    pub funnel: ProgressFunnelDto,
    #[serde(default)]
    pub quiz_scores: QuizScoreDistributionDto,
    // Grade analytics (Wave 2) — additive, serde-default so older payloads decode.
    #[serde(default)]
    pub grade_distribution: Vec<crate::analytics_panel::GradeDistributionDto>,
    #[serde(default)]
    pub grade_trend: Vec<crate::analytics_panel::ClassAverageTrendDto>,
    #[serde(default)]
    pub at_risk: Vec<crate::analytics_panel::AtRiskStudentDto>,
    #[serde(default)]
    pub at_risk_threshold: f64,
}

/// Mirrors the backend `ProgressFunnelDto` (lesson-progress funnel).
#[derive(Clone, Debug, serde::Deserialize, PartialEq, Default)]
pub struct ProgressFunnelDto {
    pub enrolled: i64,
    pub started: i64,
    pub half: i64,
    pub completed: i64,
}

/// Mirrors the backend `QuizScoreDistributionDto` (graded attempts by % bucket).
#[derive(Clone, Debug, serde::Deserialize, PartialEq, Default)]
pub struct QuizScoreDistributionDto {
    pub under_50: i64,
    pub from_50_to_69: i64,
    pub from_70_to_89: i64,
    pub from_90_up: i64,
}

/// Mirrors the backend `DailyActivityDto` (tenant activity per day).
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct DailyActivityDto {
    pub day: String,
    pub sessions: i64,
    pub attendance_joins: i64,
    pub submissions: i64,
    pub lessons_completed: i64,
}

/// `GET /v1/analytics/overview` — tenant-wide rollup. Org-admin / platform-admin
/// only (403 otherwise).
pub async fn get_analytics_overview(ctx: &ApiContext) -> Result<OverviewDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/analytics/overview", None::<&()>).await
}

/// `GET /v1/analytics/activity?days=N` — tenant activity per day for charts.
/// Org-admin / platform-admin only (403 otherwise).
pub async fn get_analytics_activity(
    ctx: &ApiContext,
    days: i32,
) -> Result<Vec<DailyActivityDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/analytics/activity?days={days}"),
        None::<&()>,
    )
    .await
}

/// `GET /v1/courses/{course_id}/analytics` — per-course rollup. Course-staff /
/// platform-admin only (403 otherwise).
pub async fn get_course_analytics(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<CourseAnalyticsDto, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/analytics"),
        None::<&()>,
    )
    .await
}

// --- Parent dashboard (read-only) ---
//
// All four endpoints require the signed-in user to have role=Parent. The
// per-child endpoints return 403 unless the parent is linked to that child, so
// callers should treat `ApiError::Status(403, _)` as "not linked / not visible"
// rather than a hard error. Field names mirror the backend DTOs exactly; Uuid
// and DateTime values serialize as JSON strings, counts as i64, grades as f64.

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ChildDto {
    pub student_user_id: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ChildGradeDto {
    pub assignment_id: String,
    pub assignment_title: String,
    pub course_title: Option<String>,
    pub status: String,
    pub numeric_grade: Option<f64>,
    pub letter_grade: Option<String>,
    pub passed: Option<bool>,
    pub student_visible_feedback: Option<String>,
    pub released_at: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ChildAttendanceDto {
    pub session_id: String,
    pub session_title: Option<String>,
    pub course_title: Option<String>,
    pub first_joined_at: String,
    pub last_left_at: Option<String>,
    pub total_seconds: i64,
    pub reconnect_count: i64,
    pub starts_at: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ChildScheduleItemDto {
    pub session_id: String,
    pub course_id: String,
    pub course_title: String,
    pub title: String,
    pub starts_at: String,
    pub duration_minutes: i32,
    pub status: String,
}

/// `GET /v1/parent/children` — list children linked to the signed-in parent.
pub async fn list_parent_children(ctx: &ApiContext) -> Result<Vec<ChildDto>, ApiError> {
    fetch_json(ctx, "GET", "/v1/parent/children", None::<&()>).await
}

/// `GET /v1/parent/children/{student_id}/grades` — released grades for a child.
pub async fn get_child_grades(
    ctx: &ApiContext,
    student_id: &str,
) -> Result<Vec<ChildGradeDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/parent/children/{student_id}/grades"),
        None::<&()>,
    )
    .await
}

/// `GET /v1/parent/children/{student_id}/attendance` — attendance rollup.
pub async fn get_child_attendance(
    ctx: &ApiContext,
    student_id: &str,
) -> Result<Vec<ChildAttendanceDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/parent/children/{student_id}/attendance"),
        None::<&()>,
    )
    .await
}

/// `GET /v1/parent/children/{student_id}/schedule?days=30` — upcoming sessions.
pub async fn get_child_schedule(
    ctx: &ApiContext,
    student_id: &str,
) -> Result<Vec<ChildScheduleItemDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/parent/children/{student_id}/schedule?days=30"),
        None::<&()>,
    )
    .await
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ChildGamificationDto {
    pub total_xp: i64,
    pub level: u32,
    pub current_streak_days: i32,
    pub longest_streak_days: i32,
    pub streak_active_today: bool,
}

/// `GET /v1/parent/children/{student_id}/gamification` — child XP/streak.
pub async fn get_child_gamification(
    ctx: &ApiContext,
    student_id: &str,
) -> Result<ChildGamificationDto, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/parent/children/{student_id}/gamification"),
        None::<&()>,
    )
    .await
}

// --- Admin: parent invitations ---
//
// Org-admin / platform-admin only. Links a parent (by email) to a student so
// the parent can view that child's grades, attendance, and schedule. Field
// names mirror the backend `ParentInvitationDto` exactly.

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ParentInvitationDto {
    pub id: String,
    pub parent_email: String,
    pub student_user_id: String,
    pub relationship: Option<String>,
    pub status: String,
    pub created_at: String,
}

#[derive(serde::Serialize)]
pub struct CreateParentInvitationBody {
    pub parent_email: String,
    pub student_user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<String>,
}

/// `POST /v1/admin/parent-invitations` — invite a parent and link them to a
/// student.
pub async fn create_parent_invitation(
    ctx: &ApiContext,
    parent_email: &str,
    student_user_id: &str,
    relationship: Option<&str>,
) -> Result<ParentInvitationDto, ApiError> {
    let body = CreateParentInvitationBody {
        parent_email: parent_email.to_string(),
        student_user_id: student_user_id.to_string(),
        relationship: relationship
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    };
    fetch_json(ctx, "POST", "/v1/admin/parent-invitations", Some(&body)).await
}

/// `GET /v1/admin/parent-invitations?student_id=<uuid>` — list invitations,
/// optionally filtered to a single student.
pub async fn list_parent_invitations(
    ctx: &ApiContext,
    student_id: Option<&str>,
) -> Result<Vec<ParentInvitationDto>, ApiError> {
    let path = match student_id {
        Some(id) if !id.is_empty() => {
            format!("/v1/admin/parent-invitations?student_id={}", urlencode(id))
        }
        _ => "/v1/admin/parent-invitations".to_string(),
    };
    fetch_json(ctx, "GET", &path, None::<&()>).await
}

/// `DELETE /v1/admin/parent-invitations/{id}` — revoke a pending invitation.
pub async fn revoke_parent_invitation(ctx: &ApiContext, id: &str) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/admin/parent-invitations/{id}"),
        None::<&()>,
    )
    .await
}

// --- Admin: tenant member invitations + seat cap ---
//
// Org-admin / platform-admin only. Invites a tenant member (by email + role) and
// enforces a tenant-wide seat cap at creation time (the backend returns
// HTTP 409 `{"error":"conflict: seat_limit_reached"}` when the cap is reached).
// Field names mirror the backend `InvitationDto` / `CreateMemberInvitation`
// exactly. `id` (Uuid) and `created_at` (DateTime<Utc>) serialize as JSON
// strings.

/// Friendly error string surfaced when the backend rejects an invite because
/// the tenant's seat cap is reached. The backend returns
/// `ApiError::Status(409, "conflict: seat_limit_reached")`.
pub const SEAT_LIMIT_REACHED: &str = "seat_limit_reached";

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct InvitationDto {
    pub id: String,
    pub email: String,
    pub role: String,
    pub status: String,
    pub created_at: String,
}

#[derive(serde::Serialize)]
pub struct CreateMemberInvitationBody {
    pub email: String,
    pub role: String,
}

/// `POST /v1/admin/member-invitations` — invite a tenant member with the given
/// role. The backend seat-checks first and returns 409 `seat_limit_reached`
/// when the tenant is at/over its seat cap (only enforced when overage behavior
/// is "block").
pub async fn create_member_invitation(
    ctx: &ApiContext,
    email: &str,
    role: &str,
) -> Result<InvitationDto, ApiError> {
    let body = CreateMemberInvitationBody {
        email: email.trim().to_string(),
        role: role.to_string(),
    };
    fetch_json(ctx, "POST", "/v1/admin/member-invitations", Some(&body)).await
}

/// `GET /v1/admin/member-invitations` — list pending member invitations.
pub async fn list_member_invitations(ctx: &ApiContext) -> Result<Vec<InvitationDto>, ApiError> {
    fetch_json(ctx, "GET", "/v1/admin/member-invitations", None::<&()>).await
}

/// `DELETE /v1/admin/member-invitations/{id}` — revoke a pending invitation.
pub async fn revoke_member_invitation(ctx: &ApiContext, id: &str) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/admin/member-invitations/{id}"),
        None::<&()>,
    )
    .await
}

// --- Admin: billing (plans, subscription, usage) ---
//
// Org-admin / platform-admin only. The backend `/v1/admin/billing` family is
// implemented in `crates/backend`; field names below mirror the backend DTOs
// (`BillingDto`, `PlanDto`, `SubscriptionDto`, `UsageDto`) EXACTLY. Counts are
// JSON numbers decoded as `i64`; any monetary value is integer cents (`i64`).

/// A single subscribable plan. `monthly_price_cents` is the monthly price in
/// integer cents; the `included_*` fields are enforced plan limits.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct PlanDto {
    pub id: String,
    pub name: String,
    pub monthly_price_cents: i64,
    pub included_seats: i64,
    pub included_class_minutes: i64,
    pub included_recording_gb: i64,
}

/// The tenant's current subscription. `status` is the Stripe-style status
/// string (e.g. "active", "trialing", "past_due"). Timestamps serialize as
/// RFC3339 strings and are `None` when not applicable.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct SubscriptionDto {
    pub plan_id: String,
    pub status: String,
    pub current_period_start: Option<String>,
    pub current_period_end: Option<String>,
    pub trial_ends_at: Option<String>,
    pub stripe_subscription_id: Option<String>,
    pub overage_behavior: String,
}

/// Measured usage. Class minutes cover the current month; seats and recording
/// storage are point-in-time. Caps are `None` for no-cap/unlimited resources.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct UsageDto {
    pub active_seats: i64,
    pub included_seats: Option<i64>,
    pub class_minutes_used: i64,
    pub included_class_minutes: Option<i64>,
    pub recording_gb_used: f64,
    pub included_recording_gb: Option<f64>,
}

/// `GET /v1/admin/billing` aggregate. `plan`/`subscription` are `None` for
/// tenants that never started checkout — the page renders a "no plan" state,
/// not a decode error (this mirror had drifted from the backend shape and the
/// whole page errored out).
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct BillingDto {
    pub plan: Option<PlanDto>,
    pub subscription: Option<SubscriptionDto>,
    pub usage: UsageDto,
    pub overage_behavior: String,
    #[serde(default)]
    pub plans: Vec<PlanDto>,
}

/// Response of `POST /v1/admin/billing/checkout-session`. The `url` is the
/// hosted Stripe Checkout URL the browser should be redirected to.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct CheckoutSessionDto {
    pub url: String,
}

/// Response of `POST /v1/admin/billing/portal-session`. The URL points to a
/// short-lived Stripe-hosted Customer Portal session and must be used directly
/// rather than stored in application state.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct BillingPortalSessionDto {
    pub url: String,
}

/// `GET /v1/admin/billing` — current subscription, usage, and available plans.
/// Org-admin / platform-admin only (403 otherwise).
pub async fn get_billing(ctx: &ApiContext) -> Result<BillingDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/admin/billing", None::<&()>).await
}

#[derive(serde::Serialize)]
struct CheckoutSessionBody<'a> {
    plan_id: &'a str,
}

/// `POST /v1/admin/billing/checkout-session` — start a Stripe Checkout session
/// for `plan_id`. Returns the hosted Checkout `url` to redirect to.
pub async fn start_checkout(
    ctx: &ApiContext,
    plan_id: &str,
) -> Result<CheckoutSessionDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        "/v1/admin/billing/checkout-session",
        Some(&CheckoutSessionBody { plan_id }),
    )
    .await
}

/// `POST /v1/admin/billing/portal-session` — open Stripe's self-service
/// subscription management portal. The return destination is selected by the
/// backend from APP_ORIGIN; the browser cannot supply an arbitrary URL.
pub async fn start_billing_portal(ctx: &ApiContext) -> Result<BillingPortalSessionDto, ApiError> {
    fetch_json(ctx, "POST", "/v1/admin/billing/portal-session", None::<&()>).await
}

#[derive(serde::Serialize)]
struct OverageBody<'a> {
    overage_behavior: &'a str,
}

/// `PATCH /v1/admin/billing/overage` — retained compatibility endpoint. The
/// backend currently accepts only protected (`block`) limits.
/// `behavior` is one of "block" | "metered". The backend replies 204 with no
/// body (the old `SubscriptionDto` return type made every toggle "fail").
pub async fn set_overage_behavior(ctx: &ApiContext, behavior: &str) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "PATCH",
        "/v1/admin/billing/overage",
        Some(&OverageBody {
            overage_behavior: behavior,
        }),
    )
    .await
}

/// Navigate the browser to `url` (a full external URL such as a Stripe Checkout
/// session). Lives here because `features-courses` already enables the
/// `web_sys` `Location` feature; a no-op off-wasm so callers don't need cfgs.
#[cfg(target_arch = "wasm32")]
pub fn redirect_to(url: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or_else(|| "Browser window unavailable.".to_string())?;
    window
        .location()
        .set_href(url)
        .map_err(|_| "The external page could not be opened.".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn redirect_to(url: &str) -> Result<(), String> {
    platform_bridge::external::open_external_url(
        url,
        platform_bridge::external::ExternalPurpose::Billing,
    )
    .map_err(|error| error.to_string())
}

/// Save an authenticated response through the native save/share bridge. This
/// centralizes bearer/workspace headers and removes browser-only export no-ops.
#[cfg(not(target_arch = "wasm32"))]
pub async fn save_authenticated_download(
    ctx: &ApiContext,
    path: &str,
    suggested_filename: &str,
    mime_type: &str,
) -> Result<platform_bridge::native_files::SaveOutcome, ApiError> {
    use std::sync::OnceLock;
    use std::time::Duration;

    fn download_client() -> Result<&'static reqwest::Client, ApiError> {
        static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
        CLIENT
            .get_or_init(|| {
                reqwest::Client::builder()
                    .connect_timeout(Duration::from_secs(8))
                    .timeout(Duration::from_secs(5 * 60))
                    .redirect(reqwest::redirect::Policy::none())
                    .user_agent(concat!("AulaLite/", env!("CARGO_PKG_VERSION")))
                    .build()
                    .map_err(|_| "native download client could not be initialized".to_string())
            })
            .as_ref()
            .map_err(|message| ApiError::Network(message.clone()))
    }

    let url = format!("{}{}", ctx.base_url, path);
    let mut request = download_client()?.get(url);
    if !ctx.id_token.is_empty() {
        request = request.header("authorization", format!("Bearer {}", ctx.id_token));
    }
    if let Some(tenant_id) = selected_workspace_id() {
        request = request.header(TENANT_HEADER, tenant_id);
    }
    let mut response = request
        .send()
        .await
        .map_err(|_| ApiError::Network("The export could not be downloaded.".into()))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        const MAX_ERROR_BYTES: usize = 64 * 1024;
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ApiError::Network("The export error could not be read.".into()))?
        {
            let remaining = MAX_ERROR_BYTES.saturating_sub(body.len());
            body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            if body.len() == MAX_ERROR_BYTES {
                break;
            }
        }
        let message = String::from_utf8_lossy(&body).into_owned();
        return Err(ApiError::Status(status, message));
    }
    if response
        .content_length()
        .is_some_and(|length| length > platform_bridge::native_files::MAX_EXPORT_BYTES as u64)
    {
        return Err(ApiError::Network(
            "The export is too large to save safely.".into(),
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ApiError::Network("The export could not be read.".into()))?
    {
        if bytes.len().saturating_add(chunk.len()) > platform_bridge::native_files::MAX_EXPORT_BYTES
        {
            return Err(ApiError::Network(
                "The export is too large to save safely.".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    platform_bridge::native_files::save_bytes(suggested_filename, mime_type, &bytes)
        .await
        .map_err(|error| ApiError::Network(error.to_string()))
}

// --- Tenant branding (admin theming + per-caller runtime theming) ---
//
// Mirrors the backend BRANDING contract EXACTLY. `logo_url` is an absolute or
// same-origin URL to the tenant's brand mark; `primary_color` / `accent_color`
// are CSS color strings (typically `#rrggbb` hex). Every field is `Option` so
// an unset field falls back to the default AulaLite theme.

/// `GET /v1/admin/branding` / `GET /v1/me/branding` shape. A field that is
/// `None` means "use the default" — the runtime theming only overrides tokens
/// for fields that are `Some`.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct BrandingDto {
    pub logo_url: Option<String>,
    pub primary_color: Option<String>,
    pub accent_color: Option<String>,
}

/// Body for `PATCH /v1/admin/branding`. Only fields that are `Some` are sent so
/// the backend leaves omitted fields untouched (matches the partial-patch DTOs
/// elsewhere in this module).
#[derive(serde::Serialize, Default)]
pub struct PatchBrandingBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent_color: Option<String>,
}

/// `GET /v1/admin/branding` — the active tenant's branding. Org-admin /
/// platform-admin only (403 otherwise).
pub async fn get_admin_branding(ctx: &ApiContext) -> Result<BrandingDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/admin/branding", None::<&()>).await
}

/// `PATCH /v1/admin/branding` — update the active tenant's branding. Only the
/// provided fields are sent. Org-admin / platform-admin only.
pub async fn patch_admin_branding(
    ctx: &ApiContext,
    logo_url: Option<String>,
    primary_color: Option<String>,
    accent_color: Option<String>,
) -> Result<BrandingDto, ApiError> {
    let body = PatchBrandingBody {
        logo_url,
        primary_color,
        accent_color,
    };
    fetch_json(ctx, "PATCH", "/v1/admin/branding", Some(&body)).await
}

/// `GET /v1/me/branding` — the caller's active-tenant branding. Any authed
/// user; drives runtime theming in the app shell.
pub async fn get_my_branding(ctx: &ApiContext) -> Result<BrandingDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/me/branding", None::<&()>).await
}

/// Per-user UI preferences. Mirrors the backend `PreferencesResponse`:
/// `theme` ∈ {system, light, dark}, `density` ∈ {compact, comfortable,
/// spacious}; `tour_dismissed` gates the first-run guided tour.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct PreferencesDto {
    pub theme: String,
    pub density: String,
    #[serde(default)]
    pub tour_dismissed: bool,
}

#[derive(serde::Serialize)]
struct PreferencesPatchBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    density: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tour_dismissed: Option<bool>,
}

/// `GET /v1/me/preferences` — the caller's persisted UI preferences.
pub async fn get_my_preferences(ctx: &ApiContext) -> Result<PreferencesDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/me/preferences", None::<&()>).await
}

/// `PATCH /v1/me/preferences` — update theme and/or density; unset fields are
/// left unchanged server-side.
pub async fn patch_my_preferences(
    ctx: &ApiContext,
    theme: Option<String>,
    density: Option<String>,
) -> Result<PreferencesDto, ApiError> {
    let body = PreferencesPatchBody {
        theme,
        density,
        tour_dismissed: None,
    };
    fetch_json(ctx, "PATCH", "/v1/me/preferences", Some(&body)).await
}

/// `PATCH /v1/me/preferences` — permanently dismiss the first-run tour.
pub async fn dismiss_tour(ctx: &ApiContext) -> Result<PreferencesDto, ApiError> {
    let body = PreferencesPatchBody {
        theme: None,
        density: None,
        tour_dismissed: Some(true),
    };
    fetch_json(ctx, "PATCH", "/v1/me/preferences", Some(&body)).await
}

// --- Lesson progress (learning-suite Cycle 2) ---

/// The caller's progress in one course. Mirrors the backend
/// `CourseProgressResponse`.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct CourseProgressDto {
    pub course_id: String,
    pub completed_lesson_ids: Vec<String>,
    pub completed: i64,
    pub total: i64,
    pub resume_lesson_id: Option<String>,
    pub resume_lesson_title: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct LessonCompletionDto {
    pub lesson_id: String,
    pub completed: bool,
}

/// Per-course progress rollup for every course the caller is an active
/// student of. Mirrors the backend `MyCourseProgress`.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct MyCourseProgressDto {
    pub course_id: String,
    pub slug: String,
    pub title: String,
    pub completed: i64,
    pub total: i64,
    pub last_activity_at: Option<String>,
    pub resume_lesson_id: Option<String>,
    pub resume_lesson_title: Option<String>,
}

/// Teacher rollup row. Mirrors the backend `StudentProgressRow`.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct StudentProgressDto {
    pub user_id: String,
    pub display_name: Option<String>,
    pub email: String,
    pub completed: i64,
    pub total: i64,
}

/// `PUT|DELETE /v1/courses/:cid/lessons/:lid/completion` — mark or unmark a
/// lesson complete for the caller (enrolled students).
pub async fn set_lesson_completion(
    ctx: &ApiContext,
    course_id: &str,
    lesson_id: &str,
    completed: bool,
) -> Result<LessonCompletionDto, ApiError> {
    let method = if completed { "PUT" } else { "DELETE" };
    fetch_json(
        ctx,
        method,
        &format!("/v1/courses/{course_id}/lessons/{lesson_id}/completion"),
        None::<&()>,
    )
    .await
}

/// `GET /v1/courses/:cid/progress` — the caller's own progress in a course.
pub async fn get_course_progress(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<CourseProgressDto, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/progress"),
        None::<&()>,
    )
    .await
}

/// `GET /v1/me/progress` — progress in every enrolled course.
pub async fn get_my_progress(ctx: &ApiContext) -> Result<Vec<MyCourseProgressDto>, ApiError> {
    fetch_json(ctx, "GET", "/v1/me/progress", None::<&()>).await
}

/// `GET /v1/courses/:cid/progress/students` — staff: per-student rollup.
pub async fn get_course_student_progress(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<Vec<StudentProgressDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/progress/students"),
        None::<&()>,
    )
    .await
}

// --- Quizzes (learning-suite Cycle 3) ---
//
// The prompt/answer JSON is the shared `core_types::quiz` vocabulary
// (tagged `kind` for prompts, `{type, value}` for answers). Students get
// `student_questions` (keys stripped); staff get `questions` with keys.

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct QuizDto {
    pub id: String,
    pub course_id: String,
    pub module_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub mode: String,
    pub time_limit_seconds: Option<i32>,
    pub max_attempts: Option<i32>,
    pub status: String,
    pub question_count: i64,
    pub my_submitted_attempts: i64,
    pub my_best_score: Option<i32>,
    pub my_best_max: Option<i32>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct AuthoringQuestionDto {
    #[serde(default)]
    pub id: Option<String>,
    pub prompt_text: String,
    pub prompt: core_types::quiz::QuizPrompt,
    #[serde(default)]
    pub explanation: String,
    pub points: i32,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct StudentQuestionDto {
    pub id: String,
    pub prompt_text: String,
    pub prompt: core_types::quiz::StudentQuizPrompt,
    pub points: i32,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct QuizDetailDto {
    #[serde(flatten)]
    pub quiz: QuizDto,
    #[serde(default)]
    pub questions: Option<Vec<AuthoringQuestionDto>>,
    #[serde(default)]
    pub student_questions: Option<Vec<StudentQuestionDto>>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct QuizAttemptDto {
    pub id: String,
    pub quiz_id: String,
    pub started_at: String,
    pub submitted_at: Option<String>,
    pub score_points: Option<i32>,
    pub max_points: Option<i32>,
    pub deadline: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct QuizSubmitResultDto {
    pub attempt_id: String,
    pub score_points: i32,
    pub max_points: i32,
    pub per_question: Vec<QuizPerQuestionDto>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct QuizPerQuestionDto {
    pub question_id: String,
    pub correct: bool,
    pub points_awarded: i32,
    pub points: i32,
}

#[derive(serde::Serialize)]
pub struct CreateQuizBody {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_limit_seconds: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<i32>,
}

pub async fn list_quizzes(ctx: &ApiContext, course_id: &str) -> Result<Vec<QuizDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/quizzes"),
        None::<&()>,
    )
    .await
}

pub async fn get_quiz(
    ctx: &ApiContext,
    course_id: &str,
    quiz_id: &str,
) -> Result<QuizDetailDto, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/quizzes/{quiz_id}"),
        None::<&()>,
    )
    .await
}

pub async fn create_quiz(
    ctx: &ApiContext,
    course_id: &str,
    body: &CreateQuizBody,
) -> Result<QuizDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/quizzes"),
        Some(body),
    )
    .await
}

pub async fn patch_quiz(
    ctx: &ApiContext,
    course_id: &str,
    quiz_id: &str,
    body: &serde_json::Value,
) -> Result<QuizDto, ApiError> {
    fetch_json(
        ctx,
        "PATCH",
        &format!("/v1/courses/{course_id}/quizzes/{quiz_id}"),
        Some(body),
    )
    .await
}

pub async fn delete_quiz(
    ctx: &ApiContext,
    course_id: &str,
    quiz_id: &str,
) -> Result<serde_json::Value, ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/courses/{course_id}/quizzes/{quiz_id}"),
        None::<&()>,
    )
    .await
}

pub async fn replace_quiz_questions(
    ctx: &ApiContext,
    course_id: &str,
    quiz_id: &str,
    questions: &[AuthoringQuestionDto],
) -> Result<serde_json::Value, ApiError> {
    fetch_json(
        ctx,
        "PUT",
        &format!("/v1/courses/{course_id}/quizzes/{quiz_id}/questions"),
        Some(&questions),
    )
    .await
}

pub async fn start_quiz_attempt(
    ctx: &ApiContext,
    course_id: &str,
    quiz_id: &str,
) -> Result<QuizAttemptDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/quizzes/{quiz_id}/attempts"),
        None::<&()>,
    )
    .await
}

pub async fn submit_quiz_attempt(
    ctx: &ApiContext,
    course_id: &str,
    quiz_id: &str,
    attempt_id: &str,
    answers: &serde_json::Value,
) -> Result<QuizSubmitResultDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/quizzes/{quiz_id}/attempts/{attempt_id}/submit"),
        Some(answers),
    )
    .await
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct QuizResultRowDto {
    pub user_id: String,
    pub display_name: Option<String>,
    pub email: String,
    pub attempts: i64,
    pub best_score: Option<i32>,
    pub best_max: Option<i32>,
}

pub async fn get_quiz_results(
    ctx: &ApiContext,
    course_id: &str,
    quiz_id: &str,
) -> Result<Vec<QuizResultRowDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/quizzes/{quiz_id}/results"),
        None::<&()>,
    )
    .await
}

// --- Gamification (learning-suite Cycle 4) ---

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct AchievementUnlockDto {
    pub id: String,
    pub title: String,
    pub description: String,
    pub seen: bool,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct GamificationDto {
    pub total_xp: i64,
    pub level: u32,
    pub xp_into_level: i64,
    pub level_span: i64,
    pub current_streak_days: i32,
    pub longest_streak_days: i32,
    pub streak_active_today: bool,
    pub unlocks: Vec<AchievementUnlockDto>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct LeaderboardRowDto {
    pub user_id: String,
    pub display_name: String,
    pub course_xp: i64,
    pub is_me: bool,
}

pub async fn get_my_gamification(ctx: &ApiContext) -> Result<GamificationDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/me/gamification", None::<&()>).await
}

pub async fn mark_gamification_seen(ctx: &ApiContext) -> Result<serde_json::Value, ApiError> {
    fetch_json(ctx, "POST", "/v1/me/gamification/seen", None::<&()>).await
}

pub async fn get_course_leaderboard(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<Vec<LeaderboardRowDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/leaderboard"),
        None::<&()>,
    )
    .await
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct LeaderboardOptOutDto {
    pub opted_out: bool,
}

pub async fn get_leaderboard_opt_out(ctx: &ApiContext) -> Result<LeaderboardOptOutDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/me/leaderboard-opt-out", None::<&()>).await
}

pub async fn put_leaderboard_opt_out(
    ctx: &ApiContext,
    opted_out: bool,
) -> Result<LeaderboardOptOutDto, ApiError> {
    fetch_json(
        ctx,
        "PUT",
        "/v1/me/leaderboard-opt-out",
        Some(&LeaderboardOptOutDto { opted_out }),
    )
    .await
}

// --- Certificates (learning-suite Cycle 5) ---
//
// Field names mirror the backend `CertificateDto`/`VerifyDto` exactly.
// `/v1/verify/:credential_id` is PUBLIC — it works without a bearer token.

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct CertificateDto {
    pub id: String,
    pub course_id: String,
    pub user_id: String,
    pub credential_id: Option<String>,
    pub status: String,
    pub recipient_name: Option<String>,
    pub course_title: Option<String>,
    pub student_display_name: Option<String>,
    pub student_email: String,
    pub issued_at: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct CertificateVerifyDto {
    pub credential_id: String,
    pub status: String,
    pub recipient_name: Option<String>,
    pub course_title: Option<String>,
    pub issued_at: Option<String>,
}

/// `GET /v1/courses/{course_id}/certificates` — staff: eligible + issued list.
pub async fn get_course_certificates(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<Vec<CertificateDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/certificates"),
        None::<&()>,
    )
    .await
}

/// `POST /v1/courses/{course_id}/certificates/{user_id}/issue` — staff.
pub async fn issue_certificate(
    ctx: &ApiContext,
    course_id: &str,
    user_id: &str,
) -> Result<CertificateDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/certificates/{user_id}/issue"),
        None::<&()>,
    )
    .await
}

/// `POST /v1/courses/{course_id}/certificates/{user_id}/revoke` — staff.
pub async fn revoke_certificate(
    ctx: &ApiContext,
    course_id: &str,
    user_id: &str,
) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/certificates/{user_id}/revoke"),
        None::<&()>,
    )
    .await
}

/// `GET /v1/me/certificates` — the caller's own certificates.
pub async fn get_my_certificates(ctx: &ApiContext) -> Result<Vec<CertificateDto>, ApiError> {
    fetch_json(ctx, "GET", "/v1/me/certificates", None::<&()>).await
}

/// `GET /v1/verify/{credential_id}` — public credential verification.
pub async fn verify_certificate(
    ctx: &ApiContext,
    credential_id: &str,
) -> Result<CertificateVerifyDto, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/verify/{credential_id}"),
        None::<&()>,
    )
    .await
}

// --- Flashcards (learning-suite Cycle 6) ---
//
// Field names mirror the backend flashcards DTOs exactly. SM-2 scheduling is
// server-authoritative; the response carries the new state for display only.

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct FlashcardDeckDto {
    pub id: String,
    pub course_id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub card_count: i64,
    pub created_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct FlashcardCardDto {
    pub id: String,
    pub deck_id: String,
    pub position: i32,
    pub front: String,
    pub back: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ReviewCardDto {
    pub id: String,
    pub front: String,
    pub back: String,
    pub repetitions: Option<i32>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ReviewOutcomeDto {
    pub card_id: String,
    pub ease: f32,
    pub interval_days: f32,
    pub repetitions: i32,
}

pub async fn list_flashcard_decks(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<Vec<FlashcardDeckDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/decks"),
        None::<&()>,
    )
    .await
}

#[derive(serde::Serialize)]
struct CreateDeckBody<'a> {
    title: &'a str,
    description: Option<&'a str>,
}

pub async fn create_flashcard_deck(
    ctx: &ApiContext,
    course_id: &str,
    title: &str,
    description: Option<&str>,
) -> Result<FlashcardDeckDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/decks"),
        Some(&CreateDeckBody { title, description }),
    )
    .await
}

#[derive(serde::Serialize)]
struct PatchDeckBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'a str>,
}

pub async fn patch_flashcard_deck(
    ctx: &ApiContext,
    course_id: &str,
    deck_id: &str,
    title: Option<&str>,
    description: Option<&str>,
    status: Option<&str>,
) -> Result<FlashcardDeckDto, ApiError> {
    fetch_json(
        ctx,
        "PATCH",
        &format!("/v1/courses/{course_id}/decks/{deck_id}"),
        Some(&PatchDeckBody {
            title,
            description,
            status,
        }),
    )
    .await
}

pub async fn delete_flashcard_deck(
    ctx: &ApiContext,
    course_id: &str,
    deck_id: &str,
) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/courses/{course_id}/decks/{deck_id}"),
        None::<&()>,
    )
    .await
}

pub async fn list_flashcards(
    ctx: &ApiContext,
    course_id: &str,
    deck_id: &str,
) -> Result<Vec<FlashcardCardDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/decks/{deck_id}/cards"),
        None::<&()>,
    )
    .await
}

#[derive(serde::Serialize)]
struct CardBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    front: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    back: Option<&'a str>,
}

pub async fn create_flashcard(
    ctx: &ApiContext,
    course_id: &str,
    deck_id: &str,
    front: &str,
    back: &str,
) -> Result<FlashcardCardDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/decks/{deck_id}/cards"),
        Some(&CardBody {
            front: Some(front),
            back: Some(back),
        }),
    )
    .await
}

pub async fn patch_flashcard(
    ctx: &ApiContext,
    course_id: &str,
    deck_id: &str,
    card_id: &str,
    front: Option<&str>,
    back: Option<&str>,
) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "PATCH",
        &format!("/v1/courses/{course_id}/decks/{deck_id}/cards/{card_id}"),
        Some(&CardBody { front, back }),
    )
    .await
}

pub async fn delete_flashcard(
    ctx: &ApiContext,
    course_id: &str,
    deck_id: &str,
    card_id: &str,
) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/courses/{course_id}/decks/{deck_id}/cards/{card_id}"),
        None::<&()>,
    )
    .await
}

pub async fn get_review_queue(
    ctx: &ApiContext,
    course_id: &str,
    deck_id: &str,
) -> Result<Vec<ReviewCardDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/decks/{deck_id}/review"),
        None::<&()>,
    )
    .await
}

#[derive(serde::Serialize)]
struct RatingBody<'a> {
    rating: &'a str,
}

pub async fn record_flashcard_review(
    ctx: &ApiContext,
    course_id: &str,
    deck_id: &str,
    card_id: &str,
    rating: &str,
) -> Result<ReviewOutcomeDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/courses/{course_id}/decks/{deck_id}/review/{card_id}"),
        Some(&RatingBody { rating }),
    )
    .await
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct DueCountDto {
    pub due: i64,
}

pub async fn get_flashcards_due_count(
    ctx: &ApiContext,
    course_id: &str,
) -> Result<DueCountDto, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/courses/{course_id}/flashcards/due-count"),
        None::<&()>,
    )
    .await
}

// --- Notifications (self-service: /v1/me/notifications + preferences + device-tokens) ---
//
// Mirrors the backend NOTIFICATIONS contract in `crates/backend` EXACTLY. The
// backend serializes `id` (Uuid) and the timestamps (`DateTime<Utc>`) as JSON
// strings, counts as i64. All endpoints are authed and caller-scoped.

/// A single notification. Mirrors the backend `NotificationDto`. `body` and
/// `link` are optional; `read_at` is `None` for unread notifications.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct NotificationDto {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub body: Option<String>,
    pub link: Option<String>,
    pub created_at: String,
    pub read_at: Option<String>,
}

/// Per-user notification channel preferences. Mirrors the backend `PrefDto`;
/// defaults are all-true server-side when no row exists.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct PrefDto {
    pub email_enabled: bool,
    pub push_enabled: bool,
    pub in_app_enabled: bool,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
struct UnreadCountDto {
    count: i64,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
struct ReadAllDto {
    updated: i64,
}

/// `GET /v1/me/notifications?limit=` — the caller's own notifications, newest
/// first. `limit` is clamped server-side to 1..=100 (default 50).
pub async fn list_notifications(
    ctx: &ApiContext,
    limit: Option<i64>,
) -> Result<Vec<NotificationDto>, ApiError> {
    let path = match limit {
        Some(l) => format!("/v1/me/notifications?limit={l}"),
        None => "/v1/me/notifications".to_string(),
    };
    fetch_json(ctx, "GET", &path, None::<&()>).await
}

/// `GET /v1/me/notifications/unread-count` — number of unread notifications.
pub async fn notifications_unread_count(ctx: &ApiContext) -> Result<i64, ApiError> {
    let dto: UnreadCountDto =
        fetch_json(ctx, "GET", "/v1/me/notifications/unread-count", None::<&()>).await?;
    Ok(dto.count)
}

/// `POST /v1/me/notifications/:id/read` — mark one notification read. Returns
/// 204 (decoded as `()`); 404 if not owner / already read.
pub async fn mark_notification_read(ctx: &ApiContext, id: &str) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/me/notifications/{id}/read"),
        None::<&()>,
    )
    .await
}

/// `POST /v1/me/notifications/read-all` — mark all unread read. Returns the
/// number of rows updated.
pub async fn mark_all_notifications_read(ctx: &ApiContext) -> Result<i64, ApiError> {
    let dto: ReadAllDto = fetch_json(
        ctx,
        "POST",
        "/v1/me/notifications/read-all",
        Some(&serde_json::json!({})),
    )
    .await?;
    Ok(dto.updated)
}

/// `GET /v1/me/notification-preferences` — defaults all-true if no row.
pub async fn get_notification_preferences(ctx: &ApiContext) -> Result<PrefDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/me/notification-preferences", None::<&()>).await
}

#[derive(serde::Serialize)]
struct PatchPrefBody {
    email_enabled: bool,
    push_enabled: bool,
    in_app_enabled: bool,
}

/// `PATCH /v1/me/notification-preferences` — set all three channel toggles.
/// Sends every field so the backend's partial patch applies the full state.
pub async fn set_notification_preferences(
    ctx: &ApiContext,
    email_enabled: bool,
    push_enabled: bool,
    in_app_enabled: bool,
) -> Result<PrefDto, ApiError> {
    let body = PatchPrefBody {
        email_enabled,
        push_enabled,
        in_app_enabled,
    };
    fetch_json(ctx, "PATCH", "/v1/me/notification-preferences", Some(&body)).await
}

#[derive(serde::Serialize)]
struct DeviceTokenBody<'a> {
    token: &'a str,
    platform: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_agent: Option<&'a str>,
}

#[derive(serde::Serialize)]
struct RemoveDeviceTokenBody<'a> {
    token: &'a str,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct DeviceTokenDto {
    pub id: String,
    pub platform: String,
    pub label: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: String,
    pub last_seen_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct DeviceTokenListDto {
    pub devices: Vec<DeviceTokenDto>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct DeliveryDto {
    pub id: String,
    pub user_id: String,
    pub notification_id: Option<String>,
    pub channel: String,
    pub provider: String,
    pub target_hash: String,
    pub target_label: Option<String>,
    pub device_token_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub provider_message_id: Option<String>,
    pub provider_status: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct DeliveryListDto {
    pub deliveries: Vec<DeliveryDto>,
}

/// `POST /v1/me/device-tokens` — register (upsert) a push device token. Wired
/// for later FCM web-push; the service-worker registration is out of scope.
/// `platform` is one of "web" | "ios" | "android".
pub async fn register_device_token(
    ctx: &ApiContext,
    token: &str,
    platform: &str,
) -> Result<(), ApiError> {
    register_device_token_with_metadata(ctx, token, platform, None, None).await
}

pub async fn register_device_token_with_metadata(
    ctx: &ApiContext,
    token: &str,
    platform: &str,
    label: Option<&str>,
    user_agent: Option<&str>,
) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "POST",
        "/v1/me/device-tokens",
        Some(&DeviceTokenBody {
            token,
            platform,
            label,
            user_agent,
        }),
    )
    .await
}

pub async fn list_device_tokens(ctx: &ApiContext) -> Result<DeviceTokenListDto, ApiError> {
    fetch_json(ctx, "GET", "/v1/me/device-tokens", None::<&()>).await
}

pub async fn revoke_device_token(ctx: &ApiContext, id: &str) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/me/device-tokens/{id}"),
        None::<&()>,
    )
    .await
}

/// Token-based removal is used during APNs/FCM rotation because the backend's
/// public device list intentionally never exposes raw push tokens.
pub async fn remove_device_token(ctx: &ApiContext, token: &str) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        "/v1/me/device-tokens",
        Some(&RemoveDeviceTokenBody { token }),
    )
    .await
}

pub async fn list_notification_deliveries(
    ctx: &ApiContext,
    channel: Option<&str>,
    status: Option<&str>,
    limit: Option<i64>,
) -> Result<DeliveryListDto, ApiError> {
    let mut path = String::from("/v1/admin/notification-deliveries");
    let mut parts = Vec::<String>::new();
    if let Some(value) = channel {
        parts.push(format!("channel={}", urlencode(value)));
    }
    if let Some(value) = status {
        parts.push(format!("status={}", urlencode(value)));
    }
    if let Some(value) = limit {
        parts.push(format!("limit={value}"));
    }
    if !parts.is_empty() {
        path.push('?');
        path.push_str(&parts.join("&"));
    }
    fetch_json(ctx, "GET", &path, None::<&()>).await
}

pub async fn get_notification_delivery(
    ctx: &ApiContext,
    id: &str,
) -> Result<DeliveryDto, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/admin/notification-deliveries/{id}"),
        None::<&()>,
    )
    .await
}

// --- Admin integrations: API keys, webhooks, SSO, and LTI ---

pub const ADMIN_API_KEY_SCOPES: &[&str] =
    &["read", "courses:read", "roster:read", "grades:read", "*"];

pub const ADMIN_WEBHOOK_EVENTS: &[&str] = &[
    "course.created",
    "course.updated",
    "course.published",
    "enrollment.created",
    "assignment.published",
    "submission.graded",
    "announcement.created",
];

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct AdminApiKeyDto {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub scopes: Vec<String>,
    pub created_by: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct MintedAdminApiKeyDto {
    #[serde(flatten)]
    pub key: AdminApiKeyDto,
    pub plaintext: String,
}

#[derive(serde::Serialize)]
struct MintAdminApiKeyBody<'a> {
    name: &'a str,
    scopes: Vec<String>,
}

pub async fn list_admin_api_keys(ctx: &ApiContext) -> Result<Vec<AdminApiKeyDto>, ApiError> {
    fetch_json(ctx, "GET", "/v1/admin/api-keys", None::<&()>).await
}

pub async fn mint_admin_api_key(
    ctx: &ApiContext,
    name: &str,
    scopes: &[String],
) -> Result<MintedAdminApiKeyDto, ApiError> {
    let body = MintAdminApiKeyBody {
        name,
        scopes: scopes.to_vec(),
    };
    fetch_json(ctx, "POST", "/v1/admin/api-keys", Some(&body)).await
}

pub async fn revoke_admin_api_key(ctx: &ApiContext, id: &str) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/admin/api-keys/{id}"),
        None::<&()>,
    )
    .await
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct AdminWebhookSubscriptionDto {
    pub id: String,
    pub url: String,
    pub events: Vec<String>,
    pub active: bool,
    pub created_at: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct CreatedAdminWebhookSubscriptionDto {
    #[serde(flatten)]
    pub subscription: AdminWebhookSubscriptionDto,
    pub secret: String,
}

#[derive(serde::Serialize)]
struct CreateAdminWebhookSubscriptionBody<'a> {
    url: &'a str,
    events: Vec<String>,
}

#[derive(Clone, Debug, Default, serde::Serialize, PartialEq)]
pub struct UpdateAdminWebhookSubscriptionBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct AdminWebhookDeliveryDto {
    pub id: String,
    pub subscription_id: String,
    pub event: String,
    pub payload_json: serde_json::Value,
    pub status: String,
    pub attempts: i32,
    pub last_attempt_at: Option<String>,
    pub response_code: Option<i32>,
    pub created_at: String,
}

pub async fn list_admin_webhooks(
    ctx: &ApiContext,
) -> Result<Vec<AdminWebhookSubscriptionDto>, ApiError> {
    fetch_json(ctx, "GET", "/v1/admin/webhooks", None::<&()>).await
}

pub async fn create_admin_webhook(
    ctx: &ApiContext,
    url: &str,
    events: &[String],
) -> Result<CreatedAdminWebhookSubscriptionDto, ApiError> {
    let body = CreateAdminWebhookSubscriptionBody {
        url,
        events: events.to_vec(),
    };
    fetch_json(ctx, "POST", "/v1/admin/webhooks", Some(&body)).await
}

pub async fn update_admin_webhook(
    ctx: &ApiContext,
    id: &str,
    body: &UpdateAdminWebhookSubscriptionBody,
) -> Result<AdminWebhookSubscriptionDto, ApiError> {
    fetch_json(
        ctx,
        "PATCH",
        &format!("/v1/admin/webhooks/{id}"),
        Some(body),
    )
    .await
}

pub async fn delete_admin_webhook(ctx: &ApiContext, id: &str) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/admin/webhooks/{id}"),
        None::<&()>,
    )
    .await
}

pub async fn list_admin_webhook_deliveries(
    ctx: &ApiContext,
    subscription_id: Option<&str>,
    limit: Option<i64>,
) -> Result<Vec<AdminWebhookDeliveryDto>, ApiError> {
    let mut path = match subscription_id {
        Some(id) => format!("/v1/admin/webhooks/{id}/deliveries"),
        None => "/v1/admin/webhooks/deliveries".to_string(),
    };
    if let Some(value) = limit {
        path.push_str(&format!("?limit={value}"));
    }
    fetch_json(ctx, "GET", &path, None::<&()>).await
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct AdminSsoConfigDto {
    pub issuer: String,
    pub client_id: String,
    pub authorize_url: String,
    pub token_url: String,
    pub jwks_url: String,
    pub enabled: bool,
    pub has_client_secret: bool,
    pub login_url: String,
}

#[derive(Clone, Debug, serde::Serialize, PartialEq)]
pub struct UpsertAdminSsoConfigBody {
    pub issuer: String,
    pub client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    pub authorize_url: String,
    pub token_url: String,
    pub jwks_url: String,
    pub enabled: bool,
}

pub async fn get_admin_sso_config(ctx: &ApiContext) -> Result<Option<AdminSsoConfigDto>, ApiError> {
    fetch_json(ctx, "GET", "/v1/admin/sso", None::<&()>).await
}

pub async fn upsert_admin_sso_config(
    ctx: &ApiContext,
    body: &UpsertAdminSsoConfigBody,
) -> Result<AdminSsoConfigDto, ApiError> {
    fetch_json(ctx, "PUT", "/v1/admin/sso", Some(body)).await
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct AdminLtiPlatformDto {
    pub id: String,
    pub name: String,
    pub issuer: String,
    pub client_id: String,
    pub auth_login_url: String,
    pub jwks_url: String,
    pub deployment_id: String,
    pub default_course_id: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, serde::Serialize, PartialEq)]
pub struct RegisterAdminLtiPlatformBody {
    pub name: String,
    pub issuer: String,
    pub client_id: String,
    pub auth_login_url: String,
    pub jwks_url: String,
    pub deployment_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_course_id: Option<String>,
}

pub async fn list_admin_lti_platforms(
    ctx: &ApiContext,
) -> Result<Vec<AdminLtiPlatformDto>, ApiError> {
    fetch_json(ctx, "GET", "/v1/admin/lti/platforms", None::<&()>).await
}

pub async fn register_admin_lti_platform(
    ctx: &ApiContext,
    body: &RegisterAdminLtiPlatformBody,
) -> Result<AdminLtiPlatformDto, ApiError> {
    fetch_json(ctx, "POST", "/v1/admin/lti/platforms", Some(body)).await
}

pub async fn delete_admin_lti_platform(ctx: &ApiContext, id: &str) -> Result<(), ApiError> {
    fetch_json(
        ctx,
        "DELETE",
        &format!("/v1/admin/lti/platforms/{id}"),
        None::<&()>,
    )
    .await
}

// --- Global search (courses + assignments) ---
//
// Mirrors the backend SEARCH contract EXACTLY. The backend serializes `id` /
// `course_id` (Uuid) as JSON strings. Any authed user may call it; the backend
// scopes visibility to what the caller can see.

/// One course search hit. Mirrors the backend `CourseHitDto`.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct CourseHitDto {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub status: String,
}

/// One assignment search hit. Mirrors the backend `AssignmentHitDto`.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct AssignmentHitDto {
    pub id: String,
    pub course_id: String,
    pub course_slug: String,
    pub title: String,
    pub status: String,
}

/// Response shape of `GET /v1/search`. Mirrors the backend `SearchResponseDto`.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct SearchResponseDto {
    pub courses: Vec<CourseHitDto>,
    pub assignments: Vec<AssignmentHitDto>,
}

/// `GET /v1/search?q=..&limit=..` — global search across courses and
/// assignments. `q` is URL-encoded; `limit` caps each result section. Any
/// authed user; the backend scopes visibility.
pub async fn search(ctx: &ApiContext, q: &str, limit: u32) -> Result<SearchResponseDto, ApiError> {
    let path = format!("/v1/search?q={}&limit={limit}", urlencode(q));
    fetch_json(ctx, "GET", &path, None::<&()>).await
}

/// Read the initial search query from the current URL's `?q=` parameter.
/// Parses `window.location.search` on wasm (form-decoding `+` → space and
/// percent-escapes); returns an empty string off-wasm (SSR / tests) or when no
/// `q` is present. Lives here because `features-courses` already enables the
/// `web_sys` `Location` feature and depends on `urlencoding`.
pub fn initial_search_query() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(win) = web_sys::window() else {
            return String::new();
        };
        let Ok(search) = win.location().search() else {
            return String::new();
        };
        // `search` is like "?q=foo+bar&limit=20". Strip the leading '?'.
        let trimmed = search.trim_start_matches('?');
        for pair in trimmed.split('&') {
            let mut it = pair.splitn(2, '=');
            if it.next() == Some("q") {
                let raw = it.next().unwrap_or("");
                let plus_decoded = raw.replace('+', " ");
                return urlencoding::decode(&plus_decoded)
                    .map(|c| c.into_owned())
                    .unwrap_or(plus_decoded);
            }
        }
        String::new()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        String::new()
    }
}

// --- Platform super-admin: tenant directory + provisioning ---
//
// Mirrors the backend `TenantSummaryDto`. Uuid (`id`) and DateTime
// (`created_at`) serialize as JSON strings; `member_count` is a JSON number
// (i64); `plan_id` is null when the tenant has no plan. All three endpoints
// require the signed-in user to be a platform admin; the backend returns 403 to
// everyone else, so callers should treat `ApiError::Status(403, _)` as
// "not visible" rather than a hard error.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct TenantSummaryDto {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub status: String,
    pub created_at: String,
    pub member_count: i64,
    pub plan_id: Option<String>,
}

#[derive(serde::Serialize)]
struct CreateTenantBody<'a> {
    slug: &'a str,
    name: &'a str,
    admin_email: &'a str,
}

#[derive(serde::Serialize)]
struct SetTenantStatusBody<'a> {
    status: &'a str,
}

/// `GET /v1/platform/tenants` — full tenant directory. Platform-admin only
/// (403 otherwise).
pub async fn platform_list_tenants(ctx: &ApiContext) -> Result<Vec<TenantSummaryDto>, ApiError> {
    fetch_json(ctx, "GET", "/v1/platform/tenants", None::<&()>).await
}

/// `POST /v1/platform/tenants` — provision a new tenant and designate its
/// initial organization owner with `admin_email`. Platform-admin only.
pub async fn platform_create_tenant(
    ctx: &ApiContext,
    slug: &str,
    name: &str,
    admin_email: &str,
) -> Result<TenantSummaryDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        "/v1/platform/tenants",
        Some(&CreateTenantBody {
            slug,
            name,
            admin_email,
        }),
    )
    .await
}

/// `PATCH /v1/platform/tenants/{id}` — flip a tenant's lifecycle status.
/// `status` must be one of `active` | `suspended` | `trialing`. Platform-admin
/// only (403 otherwise).
pub async fn platform_set_tenant_status(
    ctx: &ApiContext,
    id: &str,
    status: &str,
) -> Result<TenantSummaryDto, ApiError> {
    fetch_json(
        ctx,
        "PATCH",
        &format!("/v1/platform/tenants/{id}"),
        Some(&SetTenantStatusBody { status }),
    )
    .await
}

pub const OWNERSHIP_RECOVERY_CONFIRMATION: &str = "RECOVER OWNERSHIP";

#[derive(Clone, Debug, serde::Deserialize, PartialEq, Eq)]
pub struct OwnershipCandidateDto {
    pub user_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub role: String,
}

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct RecoverOwnershipBody {
    pub new_owner_user_id: String,
    pub confirmation: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq, Eq)]
pub struct RecoverOwnershipResponseDto {
    pub previous_owner_user_id: Option<String>,
    pub new_owner_user_id: String,
    pub transferred_at: String,
}

/// Read active privileged members before showing the break-glass platform
/// recovery control. The recovery POST independently requires an active
/// organization administrator as the new owner.
pub async fn platform_list_ownership_candidates(
    ctx: &ApiContext,
    tenant_id: &str,
) -> Result<Vec<OwnershipCandidateDto>, ApiError> {
    fetch_json(
        ctx,
        "GET",
        &format!("/v1/platform/tenants/{tenant_id}/ownership-candidates"),
        None::<&()>,
    )
    .await
}

/// Break-glass platform recovery for a workspace whose ownership is unusable.
pub async fn platform_recover_tenant_owner(
    ctx: &ApiContext,
    tenant_id: &str,
    new_owner_user_id: &str,
    confirmation: &str,
) -> Result<RecoverOwnershipResponseDto, ApiError> {
    fetch_json(
        ctx,
        "POST",
        &format!("/v1/platform/tenants/{tenant_id}/recover-owner"),
        Some(&RecoverOwnershipBody {
            new_owner_user_id: new_owner_user_id.to_string(),
            confirmation: confirmation.to_string(),
        }),
    )
    .await
}

pub const OWNER_INVITATION_REPLACEMENT_CONFIRMATION: &str = "REPLACE OWNER INVITATION";

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct ReplaceOwnerInvitationBody {
    pub email: String,
    pub confirmation: String,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq, Eq)]
pub struct ReplaceOwnerInvitationResponseDto {
    pub invitation_id: String,
    pub email: String,
    pub created_at: String,
}

/// Correct a mistyped initial owner email while a newly provisioned workspace
/// has no owner membership and exactly one pending privileged invitation.
pub async fn platform_replace_pending_owner_invitation(
    ctx: &ApiContext,
    tenant_id: &str,
    email: &str,
    confirmation: &str,
) -> Result<ReplaceOwnerInvitationResponseDto, ApiError> {
    fetch_json(
        ctx,
        "PATCH",
        &format!("/v1/platform/tenants/{tenant_id}/pending-owner-invitation"),
        Some(&ReplaceOwnerInvitationBody {
            email: email.to_string(),
            confirmation: confirmation.to_string(),
        }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_confirmation_bodies_match_backend_contracts() {
        let transfer = TransferOwnershipBody {
            new_owner_user_id: "user-2".into(),
            confirmation: OWNERSHIP_TRANSFER_CONFIRMATION.into(),
        };
        assert_eq!(
            serde_json::to_value(transfer).unwrap(),
            serde_json::json!({
                "new_owner_user_id": "user-2",
                "confirmation": "TRANSFER OWNERSHIP"
            })
        );

        let recovery = RecoverOwnershipBody {
            new_owner_user_id: "user-3".into(),
            confirmation: OWNERSHIP_RECOVERY_CONFIRMATION.into(),
        };
        assert_eq!(
            serde_json::to_value(recovery).unwrap(),
            serde_json::json!({
                "new_owner_user_id": "user-3",
                "confirmation": "RECOVER OWNERSHIP"
            })
        );

        let replacement = ReplaceOwnerInvitationBody {
            email: "corrected@example.test".into(),
            confirmation: OWNER_INVITATION_REPLACEMENT_CONFIRMATION.into(),
        };
        assert_eq!(
            serde_json::to_value(replacement).unwrap(),
            serde_json::json!({
                "email": "corrected@example.test",
                "confirmation": "REPLACE OWNER INVITATION"
            })
        );
    }

    #[test]
    fn patch_course_body_preserves_cover_asset_double_option_semantics() {
        let omitted = PatchCourseBody {
            title: None,
            description: None,
            status: None,
            cover_asset_id: None,
            syllabus_md: None,
            grading_policy_md: None,
            self_enrollment_enabled: None,
        };
        assert_eq!(
            serde_json::to_value(&omitted).unwrap(),
            serde_json::json!({})
        );

        let cleared = PatchCourseBody {
            title: None,
            description: None,
            status: None,
            cover_asset_id: Some(None),
            syllabus_md: None,
            grading_policy_md: None,
            self_enrollment_enabled: None,
        };
        assert_eq!(
            serde_json::to_value(&cleared).unwrap(),
            serde_json::json!({ "cover_asset_id": null })
        );
    }

    #[test]
    fn update_assignment_body_sends_present_values_for_double_option_clear() {
        // pass/fail → max_points null; no due date → due_at null.
        let body = UpdateAssignmentBody {
            title: "Essay",
            instructions_md: "Write.",
            grading_mode: "pass_fail",
            max_points: None,
            allow_late: true,
            lock_on_submit: false,
            accepts_text: true,
            accepts_files: false,
            release_mode: "instant",
            late_penalty_percent: 0,
            max_resubmissions: 0,
            due_at: None,
            lesson_id: None,
        };
        let v = serde_json::to_value(&body).unwrap();
        // Fields are present (not omitted) so the backend's double_option sees
        // Some(None) → clear.
        assert!(v.get("max_points").unwrap().is_null());
        assert!(v.get("due_at").unwrap().is_null());
        assert!(v.get("lesson_id").unwrap().is_null());
        assert_eq!(v.get("grading_mode").unwrap(), "pass_fail");

        // numeric mode with a due date → definite values.
        let body2 = UpdateAssignmentBody {
            grading_mode: "numeric",
            max_points: Some(50),
            due_at: Some("2026-05-30T14:30:00Z".to_string()),
            ..body
        };
        let v2 = serde_json::to_value(&body2).unwrap();
        assert_eq!(v2.get("max_points").unwrap(), 50);
        assert_eq!(v2.get("due_at").unwrap(), "2026-05-30T14:30:00Z");
    }

    #[test]
    fn patch_module_body_omits_unset_fields() {
        let omitted = PatchModuleBody { title: None };
        assert_eq!(
            serde_json::to_value(&omitted).unwrap(),
            serde_json::json!({})
        );

        let with_title = PatchModuleBody {
            title: Some("Renamed module"),
        };
        assert_eq!(
            serde_json::to_value(&with_title).unwrap(),
            serde_json::json!({ "title": "Renamed module" })
        );
    }

    #[test]
    fn patch_lesson_body_omits_unset_fields() {
        let omitted = PatchLessonBody::default();
        assert_eq!(
            serde_json::to_value(&omitted).unwrap(),
            serde_json::json!({})
        );

        let with_title = PatchLessonBody {
            title: Some("Renamed lesson"),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&with_title).unwrap(),
            serde_json::json!({ "title": "Renamed lesson" })
        );

        // body_md and live_session_id round-trip when set.
        let with_body = PatchLessonBody {
            body_md: Some("# Hi"),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&with_body).unwrap(),
            serde_json::json!({ "body_md": "# Hi" })
        );

        let with_session = PatchLessonBody {
            live_session_id: Some(Some("abc")),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&with_session).unwrap(),
            serde_json::json!({ "live_session_id": "abc" })
        );
    }

    #[test]
    fn create_parent_invitation_body_omits_blank_relationship() {
        // A blank / whitespace-only relationship is dropped so the backend's
        // Option<String> stays None rather than receiving "".
        let blank = CreateParentInvitationBody {
            parent_email: "p@x".into(),
            student_user_id: "s-1".into(),
            relationship: None,
        };
        assert_eq!(
            serde_json::to_value(&blank).unwrap(),
            serde_json::json!({ "parent_email": "p@x", "student_user_id": "s-1" })
        );

        let with_rel = CreateParentInvitationBody {
            parent_email: "p@x".into(),
            student_user_id: "s-1".into(),
            relationship: Some("mother".into()),
        };
        assert_eq!(
            serde_json::to_value(&with_rel).unwrap(),
            serde_json::json!({
                "parent_email": "p@x",
                "student_user_id": "s-1",
                "relationship": "mother"
            })
        );
    }

    #[test]
    fn billing_dto_decodes_contract_field_names() {
        // Mirrors the backend `handlers::billing::BillingDto` serialization
        // exactly; a rename on either side breaks this test.
        let json = serde_json::json!({
            "plan": {
                "id": "plan_pro",
                "name": "Pro",
                "monthly_price_cents": 9900,
                "included_seats": 25,
                "included_class_minutes": 1000,
                "included_recording_gb": 10
            },
            "subscription": {
                "plan_id": "plan_pro",
                "status": "trialing",
                "current_period_start": null,
                "current_period_end": null,
                "trial_ends_at": "2026-06-15T00:00:00Z",
                "stripe_subscription_id": "sub_123",
                "overage_behavior": "block"
            },
            "usage": {
                "active_seats": 18,
                "included_seats": 25,
                "class_minutes_used": 900,
                "included_class_minutes": 1000,
                "recording_gb_used": 12.5,
                "included_recording_gb": 10
            },
            "overage_behavior": "block",
            "plans": [
                {
                    "id": "plan_pro",
                    "name": "Pro",
                    "monthly_price_cents": 9900,
                    "included_seats": 25,
                    "included_class_minutes": 1000,
                    "included_recording_gb": 10
                }
            ]
        });
        let b: BillingDto = serde_json::from_value(json).unwrap();
        let plan = b.plan.expect("plan present");
        assert_eq!(plan.name, "Pro");
        assert_eq!(plan.monthly_price_cents, 9900);
        let sub = b.subscription.expect("subscription present");
        assert_eq!(sub.status, "trialing");
        assert_eq!(sub.overage_behavior, "block");
        assert_eq!(b.usage.active_seats, 18);
        assert_eq!(b.usage.included_seats, Some(25));
        assert_eq!(b.usage.recording_gb_used, 12.5);
        assert_eq!(b.overage_behavior, "block");
        assert_eq!(b.plans.len(), 1);
    }

    #[test]
    fn billing_dto_decodes_null_plan_and_subscription() {
        // Fresh tenants have no subscription row; the backend sends nulls and
        // the page must render a "no plan" state, not a decode error.
        let json = serde_json::json!({
            "plan": null,
            "subscription": null,
            "usage": {
                "active_seats": 0,
                "included_seats": null,
                "class_minutes_used": 0,
                "included_class_minutes": null,
                "recording_gb_used": 0.0,
                "included_recording_gb": null
            },
            "overage_behavior": "block",
            "plans": []
        });
        let b: BillingDto = serde_json::from_value(json).unwrap();
        assert!(b.plan.is_none());
        assert!(b.subscription.is_none());
        assert_eq!(b.usage.included_seats, None);
    }

    #[test]
    fn member_invitation_dto_decodes_contract_field_names() {
        // Mirrors the backend `InvitationDto` JSON shape exactly; a rename on
        // either side breaks this test.
        let json = serde_json::json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "email": "teach@example.com",
            "role": "teacher",
            "status": "pending",
            "created_at": "2026-05-29T00:00:00Z"
        });
        let dto: InvitationDto = serde_json::from_value(json).unwrap();
        assert_eq!(dto.email, "teach@example.com");
        assert_eq!(dto.role, "teacher");
        assert_eq!(dto.status, "pending");
        assert_eq!(dto.created_at, "2026-05-29T00:00:00Z");
    }

    #[test]
    fn create_member_invitation_body_uses_contract_field_names() {
        let body = CreateMemberInvitationBody {
            email: "teach@example.com".into(),
            role: "teacher".into(),
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({ "email": "teach@example.com", "role": "teacher" })
        );
    }

    #[test]
    fn checkout_and_overage_bodies_use_contract_field_names() {
        assert_eq!(
            serde_json::to_value(&CheckoutSessionBody {
                plan_id: "plan_pro"
            })
            .unwrap(),
            serde_json::json!({ "plan_id": "plan_pro" })
        );
        assert_eq!(
            serde_json::to_value(&OverageBody {
                overage_behavior: "metered"
            })
            .unwrap(),
            serde_json::json!({ "overage_behavior": "metered" })
        );

        let portal: BillingPortalSessionDto = serde_json::from_value(serde_json::json!({
            "url": "https://billing.stripe.test/session/test"
        }))
        .unwrap();
        assert_eq!(portal.url, "https://billing.stripe.test/session/test");
    }

    #[test]
    fn branding_dto_decodes_contract_field_names() {
        // Mirrors the backend `BrandingDto` JSON shape exactly; a rename on
        // either side breaks this test.
        let json = serde_json::json!({
            "logo_url": "https://cdn.example.com/logo.svg",
            "primary_color": "#244f43",
            "accent_color": null
        });
        let dto: BrandingDto = serde_json::from_value(json).unwrap();
        assert_eq!(
            dto.logo_url.as_deref(),
            Some("https://cdn.example.com/logo.svg")
        );
        assert_eq!(dto.primary_color.as_deref(), Some("#244f43"));
        assert!(dto.accent_color.is_none());
    }

    #[test]
    fn patch_branding_body_omits_unset_fields() {
        // Nothing provided → empty object (backend leaves all fields untouched).
        let omitted = PatchBrandingBody::default();
        assert_eq!(
            serde_json::to_value(&omitted).unwrap(),
            serde_json::json!({})
        );

        // Only the provided fields are sent.
        let partial = PatchBrandingBody {
            primary_color: Some("#244f43".into()),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&partial).unwrap(),
            serde_json::json!({ "primary_color": "#244f43" })
        );

        let full = PatchBrandingBody {
            logo_url: Some("https://cdn.example.com/logo.svg".into()),
            primary_color: Some("#244f43".into()),
            accent_color: Some("#b08842".into()),
        };
        assert_eq!(
            serde_json::to_value(&full).unwrap(),
            serde_json::json!({
                "logo_url": "https://cdn.example.com/logo.svg",
                "primary_color": "#244f43",
                "accent_color": "#b08842"
            })
        );
    }

    #[test]
    fn notification_dto_decodes_contract_field_names() {
        // Mirrors the backend `NotificationDto` JSON shape exactly; a rename on
        // either side breaks this test.
        let json = serde_json::json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "kind": "grade_released",
            "title": "Grade released",
            "body": "Your grade for Essay 1 is ready.",
            "link": "/app/courses/c1/assignments/a1/submission",
            "created_at": "2026-05-29T00:00:00Z",
            "read_at": null
        });
        let dto: NotificationDto = serde_json::from_value(json).unwrap();
        assert_eq!(dto.kind, "grade_released");
        assert_eq!(dto.title, "Grade released");
        assert_eq!(
            dto.body.as_deref(),
            Some("Your grade for Essay 1 is ready.")
        );
        assert!(dto.read_at.is_none());
    }

    #[test]
    fn pref_dto_round_trips_contract_field_names() {
        let json = serde_json::json!({
            "email_enabled": true,
            "push_enabled": false,
            "in_app_enabled": true
        });
        let dto: PrefDto = serde_json::from_value(json).unwrap();
        assert!(dto.email_enabled);
        assert!(!dto.push_enabled);
        assert!(dto.in_app_enabled);
    }

    #[test]
    fn patch_pref_body_sends_all_three_toggles() {
        let body = PatchPrefBody {
            email_enabled: true,
            push_enabled: false,
            in_app_enabled: true,
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({
                "email_enabled": true,
                "push_enabled": false,
                "in_app_enabled": true
            })
        );
    }

    #[test]
    fn device_token_body_uses_contract_field_names() {
        let body = DeviceTokenBody {
            token: "tok-abc",
            platform: "web",
            label: None,
            user_agent: None,
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({ "token": "tok-abc", "platform": "web" })
        );
    }

    #[test]
    fn device_token_list_dto_decodes_without_raw_token() {
        let json = serde_json::json!({
            "devices": [{
                "id": "11111111-1111-1111-1111-111111111111",
                "platform": "web",
                "label": "Chrome",
                "user_agent": "Mozilla/5.0",
                "created_at": "2026-06-18T00:00:00Z",
                "last_seen_at": "2026-06-18T00:01:00Z"
            }]
        });
        let dto: DeviceTokenListDto = serde_json::from_value(json).unwrap();
        assert_eq!(dto.devices[0].platform, "web");
        assert_eq!(dto.devices[0].label.as_deref(), Some("Chrome"));
    }

    #[test]
    fn delivery_list_dto_decodes_sanitized_provider_fields() {
        let json = serde_json::json!({
            "deliveries": [{
                "id": "11111111-1111-1111-1111-111111111111",
                "user_id": "22222222-2222-2222-2222-222222222222",
                "notification_id": null,
                "channel": "push",
                "provider": "fcm",
                "target_hash": "abc123",
                "target_label": "web - Chrome",
                "device_token_id": null,
                "kind": "grade_released",
                "status": "failed",
                "provider_message_id": null,
                "provider_status": "404",
                "error_code": "UNREGISTERED",
                "error_message": "token is not registered",
                "created_at": "2026-06-18T00:00:00Z",
                "updated_at": "2026-06-18T00:00:00Z"
            }]
        });
        let dto: DeliveryListDto = serde_json::from_value(json).unwrap();
        assert_eq!(dto.deliveries[0].target_hash, "abc123");
        assert_eq!(dto.deliveries[0].status, "failed");
    }

    #[test]
    fn admin_api_key_contracts_preserve_one_time_plaintext() {
        let body = MintAdminApiKeyBody {
            name: "Reporting",
            scopes: vec!["courses:read".into(), "grades:read".into()],
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({
                "name": "Reporting",
                "scopes": ["courses:read", "grades:read"]
            })
        );

        let json = serde_json::json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "name": "Reporting",
            "prefix": "ak_abc123",
            "scopes": ["courses:read"],
            "created_by": "22222222-2222-2222-2222-222222222222",
            "created_at": "2026-06-18T00:00:00Z",
            "last_used_at": null,
            "revoked_at": null,
            "plaintext": "ak_abc123_secret"
        });
        let dto: MintedAdminApiKeyDto = serde_json::from_value(json).unwrap();
        assert_eq!(dto.key.prefix, "ak_abc123");
        assert_eq!(dto.plaintext, "ak_abc123_secret");
    }

    #[test]
    fn admin_webhook_contracts_preserve_create_secret_and_sparse_update() {
        let create = CreateAdminWebhookSubscriptionBody {
            url: "https://hooks.example.com/aula",
            events: vec!["course.published".into()],
        };
        assert_eq!(
            serde_json::to_value(&create).unwrap(),
            serde_json::json!({
                "url": "https://hooks.example.com/aula",
                "events": ["course.published"]
            })
        );

        let update = UpdateAdminWebhookSubscriptionBody {
            url: None,
            events: None,
            active: Some(false),
        };
        assert_eq!(
            serde_json::to_value(&update).unwrap(),
            serde_json::json!({ "active": false })
        );

        let json = serde_json::json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "url": "https://hooks.example.com/aula",
            "events": ["course.published"],
            "active": true,
            "created_at": "2026-06-18T00:00:00Z",
            "secret": "whsec_one_time"
        });
        let dto: CreatedAdminWebhookSubscriptionDto = serde_json::from_value(json).unwrap();
        assert_eq!(dto.subscription.events, vec!["course.published"]);
        assert_eq!(dto.secret, "whsec_one_time");
    }

    #[test]
    fn admin_webhook_delivery_decodes_payload_json() {
        let json = serde_json::json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "subscription_id": "22222222-2222-2222-2222-222222222222",
            "event": "submission.graded",
            "payload_json": { "submission_id": "sub-1", "score": 9 },
            "status": "failed",
            "attempts": 3,
            "last_attempt_at": "2026-06-18T00:05:00Z",
            "response_code": 500,
            "created_at": "2026-06-18T00:00:00Z"
        });
        let dto: AdminWebhookDeliveryDto = serde_json::from_value(json).unwrap();
        assert_eq!(dto.payload_json["score"], 9);
        assert_eq!(dto.response_code, Some(500));
    }

    #[test]
    fn admin_sso_body_omits_absent_secret_to_preserve_existing_secret() {
        let body = UpsertAdminSsoConfigBody {
            issuer: "https://idp.example.com".into(),
            client_id: "client-1".into(),
            client_secret: None,
            authorize_url: "https://idp.example.com/authorize".into(),
            token_url: "https://idp.example.com/token".into(),
            jwks_url: "https://idp.example.com/jwks".into(),
            enabled: true,
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({
                "issuer": "https://idp.example.com",
                "client_id": "client-1",
                "authorize_url": "https://idp.example.com/authorize",
                "token_url": "https://idp.example.com/token",
                "jwks_url": "https://idp.example.com/jwks",
                "enabled": true
            })
        );
    }

    #[test]
    fn admin_sso_config_decodes_secret_presence_without_secret_value() {
        let json = serde_json::json!({
            "issuer": "https://idp.example.com",
            "client_id": "client-1",
            "authorize_url": "https://idp.example.com/authorize",
            "token_url": "https://idp.example.com/token",
            "jwks_url": "https://idp.example.com/jwks",
            "enabled": true,
            "has_client_secret": true,
            "login_url": "https://app.example.com/v1/sso/acme/start"
        });
        let dto: AdminSsoConfigDto = serde_json::from_value(json).unwrap();
        assert!(dto.enabled);
        assert!(dto.has_client_secret);
        assert_eq!(dto.login_url, "https://app.example.com/v1/sso/acme/start");
    }

    #[test]
    fn admin_lti_platform_contracts_omit_absent_default_course() {
        let body = RegisterAdminLtiPlatformBody {
            name: "Canvas".into(),
            issuer: "https://canvas.example.com".into(),
            client_id: "client-1".into(),
            auth_login_url: "https://canvas.example.com/login/oauth2/auth".into(),
            jwks_url: "https://canvas.example.com/api/lti/security/jwks".into(),
            deployment_id: "deployment-1".into(),
            default_course_id: None,
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({
                "name": "Canvas",
                "issuer": "https://canvas.example.com",
                "client_id": "client-1",
                "auth_login_url": "https://canvas.example.com/login/oauth2/auth",
                "jwks_url": "https://canvas.example.com/api/lti/security/jwks",
                "deployment_id": "deployment-1"
            })
        );

        let json = serde_json::json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "name": "Canvas",
            "issuer": "https://canvas.example.com",
            "client_id": "client-1",
            "auth_login_url": "https://canvas.example.com/login/oauth2/auth",
            "jwks_url": "https://canvas.example.com/api/lti/security/jwks",
            "deployment_id": "deployment-1",
            "default_course_id": null,
            "created_at": "2026-06-18T00:00:00Z"
        });
        let dto: AdminLtiPlatformDto = serde_json::from_value(json).unwrap();
        assert_eq!(dto.name, "Canvas");
        assert!(dto.default_course_id.is_none());
    }

    #[test]
    fn create_tenant_body_uses_contract_field_names() {
        let body = CreateTenantBody {
            slug: "acme",
            name: "Acme Inc",
            admin_email: "admin@acme.test",
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({
                "slug": "acme",
                "name": "Acme Inc",
                "admin_email": "admin@acme.test"
            })
        );
    }

    #[test]
    fn set_tenant_status_body_serializes_status_only() {
        let body = SetTenantStatusBody {
            status: "suspended",
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({ "status": "suspended" })
        );
    }

    #[test]
    fn tenant_summary_dto_decodes_contract_shape() {
        // member_count is a JSON number; plan_id may be null.
        let json = serde_json::json!({
            "id": "t-1",
            "slug": "acme",
            "name": "Acme Inc",
            "status": "active",
            "created_at": "2026-05-29T00:00:00Z",
            "member_count": 42,
            "plan_id": null
        });
        let dto: TenantSummaryDto = serde_json::from_value(json).unwrap();
        assert_eq!(dto.slug, "acme");
        assert_eq!(dto.member_count, 42);
        assert!(dto.plan_id.is_none());
    }

    #[test]
    fn mfa_challenge_body_omits_absent_optional_fields() {
        let body = MfaChallengeBody {
            code: Some("123456".into()),
            trusted_device_token: None,
            remember_device: true,
            device_label: None,
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({
                "code": "123456",
                "remember_device": true
            })
        );
    }

    #[test]
    fn mfa_dtos_decode_contract_shapes() {
        let challenge: MfaChallengeResponseDto = serde_json::from_value(serde_json::json!({
            "stepped_up": true,
            "stepup_token": "session.jwt",
            "used_recovery_code": false,
            "trusted_device_token": "device-token",
            "trusted_device_expires_at": "2026-07-17T00:00:00Z"
        }))
        .unwrap();
        assert!(challenge.stepped_up);
        assert_eq!(
            challenge.trusted_device_token.as_deref(),
            Some("device-token")
        );

        let devices: TrustedDeviceListResponseDto = serde_json::from_value(serde_json::json!({
            "devices": [{
                "id": "11111111-1111-1111-1111-111111111111",
                "label": "This device",
                "user_agent": null,
                "last_used_at": null,
                "expires_at": "2026-07-17T00:00:00Z",
                "created_at": "2026-06-17T00:00:00Z"
            }]
        }))
        .unwrap();
        assert_eq!(devices.devices[0].label, "This device");

        let reset: ResetMfaRecoveryCodesResponseDto = serde_json::from_value(serde_json::json!({
            "recovery_codes": ["abcde-23456", "fghjk-789pq"]
        }))
        .unwrap();
        assert_eq!(reset.recovery_codes.len(), 2);
    }

    #[test]
    fn api_error_body_contains_matches_status_bodies() {
        let err = ApiError::Status(400, "{\"error\":\"mfa_required\"}".into());
        assert!(api_error_body_contains(&err, "mfa_required"));
        assert!(!api_error_body_contains(
            &ApiError::Network("offline".into()),
            "offline"
        ));
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod native_tests {
    use super::*;

    // Both env states are exercised in one test to avoid races between the
    // process-global env var and parallel test threads.
    #[test]
    fn native_api_base_url_resolves_env_then_default() {
        // Explicit override is honored.
        std::env::set_var("AULALITE_API_BASE_URL", "https://api.example.com");
        assert_eq!(native_api_base_url(), "https://api.example.com");

        // Blank is treated as unset → default.
        std::env::set_var("AULALITE_API_BASE_URL", "   ");
        assert_eq!(native_api_base_url(), "http://localhost:8080");

        // Unset → default.
        std::env::remove_var("AULALITE_API_BASE_URL");
        assert_eq!(native_api_base_url(), "http://localhost:8080");
    }

    #[test]
    fn selected_workspace_id_is_trimmed_bounded_and_validated() {
        assert_eq!(
            normalized_workspace_id(" 11111111-1111-1111-1111-111111111111 ").as_deref(),
            Some("11111111-1111-1111-1111-111111111111"),
        );
        assert_eq!(normalized_workspace_id("   "), None);
        assert_eq!(normalized_workspace_id("not-a-uuid"), None);
        assert_eq!(normalized_workspace_id(&"a".repeat(20_000)), None);
    }
}
