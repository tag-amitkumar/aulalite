// crates/backend/src/handlers/calendar.rs
//! Personal calendar: the caller's visible live sessions + published-assignment
//! due dates, as JSON or as a downloadable / subscribable iCalendar feed.
//!
//! Routes (under the authed router):
//!   * GET /v1/me/calendar?from=&to=  — JSON: typed events in [from, to)
//!   * GET /v1/me/calendar.ics?from=&to=  — text/calendar (RFC 5545) of the same
//!
//! Both endpoints read ONLY what the caller can see — joined on ACTIVE
//! `course_memberships` (see `db::calendar`) — so a student sees their courses'
//! sessions + due dates and nothing else. No staff gate: every authed user has
//! a personal calendar of their own enrollments.
//!
//! The window defaults to [now - 7d, now + 60d] and is clamped to at most ~400
//! days so a crafted `?from`/`?to` can't make us scan an unbounded range.
//!
//! Also hosts the reminder sweep (`reminder_sweep_once`) the main stream spawns
//! on an interval: it finds sessions/assignments due in the next ~24h that have
//! not yet had a reminder sent and fans a notification out to enrolled students
//! via `services::notifications::notify`, recording each send in `reminders_sent`
//! to dedupe.
use axum::extract::{Extension, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{routing, Json, Router};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::context::RequestContext;
use crate::db;
use crate::error::ApiError;
use crate::AppState;

/// Default window start relative to now (look back one week so recently-passed
/// sessions still show in an agenda).
const DEFAULT_LOOKBACK_DAYS: i64 = 7;
/// Default window end relative to now.
const DEFAULT_LOOKAHEAD_DAYS: i64 = 60;
/// Hard cap on the queried window so a crafted range can't force an unbounded
/// scan.
const MAX_WINDOW_DAYS: i64 = 400;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/me/calendar", routing::get(get_calendar))
        .route("/v1/me/calendar.ics", routing::get(get_calendar_ics))
}

#[derive(Debug, Deserialize)]
pub struct CalendarQuery {
    /// RFC3339 window start (inclusive). Defaults to now - 7d.
    pub from: Option<String>,
    /// RFC3339 window end (exclusive). Defaults to now + 60d.
    pub to: Option<String>,
}

/// One calendar event: a live session or an assignment due date.
#[derive(Debug, Serialize, PartialEq)]
pub struct CalendarEvent {
    /// `"session"` | `"assignment_due"`.
    pub kind: String,
    /// The session id or assignment id.
    pub id: Uuid,
    pub course_id: Uuid,
    pub course_title: String,
    pub title: String,
    /// Event timestamp (session start, or assignment due time).
    pub starts_at: DateTime<Utc>,
    /// Session length in minutes; `None` for an assignment due date (a moment).
    pub duration_minutes: Option<i32>,
    /// Session lifecycle status; `None` for assignment due dates.
    pub status: Option<String>,
    /// In-app deep link (relative path under the SPA).
    pub link: String,
}

/// Resolve the requested window, applying defaults and the safety clamp. Pure so
/// it's unit-testable. Returns `(from, to)` with `from < to` guaranteed.
fn resolve_window(
    now: DateTime<Utc>,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<(DateTime<Utc>, DateTime<Utc>), ApiError> {
    let parse = |s: &str| -> Result<DateTime<Utc>, ApiError> {
        DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|_| ApiError::BadRequest(format!("invalid timestamp: {s}")))
    };
    let from = match from {
        Some(s) => parse(s)?,
        None => now - Duration::days(DEFAULT_LOOKBACK_DAYS),
    };
    let to = match to {
        Some(s) => parse(s)?,
        None => now + Duration::days(DEFAULT_LOOKAHEAD_DAYS),
    };
    if to <= from {
        return Err(ApiError::BadRequest("`to` must be after `from`".into()));
    }
    if to - from > Duration::days(MAX_WINDOW_DAYS) {
        return Err(ApiError::BadRequest(format!(
            "window too large; max {MAX_WINDOW_DAYS} days"
        )));
    }
    Ok((from, to))
}

