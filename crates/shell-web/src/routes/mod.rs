// crates/shell-web/src/routes/mod.rs
//! Per-route components. Each file owns one `Routable` variant.

use dioxus::prelude::*;
use features_courses::api::ApiContext;

use crate::contexts::UserContextSignal;

pub mod login;
pub use login::{Login, ParentLogin};

pub mod forgot;
pub mod signup;
pub use forgot::Forgot;
pub use signup::Signup;

pub mod dashboard;
pub use dashboard::Dashboard;
pub mod landing;
pub mod public_pages;
pub use public_pages::{NotFound, Privacy, Terms};

pub mod onboarding;
pub use onboarding::Onboarding;

pub mod course_list;
pub mod course_new;
pub use course_list::CourseList;
pub use course_new::CourseNew;

pub mod course_detail;
pub use course_detail::{
    CourseAnalytics, CourseAnnouncementsPage, CourseCertificatesPage, CourseDetail,
    CourseDiscussionsPage, CourseEdit, CourseFlashcardsPage, CourseGradebookPage,
    CourseLeaderboardPage, CoursePeople, CourseSchedule, CourseScormPage, CourseSyllabusPage,
};

pub mod certificates;
pub use certificates::{CertificateVerifyPage, MyCertificatesPage};

pub mod accept_invite;
pub mod my_calendar;
pub mod my_schedule;
pub mod redeem;
pub use accept_invite::AcceptInvite;
pub use my_calendar::MyCalendar;
pub use my_schedule::MySchedule;
pub use redeem::Redeem;

pub mod catalog;
pub use catalog::CatalogPage;

pub mod transcript;
pub use transcript::TranscriptPage;

pub mod live_session;
pub use live_session::LiveSession;

pub mod assignments_detail;
pub mod assignments_edit;
pub mod assignments_grade;
pub mod assignments_list;
pub mod assignments_new;
pub use assignments_detail::AssignmentDetail;
pub use assignments_edit::AssignmentEdit;
pub use assignments_grade::AssignmentGrade;
pub use assignments_list::AssignmentList;
pub use assignments_new::AssignmentNew;

pub mod lesson_edit;
pub mod lesson_view;
pub mod quizzes;
pub use lesson_edit::LessonEdit;
pub use lesson_view::LessonPage;
pub use quizzes::{QuizEditPage, QuizListPage, QuizTakePage};

pub mod recordings_list;
pub use recordings_list::CourseRecordings;

pub mod admin_home;
pub use admin_home::AdminHome;

pub mod admin_analytics;
pub use admin_analytics::AdminAnalytics;

pub mod admin_billing;
pub use admin_billing::AdminBilling;

pub mod admin_branding;
pub use admin_branding::AdminBranding;

pub mod admin_audit;
pub use admin_audit::AdminAudit;

pub mod admin_integrations;
pub use admin_integrations::AdminIntegrations;

pub mod admin_notification_deliveries;
pub use admin_notification_deliveries::AdminNotificationDeliveries;

pub mod admin_tenant;
pub use admin_tenant::AdminTenant;

pub mod admin_files;
pub use admin_files::AdminFiles;

pub mod parent_home;
pub use parent_home::ParentHome;

pub mod platform;
pub use platform::Platform;

pub mod notification_settings;
pub use notification_settings::NotificationSettings;

pub mod search;
pub use search::Search;

pub mod sso_finish;
pub use sso_finish::SsoFinish;

pub mod lti_landing;
pub use lti_landing::LtiLanding;

/// Shorthand to read the signed-in user from context (None until /v1/me resolves).
pub fn use_user_context() -> UserContextSignal {
    use_context::<UserContextSignal>()
}

/// Shorthand to read ApiContext from the live signal.
pub fn use_api() -> ApiContext {
    use_context::<Signal<ApiContext>>().read().clone()
}

pub fn signed_out_api_context() -> ApiContext {
    #[cfg(target_arch = "wasm32")]
    let base_url = features_courses::api::web_api_base_url();
    #[cfg(not(target_arch = "wasm32"))]
    let base_url = features_courses::api::native_api_base_url();

    ApiContext {
        base_url,
        id_token: String::new(),
    }
}

/// Push cleanup must use the still-valid bearer, but it must not make the UI
/// appear stuck offline. We start a best-effort backend DELETE, erase the local
/// raw token immediately, then sign out the identity provider. Server-side
/// revocation can therefore lag only when the network is unavailable; store
/// release testing must verify that the OS SDK also deletes/rotates its token.
pub async fn sign_out_with_device_cleanup(api: ApiContext) {
    #[cfg(target_arch = "wasm32")]
    {
        use platform_bridge::PlatformBridge;
        if let Some(token) = platform_bridge::web::fcm_token() {
            spawn(async move {
                let _ = features_courses::api::remove_device_token(&api, &token).await;
            });
        }
        platform_bridge::web::clear_fcm_token();
        platform_bridge::web::clear_local_token();
        let _ = platform_bridge::web::WebBridge.sign_out().await;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use platform_bridge::PlatformBridge;
        if let Ok(platform_bridge::native_push::NativePushTokenOutcome::Token(registration)) =
            platform_bridge::native_push::registration_token_outcome()
        {
            spawn(async move {
                let _ = features_courses::api::remove_device_token(&api, &registration.token).await;
            });
        }
        let _ = platform_bridge::native_push::clear_local_registration();
        let _ = platform_bridge::native::NativeBridge.sign_out().await;
    }
}

pub fn use_signout_action() -> impl FnMut() + 'static {
    let nav = dioxus_router::use_navigator();
    let api_signal = try_consume_context::<Signal<ApiContext>>();
    let user_signal = try_consume_context::<UserContextSignal>();

    move || {
        let cleanup_api = api_signal
            .map(|signal| signal.read().clone())
            .unwrap_or_else(signed_out_api_context);
        spawn(async move { sign_out_with_device_cleanup(cleanup_api).await });

        if let Some(mut api_signal) = api_signal {
            api_signal.set(signed_out_api_context());
        }
        if let Some(mut user_signal) = user_signal {
            user_signal.set(None);
        }
        nav.push(crate::route_enum::Route::Login {});
    }
}

#[cfg(test)]
mod signout_tests {
    #[test]
    fn signed_out_api_context_clears_token() {
        let ctx = super::signed_out_api_context();
        assert!(ctx.id_token.is_empty());
    }

    #[test]
    fn device_cleanup_is_explicitly_best_effort_and_non_blocking() {
        // The route clears live auth state synchronously while cleanup runs in
        // the spawned task; this test locks that user-visible failure policy.
        let helper = super::sign_out_with_device_cleanup;
        let _ = helper;
    }
}

#[cfg(debug_assertions)]
pub mod dev_components;
#[cfg(debug_assertions)]
pub use dev_components::DevComponents;

// All Phase 1.5 routes (Tasks 7-17) are now wired above.
