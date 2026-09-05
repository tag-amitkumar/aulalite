use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct LessonRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub module_id: Uuid,
    pub r#type: String,
    pub title: String,
    pub body_md: Option<String>,
    pub video_asset_id: Option<Uuid>,
    pub live_session_id: Option<Uuid>,
    pub sort_order: i32,
}

#[derive(Debug, sqlx::FromRow)]
pub struct ModuleRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub sort_order: i32,
}

#[derive(Debug)]
pub struct ModuleWithLessonsRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub sort_order: i32,
    pub lessons: Vec<LessonRow>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct MemberRow {
    pub user_id: Uuid,
    pub display_name: Option<String>,
    pub email: String,
    pub role: String,
    pub status: String,
}

#[derive(Debug, sqlx::FromRow)]
pub struct SessionRow {
    pub session_id: Uuid,
    pub course_id: Uuid,
    pub course_title: String,
    pub course_slug: String,
    pub title: String,
    pub starts_at: chrono::DateTime<chrono::Utc>,
    pub duration_minutes: i32,
    pub status: String,
    pub diverged: bool,
}

pub async fn outline(
    conn: &mut sqlx::PgConnection,
    course_id: Uuid,
) -> sqlx::Result<Vec<ModuleWithLessonsRow>> {
    let modules = sqlx::query_as::<_, ModuleRow>(
        "SELECT id, course_id, title, sort_order
           FROM modules
          WHERE course_id = $1
          ORDER BY sort_order, title",
    )
    .bind(course_id)
    .fetch_all(&mut *conn)
    .await?;

    let lessons = sqlx::query_as::<_, LessonRow>(
        "SELECT id, course_id, module_id, type, title, body_md,
                video_asset_id, live_session_id, sort_order
           FROM lessons
          WHERE course_id = $1
          ORDER BY module_id, sort_order, title",
    )
    .bind(course_id)
    .fetch_all(&mut *conn)
    .await?;

    Ok(modules
        .into_iter()
        .map(|module| {
            let module_lessons = lessons
                .iter()
                .filter(|lesson| lesson.module_id == module.id)
                .map(|lesson| LessonRow {
                    id: lesson.id,
                    course_id: lesson.course_id,
                    module_id: lesson.module_id,
                    r#type: lesson.r#type.clone(),
                    title: lesson.title.clone(),
                    body_md: lesson.body_md.clone(),
                    video_asset_id: lesson.video_asset_id,
                    live_session_id: lesson.live_session_id,
                    sort_order: lesson.sort_order,
                })
                .collect();
            ModuleWithLessonsRow {
                id: module.id,
                course_id: module.course_id,
                title: module.title,
                sort_order: module.sort_order,
                lessons: module_lessons,
            }
        })
        .collect())
}

pub async fn members<'e, E>(executor: E, course_id: Uuid) -> sqlx::Result<Vec<MemberRow>>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, MemberRow>(
        "SELECT cm.user_id,
                u.display_name,
                u.email::text AS email,
                cm.role,
                cm.status
           FROM course_memberships cm
           JOIN users u ON u.id = cm.user_id
          WHERE cm.course_id = $1
            AND cm.status = 'active'
          ORDER BY cm.role, u.display_name NULLS LAST, u.email::text, cm.user_id",
    )
    .bind(course_id)
    .fetch_all(executor)
    .await
}

pub async fn sessions<'e, E>(executor: E, course_id: Uuid) -> sqlx::Result<Vec<SessionRow>>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, SessionRow>(
        "SELECT s.id AS session_id,
                s.course_id,
                c.title AS course_title,
                c.slug AS course_slug,
                s.title,
                s.starts_at,
                s.duration_minutes,
                s.status,
                s.diverged
           FROM live_sessions s
           JOIN courses c ON c.id = s.course_id
          WHERE s.course_id = $1
          ORDER BY s.starts_at, s.occurrence_index",
    )
    .bind(course_id)
    .fetch_all(executor)
    .await
}