/// Deep link to a live session within the SPA.
fn session_link(slug: &str, session_id: Uuid) -> String {
    format!("/courses/{slug}/sessions/{session_id}")
}

/// Deep link to an assignment within the SPA.
fn assignment_link(slug: &str, assignment_id: Uuid) -> String {
    format!("/courses/{slug}/assignments/{assignment_id}")
}

/// Gather the caller's visible events in `[from, to)`, merged + sorted by time.
async fn gather_events(
    state: &AppState,
    ctx: &RequestContext,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<CalendarEvent>, ApiError> {
    let sessions =
        db::calendar::list_visible_sessions(&state.pool, ctx.user_id, ctx.tenant_id, from, to)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    let assignments = db::calendar::list_visible_assignment_due(
        &state.pool,
        ctx.user_id,
        ctx.tenant_id,
        from,
        to,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut events: Vec<CalendarEvent> = Vec::with_capacity(sessions.len() + assignments.len());
    for s in sessions {
        events.push(CalendarEvent {
            kind: "session".into(),
            id: s.session_id,
            course_id: s.course_id,
            course_title: s.course_title,
            link: session_link(&s.course_slug, s.session_id),
            title: s.title,
            starts_at: s.starts_at,
            duration_minutes: Some(s.duration_minutes),
            status: Some(s.status),
        });
    }
    for a in assignments {
        events.push(CalendarEvent {
            kind: "assignment_due".into(),
            id: a.assignment_id,
            course_id: a.course_id,
            course_title: a.course_title,
            link: assignment_link(&a.course_slug, a.assignment_id),
            title: a.title,
            starts_at: a.due_at,
            duration_minutes: None,
            status: None,
        });
    }
    events.sort_by(|x, y| x.starts_at.cmp(&y.starts_at).then(x.id.cmp(&y.id)));
    Ok(events)
}

async fn get_calendar(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Query(q): Query<CalendarQuery>,
) -> Result<Json<Vec<CalendarEvent>>, ApiError> {
    let (from, to) = resolve_window(Utc::now(), q.from.as_deref(), q.to.as_deref())?;
    let events = gather_events(&state, &ctx, from, to).await?;
    Ok(Json(events))
}

async fn get_calendar_ics(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Query(q): Query<CalendarQuery>,
) -> Result<Response, ApiError> {
    let (from, to) = resolve_window(Utc::now(), q.from.as_deref(), q.to.as_deref())?;
    let events = gather_events(&state, &ctx, from, to).await?;
    let origin = state.app_origin.trim_end_matches('/');
    let body = build_ics(&events, origin, Utc::now());
    Ok((
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                "text/calendar; charset=utf-8".to_string(),
            ),
            (
                header::CONTENT_DISPOSITION,
                "inline; filename=\"aulalite.ics\"".to_string(),
            ),
        ],
        body,
    )
        .into_response())
}

// ===========================================================================
// iCalendar (RFC 5545) hand-builder — no extra crate.
// ===========================================================================

/// Escape a value for an iCalendar TEXT field per RFC 5545 §3.3.11: backslash,
/// semicolon, comma, and newlines are escaped. (Colons are NOT special in TEXT.)
fn ics_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            _ => out.push(c),
        }
    }
    out
}

/// Format a UTC instant as an iCalendar UTC date-time: `YYYYMMDDTHHMMSSZ`.
fn ics_dt(dt: DateTime<Utc>) -> String {
    dt.format("%Y%m%dT%H%M%SZ").to_string()
}

/// Fold a content line to <=75 octets per RFC 5545 §3.1 (CRLF + single leading
/// space on each continuation). Operates on bytes so multi-byte UTF-8 chars are
/// not split mid-codepoint.
fn fold_line(line: &str) -> String {
    let bytes = line.as_bytes();
    if bytes.len() <= 75 {
        return line.to_string();
    }
    let mut out = String::with_capacity(bytes.len() + bytes.len() / 70);
    let mut idx = 0usize;
    let mut first = true;
    while idx < bytes.len() {
        // First line: 75 octets; continuations: 74 (the leading space counts).
        let budget = if first { 75 } else { 74 };
        let mut end = (idx + budget).min(bytes.len());
        // Back up so we never split a UTF-8 codepoint (continuation bytes are
        // 0b10xxxxxx).
        while end > idx && end < bytes.len() && (bytes[end] & 0xC0) == 0x80 {
            end -= 1;
        }
        if !first {
            out.push_str("\r\n ");
        }
        // Safe: `end`/`idx` sit on char boundaries by construction.
        out.push_str(&line[idx..end]);
        idx = end;
        first = false;
    }
    out
}

