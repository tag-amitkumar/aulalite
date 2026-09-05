//! Parent-role HTTP endpoint integration tests.
//!
//! Flow exercised:
//!   * org admin creates a parent invitation for a student in their tenant;
//!   * the parent JIT-provisioning path (`accept_pending_for_email`) links the
//!     parent to the child + grants the 'parent' membership;
//!   * `/v1/parent/children` returns the child for the parent;
//!   * a parent reading an UNLINKED student's grades gets 403;
//!   * a non-parent (student) hitting `/v1/parent/*` gets 403;
//!   * the grades endpoint returns ONLY released grades.
//!
//! These require a live Postgres (DATABASE_URL); they are compile-checked in CI
//! and run locally against the dev database.

mod fixtures;

use fixtures::*;
use serde_json::json;
use uuid::Uuid;

async fn seed_blocking_seat_cap(pool: &sqlx::PgPool, tenant: Uuid, seats: i32) {
    let plan_id = format!("plan-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO plans (id, name, monthly_price_cents, included_seats,
                            included_class_minutes, included_recording_gb)
         VALUES ($1, 'Parent seat test', 0, $2, 0, 0)",
    )
    .bind(&plan_id)
    .bind(seats)
    .execute(pool)
    .await
    .unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO subscriptions (tenant_id, plan_id, status, overage_behavior)
         VALUES ($1, $2, 'active', 'block')",
    )
    .bind(tenant)
    .bind(plan_id)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

/// Insert a course (published) owned by `teacher` in `tenant`.
async fn create_course(pool: &sqlx::PgPool, tenant: Uuid, teacher: Uuid) -> Uuid {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id, status)
         VALUES ($1,$2,'C',$3,'published') RETURNING id",
    )
    .bind(tenant)
    .bind(format!("c-{}", Uuid::new_v4()))
    .bind(teacher)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Insert a published numeric assignment in `course`.
