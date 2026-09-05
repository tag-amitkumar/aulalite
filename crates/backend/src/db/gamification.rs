//! Gamification queries + the award engine (learning-suite Cycle 4).
//!
//! `award` is the single entry point: idempotent via `xp_events.dedup_key`,
//! it bumps `learner_stats` (XP + daily streak) and evaluates achievement
//! unlocks inside the caller's transaction. Callers treat it as best-effort
//! — a gamification failure must never fail the triggering action.

use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;

/// XP values per award kind.
pub const XP_LESSON_COMPLETED: i32 = 10;
pub const XP_QUIZ_SUBMITTED: i32 = 25;
pub const XP_QUIZ_PERFECT: i32 = 15;
pub const XP_SESSION_ATTENDED: i32 = 15;

/// Level curve: level n spans 100·n XP, so the cumulative XP to FINISH
/// level n is 100·n·(n+1)/2. Returns (level, xp_into_level, level_span).
pub fn level_for_xp(total_xp: i64) -> (u32, i64, i64) {
    let mut level = 1u32;
    let mut consumed = 0i64;
    loop {
        let span = 100 * level as i64;
        if total_xp < consumed + span {
            return (level, total_xp - consumed, span);
        }
        consumed += span;
        level += 1;
    }
}

pub struct AwardOutcome {
    /// False when the dedup key already existed (no-op).
    pub awarded: bool,
    /// Newly unlocked achievement ids.
    pub unlocked: Vec<String>,
}