/// Build a complete VCALENDAR document for `events`. `origin` seeds the UID
/// domain + the per-event URL property. `now` is the DTSTAMP used on every
/// VEVENT. CRLF line endings per spec.
pub fn build_ics(events: &[CalendarEvent], origin: &str, now: DateTime<Utc>) -> String {
    // UID host: strip the scheme from the origin for a stable RHS.
    let host = origin
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("aulalite.app");
    let dtstamp = ics_dt(now);

    let mut lines: Vec<String> = Vec::with_capacity(events.len() * 8 + 6);
    lines.push("BEGIN:VCALENDAR".into());
    lines.push("VERSION:2.0".into());
    lines.push("PRODID:-//AulaLite//Calendar//EN".into());
    lines.push("CALSCALE:GREGORIAN".into());
    lines.push("METHOD:PUBLISH".into());
    lines.push("X-WR-CALNAME:AulaLite".into());

    for e in events {
        lines.push("BEGIN:VEVENT".into());
        lines.push(format!("UID:{}-{}@{}", e.kind, e.id, host));
        lines.push(format!("DTSTAMP:{dtstamp}"));
        lines.push(format!("DTSTART:{}", ics_dt(e.starts_at)));
        match e.duration_minutes {
            // A session: emit a real end time so the block has duration.
            Some(mins) if mins > 0 => {
                let end = e.starts_at + Duration::minutes(mins as i64);
                lines.push(format!("DTEND:{}", ics_dt(end)));
            }
            // Assignment due dates (and zero-length sessions) are a moment.
            _ => {}
        }
        let summary = match e.kind.as_str() {
            "assignment_due" => format!("Due: {}", e.title),
            _ => e.title.clone(),
        };
        lines.push(format!("SUMMARY:{}", ics_escape(&summary)));
        lines.push(format!(
            "DESCRIPTION:{}",
            ics_escape(&format!("{} — {}", e.course_title, summary))
        ));
        // Join origin + link with exactly one slash: `e.link` always starts
        // with `/`, so strip any trailing slash from the origin to avoid a
        // `//` when APP_ORIGIN is configured with a trailing slash.
        lines.push(format!("URL:{}{}", origin.trim_end_matches('/'), e.link));
        lines.push("END:VEVENT".into());
    }
    lines.push("END:VCALENDAR".into());

    // Fold each logical line, then join with CRLF and terminate.
    let folded: Vec<String> = lines.iter().map(|l| fold_line(l)).collect();
    let mut body = folded.join("\r\n");
    body.push_str("\r\n");
    body
}

// ===========================================================================
// Reminder sweep (spawned by main on an interval).
// ===========================================================================