async fn create_assignment(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    course_id: Uuid,
    teacher: Uuid,
) -> Uuid {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query_scalar(
        "INSERT INTO assignments
            (tenant_id, course_id, title, grading_mode, max_points, status,
             published_at, created_by)
         VALUES ($1,$2,'Essay','numeric',100,'published',now(),$3)
         RETURNING id",
    )
    .bind(tenant)
    .bind(course_id)
    .bind(teacher)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Insert a graded submission. When `released` is true, `released_at` is set so
/// the row is visible to the student (and therefore the parent).
async fn create_graded_submission(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    aid: Uuid,
    course_id: Uuid,
    student: Uuid,
    grade: f64,
    released: bool,
) -> Uuid {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query_scalar(
        "INSERT INTO submissions
            (tenant_id, assignment_id, course_id, student_user_id, status,
             submitted_at, numeric_grade, student_visible_feedback,
             teacher_only_notes, graded_at, released_at)
         VALUES ($1,$2,$3,$4,'graded',now(),$5,'nice work','SECRET notes',
                 now(), CASE WHEN $6 THEN now() ELSE NULL END)
         RETURNING id",
    )
    .bind(tenant)
    .bind(aid)
    .bind(course_id)
    .bind(student)
    .bind(grade)
    .bind(released)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Drive the parent JIT-acceptance path: provision the parent's link +
/// membership for every pending invitation matching `email`, exactly as the
/// auth flow does on first sign-in.
async fn accept_as_parent(pool: &sqlx::PgPool, parent_user_id: Uuid, email: &str) -> usize {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(parent_user_id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let n = backend::db::parent::accept_pending_for_email(&mut tx, parent_user_id, email)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    n.accepted
}

fn parent_app(
    pool: &sqlx::PgPool,
    parent: Uuid,
    email: String,
    tenant: Option<Uuid>,
) -> axum::Router {
    build_test_app(
        backend::handlers::parent::router_for_tests(
            pool.clone(),
            std::sync::Arc::new(backend::services::invitations::mock::MockEmailLinkSender::new()),
            "http://localhost:3000".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: parent,
            firebase_uid: format!("fb-{}", Uuid::new_v4()),
            email,
            tenant_id: tenant,
            tenant_role: Some(core_types::TenantRole::Parent),
        },
    )
}

#[tokio::test]
async fn admin_invites_then_parent_sees_linked_child() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, fb_a, em_a) = create_user(&pool).await;
    let (student, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    attach_membership(&pool, tenant, student, "student").await;

    let parent_email = format!("parent-{}@example.test", Uuid::new_v4());

    // Org admin creates the parent invitation.
    let mock =
        std::sync::Arc::new(backend::services::invitations::mock::MockEmailLinkSender::new());
    let admin_app = build_test_app(
        backend::handlers::parent::router_for_tests(
            pool.clone(),
            mock.clone(),
            "http://localhost:3000".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: fb_a,
            email: em_a,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::OrgAdmin),
        },
    );
    let (status, body) = fire(
        &admin_app,
        "POST",
        "/v1/admin/parent-invitations",
        Some(json!({
            "parent_email": parent_email,
            "student_user_id": student.to_string(),
            "relationship": "mother",
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["parent_email"].as_str().unwrap(), parent_email);
    assert_eq!(body["status"], "pending");

    // The invite email was sent with a continue URL built from app_origin.
    let calls = mock.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, parent_email);
    assert!(calls[0].1.starts_with("http://localhost:3000/parent/login"));

    // Parent signs in for the first time -> JIT acceptance.
    let (parent, _, _) = create_user(&pool).await;
    sqlx::query("UPDATE users SET email = $1 WHERE id = $2")
        .bind(&parent_email)
        .bind(parent)
        .execute(&pool)
        .await
        .unwrap();
    let accepted = accept_as_parent(&pool, parent, &parent_email).await;
    assert_eq!(accepted, 1);

    // /v1/parent/children returns the child for the parent.
    let app = parent_app(&pool, parent, parent_email.clone(), Some(tenant));
    let (status, body) = fire(&app, "GET", "/v1/parent/children", None).await;
    assert_eq!(status, 200, "{body}");
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(
        arr[0]["student_user_id"].as_str().unwrap(),
        student.to_string()
    );
}

#[tokio::test]
async fn parent_invites_reserve_one_seat_per_email_and_accept_that_reservation() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (admin, fb_a, em_a) = create_user(&pool).await;
    let (first_student, _, _) = create_user(&pool).await;
    let (second_student, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    attach_membership(&pool, tenant, first_student, "student").await;
    attach_membership(&pool, tenant, second_student, "student").await;
    // Three active seats and one remaining seat.
    seed_blocking_seat_cap(&pool, tenant, 4).await;

    let mock =
        std::sync::Arc::new(backend::services::invitations::mock::MockEmailLinkSender::new());
    let app = build_test_app(
        backend::handlers::parent::router_for_tests(
            pool.clone(),
            mock.clone(),
            "http://localhost:3000".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: fb_a,
            email: em_a,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::OrgAdmin),
        },
    );
    let parent_email = format!("reserved-parent-{}@example.test", Uuid::new_v4());

    for student in [first_student, second_student] {
        let (status, body) = fire(
            &app,
            "POST",
            "/v1/admin/parent-invitations",
            Some(json!({
                "parent_email": parent_email,
                "student_user_id": student,
                "relationship": "guardian"
            })),
        )
        .await;
        assert_eq!(status, 200, "{body}");
    }

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/admin/parent-invitations",
        Some(json!({
            "parent_email": format!("other-parent-{}@example.test", Uuid::new_v4()),
            "student_user_id": first_student
        })),
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"], "conflict: seat_limit_reached");
    assert_eq!(mock.calls().len(), 2, "blocked invite is not emailed");

    let (parent, _, _) = create_user(&pool).await;
    sqlx::query("UPDATE users SET email = $1 WHERE id = $2")
        .bind(&parent_email)
        .bind(parent)
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(parent.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let outcome = backend::db::parent::accept_pending_for_email(&mut tx, parent, &parent_email)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(outcome.accepted, 2);
    assert_eq!(outcome.blocked_tenants, 0);
}

#[tokio::test]
async fn parent_forbidden_on_unlinked_student_grades() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (parent, _, _) = create_user(&pool).await;
    let (other_student, _, _) = create_user(&pool).await;
    let parent_email = format!("parent-{}@example.test", Uuid::new_v4());
    sqlx::query("UPDATE users SET email = $1 WHERE id = $2")
        .bind(&parent_email)
        .bind(parent)
        .execute(&pool)
        .await
        .unwrap();
    // Parent is a member but linked to NO children.
    attach_membership(&pool, tenant, parent, "parent").await;
    attach_membership(&pool, tenant, other_student, "student").await;

    let app = parent_app(&pool, parent, parent_email, Some(tenant));
    let (status, _) = fire(
        &app,
        "GET",
        &format!("/v1/parent/children/{other_student}/grades"),
        None,
    )
    .await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn parent_reads_are_bound_to_selected_active_workspace() {
    let pool = pool().await;
    let selected_tenant = create_tenant(&pool).await;
    let other_tenant = create_tenant(&pool).await;
    let (parent, _, parent_email) = create_user(&pool).await;
    let (other_child, _, _) = create_user(&pool).await;
    attach_membership(&pool, selected_tenant, parent, "parent").await;
    attach_membership(&pool, other_tenant, parent, "parent").await;
    attach_membership(&pool, other_tenant, other_child, "student").await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(other_tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO parent_links
            (tenant_id, parent_user_id, student_user_id, relationship)
         VALUES ($1, $2, $3, 'guardian')",
    )
    .bind(other_tenant)
    .bind(parent)
    .bind(other_child)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // Selecting tenant A cannot follow an otherwise valid link from tenant B.
    let app = parent_app(&pool, parent, parent_email.clone(), Some(selected_tenant));
    let (status, body) = fire(&app, "GET", "/v1/parent/children", None).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.as_array().unwrap().is_empty());
    let (status, _) = fire(
        &app,
        "GET",
        &format!("/v1/parent/children/{other_child}/grades"),
        None,
    )
    .await;
    assert_eq!(status, 403);

    // A stale Parent capability cannot resurrect a suspended tenant B
    // membership even when tenant B is explicitly selected.
    sqlx::query(
        "UPDATE tenant_memberships SET status = 'suspended'
          WHERE tenant_id = $1 AND user_id = $2",
    )
    .bind(other_tenant)
    .bind(parent)
    .execute(&pool)
    .await
    .unwrap();
    let app = parent_app(&pool, parent, parent_email, Some(other_tenant));
    let (status, body) = fire(&app, "GET", "/v1/parent/children", None).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.as_array().unwrap().is_empty());
    let (status, _) = fire(
        &app,
        "GET",
        &format!("/v1/parent/children/{other_child}/grades"),
        None,
    )
    .await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn non_parent_forbidden_on_parent_routes() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (student, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;

    // A student (non-parent) hits /v1/parent/children -> 403.
    let app = build_test_app(
        backend::handlers::parent::router_for_tests(
            pool.clone(),
            std::sync::Arc::new(backend::services::invitations::mock::MockEmailLinkSender::new()),
            "http://localhost:3000".into(),
        ),
        StubAuth {
            pool: pool.clone(),
            user_id: student,
            firebase_uid: format!("fb-{}", Uuid::new_v4()),
            email: format!("s-{}@example.test", Uuid::new_v4()),
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Student),
        },
    );
    let (status, _) = fire(&app, "GET", "/v1/parent/children", None).await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn grades_endpoint_returns_only_released() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (teacher, _, _) = create_user(&pool).await;
    let (student, _, _) = create_user(&pool).await;
    let (parent, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, teacher, "teacher").await;
    attach_membership(&pool, tenant, student, "student").await;

    let parent_email = format!("parent-{}@example.test", Uuid::new_v4());
    sqlx::query("UPDATE users SET email = $1 WHERE id = $2")
        .bind(&parent_email)
        .bind(parent)
        .execute(&pool)
        .await
        .unwrap();

    let course_id = create_course(&pool, tenant, teacher).await;
    let released_aid = create_assignment(&pool, tenant, course_id, teacher).await;
    let unreleased_aid = create_assignment(&pool, tenant, course_id, teacher).await;
    create_graded_submission(&pool, tenant, released_aid, course_id, student, 91.0, true).await;
    create_graded_submission(
        &pool,
        tenant,
        unreleased_aid,
        course_id,
        student,
        42.0,
        false,
    )
    .await;

    // Link the parent to the child directly via the JIT acceptance helper.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO parent_invitations
            (tenant_id, parent_email, student_user_id, relationship)
         VALUES ($1,$2,$3,'guardian')",
    )
    .bind(tenant)
    .bind(&parent_email)
    .bind(student)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let accepted = accept_as_parent(&pool, parent, &parent_email).await;
    assert_eq!(accepted, 1);

    let app = parent_app(&pool, parent, parent_email, Some(tenant));
    let (status, body) = fire(
        &app,
        "GET",
        &format!("/v1/parent/children/{student}/grades"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let arr = body.as_array().unwrap();
    // Only the released grade appears.
    assert_eq!(arr.len(), 1);
    assert_eq!(
        arr[0]["assignment_id"].as_str().unwrap(),
        released_aid.to_string()
    );
    assert_eq!(arr[0]["numeric_grade"].as_f64().unwrap(), 91.0);
    assert_eq!(arr[0]["student_visible_feedback"], "nice work");
    // teacher_only_notes is never serialized into the DTO.
    assert!(arr[0].get("teacher_only_notes").is_none());
}

#[tokio::test]
async fn parent_invitation_acceptance_rejects_a_mismatched_transaction_actor() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (student, _, _) = create_user(&pool).await;
    let (victim, _, victim_email) = create_user(&pool).await;
    let (attacker, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    let invitation_id: Uuid = sqlx::query_scalar(
        "INSERT INTO parent_invitations
            (tenant_id, parent_email, student_user_id, relationship)
         VALUES ($1, $2, $3, 'guardian') RETURNING id",
    )
    .bind(tenant)
    .bind(&victim_email)
    .bind(student)
    .fetch_one(&pool)
    .await
    .unwrap();

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.user_id', $1, true)")
        .bind(attacker.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let error = backend::db::parent::accept_pending_for_email(&mut tx, victim, &victim_email)
        .await
        .unwrap_err();
    tx.rollback().await.unwrap();
    let sqlx::Error::Database(database_error) = error else {
        panic!("expected database authorization error");
    };
    assert_eq!(database_error.code().as_deref(), Some("42501"));

    let status: String = sqlx::query_scalar("SELECT status FROM parent_invitations WHERE id = $1")
        .bind(invitation_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "pending");
    let link_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM parent_links
          WHERE tenant_id = $1 AND parent_user_id = $2",
    )
    .bind(tenant)
    .bind(victim)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(link_count, 0);
}

#[tokio::test]
async fn parent_invitation_cannot_repurpose_an_existing_admin() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (student, _, _) = create_user(&pool).await;
    let (admin, _, admin_email) = create_user(&pool).await;
    attach_membership(&pool, tenant, student, "student").await;
    attach_membership(&pool, tenant, admin, "org_admin").await;
    let invitation_id: Uuid = sqlx::query_scalar(
        "INSERT INTO parent_invitations
            (tenant_id, parent_email, student_user_id, relationship)
         VALUES ($1, $2, $3, 'guardian') RETURNING id",
    )
    .bind(tenant)
    .bind(&admin_email)
    .bind(student)
    .fetch_one(&pool)
    .await
    .unwrap();

    let accepted = accept_as_parent(&pool, admin, &admin_email).await;
    assert_eq!(accepted, 0);
    let membership: (String, String) = sqlx::query_as(
        "SELECT role, status FROM tenant_memberships
          WHERE tenant_id = $1 AND user_id = $2",
    )
    .bind(tenant)
    .bind(admin)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(membership, ("org_admin".into(), "active".into()));
    let status: String = sqlx::query_scalar("SELECT status FROM parent_invitations WHERE id = $1")
        .bind(invitation_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "revoked");
    let link_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM parent_links
          WHERE tenant_id = $1 AND parent_user_id = $2",
    )
    .bind(tenant)
    .bind(admin)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(link_count, 0);
}
