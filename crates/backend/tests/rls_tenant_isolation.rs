//! Verifies Postgres RLS prevents cross-tenant reads when app.tenant_id is set.

use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn pool() -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://aulalite:changeme@localhost:55432/aulalite".into());
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to test DB")
}

fn role_ident(role_name: &str) -> String {
    format!(r#""{role_name}""#)
}

async fn create_rls_test_role(conn: &mut sqlx::PgConnection) -> anyhow::Result<String> {
    let role_name = format!("rls_test_{}", Uuid::new_v4().simple());
    let role = role_ident(&role_name);

    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE ROLE {role}")))
        .execute(&mut *conn)
        .await?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "GRANT USAGE ON SCHEMA public TO {role}"
    )))
    .execute(&mut *conn)
    .await?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "GRANT SELECT ON tenants, tenant_memberships TO {role}"
    )))
    .execute(&mut *conn)
    .await?;

    Ok(role_name)
}

async fn seed_membership(
    conn: &mut sqlx::PgConnection,
    tenant_id: Uuid,
    label: &str,
) -> anyhow::Result<()> {
    // This fixture only exercises tenant isolation and intentionally has no
    // membership graph. Keep it in the legacy/uninitialized state so the
    // deferred organization-owner invariant does not apply.
    sqlx::query(
        "INSERT INTO tenants (id, slug, name, status, ownership_initialized_at) \
         VALUES ($1, $2, $3, 'active', NULL)",
    )
    .bind(tenant_id)
    .bind(format!("ten_{}_{}", label, tenant_id.simple()))
    .bind(format!("Tenant {label}"))
    .execute(&mut *conn)
    .await?;

    Ok(())
}

async fn seed_user(conn: &mut sqlx::PgConnection, user_id: Uuid) -> anyhow::Result<()> {
    let firebase_uid = format!("fbuid_{}", Uuid::new_v4().simple());
    let email = format!("user_{}@example.test", Uuid::new_v4().simple());

    sqlx::query("INSERT INTO users (id, firebase_uid, email) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(firebase_uid)
        .bind(email)
        .execute(&mut *conn)
        .await?;

    Ok(())
}

async fn set_local_tenant(conn: &mut sqlx::PgConnection, tenant_id: Uuid) -> anyhow::Result<()> {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *conn)
        .await?;
    Ok(())
}

#[tokio::test]
async fn rls_tables_force_row_security() -> anyhow::Result<()> {
    let pool = pool().await;
    let forced: Vec<(String, bool)> = sqlx::query_as(
        "SELECT relname, relforcerowsecurity
         FROM pg_class
         WHERE oid IN ('tenants'::regclass, 'tenant_memberships'::regclass)
         ORDER BY relname",
    )
    .fetch_all(&pool)
    .await?;

    assert_eq!(
        forced,
        vec![
            ("tenant_memberships".to_string(), true),
            ("tenants".to_string(), true),
        ],
        "tenant-scoped tables must force RLS so table owners cannot bypass policies"
    );

    Ok(())
}

#[tokio::test]
async fn rls_blocks_cross_tenant_reads_on_tenant_memberships() -> anyhow::Result<()> {
    let pool = pool().await;

    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN").execute(&mut *conn).await?;

    let test_result = async {
        seed_membership(&mut *conn, tenant_a, "a").await?;
        seed_membership(&mut *conn, tenant_b, "b").await?;
        seed_user(&mut *conn, user_id).await?;
        sqlx::query(
            "INSERT INTO tenant_memberships (tenant_id, user_id, role) VALUES ($1, $2, 'org_admin')",
        )
        .bind(tenant_a)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;

        let role_name = create_rls_test_role(&mut *conn).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!("SET LOCAL ROLE {}", role_ident(&role_name))))
            .execute(&mut *conn)
            .await?;
        set_local_tenant(&mut *conn, tenant_b).await?;

        let visible: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM tenant_memberships WHERE tenant_id = $1",
        )
        .bind(tenant_a)
        .fetch_one(&mut *conn)
        .await?;

        anyhow::Ok(visible.0)
    }
    .await;

    let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;

    assert_eq!(
        test_result?, 0,
        "tenant B must not see tenant A's memberships"
    );

    Ok(())
}