/// Run ONE pass of the reminder sweep: find sessions / assignments due in the
/// next `window` that have not yet had a reminder sent, fan a notification out
/// to enrolled students, and record each in `reminders_sent` to dedupe.
///
/// Best-effort throughout: a per-entity failure is logged and the sweep moves
/// on. Designed to be called on an interval (every ~15 min) from a tokio task.
/// `app_origin` is used to build absolute deep links in the notification body.
/// Returns the number of (session + assignment) reminders recorded this pass.
pub async fn reminder_sweep_once(
    pool: &PgPool,
    email: &dyn crate::services::notifications::EmailNotifier,
    push: &dyn crate::services::notifications::PushSender,
    app_origin: &str,
    window: Duration,
) -> usize {
    const BATCH: i64 = 200;
    let origin = app_origin.trim_end_matches('/');
    let mut recorded = 0usize;

    // --- Sessions starting soon ---
    match db::calendar::due_sessions_for_reminders(pool, window, BATCH).await {
        Ok(sessions) => {
            for s in sessions {
                let students = match db::calendar::active_student_ids_system(pool, s.course_id)
                    .await
                {
                    Ok(ids) => ids,
                    Err(e) => {
                        tracing::warn!(?e, session_id = %s.session_id, "reminder: student lookup failed");
                        continue;
                    }
                };
                let title = format!("Class starting soon: {}", s.title);
                let body = format!("{} starts at {}.", s.course_title, s.starts_at.to_rfc3339());
                let link = format!("{origin}{}", session_link(&s.course_slug, s.session_id));
                for student_id in students {
                    crate::services::notifications::notify(
                        pool,
                        email,
                        push,
                        s.tenant_id,
                        student_id,
                        "session_reminder",
                        &title,
                        Some(&body),
                        Some(&link),
                    )
                    .await;
                }
                // Record AFTER fan-out so a crash mid-fan-out retries next pass.
                if let Err(e) = db::calendar::mark_reminder_sent(
                    pool,
                    s.tenant_id,
                    "session",
                    s.session_id,
                    "session_starting",
                )
                .await
                {
                    tracing::warn!(?e, session_id = %s.session_id, "reminder: mark_sent failed");
                } else {
                    recorded += 1;
                }
            }
        }
        Err(e) => tracing::warn!(?e, "reminder: due_sessions_for_reminders failed"),
    }

    // --- Assignments due soon ---
    match db::calendar::due_assignments_for_reminders(pool, window, BATCH).await {
        Ok(assignments) => {
            for a in assignments {
                let students = match db::calendar::active_student_ids_system(pool, a.course_id)
                    .await
                {
                    Ok(ids) => ids,
                    Err(e) => {
                        tracing::warn!(?e, assignment_id = %a.assignment_id, "reminder: student lookup failed");
                        continue;
                    }
                };
                let title = format!("Assignment due soon: {}", a.title);
                let body = format!("{} is due at {}.", a.title, a.due_at.to_rfc3339());
                let link = format!(
                    "{origin}{}",
                    assignment_link(&a.course_slug, a.assignment_id)
                );
                for student_id in students {
                    crate::services::notifications::notify(
                        pool,
                        email,
                        push,
                        a.tenant_id,
                        student_id,
                        "assignment_reminder",
                        &title,
                        Some(&body),
                        Some(&link),
                    )
                    .await;
                }
                if let Err(e) = db::calendar::mark_reminder_sent(
                    pool,
                    a.tenant_id,
                    "assignment",
                    a.assignment_id,
                    "assignment_due",
                )
                .await
                {
                    tracing::warn!(?e, assignment_id = %a.assignment_id, "reminder: mark_sent failed");
                } else {
                    recorded += 1;
                }
            }
        }
        Err(e) => tracing::warn!(?e, "reminder: due_assignments_for_reminders failed"),
    }

    recorded
}

