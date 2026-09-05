use dioxus::prelude::*;
use features_auth::login_internals::{
    classify_me_error, decide_outcome, LocalAttempt, LoginOutcome, MeCheckOutcome,
};

#[test]
fn local_login_success_yields_local_token() {
    let outcome = decide_outcome(LocalAttempt::Ok("local-token".into()), None);
    assert!(matches!(
        outcome,
        LoginOutcome::Token {
            ref token,
            persist_local_token: true
        } if token == "local-token"
    ));
}

#[test]
fn local_login_failure_falls_back_to_firebase_success() {
    let outcome = decide_outcome(
        LocalAttempt::Err("401".into()),
        Some(Ok("firebase-token".into())),
    );
    assert!(matches!(
        outcome,
        LoginOutcome::Token {
            ref token,
            persist_local_token: false
        } if token == "firebase-token"
    ));
}

#[test]
fn both_failing_yields_error_with_firebase_message() {
    let outcome = decide_outcome(
        LocalAttempt::Err("401".into()),
        Some(Err("invalid password".into())),
    );
    assert!(matches!(outcome, LoginOutcome::Err(ref m) if m.contains("invalid password")));
}

#[test]
fn me_error_classifier_detects_mfa_required() {
    let err =
        features_courses::api::ApiError::Status(401, r#"{"error":"mfa_required"}"#.to_string());
    assert_eq!(classify_me_error(&err), MeCheckOutcome::MfaRequired);
}

#[test]
fn me_error_classifier_keeps_other_unauthorized_as_failure() {
    let err = features_courses::api::ApiError::Status(401, "unauthorized".to_string());
    assert_eq!(
        classify_me_error(&err),
        MeCheckOutcome::Failed("status 401: unauthorized".to_string())
    );
}

#[test]
fn login_renders_email_password_and_submit_controls() {
    use design_system::ToastQueue;

    fn app() -> Element {
        use_context_provider::<Signal<ToastQueue>>(|| Signal::new(ToastQueue::new()));
        rsx! {
            features_auth::Login {
                on_success: |_| {},
            }
        }
    }

    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);

    assert!(html.contains("placeholder=\"you@example.com\""));
    assert!(html.contains("type=\"password\""));
    assert!(html.contains("Sign in"));
    assert!(html.contains("auth-login-card"));
    // PageHeader (Hero) renders the title under the design-system class.
    assert!(html.contains("ds-page-header"));
    assert!(html.contains("ds-page-header--hero"));
    assert!(html.contains("auth-form"));
    // Field composite renders labels via ds-field-label.
    assert!(html.contains("ds-field-label"));
    assert!(html.contains("AulaLite"));
}
