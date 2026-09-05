// crates/shell-web/src/route_enum.rs
use dioxus::prelude::*;
use dioxus_router::Routable;

// Bring every routed component into scope under its variant name.
// The `Routable` derive emits `rsx! { Login {} }` etc., so each variant ident
// must resolve to a component in this module.
#[cfg(debug_assertions)]
use crate::routes::DevComponents;
use crate::routes::{
    AcceptInvite, AdminAnalytics, AdminAudit, AdminBilling, AdminBranding, AdminFiles, AdminHome,
    AdminIntegrations, AdminNotificationDeliveries, AdminTenant, AssignmentDetail, AssignmentEdit,
    AssignmentGrade, AssignmentList, AssignmentNew, CatalogPage, CertificateVerifyPage,
    CourseAnalytics, CourseAnnouncementsPage, CourseCertificatesPage, CourseDetail,
    CourseDiscussionsPage, CourseEdit, CourseFlashcardsPage, CourseGradebookPage,
    CourseLeaderboardPage, CourseList, CourseNew, CoursePeople, CourseRecordings, CourseSchedule,
    CourseScormPage, CourseSyllabusPage, Dashboard, Forgot, LessonEdit, LessonPage, LiveSession,
    Login, LtiLanding, MyCalendar, MyCertificatesPage, MySchedule, NotFound, NotificationSettings,
    Onboarding, ParentHome, ParentLogin, Platform, Privacy, QuizEditPage, QuizListPage,
    QuizTakePage, Redeem, Search, Signup, SsoFinish, Terms, TranscriptPage,
};

#[rustfmt::skip]
#[derive(Routable, Clone, PartialEq, Eq)]
pub enum Route {
    #[layout(crate::native_navigation::NativeNavigationLayout)]
    #[route("/login")]
    Login {},
    #[route("/parent/login")]
    ParentLogin {},
    #[route("/sso/finish")]
    SsoFinish {},
    #[route("/lti/landing")]
    LtiLanding {},
    #[route("/signup")]
    Signup {},
    #[route("/forgot")]
    Forgot {},
    #[route("/privacy")]
    Privacy {},
    #[route("/terms")]
    Terms {},
    #[route("/")]
    Dashboard {},
    #[route("/onboarding")]
    Onboarding {},
    #[route("/accept-invite/:token")]
    AcceptInvite { token: String },
    #[route("/courses")]
    CourseList {},
    #[route("/courses/new")]
    CourseNew {},
    #[route("/courses/:slug")]
    CourseDetail { slug: String },
    #[route("/courses/:slug/people")]
    CoursePeople { slug: String },
    #[route("/courses/:slug/edit")]
    CourseEdit { slug: String },
    #[route("/courses/:slug/schedule")]
    CourseSchedule { slug: String },
    #[route("/courses/:slug/sessions/:session_id")]
    LiveSession { slug: String, session_id: String },
    #[route("/courses/:slug/recordings")]
    CourseRecordings { slug: String },
    #[route("/courses/:slug/analytics")]
    CourseAnalytics { slug: String },
    #[route("/courses/:slug/modules/:module_id/lessons/:lesson_id/edit")]
    LessonEdit { slug: String, module_id: String, lesson_id: String },
    #[route("/courses/:slug/lessons/:lesson_id")]
    LessonPage { slug: String, lesson_id: String },
    #[route("/courses/:slug/quizzes")]
    QuizListPage { slug: String },
    #[route("/courses/:slug/quizzes/:quiz_id")]
    QuizTakePage { slug: String, quiz_id: String },
    #[route("/courses/:slug/quizzes/:quiz_id/edit")]
    QuizEditPage { slug: String, quiz_id: String },
    #[route("/courses/:slug/leaderboard")]
    CourseLeaderboardPage { slug: String },
    #[route("/courses/:slug/announcements")]
    CourseAnnouncementsPage { slug: String },
    #[route("/courses/:slug/discussions")]
    CourseDiscussionsPage { slug: String },
    #[route("/courses/:slug/scorm")]
    CourseScormPage { slug: String },
    #[route("/courses/:slug/syllabus")]
    CourseSyllabusPage { slug: String },
    #[route("/courses/:slug/gradebook")]
    CourseGradebookPage { slug: String },
    #[route("/courses/:slug/certificates")]
    CourseCertificatesPage { slug: String },
    #[route("/courses/:slug/flashcards")]
    CourseFlashcardsPage { slug: String },
    #[route("/certificates")]
    MyCertificatesPage {},
    #[route("/verify/:credential_id")]
    CertificateVerifyPage { credential_id: String },
    #[route("/courses/:slug/assignments")]
    AssignmentList { slug: String },
    #[route("/courses/:slug/assignments/new")]
    AssignmentNew { slug: String },
    #[route("/courses/:slug/assignments/:id")]
    AssignmentDetail { slug: String, id: String },
    #[route("/courses/:slug/assignments/:id/edit")]
    AssignmentEdit { slug: String, id: String },
    #[route("/courses/:slug/assignments/:id/grade")]
    AssignmentGrade { slug: String, id: String },
    #[route("/search")]
    Search {},
    #[route("/redeem")]
    Redeem {},
    #[route("/catalog")]
    CatalogPage {},
    #[route("/transcript")]
    TranscriptPage {},
    #[route("/schedule")]
    MySchedule {},
    #[route("/calendar")]
    MyCalendar {},
    #[route("/parent")]
    ParentHome {},
    #[route("/settings/notifications")]
    NotificationSettings {},
    #[route("/admin")]
    AdminHome {},
    #[route("/admin/analytics")]
    AdminAnalytics {},
    #[route("/admin/billing")]
    AdminBilling {},
    #[route("/admin/branding")]
    AdminBranding {},
    #[route("/admin/audit")]
    AdminAudit {},
    #[route("/admin/integrations")]
    AdminIntegrations {},
    #[route("/admin/notifications")]
    AdminNotificationDeliveries {},
    #[route("/admin/tenant")]
    AdminTenant {},
    #[route("/admin/files")]
    AdminFiles {},
    #[route("/platform")]
    Platform {},
    #[cfg(debug_assertions)]
    #[route("/dev/components")]
    DevComponents {},
    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}
