mod fixtures;

use fixtures::*;
use serde_json::json;

#[tokio::test]
async fn teacher_can_create_course_then_read_back() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fbuid, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;

    let stub = StubAuth {
        pool: pool.clone(),
        user_id: user,
        firebase_uid: fbuid,
        email: email.clone(),
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };

    let app = build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        stub,
    );

    // POST /v1/courses
    let (status, body) = fire(
        &app,
        "POST",
        "/v1/courses",
        Some(json!({ "title": "Intro to Calculus" })),
    )
    .await;
    assert_eq!(status, 200, "create returned {body}");
    assert_eq!(body["slug"], "intro-to-calculus");
    let id = body["id"].as_str().unwrap().to_string();

    // GET /v1/courses/:id
    let (status, body) = fire(&app, "GET", &format!("/v1/courses/{id}"), None).await;
    assert_eq!(status, 200);
    assert_eq!(body["title"], "Intro to Calculus");
    assert_eq!(body["status"], "draft");

    // GET /v1/courses (list)
    let (status, body) = fire(&app, "GET", "/v1/courses", None).await;
    assert_eq!(status, 200);
    let arr = body.as_array().unwrap();
    // Must contain the created course (other tests may have created courses too)
    assert!(arr.iter().any(|c| c["id"].as_str() == Some(&id)));
}

#[tokio::test]
async fn slug_dedups_when_title_collides() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fbuid, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;

    let stub = StubAuth {
        pool: pool.clone(),
        user_id: user,
        firebase_uid: fbuid,
        email,
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        stub,
    );

    let (s1, _) = fire(
        &app,
        "POST",
        "/v1/courses",
        Some(json!({ "title": "Algebra" })),
    )
    .await;
    let (s2, b2) = fire(
        &app,
        "POST",
        "/v1/courses",
        Some(json!({ "title": "Algebra" })),
    )
    .await;
    assert_eq!(s1, 200);
    assert_eq!(s2, 200);
    assert_eq!(b2["slug"], "algebra-2");
}

#[tokio::test]
async fn student_cannot_create_course() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fbuid, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "student").await;

    let stub = StubAuth {
        pool: pool.clone(),
        user_id: user,
        firebase_uid: fbuid,
        email,
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Student),
    };
    let app = build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        stub,
    );

    let (status, _) = fire(&app, "POST", "/v1/courses", Some(json!({ "title": "x" }))).await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn owner_can_patch_title_and_status() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fbuid, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;

    let stub = StubAuth {
        pool: pool.clone(),
        user_id: user,
        firebase_uid: fbuid,
        email,
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        stub,
    );

    let (_, body) = fire(
        &app,
        "POST",
        "/v1/courses",
        Some(serde_json::json!({ "title": "Stats" })),
    )
    .await;
    let id = body["id"].as_str().unwrap().to_string();

    let (status, body) = fire(
        &app,
        "PATCH",
        &format!("/v1/courses/{id}"),
        Some(serde_json::json!({ "title": "Statistics", "status": "published" })),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["title"], "Statistics");
    assert_eq!(body["status"], "published");
}

