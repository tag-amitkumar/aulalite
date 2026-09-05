// crates/backend/src/handlers/attendance_export.rs
//! Staff-only CSV export of a live session's durable attendance.
//!
//! `GET /v1/sessions/{id}/attendance.csv` returns a `text/csv` download with a
//! `Content-Disposition: attachment` header and the columns
//! `display_name, user_id, first_join, last_leave, total_seconds,
//! reconnect_count`.
//!
//! Auth mirrors the JSON report in `live_sessions::attendance`: the session is
//! resolved under the caller's tenant context, then access is gated on either
//! platform-admin or `db::courses::caller_can_staff_course` (org_admin / owner /
//! assigned teacher|ta). We reuse `db::attendance::list_for_session` for the
//! rows — the same read the on-screen panel uses — so the CSV and the table can
//! never drift.
//!
//! The CSV is hand-rolled (the `csv` crate is not a dependency). Each field is
//! escaped per RFC 4180: a value is wrapped in double-quotes and its inner
//! double-quotes doubled whenever it contains a comma, quote, CR, or LF. This
//! defends against display names like `Doe, Jane` corrupting the column layout.
use axum::extract::{Extension, Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

fn is_org_admin(ctx: &RequestContext) -> bool {
    ctx.can_manage_organization()
}

/// RFC 4180 field escaping: quote the field (doubling any embedded quotes) iff
/// it contains a character that would otherwise break the row/column framing.
fn csv_escape(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// Render an attendance row set to an RFC 4180 CSV string with a header row.
/// Columns: display_name, user_id, first_join, last_leave, total_seconds,
/// reconnect_count. Timestamps are RFC 3339 (UTC); an absent `last_left_at`
/// (still-open / dropped socket) renders as an empty cell. `display_name`
/// falls back to the email, then the raw user id, so the column is never blank.
fn rows_to_csv(rows: &[db::attendance::AttendanceRow]) -> String {
    let mut out = String::from(
        "display_name,user_id,first_join,last_leave,total_seconds,reconnect_count\r\n",
    );
    for r in rows {
        let display_name = r
            .display_name
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(r.email.as_deref().filter(|s| !s.is_empty()))
            .map(|s| s.to_string())
            .unwrap_or_else(|| r.user_id.to_string());
        let first_join = r.first_joined_at.to_rfc3339();
        let last_leave = r.last_left_at.map(|t| t.to_rfc3339()).unwrap_or_default();
        out.push_str(&csv_escape(&display_name));
        out.push(',');
        out.push_str(&csv_escape(&r.user_id.to_string()));
        out.push(',');
        out.push_str(&csv_escape(&first_join));
        out.push(',');
        out.push_str(&csv_escape(&last_leave));
        out.push(',');
        out.push_str(&r.total_seconds.to_string());
        out.push(',');
        out.push_str(&r.reconnect_count.to_string());
        out.push_str("\r\n");
    }
    out
}

async fn attendance_csv_inner(
    pool: &PgPool,
    ctx: &RequestContext,
    session_id: Uuid,
) -> Result<Response, ApiError> {
    // Resolve the session (and its course) under the caller's tenant context.
    let session =
        db::live_sessions::load_for_join_with_context(pool, ctx.user_id, ctx.tenant_id, session_id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or(ApiError::NotFound)?;
    let tenant_id = ctx
        .tenant_id
        .ok_or_else(|| ApiError::BadRequest("no tenant".into()))?;
    if session.tenant_id != tenant_id {
        return Err(ApiError::NotFound);
    }

    // Staff-only: platform admins bypass; otherwise the caller must be able to
    // staff the course (org_admin / owner / assigned teacher|ta). Mirrors the
    // JSON report so the CSV is never visible to a viewer the table hides from.
    if !ctx.can_manage_organization() {
        let allowed = db::courses::caller_can_staff_course(
            pool,
            session.course_id,
            ctx.user_id,
            ctx.tenant_id,
            is_org_admin(ctx),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        if !allowed {
            return Err(ApiError::Forbidden);
        }
    }

    let rows = db::attendance::list_for_session(pool, tenant_id, session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let body = rows_to_csv(&rows);

    // Stable, filesystem-safe download name. The full session id keeps the file
    // unambiguous when a user exports several sessions.
    let filename = format!("attendance-{session_id}.csv");

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        body,
    )
        .into_response())
}

async fn attendance_csv(
    State(s): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Path(session_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    attendance_csv_inner(&s.pool, &ctx, session_id).await
}

/// Routes for the attendance CSV export. Merged into the authed router by
/// `lib.rs`. Kept in its own `Router<AppState>` so the route table mirrors the
/// other handler modules.
pub fn routes() -> axum::Router<AppState> {
    axum::Router::new().route(
        "/v1/sessions/{id}/attendance.csv",
        axum::routing::get(attendance_csv),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        name: Option<&str>,
        email: Option<&str>,
        total_seconds: i32,
        reconnect_count: i32,
        leave: bool,
    ) -> db::attendance::AttendanceRow {
        let joined = chrono::DateTime::parse_from_rfc3339("2026-05-29T10:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        db::attendance::AttendanceRow {
            user_id: Uuid::nil(),
            display_name: name.map(|s| s.to_string()),
            email: email.map(|s| s.to_string()),
            first_joined_at: joined,
            last_left_at: leave.then(|| joined + chrono::Duration::minutes(30)),
            total_seconds,
            reconnect_count,
        }
    }

    #[test]
    fn header_row_has_the_contract_columns() {
        let csv = rows_to_csv(&[]);
        assert_eq!(
            csv,
            "display_name,user_id,first_join,last_leave,total_seconds,reconnect_count\r\n"
        );
    }

    #[test]
    fn escapes_comma_quote_and_newline_in_display_name() {
        let csv = rows_to_csv(&[row(Some("Doe, \"Jane\"\n"), None, 605, 2, true)]);
        // The name field must be quoted with its inner quotes doubled and the
        // newline preserved inside the quotes.
        assert!(csv.contains("\"Doe, \"\"Jane\"\"\n\""));
        // Numeric columns are never quoted.
        assert!(csv.contains(",605,2\r\n"));
    }

    #[test]
    fn display_name_falls_back_to_email_then_user_id() {
        let csv = rows_to_csv(&[row(None, Some("ada@example.com"), 0, 0, false)]);
        assert!(csv.contains("ada@example.com"));

        let csv = rows_to_csv(&[row(None, None, 0, 0, false)]);
        // Falls back to the (nil) user id; the cell is never blank.
        assert!(csv.contains(&Uuid::nil().to_string()));
    }

    #[test]
    fn open_row_leaves_last_leave_empty() {
        // No leave → empty last_leave cell between the two timestamp commas.
        let csv = rows_to_csv(&[row(Some("Bob"), None, 10, 0, false)]);
        let data_line = csv.lines().nth(1).unwrap();
        // ...,<first_join>,,10,0  — the empty field is the double comma.
        assert!(data_line.contains(",,10,0"));
    }
}