#[tokio::test]
async fn rls_allows_same_tenant_reads_on_tenant_memberships() -> anyhow::Result<()> {
    let pool = pool().await;

    let tenant = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN").execute(&mut *conn).await?;

    let test_result = async {
        seed_membership(&mut *conn, tenant, "same").await?;
        seed_user(&mut *conn, user_id).await?;
        sqlx::query(
            "INSERT INTO tenant_memberships (tenant_id, user_id, role) VALUES ($1, $2, 'org_admin')",
        )
        .bind(tenant)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;

        let role_name = create_rls_test_role(&mut *conn).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!("SET LOCAL ROLE {}", role_ident(&role_name))))
            .execute(&mut *conn)
            .await?;
        set_local_tenant(&mut *conn, tenant).await?;

        let visible: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM tenant_memberships WHERE tenant_id = $1",
        )
        .bind(tenant)
        .fetch_one(&mut *conn)
        .await?;

        anyhow::Ok(visible.0)
    }
    .await;

    let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;

    assert_eq!(test_result?, 1, "same tenant must see its own memberships");

    Ok(())
}

/// Phase 1a tables that all carry `tenant_id` and `FORCE ROW LEVEL SECURITY`.
const PHASE_1A_TENANT_TABLES: &[&str] = &[
    "courses",
    "modules",
    "lessons",
    "course_memberships",
    "enrollment_codes",
    "course_invitations",
    "live_session_series",
    "live_sessions",
    "file_assets",
];

/// Phase 1b-γ/δ tables that carry `tenant_id` and `FORCE ROW LEVEL SECURITY`.
const PHASE_1B_GAMMA_TABLES: &[&str] = &[
    "live_room_messages",
    "live_room_kicks",
    "recordings",
    "assignments",
    "submissions",
];

async fn create_rls_test_role_phase_1a(conn: &mut sqlx::PgConnection) -> anyhow::Result<String> {
    let role_name = format!("rls_test_p1a_{}", Uuid::new_v4().simple());
    let role = role_ident(&role_name);

    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE ROLE {role}")))
        .execute(&mut *conn)
        .await?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "GRANT USAGE ON SCHEMA public TO {role}"
    )))
    .execute(&mut *conn)
    .await?;
    let p1a_grants = PHASE_1A_TENANT_TABLES.join(", ");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "GRANT SELECT ON {p1a_grants} TO {role}"
    )))
    .execute(&mut *conn)
    .await?;
    let p1b_grants = PHASE_1B_GAMMA_TABLES.join(", ");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "GRANT SELECT ON {p1b_grants} TO {role}"
    )))
    .execute(&mut *conn)
    .await?;

    Ok(role_name)
}

#[tokio::test]
async fn rls_phase_1a_tables_force_row_security() -> anyhow::Result<()> {
    let pool = pool().await;

    let rows: Vec<(String, bool)> = sqlx::query_as(
        "SELECT relname, relforcerowsecurity
         FROM pg_class
         WHERE relname = ANY($1::text[])
           AND relkind = 'r'
           AND relnamespace = 'public'::regnamespace
         ORDER BY relname",
    )
    .bind(PHASE_1A_TENANT_TABLES)
    .fetch_all(&pool)
    .await?;

    let actual_forced: Vec<String> = rows
        .iter()
        .filter(|(_, force)| *force)
        .map(|(name, _)| name.clone())
        .collect();
    let mut expected: Vec<String> = PHASE_1A_TENANT_TABLES
        .iter()
        .map(|s| s.to_string())
        .collect();
    expected.sort();

    assert_eq!(
        actual_forced, expected,
        "every Phase 1a tenant table must exist in public schema and FORCE RLS"
    );

    Ok(())
}