#[tokio::test]
async fn owner_patch_sanitizes_syllabus_and_grading_markdown() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fbuid, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;

    let app = build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: fbuid,
            email,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (_, body) = fire(
        &app,
        "POST",
        "/v1/courses",
        Some(serde_json::json!({ "title": "Safety" })),
    )
    .await;
    let id = body["id"].as_str().unwrap().to_string();

    let (status, body) = fire(
        &app,
        "PATCH",
        &format!("/v1/courses/{id}"),
        Some(serde_json::json!({
            "syllabus_md": "# Course plan\n<script>alert(1)</script>\n[bad](javascript:alert(1))",
            "grading_policy_md": "Pass <img src=x onerror=alert(1)> **work** <b>only</b>"
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    let (status, body) = fire(&app, "GET", &format!("/v1/courses/{id}/syllabus"), None).await;
    assert_eq!(status, 200, "{body}");

    let syllabus = body["syllabus_md"].as_str().unwrap();
    let grading = body["grading_policy_md"].as_str().unwrap();

    assert!(syllabus.contains("# Course plan"), "{syllabus}");
    assert!(
        !syllabus.to_ascii_lowercase().contains("<script"),
        "{syllabus}"
    );
    assert!(
        !syllabus.to_ascii_lowercase().contains("javascript:"),
        "{syllabus}"
    );
    assert!(grading.contains("**work**"), "{grading}");
    assert!(grading.contains("only"), "{grading}");
    assert!(!grading.to_ascii_lowercase().contains("<img"), "{grading}");
    assert!(
        !grading.to_ascii_lowercase().contains("onerror"),
        "{grading}"
    );
    assert!(!grading.to_ascii_lowercase().contains("<b>"), "{grading}");
}

#[tokio::test]
async fn owner_patch_preserves_and_clears_optional_syllabus_fields() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fbuid, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;

    let app = build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: user,
            firebase_uid: fbuid,
            email,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );

    let (status, body) = fire(
        &app,
        "POST",
        "/v1/courses",
        Some(json!({ "title": "Patch Semantics" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let id = body["id"].as_str().unwrap().to_string();

    let (status, body) = fire(
        &app,
        "PATCH",
        &format!("/v1/courses/{id}"),
        Some(json!({
            "syllabus_md": "# Keep syllabus",
            "grading_policy_md": "Grade by **rubric**"
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    let (status, body) = fire(
        &app,
        "PATCH",
        &format!("/v1/courses/{id}"),
        Some(json!({ "title": "Patch Semantics Updated" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["syllabus_md"].as_str(), Some("# Keep syllabus"));
    assert_eq!(
        body["grading_policy_md"].as_str(),
        Some("Grade by **rubric**")
    );

    let (status, body) = fire(
        &app,
        "PATCH",
        &format!("/v1/courses/{id}"),
        Some(json!({ "syllabus_md": null })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body.get("syllabus_md"),
        Some(&serde_json::Value::Null),
        "{body}"
    );
    assert_eq!(
        body["grading_policy_md"].as_str(),
        Some("Grade by **rubric**")
    );

    let (status, body) = fire(
        &app,
        "PATCH",
        &format!("/v1/courses/{id}"),
        Some(json!({ "grading_policy_md": "<script>alert(1)</script>" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body.get("syllabus_md"),
        Some(&serde_json::Value::Null),
        "{body}"
    );
    assert_eq!(
        body.get("grading_policy_md"),
        Some(&serde_json::Value::Null),
        "{body}"
    );

    let (status, body) = fire(&app, "GET", &format!("/v1/courses/{id}/syllabus"), None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body.get("syllabus_md"),
        Some(&serde_json::Value::Null),
        "{body}"
    );
    assert_eq!(
        body.get("grading_policy_md"),
        Some(&serde_json::Value::Null),
        "{body}"
    );
}

#[tokio::test]
async fn invalid_status_transition_rejected() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (user, fbuid, email) = create_user(&pool).await;
    attach_membership(&pool, tenant, user, "teacher").await;

    let stub = StubAuth {
        pool: pool.clone(),
        user_id: user,
        firebase_uid: fbuid,
        email,
        tenant_id: Some(tenant),
        tenant_role: Some(core_types::TenantRole::Teacher),
    };
    let app = build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        stub,
    );
    let (_, body) = fire(
        &app,
        "POST",
        "/v1/courses",
        Some(serde_json::json!({ "title": "x" })),
    )
    .await;
    let id = body["id"].as_str().unwrap().to_string();

    fire(
        &app,
        "PATCH",
        &format!("/v1/courses/{id}"),
        Some(serde_json::json!({ "status": "archived" })),
    )
    .await;
    let (status, body) = fire(
        &app,
        "PATCH",
        &format!("/v1/courses/{id}"),
        Some(serde_json::json!({ "status": "draft" })),
    )
    .await;
    assert_eq!(status, 400, "expected reject; got body {body}");
}

#[tokio::test]
async fn other_teacher_cannot_edit_my_course() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (owner, fbuid_o, email_o) = create_user(&pool).await;
    attach_membership(&pool, tenant, owner, "teacher").await;
    let (other, fbuid_x, email_x) = create_user(&pool).await;
    attach_membership(&pool, tenant, other, "teacher").await;

    let owner_app = build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: owner,
            firebase_uid: fbuid_o,
            email: email_o,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (_, body) = fire(
        &owner_app,
        "POST",
        "/v1/courses",
        Some(serde_json::json!({ "title": "Mine" })),
    )
    .await;
    let id = body["id"].as_str().unwrap().to_string();

    let other_app = build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: other,
            firebase_uid: fbuid_x,
            email: email_x,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (status, _) = fire(
        &other_app,
        "PATCH",
        &format!("/v1/courses/{id}"),
        Some(serde_json::json!({ "title": "Hijacked" })),
    )
    .await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn org_admin_can_edit_any_course_in_tenant() {
    let pool = pool().await;
    let tenant = create_tenant(&pool).await;
    let (owner, fbuid_o, email_o) = create_user(&pool).await;
    attach_membership(&pool, tenant, owner, "teacher").await;
    let (admin, fbuid_a, email_a) = create_user(&pool).await;
    attach_membership(&pool, tenant, admin, "org_admin").await;

    let owner_app = build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: owner,
            firebase_uid: fbuid_o,
            email: email_o,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::Teacher),
        },
    );
    let (_, body) = fire(
        &owner_app,
        "POST",
        "/v1/courses",
        Some(serde_json::json!({ "title": "Course" })),
    )
    .await;
    let id = body["id"].as_str().unwrap().to_string();

    let admin_app = build_test_app(
        backend::handlers::courses::router_for_tests(pool.clone()),
        StubAuth {
            pool: pool.clone(),
            user_id: admin,
            firebase_uid: fbuid_a,
            email: email_a,
            tenant_id: Some(tenant),
            tenant_role: Some(core_types::TenantRole::OrgAdmin),
        },
    );
    let (status, body) = fire(
        &admin_app,
        "PATCH",
        &format!("/v1/courses/{id}"),
        Some(serde_json::json!({ "title": "Renamed by admin" })),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["title"], "Renamed by admin");

    let (status, _) = fire(&admin_app, "DELETE", &format!("/v1/courses/{id}"), None).await;
    assert_eq!(status, 204);
}