/// The long-running reminder loop the main stream spawns. Ticks every
/// `interval` and runs `reminder_sweep_once` with a ~24h lookahead window. Owns
/// its inputs so it can be moved into `tokio::spawn`.
pub async fn run_reminder_loop(
    pool: PgPool,
    email: std::sync::Arc<dyn crate::services::notifications::EmailNotifier>,
    push: std::sync::Arc<dyn crate::services::notifications::PushSender>,
    app_origin: String,
    interval: std::time::Duration,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let window = Duration::hours(24);
    loop {
        ticker.tick().await;
        let n =
            reminder_sweep_once(&pool, email.as_ref(), push.as_ref(), &app_origin, window).await;
        if n > 0 {
            tracing::info!(count = n, "calendar reminders dispatched");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: &str, title: &str, ts: &str, dur: Option<i32>) -> CalendarEvent {
        CalendarEvent {
            kind: kind.into(),
            id: Uuid::nil(),
            course_id: Uuid::nil(),
            course_title: "Calc 1".into(),
            title: title.into(),
            starts_at: DateTime::parse_from_rfc3339(ts)
                .unwrap()
                .with_timezone(&Utc),
            duration_minutes: dur,
            status: None,
            link: "/courses/calc-1/x".into(),
        }
    }

    #[test]
    fn window_defaults_span_lookback_to_lookahead() {
        let now = DateTime::parse_from_rfc3339("2026-06-14T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let (from, to) = resolve_window(now, None, None).unwrap();
        assert_eq!(from, now - Duration::days(DEFAULT_LOOKBACK_DAYS));
        assert_eq!(to, now + Duration::days(DEFAULT_LOOKAHEAD_DAYS));
    }

    #[test]
    fn window_rejects_inverted_and_oversized_ranges() {
        let now = Utc::now();
        // to <= from
        assert!(resolve_window(
            now,
            Some("2026-06-10T00:00:00Z"),
            Some("2026-06-09T00:00:00Z")
        )
        .is_err());
        // > MAX_WINDOW_DAYS
        assert!(resolve_window(
            now,
            Some("2020-01-01T00:00:00Z"),
            Some("2026-01-01T00:00:00Z")
        )
        .is_err());
    }

    #[test]
    fn window_rejects_unparseable_timestamp() {
        let now = Utc::now();
        assert!(resolve_window(now, Some("not-a-date"), None).is_err());
    }

    #[test]
    fn ics_escape_handles_special_chars() {
        assert_eq!(ics_escape("a;b,c\\d"), "a\\;b\\,c\\\\d");
        assert_eq!(ics_escape("line1\r\nline2"), "line1\\nline2");
    }

    #[test]
    fn ics_dt_is_utc_basic_format() {
        let dt = DateTime::parse_from_rfc3339("2026-06-17T22:39:04Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(ics_dt(dt), "20260617T223904Z");
    }

    #[test]
    fn fold_line_keeps_short_lines_and_folds_long_ones() {
        assert_eq!(fold_line("short"), "short");
        let long = "X".repeat(200);
        let folded = fold_line(&long);
        // Every physical line must be <= 75 octets.
        for (i, l) in folded.split("\r\n").enumerate() {
            let octets = l.len();
            assert!(octets <= 75, "physical line {i} too long: {octets}");
        }
        // Unfolding (drop CRLF + the single leading space) restores the original.
        let unfolded = folded.replace("\r\n ", "");
        assert_eq!(unfolded, long);
    }

    #[test]
    fn build_ics_emits_valid_envelope_and_events() {
        let events = vec![
            ev("session", "Limits", "2026-06-17T22:39:04Z", Some(45)),
            ev(
                "assignment_due",
                "Problem Set 1",
                "2026-06-20T23:59:00Z",
                None,
            ),
        ];
        let now = DateTime::parse_from_rfc3339("2026-06-14T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ics = build_ics(&events, "https://app.example.com/", now);
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(ics.trim_end().ends_with("END:VCALENDAR"));
        assert!(ics.contains("VERSION:2.0"));
        assert_eq!(ics.matches("BEGIN:VEVENT").count(), 2);
        assert_eq!(ics.matches("END:VEVENT").count(), 2);
        // Session has DTSTART + DTEND (start + 45 min).
        assert!(ics.contains("DTSTART:20260617T223904Z"));
        assert!(ics.contains("DTEND:20260617T232404Z"));
        // Assignment due renders a "Due:" summary and no DTEND.
        assert!(ics.contains("SUMMARY:Due: Problem Set 1"));
        // UID host is derived from the origin (scheme stripped).
        assert!(ics.contains("@app.example.com"));
        // URL property uses the absolute origin + link.
        assert!(ics.contains("URL:https://app.example.com/courses/calc-1/x"));
        // CRLF line endings throughout.
        assert!(ics.contains("\r\n"));
    }

    #[test]
    fn build_ics_empty_is_still_a_valid_calendar() {
        let ics = build_ics(&[], "https://x", Utc::now());
        assert!(ics.contains("BEGIN:VCALENDAR"));
        assert!(ics.contains("END:VCALENDAR"));
        assert_eq!(ics.matches("BEGIN:VEVENT").count(), 0);
    }
}
