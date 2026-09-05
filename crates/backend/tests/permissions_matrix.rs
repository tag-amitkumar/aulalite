mod fixtures;

use fixtures::*;
use serde_json::json;

async fn make_course(
    pool: &sqlx::PgPool,
    tenant: uuid::Uuid,
    owner: uuid::Uuid,
) -> (uuid::Uuid, uuid::Uuid) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let cid: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO courses (tenant_id, slug, title, owner_user_id)
         VALUES ($1, $2, 'C', $3) RETURNING id",
    )
    .bind(tenant)
    .bind(format!("c-{}", uuid::Uuid::new_v4()))
    .bind(owner)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role)
         VALUES ($1,$2,$3,'teacher')",
    )
    .bind(cid)
    .bind(owner)
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .unwrap();
    let mid: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO modules (tenant_id, course_id, title, sort_order)
         VALUES ($1,$2,'M',10) RETURNING id",
    )
    .bind(tenant)
    .bind(cid)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    (cid, mid)
}

fn role_to_enum(r: &str) -> Option<core_types::TenantRole> {
    Some(match r {
        "org_admin" => core_types::TenantRole::OrgAdmin,
        "teacher" => core_types::TenantRole::Teacher,
        "ta" => core_types::TenantRole::Ta,
        "student" => core_types::TenantRole::Student,
        _ => return None,
    })
}

#[tokio::test]
async fn course_create_module_permissions_matrix() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (owner, _, _) = create_user(&pool).await;
    attach_membership(&pool, tenant, owner, "teacher").await;
    let (course, _module) = make_course(&pool, tenant, owner).await;

    for (role, expected_create_module) in [
        ("org_admin", 200),
        ("teacher", 403), // not the course owner
        ("ta", 403),
        ("student", 403),
    ] {
        let (uid, fb, em) = create_user(&pool).await;
        attach_membership(&pool, tenant, uid, role).await;

        let app = build_test_app(
            backend::handlers::modules::router_for_tests(pool.clone()),
            StubAuth {
                pool: pool.clone(),
                user_id: uid,
                firebase_uid: fb,
                email: em,
                tenant_id: Some(tenant),
                tenant_role: role_to_enum(role),
            },
        );
        let (status, _) = fire(
            &app,
            "POST",
            &format!("/v1/courses/{course}/modules"),
            Some(json!({ "title": "x" })),
        )
        .await;
        assert_eq!(
            status.as_u16(),
            expected_create_module,
            "role={role} should produce {expected_create_module}"
        );
    }

    // A teacher explicitly assigned to someone else's course is an author for
    // that course. Tenant role alone is insufficient (covered by the 403
    // above); the active course membership is the second half of the grant.
    let (assigned_teacher, fb, em) = create_user(&pool).await;
    attach_membership(&pool, tenant, assigned_teacher, "teacher").await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO course_memberships (course_id, user_id, tenant_id, role, status)
         VALUES ($1, $2, $3, 'teacher', 'active')",
    )
    .bind(course)
    .bind(assigned_teacher)
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let app = build_test_app(
        backend::handlers::modules::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: assigned_teacher,
            firebase_uid: fb,
            email: em,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (status, _) = fire(
        &app,
        "POST",
        &format!("/v1/courses/{course}/modules"),
        Some(json!({ "title": "Assigned teacher module" })),
    )
    .await;
    assert_eq!(status.as_u16(), 200);
}