#[tokio::test]
async fn system_context_unblocks_cross_tenant_sweep_reads() -> anyhow::Result<()> {
    // H1: the background sweeps (auto-end, recording, retention) read across
    // tenants on a connection that sets NO app.tenant_id. Under a NOBYPASSRLS
    // role the tenant_isolation policy filters every row (the bug); the
    // system_context_* policies (migration 20260530000030) restore visibility
    // ONLY when app.system = 'on', which exclusively the trusted sweep code sets.
    let pool = pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();

    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN").execute(&mut *conn).await?;

    let result = async {
        seed_membership(&mut *conn, tenant_a, "sysa").await?;
        seed_membership(&mut *conn, tenant_b, "sysb").await?;

        let role_name = create_rls_test_role(&mut *conn).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SET LOCAL ROLE {}",
            role_ident(&role_name)
        )))
        .execute(&mut *conn)
        .await?;

        // No app.tenant_id and no app.system → tenant_isolation filters all rows.
        let without: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM tenants WHERE id = $1 OR id = $2")
                .bind(tenant_a)
                .bind(tenant_b)
                .fetch_one(&mut *conn)
                .await?;

        // app.system = 'on' → system_context_select permits the cross-tenant read.
        sqlx::query("SELECT set_config('app.system', 'on', true)")
            .execute(&mut *conn)
            .await?;
        let with_system: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM tenants WHERE id = $1 OR id = $2")
                .bind(tenant_a)
                .bind(tenant_b)
                .fetch_one(&mut *conn)
                .await?;

        anyhow::Ok((without.0, with_system.0))
    }
    .await;

    let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;

    let (without, with_system) = result?;
    assert_eq!(
        without, 0,
        "without app.system a non-bypass sweep sees no cross-tenant rows (the H1 bug)"
    );
    assert_eq!(
        with_system, 2,
        "with app.system='on' the sweep sees both tenants' rows"
    );

    Ok(())
}

