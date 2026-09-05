// crates/backend/src/db/privacy.rs
//! GDPR data layer: self-service export, right-to-erasure, and token
//! revocation for the calling user.
//!
//! Every read here is scoped to a SINGLE `user_id` (the caller) and runs inside
//! a tx with the RLS GUCs set via `db::begin_with_context`, mirroring
//! `handlers::me::my_courses`. We never expose another user's rows: the WHERE
//! clauses pin `user_id`/`student_user_id`/`author_user_id` to the caller and
//! the `tenant_isolation` RLS policy keeps the gather inside the active tenant.
//!
//! Erasure ANONYMIZES rather than hard-deletes: FK-referenced rows (gradebook,
//! authored posts, attendance) stay intact for institutional integrity, but the
//! `users` row is overwritten with a tombstone (`deleted-user-<short>`), the
//! avatar/display name are cleared, and `deleted_at`/`anonymized_at` are
//! stamped. Token revocation bumps `tokens_valid_after = now()` so every
//! previously-issued token (whose `iat` predates that instant) is rejected by
//! the auth middleware.
//!
//! Uses ONLY runtime sqlx (no compile-time macros — there is no DATABASE_URL at
//! build).
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db;

// ─── Export bundle ───────────────────────────────────────────────────────────

/// The caller's own profile, as stored in `users`.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ProfileExport {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub locale: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// One course the caller is (or was) a member of.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EnrollmentExport {
    pub course_id: Uuid,
    pub course_slug: String,
    pub course_title: String,
    pub role: String,
    pub status: String,
    pub joined_at: chrono::DateTime<chrono::Utc>,
}

/// One of the caller's own assignment submissions, including the grade they can
/// already see. `teacher_only_notes` is deliberately NOT exported.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SubmissionExport {
    pub submission_id: Uuid,
    pub assignment_id: Uuid,
    pub assignment_title: Option<String>,
    pub course_id: Uuid,
    pub status: String,
    pub text_answer: Option<String>,
    pub submitted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub is_late: bool,
    pub numeric_grade: Option<f64>,
    pub letter_grade: Option<String>,
    pub passed: Option<bool>,
    pub student_visible_feedback: Option<String>,
    pub graded_at: Option<chrono::DateTime<chrono::Utc>>,
    pub released_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// One quiz attempt by the caller (the gradebook-relevant rollup).
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct QuizAttemptExport {
    pub attempt_id: Uuid,
    pub quiz_id: Uuid,
    pub quiz_title: Option<String>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub submitted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub score_points: Option<i32>,
    pub max_points: Option<i32>,
}

/// One discussion post the caller authored.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DiscussionPostExport {
    pub post_id: Uuid,
    pub discussion_id: Uuid,
    pub discussion_title: Option<String>,
    pub course_id: Option<Uuid>,
    pub body_md: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// One personal note the caller wrote on a lesson.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct NoteExport {
    pub lesson_id: Uuid,
    pub lesson_title: Option<String>,
    pub course_id: Option<Uuid>,
    pub body: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// One lesson the caller bookmarked.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct BookmarkExport {
    pub lesson_id: Uuid,
    pub lesson_title: Option<String>,
    pub course_id: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// One lesson-completion record for the caller.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CompletionExport {
    pub lesson_id: Uuid,
    pub course_id: Uuid,
    pub completed_at: chrono::DateTime<chrono::Utc>,
}

/// One durable attendance record for the caller across the tenant's sessions.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AttendanceExport {
    pub session_id: Uuid,
    pub session_title: Option<String>,
    pub course_title: Option<String>,
    pub first_joined_at: chrono::DateTime<chrono::Utc>,
    pub last_left_at: Option<chrono::DateTime<chrono::Utc>>,
    pub total_seconds: i32,
    pub reconnect_count: i32,
}