/// Record an XP event (idempotent), update streak/XP rollups, and evaluate
/// achievements. `dedup_key` must uniquely identify the logical action.
pub async fn award(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    user_id: Uuid,
    course_id: Option<Uuid>,
    kind: &str,
    points: i32,
    dedup_key: &str,
) -> sqlx::Result<AwardOutcome> {
    let inserted = sqlx::query(
        "INSERT INTO xp_events (tenant_id, user_id, course_id, kind, points, dedup_key) \
         VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (dedup_key) DO NOTHING",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(course_id)
    .bind(kind)
    .bind(points)
    .bind(dedup_key)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        > 0;
    if !inserted {
        return Ok(AwardOutcome {
            awarded: false,
            unlocked: Vec::new(),
        });
    }

    // Streak: any qualifying activity today extends (or resets) the chain.
    // The upsert arithmetic runs entirely server-side on CURRENT_DATE.
    sqlx::query(
        "INSERT INTO learner_stats \
            (tenant_id, user_id, total_xp, current_streak_days, longest_streak_days, last_activity_date) \
         VALUES ($1, $2, $3, 1, 1, CURRENT_DATE) \
         ON CONFLICT (tenant_id, user_id) DO UPDATE SET \
            total_xp = learner_stats.total_xp + $3, \
            current_streak_days = CASE \
                WHEN learner_stats.last_activity_date = CURRENT_DATE \
                    THEN learner_stats.current_streak_days \
                WHEN learner_stats.last_activity_date = CURRENT_DATE - 1 \
                    THEN learner_stats.current_streak_days + 1 \
                ELSE 1 END, \
            longest_streak_days = GREATEST(learner_stats.longest_streak_days, CASE \
                WHEN learner_stats.last_activity_date = CURRENT_DATE \
                    THEN learner_stats.current_streak_days \
                WHEN learner_stats.last_activity_date = CURRENT_DATE - 1 \
                    THEN learner_stats.current_streak_days + 1 \
                ELSE 1 END), \
            last_activity_date = CURRENT_DATE",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(points as i64)
    .execute(&mut **tx)
    .await?;

    // Achievement evaluation on the fresh stats.
    let (total_xp, streak): (i64, i32) = sqlx::query_as(
        "SELECT total_xp, current_streak_days FROM learner_stats \
         WHERE tenant_id = $1 AND user_id = $2",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await?;

    let mut candidates: Vec<&str> = Vec::new();
    match kind {
        "lesson_completed" => candidates.push("first_lesson"),
        "quiz_submitted" => candidates.push("first_quiz"),
        "quiz_perfect" => candidates.push("perfect_quiz"),
        _ => {}
    }
    if streak >= 7 {
        candidates.push("streak_7");
    }
    if total_xp >= 1000 {
        candidates.push("xp_1000");
    }

    let mut unlocked = Vec::new();
    for achievement_id in candidates {
        let fresh = sqlx::query(
            "INSERT INTO achievement_unlocks (tenant_id, user_id, achievement_id) \
             VALUES ($1, $2, $3) ON CONFLICT (user_id, achievement_id) DO NOTHING",
        )
        .bind(tenant_id)
        .bind(user_id)
        .bind(achievement_id)
        .execute(&mut **tx)
        .await?
        .rows_affected()
            > 0;
        if fresh {
            unlocked.push(achievement_id.to_string());
        }
    }

    Ok(AwardOutcome {
        awarded: true,
        unlocked,
    })
}

#[derive(Debug, sqlx::FromRow)]
pub struct LearnerStatsRow {
    pub total_xp: i64,
    pub current_streak_days: i32,
    pub longest_streak_days: i32,
}

pub async fn learner_stats(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<LearnerStatsRow>> {
    sqlx::query_as(
        "SELECT total_xp, current_streak_days, longest_streak_days \
         FROM learner_stats WHERE tenant_id = $1 AND user_id = $2",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_optional(conn)
    .await
}

/// Did the learner do anything today? (Streak display: an unbroken chain
/// that hasn't ticked today renders as inactive/nudge.)
pub async fn active_today(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<bool> {
    let row: Option<bool> = sqlx::query_scalar(
        "SELECT last_activity_date = CURRENT_DATE FROM learner_stats \
         WHERE tenant_id = $1 AND user_id = $2",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_optional(conn)
    .await?;
    Ok(row.unwrap_or(false))
}

/// Unlocks for the user, newest first: (id, title, description, seen).
pub async fn unlocks(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Vec<(String, String, String, bool)>> {
    sqlx::query_as(
        "SELECT a.id, a.title, a.description, au.seen \
         FROM achievement_unlocks au JOIN achievements a ON a.id = au.achievement_id \
         WHERE au.tenant_id = $1 AND au.user_id = $2 \
         ORDER BY au.unlocked_at DESC",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_all(conn)
    .await
}

pub async fn mark_unlocks_seen(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE achievement_unlocks SET seen = TRUE \
         WHERE tenant_id = $1 AND user_id = $2 AND seen = FALSE",
    )
    .bind(tenant_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Course leaderboard: XP earned within the course by active students who
/// have not opted out: (user_id, display_name, email, course_xp).
pub async fn course_leaderboard(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    course_id: Uuid,
    limit: i64,
) -> sqlx::Result<Vec<(Uuid, Option<String>, String, i64)>> {
    sqlx::query_as(
        "SELECT cm.user_id, u.display_name, u.email::text, \
                COALESCE(sum(x.points), 0) AS course_xp \
         FROM course_memberships cm \
         JOIN users u ON u.id = cm.user_id \
         LEFT JOIN xp_events x \
             ON x.user_id = cm.user_id AND x.course_id = cm.course_id \
         WHERE cm.course_id = $1 AND cm.tenant_id = $2 \
           AND cm.role = 'student' AND cm.status = 'active' \
           AND NOT EXISTS (SELECT 1 FROM leaderboard_opt_outs o \
                           WHERE o.tenant_id = $2 AND o.user_id = cm.user_id) \
         GROUP BY cm.user_id, u.display_name, u.email \
         ORDER BY course_xp DESC, u.display_name NULLS LAST \
         LIMIT $3",
    )
    .bind(course_id)
    .bind(tenant_id)
    .bind(limit)
    .fetch_all(conn)
    .await
}

pub async fn set_leaderboard_opt_out(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    user_id: Uuid,
    opted_out: bool,
) -> sqlx::Result<()> {
    if opted_out {
        sqlx::query(
            "INSERT INTO leaderboard_opt_outs (tenant_id, user_id) VALUES ($1, $2) \
             ON CONFLICT DO NOTHING",
        )
        .bind(tenant_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    } else {
        sqlx::query("DELETE FROM leaderboard_opt_outs WHERE tenant_id = $1 AND user_id = $2")
            .bind(tenant_id)
            .bind(user_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

pub async fn is_opted_out(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<bool> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM leaderboard_opt_outs WHERE tenant_id = $1 AND user_id = $2",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_one(conn)
    .await?;
    Ok(count > 0)
}

#[cfg(test)]
mod tests {
    use super::level_for_xp;

    #[test]
    fn level_curve_is_triangular() {
        // Level 1 spans 0..100, level 2 spans 100..300, level 3 spans 300..600.
        assert_eq!(level_for_xp(0), (1, 0, 100));
        assert_eq!(level_for_xp(99), (1, 99, 100));
        assert_eq!(level_for_xp(100), (2, 0, 200));
        assert_eq!(level_for_xp(299), (2, 199, 200));
        assert_eq!(level_for_xp(300), (3, 0, 300));
    }
}