#[tokio::test]
async fn rls_blocks_cross_tenant_reads_on_phase_1a_tables() -> anyhow::Result<()> {
    let pool = pool().await;

    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let course_id = Uuid::new_v4();
    let module_id = Uuid::new_v4();
    let lesson_id = Uuid::new_v4();
    let series_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let invite_id = Uuid::new_v4();
    let code_id = Uuid::new_v4();
    let asset_id = Uuid::new_v4();

    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN").execute(&mut *conn).await?;

    let test_result = async {
        seed_membership(&mut *conn, tenant_a, "a").await?;
        seed_membership(&mut *conn, tenant_b, "b").await?;
        seed_user(&mut *conn, user_id).await?;

        // Run all Phase 1a inserts under tenant A's GUC, as the connection
        // role (aulalite/superuser/BYPASSRLS — needed to satisfy CHECKs and
        // pre-policy seed before we step down to a non-superuser role).
        set_local_tenant(&mut *conn, tenant_a).await?;

        sqlx::query(
            "INSERT INTO courses (id, tenant_id, slug, title, owner_user_id)
             VALUES ($1, $2, $3, 'C', $4)",
        )
        .bind(course_id)
        .bind(tenant_a)
        .bind(format!("rls-c-{}", Uuid::new_v4().simple()))
        .bind(user_id)
        .execute(&mut *conn)
        .await?;

        sqlx::query(
            "INSERT INTO modules (id, tenant_id, course_id, title, sort_order)
             VALUES ($1, $2, $3, 'M', 10)",
        )
        .bind(module_id)
        .bind(tenant_a)
        .bind(course_id)
        .execute(&mut *conn)
        .await?;

        sqlx::query(
            "INSERT INTO lessons (id, tenant_id, course_id, module_id, type, title, sort_order)
             VALUES ($1, $2, $3, $4, 'rich_text', 'L', 10)",
        )
        .bind(lesson_id)
        .bind(tenant_a)
        .bind(course_id)
        .bind(module_id)
        .execute(&mut *conn)
        .await?;

        sqlx::query(
            "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
             VALUES ($1, $2, $3, 'teacher')",
        )
        .bind(course_id)
        .bind(user_id)
        .bind(tenant_a)
        .execute(&mut *conn)
        .await?;

        sqlx::query(
            "INSERT INTO enrollment_codes (id, tenant_id, course_id, code, created_by)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(code_id)
        .bind(tenant_a)
        .bind(course_id)
        .bind(format!("RLS{}", &Uuid::new_v4().simple().to_string()[..8]))
        .bind(user_id)
        .execute(&mut *conn)
        .await?;

        sqlx::query(
            "INSERT INTO course_invitations
                (id, tenant_id, course_id, email, role, token, expires_at, created_by)
             VALUES ($1, $2, $3, 'rls@example.test', 'student', $4, now() + interval '14 days', $5)",
        )
        .bind(invite_id)
        .bind(tenant_a)
        .bind(course_id)
        .bind(format!("tok-{}", Uuid::new_v4().simple()))
        .bind(user_id)
        .execute(&mut *conn)
        .await?;

        sqlx::query(
            "INSERT INTO live_session_series
                (id, tenant_id, course_id, title, starts_at, duration_minutes,
                 frequency, byweekday, end_kind, occurrence_count, primary_teacher_id)
             VALUES ($1, $2, $3, 'S', now(), 60, 'none', NULL, 'count', 1, $4)",
        )
        .bind(series_id)
        .bind(tenant_a)
        .bind(course_id)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;

        sqlx::query(
            "INSERT INTO live_sessions
                (id, tenant_id, course_id, series_id, occurrence_index, title,
                 starts_at, duration_minutes, primary_teacher_id, recording_enabled)
             VALUES ($1, $2, $3, $4, 0, 'S', now(), 60, $5, true)",
        )
        .bind(session_id)
        .bind(tenant_a)
        .bind(course_id)
        .bind(series_id)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;

        sqlx::query(
            "INSERT INTO file_assets
                (id, tenant_id, owner_user_id, bucket, object_key, content_type, size_bytes)
             VALUES ($1, $2, $3, 'aulalite', $4, 'application/octet-stream', 1)",
        )
        .bind(asset_id)
        .bind(tenant_a)
        .bind(user_id)
        .bind(format!("k-{}", Uuid::new_v4().simple()))
        .execute(&mut *conn)
        .await?;

        // Now step down to a non-superuser role and switch tenant context.
        let role_name = create_rls_test_role_phase_1a(&mut *conn).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!("SET LOCAL ROLE {}", role_ident(&role_name))))
            .execute(&mut *conn)
            .await?;
        set_local_tenant(&mut *conn, tenant_b).await?;

        // For each table, count rows visible under tenant B — must be 0.
        let mut visible_counts: Vec<(String, i64)> = Vec::new();
        for table in PHASE_1A_TENANT_TABLES {
            let q = format!("SELECT COUNT(*) FROM {table}");
            let (count,): (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(q.as_str())).fetch_one(&mut *conn).await?;
            visible_counts.push(((*table).to_string(), count));
        }

        anyhow::Ok(visible_counts)
    }
    .await;

    let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;

    let counts = test_result?;
    for (table, count) in counts {
        assert_eq!(
            count, 0,
            "tenant B must not see tenant A's rows in {table} (got {count})"
        );
    }

    Ok(())
}

#[tokio::test]
async fn cross_tenant_get_url_route_returns_404() -> anyhow::Result<()> {
    let pool = pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let user_a_id = Uuid::new_v4();
    let user_b_id = Uuid::new_v4();
    let asset_id = Uuid::new_v4();

    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN").execute(&mut *conn).await?;

    let test_result = async {
        seed_membership(&mut *conn, tenant_a, "a").await?;
        seed_membership(&mut *conn, tenant_b, "b").await?;
        seed_user(&mut *conn, user_a_id).await?;
        seed_user(&mut *conn, user_b_id).await?;

        sqlx::query(
            "INSERT INTO file_assets
                (id, tenant_id, owner_user_id, bucket, object_key,
                 content_type, size_bytes)
             VALUES ($1, $2, $3, 'aulalite', $4, 'image/png', 1)",
        )
        .bind(asset_id)
        .bind(tenant_a)
        .bind(user_a_id)
        .bind(format!("k-{}", Uuid::new_v4().simple()))
        .execute(&mut *conn)
        .await?;

        let role_name = create_rls_test_role_phase_1a(&mut *conn).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SET LOCAL ROLE {}",
            role_ident(&role_name)
        )))
        .execute(&mut *conn)
        .await?;
        set_local_tenant(&mut *conn, tenant_b).await?;

        let visible: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM file_assets WHERE id = $1")
            .bind(asset_id)
            .fetch_one(&mut *conn)
            .await?;

        anyhow::Ok(visible.0)
    }
    .await;

    let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;

    assert_eq!(
        test_result?, 0,
        "tenant B must not see tenant A's file_assets"
    );

    Ok(())
}