/// The full GDPR export bundle. Serialized as the JSON download body. Every
/// collection holds only the caller's own rows within the active tenant.
#[derive(Debug, Serialize)]
pub struct ExportBundle {
    /// RFC 3339 instant the bundle was generated.
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub tenant_id: Option<Uuid>,
    pub profile: ProfileExport,
    pub enrollments: Vec<EnrollmentExport>,
    pub submissions: Vec<SubmissionExport>,
    pub quiz_attempts: Vec<QuizAttemptExport>,
    pub discussion_posts: Vec<DiscussionPostExport>,
    pub notes: Vec<NoteExport>,
    pub bookmarks: Vec<BookmarkExport>,
    pub lesson_completions: Vec<CompletionExport>,
    pub attendance: Vec<AttendanceExport>,
}

/// Gather the caller's entire data footprint within the active tenant into a
/// single bundle. All reads run inside one tx with the RLS GUCs set so the
/// gather is consistent and tenant-scoped; every query additionally pins the
/// caller's `user_id`. `tenant_id` may be `None` for a user with no active
/// tenant membership — in that case the tenant-scoped collections come back
/// empty (the RLS predicate `tenant_id::text = current_setting('app.tenant_id')`
/// matches nothing without the GUC) and only the profile is populated.
pub async fn export_for_user(
    pool: &PgPool,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
) -> sqlx::Result<ExportBundle> {
    let mut tx = db::begin_with_context(pool, user_id, tenant_id).await?;

    let profile = sqlx::query_as::<_, ProfileExport>(
        "SELECT id AS user_id, email::text AS email, display_name, avatar_url, locale,
                created_at, last_seen_at
           FROM users
          WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;

    let enrollments = sqlx::query_as::<_, EnrollmentExport>(
        "SELECT cm.course_id, c.slug AS course_slug, c.title AS course_title,
                cm.role, cm.status, cm.joined_at
           FROM course_memberships cm
           JOIN courses c ON c.id = cm.course_id
          WHERE cm.user_id = $1
          ORDER BY cm.joined_at DESC",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;

    let submissions = sqlx::query_as::<_, SubmissionExport>(
        "SELECT s.id AS submission_id, s.assignment_id, a.title AS assignment_title,
                s.course_id, s.status::text AS status, s.text_answer, s.submitted_at,
                s.is_late,
                s.numeric_grade::float8 AS numeric_grade, s.letter_grade, s.passed,
                s.student_visible_feedback, s.graded_at, s.released_at
           FROM submissions s
           LEFT JOIN assignments a ON a.id = s.assignment_id
          WHERE s.student_user_id = $1
          ORDER BY s.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;

    let quiz_attempts = sqlx::query_as::<_, QuizAttemptExport>(
        "SELECT qa.id AS attempt_id, qa.quiz_id, q.title AS quiz_title,
                qa.started_at, qa.submitted_at, qa.score_points, qa.max_points
           FROM quiz_attempts qa
           LEFT JOIN quizzes q ON q.id = qa.quiz_id
          WHERE qa.user_id = $1
          ORDER BY qa.started_at DESC",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;

    let discussion_posts = sqlx::query_as::<_, DiscussionPostExport>(
        "SELECT p.id AS post_id, p.discussion_id, d.title AS discussion_title,
                d.course_id, p.body_md, p.created_at
           FROM discussion_posts p
           LEFT JOIN discussions d ON d.id = p.discussion_id
          WHERE p.author_user_id = $1
          ORDER BY p.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;

    let notes = sqlx::query_as::<_, NoteExport>(
        "SELECT n.lesson_id, l.title AS lesson_title, l.course_id, n.body, n.updated_at
           FROM student_notes n
           LEFT JOIN lessons l ON l.id = n.lesson_id
          WHERE n.user_id = $1
          ORDER BY n.updated_at DESC",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;

    let bookmarks = sqlx::query_as::<_, BookmarkExport>(
        "SELECT b.lesson_id, l.title AS lesson_title, l.course_id, b.created_at
           FROM lesson_bookmarks b
           LEFT JOIN lessons l ON l.id = b.lesson_id
          WHERE b.user_id = $1
          ORDER BY b.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;

    let lesson_completions = sqlx::query_as::<_, CompletionExport>(
        "SELECT lesson_id, course_id, completed_at
           FROM lesson_completions
          WHERE user_id = $1
          ORDER BY completed_at DESC",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;

    let attendance = sqlx::query_as::<_, AttendanceExport>(
        "SELECT att.session_id, ls.title AS session_title, c.title AS course_title,
                att.first_joined_at, att.last_left_at, att.total_seconds,
                att.reconnect_count
           FROM attendance att
           LEFT JOIN live_sessions ls ON ls.id = att.session_id
           LEFT JOIN courses c ON c.id = ls.course_id
          WHERE att.user_id = $1
          ORDER BY att.first_joined_at DESC",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(ExportBundle {
        generated_at: chrono::Utc::now(),
        tenant_id,
        profile,
        enrollments,
        submissions,
        quiz_attempts,
        discussion_posts,
        notes,
        bookmarks,
        lesson_completions,
        attendance,
    })
}

// ─── Right to erasure (anonymize) ────────────────────────────────────────────

/// Build a short tombstone identifier from a user id, e.g.
/// `deleted-user-3f9a1c2b`. Stable for a given id; collision-safe enough for a
/// human-readable tombstone (the full uuid still uniquely keys the row).
pub fn tombstone_label(user_id: Uuid) -> String {
    let short = user_id.simple().to_string();
    format!("deleted-user-{}", &short[..8])
}

/// Whether the caller's `users` row has already been anonymized.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ErasureStatus {
    pub anonymized_at: Option<chrono::DateTime<chrono::Utc>>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Anonymize and disable the caller without cascading FK-referenced learning
/// records. The privileged DB function locks all affected organizations,
/// refuses to erase an active organization owner, deactivates every other
/// tenant membership, and tombstones the identity in one transaction. The
/// transaction-local actor GUC is bound to `user_id` before invoking it.
///
/// Overwrites `display_name` + `email` with a per-user tombstone, nulls
/// `avatar_url`/`locale`, stamps `deleted_at`/`anonymized_at`, and bumps
/// `tokens_valid_after` so every outstanding token is immediately rejected.
/// Idempotent: a second call on an already-anonymized row is a no-op (the
/// `anonymized_at IS NULL` guard) and returns the existing stamps.
///
/// The tombstone email is suffixed with `@deleted.invalid` (a reserved,
/// non-routable TLD) so it can never collide with a real address while still
/// satisfying the `users_email_unique` CITEXT constraint via the embedded uuid.
pub async fn anonymize_user(pool: &PgPool, user_id: Uuid) -> sqlx::Result<ErasureStatus> {
    let label = tombstone_label(user_id);
    let tombstone_email = format!("{}+{}@deleted.invalid", label, user_id.simple());

    let mut tx = db::begin_with_context(pool, user_id, None).await?;
    let row = sqlx::query_as::<_, ErasureStatus>(
        "SELECT anonymized_at, deleted_at
           FROM anonymize_user_account($1, $2, $3)",
    )
    .bind(user_id)
    .bind(&label)
    .bind(&tombstone_email)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(row)
}

// ─── Token revocation ────────────────────────────────────────────────────────

/// Stamp `tokens_valid_after = now()` for the caller so that every token issued
/// before this instant is rejected by the auth middleware ("sign out
/// everywhere"). Returns the new cutoff so the handler can echo it.
pub async fn revoke_all_tokens(
    pool: &PgPool,
    user_id: Uuid,
) -> sqlx::Result<chrono::DateTime<chrono::Utc>> {
    let cutoff: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "UPDATE users SET tokens_valid_after = now() WHERE id = $1 RETURNING tokens_valid_after",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(cutoff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tombstone_label_is_stable_and_prefixed() {
        let id = Uuid::parse_str("3f9a1c2b-0000-0000-0000-000000000000").unwrap();
        let label = tombstone_label(id);
        assert_eq!(label, "deleted-user-3f9a1c2b");
        // Stable for the same id.
        assert_eq!(label, tombstone_label(id));
    }

    #[test]
    fn tombstone_label_differs_by_user() {
        // The label is derived from the FIRST UUID segment (first 8 hex chars),
        // so the two ids must differ there — `from_u128(1)`/`from_u128(2)` only
        // differ in the last digit and would collide on the label.
        let a = tombstone_label(Uuid::from_u128(0x1111_1111_2222_3333_4444_5555_6666_7777));
        let b = tombstone_label(Uuid::from_u128(0x8888_8888_2222_3333_4444_5555_6666_7777));
        assert_ne!(a, b);
    }
}