#[tokio::test]
async fn cross_tenant_live_session_with_publish_nonce_masked() -> anyhow::Result<()> {
    let pool = pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let user_a = Uuid::new_v4();
    let course_a = Uuid::new_v4();
    let series_a = Uuid::new_v4();
    let session_a = Uuid::new_v4();

    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN").execute(&mut *conn).await?;

    let test_result = async {
        seed_membership(&mut *conn, tenant_a, "a").await?;
        seed_membership(&mut *conn, tenant_b, "b").await?;
        seed_user(&mut *conn, user_a).await?;

        sqlx::query(
            "INSERT INTO courses (id, tenant_id, slug, title, owner_user_id)
             VALUES ($1, $2, $3, 'C', $4)",
        )
        .bind(course_a)
        .bind(tenant_a)
        .bind(format!("c-{}", Uuid::new_v4().simple()))
        .bind(user_a)
        .execute(&mut *conn)
        .await?;

        sqlx::query(
            "INSERT INTO live_session_series
                (id, tenant_id, course_id, title, starts_at, duration_minutes,
                 frequency, byweekday, end_kind, occurrence_count,
                 primary_teacher_id, transport_mode)
             VALUES ($1, $2, $3, 'S', now(), 60,
                     'none', NULL, 'count', 1,
                     $4, 'webrtc')",
        )
        .bind(series_a)
        .bind(tenant_a)
        .bind(course_a)
        .bind(user_a)
        .execute(&mut *conn)
        .await?;

        sqlx::query(
            "INSERT INTO live_sessions
                (id, tenant_id, course_id, series_id, occurrence_index, title, status,
                 starts_at, duration_minutes, primary_teacher_id, mode,
                 recording_enabled, transport_mode,
                 main_path, screen_path, publish_nonce, publish_nonce_expires_at)
             VALUES ($1, $2, $3, $4, 0, 'L', 'live',
                     now(), 60, $5, 'lecture',
                     false, 'webrtc',
                     'aula/main', 'aula/main/screen', 'somenoncehash',
                     now() + interval '1 hour')",
        )
        .bind(session_a)
        .bind(tenant_a)
        .bind(course_a)
        .bind(series_a)
        .bind(user_a)
        .execute(&mut *conn)
        .await?;

        let role_name = create_rls_test_role_phase_1a(&mut *conn).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SET LOCAL ROLE {}",
            role_ident(&role_name)
        )))
        .execute(&mut *conn)
        .await?;
        set_local_tenant(&mut *conn, tenant_b).await?;

        let visible: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM live_sessions WHERE id = $1")
            .bind(session_a)
            .fetch_one(&mut *conn)
            .await?;

        anyhow::Ok(visible.0)
    }
    .await;

    let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
    assert_eq!(
        test_result?, 0,
        "tenant B must not see tenant A's live_sessions row"
    );
    Ok(())
}

#[tokio::test]
async fn cross_tenant_live_room_messages_masked() -> anyhow::Result<()> {
    let pool = pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let user_a = Uuid::new_v4();
    let course_a = Uuid::new_v4();
    let series_a = Uuid::new_v4();
    let session_a = Uuid::new_v4();
    let msg_a = Uuid::new_v4();

    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN").execute(&mut *conn).await?;

    let result = async {
        seed_membership(&mut *conn, tenant_a, "a").await?;
        seed_membership(&mut *conn, tenant_b, "b").await?;
        seed_user(&mut *conn, user_a).await?;
        sqlx::query("INSERT INTO courses (id, tenant_id, slug, title, owner_user_id) VALUES ($1,$2,$3,'C',$4)")
            .bind(course_a).bind(tenant_a).bind(format!("c-{}", Uuid::new_v4())).bind(user_a)
            .execute(&mut *conn).await?;
        sqlx::query(
            "INSERT INTO live_session_series (id, tenant_id, course_id, title, starts_at,
                                              duration_minutes, frequency, end_kind,
                                              transport_mode, primary_teacher_id, recording_enabled)
             VALUES ($1, $2, $3, 'S', now(), 60, 'none', 'open', 'webrtc', $4, false)"
        ).bind(series_a).bind(tenant_a).bind(course_a).bind(user_a).execute(&mut *conn).await?;
        sqlx::query(
            "INSERT INTO live_sessions (id, tenant_id, course_id, series_id, occurrence_index, title,
                                        status, starts_at, duration_minutes, primary_teacher_id, mode,
                                        recording_enabled, transport_mode)
             VALUES ($1, $2, $3, $4, 0, 'L', 'live', now(), 60, $5, 'lecture', false, 'webrtc')"
        ).bind(session_a).bind(tenant_a).bind(course_a).bind(series_a).bind(user_a).execute(&mut *conn).await?;
        sqlx::query(
            "INSERT INTO live_room_messages (id, tenant_id, session_id, sender_user_id, body)
             VALUES ($1, $2, $3, $4, 'tenant-A-secret')"
        ).bind(msg_a).bind(tenant_a).bind(session_a).bind(user_a).execute(&mut *conn).await?;

        let role_name = create_rls_test_role_phase_1a(&mut *conn).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!("SET LOCAL ROLE {}", role_ident(&role_name))))
            .execute(&mut *conn).await?;
        set_local_tenant(&mut *conn, tenant_b).await?;

        let visible: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM live_room_messages WHERE id = $1"
        ).bind(msg_a).fetch_one(&mut *conn).await?;
        anyhow::Ok(visible.0)
    }.await;

    let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
    assert_eq!(result?, 0, "tenant B must not see tenant A's chat messages");
    Ok(())
}

#[tokio::test]
async fn cross_tenant_live_room_kicks_masked() -> anyhow::Result<()> {
    let pool = pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let user_a = Uuid::new_v4();
    let kick_a = Uuid::new_v4();
    let course_a = Uuid::new_v4();
    let series_a = Uuid::new_v4();
    let session_a = Uuid::new_v4();

    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN").execute(&mut *conn).await?;

    let result = async {
        seed_membership(&mut *conn, tenant_a, "a").await?;
        seed_membership(&mut *conn, tenant_b, "b").await?;
        seed_user(&mut *conn, user_a).await?;
        sqlx::query("INSERT INTO courses (id, tenant_id, slug, title, owner_user_id) VALUES ($1,$2,$3,'C',$4)")
            .bind(course_a).bind(tenant_a).bind(format!("c-{}", Uuid::new_v4())).bind(user_a)
            .execute(&mut *conn).await?;
        sqlx::query(
            "INSERT INTO live_session_series (id, tenant_id, course_id, title, starts_at,
                                              duration_minutes, frequency, end_kind,
                                              transport_mode, primary_teacher_id, recording_enabled)
             VALUES ($1, $2, $3, 'S', now(), 60, 'none', 'open', 'webrtc', $4, false)"
        ).bind(series_a).bind(tenant_a).bind(course_a).bind(user_a).execute(&mut *conn).await?;
        sqlx::query(
            "INSERT INTO live_sessions (id, tenant_id, course_id, series_id, occurrence_index, title,
                                        status, starts_at, duration_minutes, primary_teacher_id, mode,
                                        recording_enabled, transport_mode)
             VALUES ($1, $2, $3, $4, 0, 'L', 'live', now(), 60, $5, 'lecture', false, 'webrtc')"
        ).bind(session_a).bind(tenant_a).bind(course_a).bind(series_a).bind(user_a).execute(&mut *conn).await?;
        sqlx::query(
            "INSERT INTO live_room_kicks (id, tenant_id, session_id, user_id, kicked_by_user_id)
             VALUES ($1, $2, $3, $4, $5)"
        ).bind(kick_a).bind(tenant_a).bind(session_a).bind(user_a).bind(user_a)
            .execute(&mut *conn).await?;

        let role_name = create_rls_test_role_phase_1a(&mut *conn).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!("SET LOCAL ROLE {}", role_ident(&role_name))))
            .execute(&mut *conn).await?;
        set_local_tenant(&mut *conn, tenant_b).await?;

        let visible: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM live_room_kicks WHERE id = $1"
        ).bind(kick_a).fetch_one(&mut *conn).await?;
        anyhow::Ok(visible.0)
    }.await;

    let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
    assert_eq!(result?, 0, "tenant B must not see tenant A's kick records");
    Ok(())
}

#[tokio::test]
async fn cross_tenant_recordings_masked() -> anyhow::Result<()> {
    let pool = pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let user_a = Uuid::new_v4();
    let course_a = Uuid::new_v4();
    let series_a = Uuid::new_v4();
    let session_a = Uuid::new_v4();
    let recording_a = Uuid::new_v4();

    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN").execute(&mut *conn).await?;

    let result = async {
        seed_membership(&mut *conn, tenant_a, "a").await?;
        seed_membership(&mut *conn, tenant_b, "b").await?;
        seed_user(&mut *conn, user_a).await?;
        sqlx::query("INSERT INTO courses (id, tenant_id, slug, title, owner_user_id) VALUES ($1,$2,$3,'C',$4)")
            .bind(course_a).bind(tenant_a).bind(format!("c-{}", Uuid::new_v4())).bind(user_a)
            .execute(&mut *conn).await?;
        sqlx::query(
            "INSERT INTO live_session_series (id, tenant_id, course_id, title, starts_at,
                                              duration_minutes, frequency, end_kind,
                                              transport_mode, primary_teacher_id, recording_enabled)
             VALUES ($1, $2, $3, 'S', now(), 60, 'none', 'open', 'webrtc', $4, true)"
        ).bind(series_a).bind(tenant_a).bind(course_a).bind(user_a).execute(&mut *conn).await?;
        sqlx::query(
            "INSERT INTO live_sessions (id, tenant_id, course_id, series_id, occurrence_index, title,
                                        status, starts_at, duration_minutes, primary_teacher_id, mode,
                                        recording_enabled, transport_mode)
             VALUES ($1, $2, $3, $4, 0, 'L', 'ended', now(), 60, $5, 'lecture', true, 'webrtc')"
        ).bind(session_a).bind(tenant_a).bind(course_a).bind(series_a).bind(user_a).execute(&mut *conn).await?;
        sqlx::query(
            "INSERT INTO recordings (id, tenant_id, session_id, started_at, ended_at,
                                     duration_seconds, processing_status)
             VALUES ($1, $2, $3, now() - interval '1 hour', now(), 3600, 'available')"
        ).bind(recording_a).bind(tenant_a).bind(session_a).execute(&mut *conn).await?;

        let role_name = create_rls_test_role_phase_1a(&mut *conn).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!("SET LOCAL ROLE {}", role_ident(&role_name))))
            .execute(&mut *conn).await?;
        set_local_tenant(&mut *conn, tenant_b).await?;

        let visible: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM recordings WHERE id = $1"
        ).bind(recording_a).fetch_one(&mut *conn).await?;
        anyhow::Ok(visible.0)
    }.await;

    let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
    assert_eq!(result?, 0, "tenant B must not see tenant A's recordings");
    Ok(())
}

#[tokio::test]
async fn cross_tenant_assignments_masked() -> anyhow::Result<()> {
    let pool = pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let teacher_a = Uuid::new_v4();

    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN").execute(&mut *conn).await?;

    let result = async {
        seed_membership(&mut *conn, tenant_a, "a").await?;
        seed_membership(&mut *conn, tenant_b, "b").await?;
        seed_user(&mut *conn, teacher_a).await?;
        sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
            .bind(tenant_a.to_string())
            .execute(&mut *conn)
            .await?;
        let course_a: Uuid = sqlx::query_scalar(
            "INSERT INTO courses (tenant_id, slug, title, owner_user_id, status)
             VALUES ($1,$2,'A',$3,'published') RETURNING id",
        )
        .bind(tenant_a)
        .bind(format!("a-{}", tenant_a.simple()))
        .bind(teacher_a)
        .fetch_one(&mut *conn)
        .await?;
        let assn_a: Uuid = sqlx::query_scalar(
            "INSERT INTO assignments (tenant_id, course_id, title, grading_mode, max_points,
                                      status, created_by)
             VALUES ($1,$2,'X','numeric',100,'published',$3) RETURNING id",
        )
        .bind(tenant_a)
        .bind(course_a)
        .bind(teacher_a)
        .fetch_one(&mut *conn)
        .await?;

        let role_name = create_rls_test_role_phase_1a(&mut *conn).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SET LOCAL ROLE {}",
            role_ident(&role_name)
        )))
        .execute(&mut *conn)
        .await?;
        set_local_tenant(&mut *conn, tenant_b).await?;
        let visible: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM assignments WHERE id = $1")
            .bind(assn_a)
            .fetch_one(&mut *conn)
            .await?;
        anyhow::Ok(visible.0)
    }
    .await;

    let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
    assert_eq!(result?, 0, "tenant B should not see tenant A assignments");
    Ok(())
}

#[tokio::test]
async fn cross_tenant_submissions_masked() -> anyhow::Result<()> {
    let pool = pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let teacher_a = Uuid::new_v4();
    let student_a = Uuid::new_v4();

    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN").execute(&mut *conn).await?;

    let result =
        async {
            seed_membership(&mut *conn, tenant_a, "a").await?;
            seed_membership(&mut *conn, tenant_b, "b").await?;
            seed_user(&mut *conn, teacher_a).await?;
            seed_user(&mut *conn, student_a).await?;
            sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
                .bind(tenant_a.to_string())
                .execute(&mut *conn)
                .await?;
            let course_a: Uuid = sqlx::query_scalar(
                "INSERT INTO courses (tenant_id, slug, title, owner_user_id, status)
             VALUES ($1,$2,'A',$3,'published') RETURNING id",
            )
            .bind(tenant_a)
            .bind(format!("a-{}", tenant_a.simple()))
            .bind(teacher_a)
            .fetch_one(&mut *conn)
            .await?;
            let assn_a: Uuid = sqlx::query_scalar(
                "INSERT INTO assignments (tenant_id, course_id, title, grading_mode, max_points,
                                      status, created_by)
             VALUES ($1,$2,'X','numeric',100,'published',$3) RETURNING id",
            )
            .bind(tenant_a)
            .bind(course_a)
            .bind(teacher_a)
            .fetch_one(&mut *conn)
            .await?;
            let sub_a: Uuid = sqlx::query_scalar(
            "INSERT INTO submissions (tenant_id, assignment_id, course_id, student_user_id, status)
             VALUES ($1,$2,$3,$4,'draft') RETURNING id",
        ).bind(tenant_a).bind(assn_a).bind(course_a).bind(student_a)
         .fetch_one(&mut *conn).await?;

            let role_name = create_rls_test_role_phase_1a(&mut *conn).await?;
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "SET LOCAL ROLE {}",
                role_ident(&role_name)
            )))
            .execute(&mut *conn)
            .await?;
            set_local_tenant(&mut *conn, tenant_b).await?;
            let visible: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM submissions WHERE id = $1")
                .bind(sub_a)
                .fetch_one(&mut *conn)
                .await?;
            anyhow::Ok(visible.0)
        }
        .await;

    let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
    assert_eq!(result?, 0, "tenant B should not see tenant A submissions");
    Ok(())
}
