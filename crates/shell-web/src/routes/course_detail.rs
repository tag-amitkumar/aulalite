// crates/shell-web/src/routes/course_detail.rs
use design_system::kinetics_ui::{
    BarChart, Breadcrumb, BreadcrumbItem, ChartSeries, ChartTone, DataTable, DataTableColumn,
    DataTableRow, DonutGauge, MetricCard, MetricTone,
};
use design_system::{
    use_toast_sender, Button, ButtonVariant, DateTimePicker, Field, Input, Modal, Select,
    SelectOption, SkeletonCard, ToastLevel,
};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::active_session::{use_active_session_poll, PollState};
use features_courses::api;
use features_courses::api::{StartNowBody, StartNowOutcome};
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::bulk_import::BulkImport;
use features_courses::code_modal::CodeModal;
use features_courses::course_builder::{CourseBuilder, LessonNode, ModuleNode};
use features_courses::course_detail::CourseDetail as CourseDetailView;
use features_courses::course_people::{
    ActiveCode, CoursePeople as CoursePeopleView, Member, PendingInvite,
};
use features_courses::invite_modal::InviteModal;
use features_courses::live_now_banner::LiveNowBanner;
use features_courses::schedule_view::{ScheduleEntry, ScheduleView};
use features_courses::series_scheduler::{SeriesDraft, SeriesScheduler};
use features_courses::start_now_button::{StartNowButton, StartNowState};
use features_courses::start_now_modal::{StartNowDraft, StartNowModal};

use crate::contexts::UserContext;
use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

const DEFAULT_LESSON_TYPE: &str = "rich_text";

fn default_quick_session_title() -> String {
    let now = chrono::Utc::now();
    format!("Quick session — {}", now.format("%b %-d, %Y %-I:%M %p UTC"))
}

fn can_admin_course(user: &UserContext, course: &api::CourseDto) -> bool {
    user.can_manage_organization()
        || (user.can_teach()
            && (course.owner_user_id == user.user_id
                || course.caller_course_role.as_deref() == Some("teacher")))
}

fn derive_start_now_state(poll: &PollState, submitting: bool) -> StartNowState {
    if submitting {
        return StartNowState::Submitting;
    }
    match poll {
        PollState::Active(info) => StartNowState::Conflict {
            active_session_id: info.session_id.clone(),
        },
        _ => StartNowState::Idle,
    }
}

#[derive(Props, Clone, PartialEq)]
struct StartNowSlotsProps {
    course_id: String,
    slug: String,
    can_admin: bool,
}

#[component]
fn StartNowSlots(props: StartNowSlotsProps) -> Element {
    // All hooks at top level — required by dioxus rules of hooks.
    let nav = use_navigator();
    let api = use_api();
    let poll_signal = use_active_session_poll(props.course_id.clone());
    let mut submitting = use_signal(|| false);
    let mut show_modal = use_signal(|| false);
    let mut toast = use_toast_sender();
    let mut optimistic_conflict = use_signal(|| None::<api::StartNowConflictDto>);

    let course_id = props.course_id.clone();
    let slug = props.slug.clone();
    let can_admin = props.can_admin;

    let on_quick_start = {
        let api = api.clone();
        let course_id = course_id.clone();
        let slug = slug.clone();
        move |_: ()| {
            let api = api.clone();
            let course_id = course_id.clone();
            let slug = slug.clone();
            spawn(async move {
                submitting.set(true);
                let outcome =
                    api::start_session_now(&api, &course_id, &StartNowBody::default()).await;
                match outcome {
                    StartNowOutcome::Created(dto) => {
                        nav.push(Route::LiveSession {
                            slug: slug.clone(),
                            session_id: dto.session_id,
                        });
                    }
                    StartNowOutcome::Conflict(conflict) => {
                        toast.push(
                            ToastLevel::Warning,
                            "Already live",
                            "A session is already live in this course. Click 'Join active session' to enter it.",
                        );
                        optimistic_conflict.set(Some(conflict));
                        show_modal.set(false);
                    }
                    StartNowOutcome::Failed(e) => {
                        let (title_text, body_text) = match &e {
                            api::ApiError::Status(403, _) => (
                                "You no longer have permission",
                                "Refresh the page and sign in again.".to_string(),
                            ),
                            _ => ("Couldn't start the session", format!("{e}")),
                        };
                        toast.push(ToastLevel::Danger, title_text, body_text);
                    }
                }
                submitting.set(false);
            });
        }
    };

    let on_customize = move |_: ()| show_modal.set(true);
    let on_modal_cancel = move |_: ()| show_modal.set(false);

    let on_modal_submit = {
        let api = api.clone();
        let course_id = course_id.clone();
        let slug = slug.clone();
        move |d: StartNowDraft| {
            let api = api.clone();
            let course_id = course_id.clone();
            let slug = slug.clone();
            spawn(async move {
                submitting.set(true);
                let body = StartNowBody {
                    title: Some(d.title),
                    duration_minutes: Some(d.duration_minutes),
                    recording_enabled: Some(d.recording_enabled),
                };
                let outcome = api::start_session_now(&api, &course_id, &body).await;
                match outcome {
                    StartNowOutcome::Created(dto) => {
                        show_modal.set(false);
                        nav.push(Route::LiveSession {
                            slug,
                            session_id: dto.session_id,
                        });
                    }
                    StartNowOutcome::Conflict(conflict) => {
                        toast.push(
                            ToastLevel::Warning,
                            "Already live",
                            "A session is already live in this course. Click 'Join active session' to enter it.",
                        );
                        optimistic_conflict.set(Some(conflict));
                        show_modal.set(false);
                    }
                    StartNowOutcome::Failed(e) => {
                        let (title_text, body_text) = match &e {
                            api::ApiError::Status(403, _) => (
                                "You no longer have permission",
                                "Refresh the page and sign in again.".to_string(),
                            ),
                            _ => ("Couldn't start the session", format!("{e}")),
                        };
                        toast.push(ToastLevel::Danger, title_text, body_text);
                    }
                }
                submitting.set(false);
            });
        }
    };

    let on_join_active = {
        let slug = slug.clone();
        move |session_id: String| {
            nav.push(Route::LiveSession {
                slug: slug.clone(),
                session_id,
            });
        }
    };

    let poll_snapshot = poll_signal.read().clone();
    let optimistic = optimistic_conflict.read().clone();
    let start_state = if *submitting.read() {
        StartNowState::Submitting
    } else if let Some(conflict) = optimistic {
        StartNowState::Conflict {
            active_session_id: conflict.active_session_id,
        }
    } else {
        derive_start_now_state(&poll_snapshot, false)
    };

    if can_admin {
        let modal_open = *show_modal.read();
        rsx! {
            StartNowButton {
                state: start_state,
                on_quick_start: on_quick_start,
                on_customize: on_customize,
                on_join_active: on_join_active.clone(),
            }
            if modal_open {
                StartNowModal {
                    initial: StartNowDraft {
                        title: default_quick_session_title(),
                        duration_minutes: 60,
                        recording_enabled: true,
                    },
                    submitting: *submitting.read(),
                    on_submit: on_modal_submit,
                    on_cancel: on_modal_cancel,
                }
            }
        }
    } else {
        match &poll_snapshot {
            PollState::Active(info) => {
                let active_id = info.session_id.clone();
                let title = info.title.clone();
                rsx! {
                    LiveNowBanner {
                        title,
                        on_join: move |_| on_join_active(active_id.clone()),
                    }
                }
            }
            _ => rsx! {},
        }
    }
}

fn map_modules(modules: Vec<api::ModuleWithLessonsDto>) -> Vec<ModuleNode> {
    modules
        .into_iter()
        .map(|m| ModuleNode {
            id: m.id,
            title: m.title,
            lessons: m
                .lessons
                .into_iter()
                .map(|l| LessonNode {
                    id: l.id,
                    title: l.title,
                    r#type: l.r#type,
                })
                .collect(),
        })
        .collect()
}

fn map_members(members: Vec<api::CourseMemberDto>) -> Vec<Member> {
    members
        .into_iter()
        .map(|m| Member {
            user_id: m.user_id,
            display_name: m.display_name.unwrap_or_else(|| m.email.clone()),
            email: m.email,
            role: m.role,
            status: m.status,
        })
        .collect()
}

fn map_sessions(sessions: Vec<api::CourseSessionDto>, can_edit: bool) -> Vec<ScheduleEntry> {
    sessions
        .into_iter()
        .map(|s| ScheduleEntry {
            session_id: s.session_id,
            course_title: s.course_title,
            course_slug: s.course_slug,
            title: s.title,
            starts_at_display: s.starts_at,
            duration_minutes: s.duration_minutes,
            status: s.status,
            diverged: s.diverged,
            can_edit,
        })
        .collect()
}

fn map_invites(invites: Vec<api::CourseInvitationDto>) -> Vec<PendingInvite> {
    invites
        .into_iter()
        .filter(|i| i.status == "pending")
        .map(|i| PendingInvite {
            id: i.id,
            email: i.email,
            role: i.role,
            expires_at: i.expires_at,
        })
        .collect()
}

fn map_codes(codes: Vec<api::CodeSummaryDto>) -> Vec<ActiveCode> {
    codes
        .into_iter()
        .map(|c| ActiveCode {
            id: c.id,
            last4: c.last4,
            uses: c.uses,
            max_uses: c.max_uses,
        })
        .collect()
}

pub(crate) fn render_with_tab(slug: String, active_tab: &'static str) -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => {
            nav.push(Route::Login {});
            return rsx! { p { "Redirecting…" } };
        }
    };

    let course_resource = use_resource({
        let api = api.clone();
        let slug = slug.clone();
        move || {
            let api = api.clone();
            let slug = slug.clone();
            async move {
                let all = api::list_courses(&api).await?;
                all.into_iter()
                    .find(|c| c.slug == slug)
                    .ok_or(api::ApiError::Status(404, "course not found".into()))
            }
        }
    });

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    let snap = course_resource.read_unchecked();
    let course_block: Element = match snap.as_ref() {
        Some(Ok(c)) => {
            let title = c.title.clone();
            let status = c.status.clone();
            let cover = c.cover_asset_id.clone();
            let can_admin = can_admin_course(&user, c);
            let course_id = c.id.clone();

            let extra_actions: Option<Element> = if can_admin {
                Some(rsx! {
                    StartNowSlots {
                        course_id: course_id.clone(),
                        slug: slug.clone(),
                        can_admin: true,
                    }
                })
            } else {
                None
            };

            let banner: Option<Element> = if !can_admin {
                Some(rsx! {
                    StartNowSlots {
                        course_id: course_id.clone(),
                        slug: slug.clone(),
                        can_admin: false,
                    }
                })
            } else {
                None
            };

            let slug_for_tab = slug.clone();
            let tab_body = match active_tab {
                "outline" => rsx! {
                    // Draft courses guide teachers through setup; the card is
                    // dismissible and disappears once the course is published.
                    if can_admin && status == "draft" {
                        features_courses::course_checklist::CourseSetupChecklist {
                            course_id: course_id.clone(),
                            slug: slug.clone(),
                            course_status: status.clone(),
                        }
                    }
                    OutlineTab {
                        course_id: course_id.clone(),
                        course_slug: slug.clone(),
                        can_edit: can_admin,
                    }
                },
                "people" if can_admin => rsx! {
                    PeopleTab {
                        course_id: course_id.clone(),
                    }
                },
                "people" => rsx! { p { "You do not have access to course people." } },
                "edit" if can_admin => rsx! {
                    CourseEditTab {
                        course: c.clone(),
                    }
                },
                "edit" => rsx! { p { "You do not have access to course settings." } },
                "schedule" => rsx! {
                    ScheduleTab {
                        course_id: course_id.clone(),
                        can_edit: can_admin,
                    }
                },
                "analytics" => rsx! {
                    AnalyticsTab {
                        course_id: course_id.clone(),
                    }
                },
                // Quizzes / assignments / recordings render INSIDE the course
                // shell (header + tab bar) like every other tab — they were
                // previously standalone scaffolds that dropped the chrome.
                "quizzes" => rsx! {
                    features_courses::quiz_list::QuizList {
                        course_id: course_id.clone(),
                        can_author: can_admin,
                        can_take: user.can_learn(),
                        is_staff: user.is_course_staff(),
                        on_open: {
                            let slug = slug.clone();
                            move |quiz_id: String| {
                                nav.push(Route::QuizTakePage { slug: slug.clone(), quiz_id });
                            }
                        },
                        on_edit: {
                            let slug = slug.clone();
                            move |quiz_id: String| {
                                nav.push(Route::QuizEditPage { slug: slug.clone(), quiz_id });
                            }
                        },
                    }
                },
                "assignments" => rsx! {
                    features_courses::assignment_list::AssignmentList {
                        api: api.clone(),
                        course_slug: slug.clone(),
                        course_id: course_id.clone(),
                        can_author: can_admin,
                        can_view_drafts: user.is_course_staff(),
                    }
                },
                "recordings" => rsx! {
                    RecordingsTab {
                        course_id: course_id.clone(),
                        course_slug: slug.clone(),
                    }
                },
                "leaderboard" => rsx! {
                    features_courses::gamify_panel::CourseLeaderboard {
                        course_id: course_id.clone(),
                    }
                },
                "certificates" => rsx! {
                    features_courses::certificates_panel::CourseCertificatesTab {
                        course_id: course_id.clone(),
                    }
                },
                "flashcards" => rsx! {
                    features_courses::flashcards_panel::CourseFlashcardsTab {
                        course_id: course_id.clone(),
                        can_edit: can_admin,
                    }
                },
                "announcements" => rsx! {
                    features_courses::announcements::Announcements {
                        course_id: course_id.clone(),
                        is_teacher: user.is_course_staff(),
                    }
                },
                "discussions" => rsx! {
                    features_courses::discussions::Discussions {
                        course_id: course_id.clone(),
                        is_teacher: user.is_course_staff(),
                    }
                },
                "scorm" => rsx! {
                    features_courses::scorm_player::ScormTab {
                        course_id: course_id.clone(),
                        is_teacher: can_admin,
                    }
                },
                "syllabus" => rsx! {
                    features_courses::course_syllabus::CourseSyllabusView {
                        course_id: course_id.clone(),
                    }
                },
                "gradebook" => rsx! {
                    features_courses::gradebook_panel::GradebookPanel {
                        course_id: course_id.clone(),
                        is_teacher: user.can_grade(),
                        can_configure: can_admin,
                    }
                },
                _ => rsx! { p { "Unknown course tab." } },
            };
            // Trail: My Courses › {course} [› {tab}] — one insertion covers
            // every course-tab route since they all render through here.
            let tab_label = match active_tab {
                "outline" => None,
                "people" => Some("People"),
                "edit" => Some("Settings"),
                "schedule" => Some("Schedule"),
                "recordings" => Some("Recordings"),
                "analytics" => Some("Analytics"),
                "assignments" => Some("Assignments"),
                "quizzes" => Some("Quizzes"),
                "leaderboard" => Some("Leaderboard"),
                "certificates" => Some("Certificates"),
                "flashcards" => Some("Flashcards"),
                "announcements" => Some("Announcements"),
                "discussions" => Some("Discussions"),
                "scorm" => Some("SCORM"),
                "syllabus" => Some("Syllabus"),
                "gradebook" => Some("Gradebook"),
                _ => None,
            };
            let mut crumbs = vec![BreadcrumbItem::link("My Courses", "/courses")];
            match tab_label {
                Some(tab) => {
                    crumbs.push(BreadcrumbItem::link(
                        title.clone(),
                        format!("/courses/{slug}"),
                    ));
                    crumbs.push(BreadcrumbItem::current(tab));
                }
                None => crumbs.push(BreadcrumbItem::current(title.clone())),
            }
            rsx! {
                Breadcrumb { items: crumbs, aria_label: "Course navigation".to_string() }
                CourseDetailView {
                    course_title: title,
                    course_status: status,
                    course_cover_asset_id: cover,
                    can_admin: can_admin,
                    active_tab: active_tab.to_string(),
                    on_tab_change: move |tab: String| {
                        let slug = slug_for_tab.clone();
                        match tab.as_str() {
                            "outline" => { nav.push(Route::CourseDetail { slug }); }
                            "people" if can_admin => { nav.push(Route::CoursePeople { slug }); }
                            "edit" if can_admin => { nav.push(Route::CourseEdit { slug }); }
                            "schedule" => { nav.push(Route::CourseSchedule { slug }); }
                            "recordings" => { nav.push(Route::CourseRecordings { slug }); }
                            "analytics" => { nav.push(Route::CourseAnalytics { slug }); }
                            "assignments" => { nav.push(Route::AssignmentList { slug }); }
                            "quizzes" => { nav.push(Route::QuizListPage { slug }); }
                            "leaderboard" => { nav.push(Route::CourseLeaderboardPage { slug }); }
                            "certificates" => { nav.push(Route::CourseCertificatesPage { slug }); }
                            "flashcards" => { nav.push(Route::CourseFlashcardsPage { slug }); }
                            "announcements" => { nav.push(Route::CourseAnnouncementsPage { slug }); }
                            "discussions" => { nav.push(Route::CourseDiscussionsPage { slug }); }
                            "scorm" => { nav.push(Route::CourseScormPage { slug }); }
                            "syllabus" => { nav.push(Route::CourseSyllabusPage { slug }); }
                            "gradebook" if can_admin => { nav.push(Route::CourseGradebookPage { slug }); }
                            _ => {}
                        };
                    },
                    extra_actions: extra_actions,
                    banner: banner,
                    { tab_body }
                }
            }
        }
        Some(Err(e)) => rsx! { p { class: "error", "Course not found: {e}" } },
        None => rsx! { p { "Loading…" } },
    };
    drop(snap);

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                {
                    use platform_bridge::PlatformBridge;
                    spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                }
                nav.push(Route::Login {});
            },
            { course_block }
        }
    }
}

#[component]
fn CourseEditTab(course: api::CourseDto) -> Element {
    let api = use_api();
    let api_for_delete = api.clone();
    let mut title = use_signal(|| course.title.clone());
    let mut description = use_signal(|| course.description.clone().unwrap_or_default());
    let mut status = use_signal(|| course.status.clone());
    let mut saving = use_signal(|| false);
    let mut error: Signal<Option<String>> = use_signal(|| None);
    let mut saved = use_signal(|| false);
    // Publish→schedule handoff: set when a save transitions the course from
    // non-published to published, surfacing the "schedule your first class"
    // prompt below the form.
    let mut just_published = use_signal(|| false);
    let was_published = course.status == "published";
    let mut toast = use_toast_sender();
    let course_id = course.id.clone();
    let course_id_for_delete = course.id.clone();
    let course_id_for_syllabus = course.id.clone();
    let course_slug = course.slug.clone();
    let initial_syllabus_md = course.syllabus_md.clone();
    let initial_grading_policy_md = course.grading_policy_md.clone();

    let on_submit = move |event: FormEvent| {
        event.prevent_default();

        let api = api.clone();
        let course_id = course_id.clone();
        spawn(async move {
            saving.set(true);
            error.set(None);
            saved.set(false);

            let title_value = title.read().clone();
            let description_value = description.read().clone();
            let status_value = status.read().clone();
            let body = api::PatchCourseBody {
                title: Some(title_value.as_str()),
                description: Some(description_value.as_str()),
                status: Some(status_value.as_str()),
                ..Default::default()
            };

            match api::patch_course(&api, &course_id, &body).await {
                Ok(_) => {
                    saved.set(true);
                    toast.push(ToastLevel::Success, "Saved", "Course settings updated.");
                    if status_value == "published" && !was_published {
                        just_published.set(true);
                    }
                }
                Err(e) => {
                    let msg = format!("{e}");
                    toast.push(ToastLevel::Danger, "Save failed", msg.clone());
                    error.set(Some(msg));
                }
            }

            saving.set(false);
        });
    };

    rsx! {
        form { class: "course-edit-tab", onsubmit: on_submit,
            h2 { "Course settings" }
            Field { label: "Title".to_string(),
                Input {
                    value: title.read().clone(),
                    input_type: "text".to_string(),
                    disabled: *saving.read(),
                    on_input: move |v: String| title.set(v),
                }
            }

            Field {
                label: "Description".to_string(),
                for_id: "course-description".to_string(),
                helper: "Plain text. Markdown rendering lands in a follow-up.".to_string(),
                textarea {
                    id: "course-description",
                    name: "description",
                    "aria-describedby": "course-description-description",
                    class: "ds-input",
                    rows: 5,
                    value: "{description}",
                    oninput: move |e| description.set(e.value()),
                    disabled: *saving.read(),
                }
            }

            Field { label: "Status".to_string(),
                Select {
                    value: status.read().clone(),
                    options: vec![
                        SelectOption { value: "draft".to_string(), label: "Draft".to_string() },
                        SelectOption { value: "published".to_string(), label: "Published".to_string() },
                        SelectOption { value: "archived".to_string(), label: "Archived".to_string() },
                    ],
                    disabled: *saving.read(),
                    on_change: move |v: String| status.set(v),
                }
            }

            if let Some(err) = error.read().as_ref() {
                p { class: "error", "{err}" }
            }
            if *saved.read() {
                p { class: "muted", "Course settings saved." }
            }
            if *just_published.read() {
                div { class: "course-publish-handoff",
                    span { class: "course-publish-handoff-kicker", "🎉 Course is live" }
                    p { class: "course-publish-handoff-copy",
                        "Students can now enroll and learn. Ready to teach it live?"
                    }
                    a {
                        class: "ds-button ds-button--primary",
                        href: "/courses/{course_slug}/schedule",
                        "Schedule your first live class →"
                    }
                }
            }
            features_courses::course_syllabus::CourseSyllabusSettings {
                course_id: course_id_for_syllabus.clone(),
                initial_syllabus_md,
                initial_grading_policy_md,
                initial_self_enrollment_enabled: course.self_enrollment_enabled,
                on_duplicated: move |new_slug: String| {
                    #[cfg(target_arch = "wasm32")]
                    if let Some(win) = web_sys::window() {
                        let _ = win.location().set_href(&format!("/courses/{new_slug}/edit"));
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    let _ = new_slug;
                },
            }
            div { class: "course-edit-actions",
                Button {
                    label: if *saving.read() { "Saving…".to_string() } else { "Save course".to_string() },
                    variant: ButtonVariant::Primary,
                    button_type: "submit".to_string(),
                    disabled: *saving.read(),
                    on_click: move |_| {},
                }
                Button {
                    label: "Delete course".to_string(),
                    variant: ButtonVariant::Danger,
                    disabled: *saving.read(),
                    on_click: move |_| {
                        let api = api_for_delete.clone();
                        let course_id = course_id_for_delete.clone();
                        spawn(async move {
                            saving.set(true);
                            match api::delete_course(&api, &course_id).await {
                                Ok(_) => {
                                    toast.push(
                                        ToastLevel::Success,
                                        "Course deleted",
                                        "The course was removed.",
                                    );
                                    #[cfg(target_arch = "wasm32")]
                                    if let Some(win) = web_sys::window() {
                                        let _ = win.location().set_href("/courses");
                                    }
                                }
                                Err(e) => {
                                    let msg = format!("{e}");
                                    toast.push(ToastLevel::Danger, "Delete failed", msg.clone());
                                    error.set(Some(msg));
                                }
                            }
                            saving.set(false);
                        });
                    },
                }
            }
        }
    }
}

#[component]
fn OutlineTab(course_id: String, course_slug: String, can_edit: bool) -> Element {
    let api = use_api();
    let nav = use_navigator();
    let outline = use_resource({
        let api = api.clone();
        let course_id = course_id.clone();
        move || {
            let api = api.clone();
            let course_id = course_id.clone();
            async move { api::get_course_outline(&api, &course_id).await }
        }
    });

    let snap = outline.read_unchecked();
    let content = match snap.as_ref() {
        Some(Ok(modules)) => {
            // Keep a cheap lookup of (lesson_id -> module_id) so the lesson
            // click handler can resolve the path without re-fetching.
            let lesson_to_module: Vec<(String, String)> = modules
                .iter()
                .flat_map(|m| {
                    let mid = m.id.clone();
                    m.lessons.iter().map(move |l| (l.id.clone(), mid.clone()))
                })
                .collect();
            let modules = map_modules(modules.clone());
            if can_edit {
                rsx! {
                    CourseBuilder {
                        modules,
                        on_add_module: {
                            let api = api.clone();
                            let course_id = course_id.clone();
                            move |_| {
                                let api = api.clone();
                                let course_id = course_id.clone();
                                let mut outline = outline;
                                spawn(async move {
                                    let body = api::CreateModuleBody { title: "Untitled module" };
                                    if api::create_module(&api, &course_id, &body).await.is_ok() {
                                        outline.restart();
                                    }
                                });
                            }
                        },
                        on_add_lesson: {
                            let api = api.clone();
                            let course_id = course_id.clone();
                            move |module_id: String| {
                                let api = api.clone();
                                let course_id = course_id.clone();
                                let mut outline = outline;
                                spawn(async move {
                                    let body = api::CreateLessonBody {
                                        r#type: DEFAULT_LESSON_TYPE,
                                        title: "Untitled lesson",
                                        body_md: None,
                                        live_session_id: None,
                                    };
                                    if api::create_lesson(&api, &course_id, &module_id, &body).await.is_ok() {
                                        outline.restart();
                                    }
                                });
                            }
                        },
                        on_lesson_clicked: {
                            let course_slug = course_slug.clone();
                            let lesson_to_module = lesson_to_module.clone();
                            move |lesson_id: String| {
                                if let Some((_, module_id)) =
                                    lesson_to_module.iter().find(|(lid, _)| *lid == lesson_id)
                                {
                                    nav.push(Route::LessonEdit {
                                        slug: course_slug.clone(),
                                        module_id: module_id.clone(),
                                        lesson_id,
                                    });
                                }
                            }
                        },
                        on_modules_reordered: {
                            let api = api.clone();
                            let course_id = course_id.clone();
                            move |module_ids: Vec<String>| {
                                let api = api.clone();
                                let course_id = course_id.clone();
                                let mut outline = outline;
                                spawn(async move {
                                    let body = api::ReorderModulesBody { module_ids };
                                    if api::reorder_modules(&api, &course_id, &body).await.is_ok() {
                                        outline.restart();
                                    }
                                });
                            }
                        },
                        on_lessons_reordered: {
                            let api = api.clone();
                            let course_id = course_id.clone();
                            move |(module_id, lesson_ids): (String, Vec<String>)| {
                                let api = api.clone();
                                let course_id = course_id.clone();
                                let mut outline = outline;
                                spawn(async move {
                                    let body = api::ReorderLessonsBody { lesson_ids };
                                    if api::reorder_lessons(&api, &course_id, &module_id, &body).await.is_ok() {
                                        outline.restart();
                                    }
                                });
                            }
                        },
                        on_module_renamed: {
                            let api = api.clone();
                            let course_id = course_id.clone();
                            move |(module_id, new_title): (String, String)| {
                                let api = api.clone();
                                let course_id = course_id.clone();
                                let mut outline = outline;
                                spawn(async move {
                                    let body = api::PatchModuleBody {
                                        title: Some(new_title.as_str()),
                                    };
                                    match api::patch_module(&api, &course_id, &module_id, &body)
                                        .await
                                    {
                                        Ok(_) => outline.restart(),
                                        Err(e) => tracing::warn!("rename module failed: {e}"),
                                    }
                                });
                            }
                        },
                        on_lesson_renamed: {
                            let api = api.clone();
                            let course_id = course_id.clone();
                            move |(module_id, lesson_id, new_title): (String, String, String)| {
                                let api = api.clone();
                                let course_id = course_id.clone();
                                let mut outline = outline;
                                spawn(async move {
                                    let body = api::PatchLessonBody {
                                        title: Some(new_title.as_str()),
                                        ..Default::default()
                                    };
                                    match api::patch_lesson(&api, &course_id, &module_id, &lesson_id, &body)
                                        .await
                                    {
                                        Ok(_) => outline.restart(),
                                        Err(e) => tracing::warn!("rename lesson failed: {e}"),
                                    }
                                });
                            }
                        },
                        on_module_deleted: {
                            let api = api.clone();
                            let course_id = course_id.clone();
                            move |module_id: String| {
                                let api = api.clone();
                                let course_id = course_id.clone();
                                let mut outline = outline;
                                spawn(async move {
                                    if let Err(e) =
                                        api::delete_module(&api, &course_id, &module_id).await
                                    {
                                        tracing::warn!("delete module failed: {e}");
                                    }
                                    outline.restart();
                                });
                            }
                        },
                        on_lesson_deleted: {
                            let api = api.clone();
                            let course_id = course_id.clone();
                            move |(module_id, lesson_id): (String, String)| {
                                let api = api.clone();
                                let course_id = course_id.clone();
                                let mut outline = outline;
                                spawn(async move {
                                    if let Err(e) = api::delete_lesson(
                                        &api,
                                        &course_id,
                                        &module_id,
                                        &lesson_id,
                                    )
                                    .await
                                    {
                                        tracing::warn!("delete lesson failed: {e}");
                                    }
                                    outline.restart();
                                });
                            }
                        },
                    }
                }
            } else {
                rsx! {
                    StudentOutlineTab {
                        course_id: course_id.clone(),
                        course_slug: course_slug.clone(),
                    }
                }
            }
        }
        Some(Err(e)) => rsx! { p { class: "error", "Could not load course outline: {e}" } },
        None => rsx! { p { "Loading outline…" } },
    };
    drop(snap);

    content
}

/// Student outline: the kinetics progress card + outline tree, with lesson
/// selection navigating to the lesson page. Falls back to the static
/// `ReadOnlyOutline` while progress is loading or when it fails (e.g. an
/// org-admin browsing a course they're not enrolled in).
#[component]
fn StudentOutlineTab(course_id: String, course_slug: String) -> Element {
    let api = use_api();
    let nav = use_navigator();
    let modules = use_resource({
        let api = api.clone();
        let course_id = course_id.clone();
        move || {
            let api = api.clone();
            let course_id = course_id.clone();
            async move { api::get_course_outline(&api, &course_id).await }
        }
    });
    let progress = use_resource({
        let api = api.clone();
        let course_id = course_id.clone();
        move || {
            let api = api.clone();
            let course_id = course_id.clone();
            async move { api::get_course_progress(&api, &course_id).await }
        }
    });

    let modules_snap = modules.read_unchecked();
    let progress_snap = progress.read_unchecked();
    let content = match (modules_snap.as_ref(), progress_snap.as_ref()) {
        (Some(Ok(modules_data)), Some(Ok(progress_data))) => {
            let slug = course_slug.clone();
            rsx! {
                features_courses::course_progress::StudentCourseOutline {
                    modules: modules_data.clone(),
                    progress: progress_data.clone(),
                    on_select: move |lesson_id: String| {
                        nav.push(Route::LessonPage {
                            slug: slug.clone(),
                            lesson_id,
                        });
                    },
                }
            }
        }
        (Some(Ok(modules_data)), Some(Err(_))) => rsx! {
            ReadOnlyOutline { modules: map_modules(modules_data.clone()) }
        },
        (Some(Err(e)), _) => rsx! { p { class: "error", "Could not load course outline: {e}" } },
        _ => rsx! { p { "Loading outline…" } },
    };
    drop(modules_snap);
    drop(progress_snap);

    content
}

#[component]
fn ReadOnlyOutline(modules: Vec<ModuleNode>) -> Element {
    if modules.is_empty() {
        return rsx! {
            div { class: "course-outline-readonly empty",
                h2 { "No modules yet" }
                p { "Course outline content will appear here when modules are published." }
            }
        };
    }

    rsx! {
        div { class: "course-outline-readonly",
            ol { class: "outline-modules",
                for module in &modules {
                    li { key: "{module.id}", class: "outline-module",
                        h3 { "{module.title}" }
                        if module.lessons.is_empty() {
                            p { class: "muted", "No lessons yet." }
                        } else {
                            ol { class: "outline-lessons",
                                for lesson in &module.lessons {
                                    li { key: "{lesson.id}", class: "outline-lesson",
                                        span { class: "type-pill", "{lesson.r#type}" }
                                        span { class: "lesson-title", "{lesson.title}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn PeopleTab(course_id: String) -> Element {
    let api = use_api();
    let mut members = use_resource({
        let api = api.clone();
        let course_id = course_id.clone();
        move || {
            let api = api.clone();
            let course_id = course_id.clone();
            async move { api::list_course_members(&api, &course_id).await }
        }
    });
    let mut invites = use_resource({
        let api = api.clone();
        let course_id = course_id.clone();
        move || {
            let api = api.clone();
            let course_id = course_id.clone();
            async move { api::list_course_invitations(&api, &course_id).await }
        }
    });
    let mut codes = use_resource({
        let api = api.clone();
        let course_id = course_id.clone();
        move || {
            let api = api.clone();
            let course_id = course_id.clone();
            async move { api::list_enrollment_codes(&api, &course_id).await }
        }
    });

    let mut invite_open = use_signal(|| false);
    let mut invite_submitting = use_signal(|| false);
    let mut invite_error = use_signal(|| Option::<String>::None);
    let mut code_open = use_signal(|| false);
    let mut code_submitting = use_signal(|| false);
    let mut code_error = use_signal(|| Option::<String>::None);
    let mut code_generated = use_signal(|| Option::<String>::None);
    let toast = use_toast_sender();

    let members_snap = members.read_unchecked();
    let invites_snap = invites.read_unchecked();
    let codes_snap = codes.read_unchecked();

    let content = match (
        members_snap.as_ref(),
        invites_snap.as_ref(),
        codes_snap.as_ref(),
    ) {
        (Some(Ok(members_data)), Some(Ok(invites_data)), Some(Ok(codes_data))) => rsx! {
            CoursePeopleView {
                members: map_members(members_data.clone()),
                pending_invites: map_invites(invites_data.clone()),
                active_codes: map_codes(codes_data.clone()),
                on_invite_clicked: move |_| {
                    invite_error.set(None);
                    invite_open.set(true);
                },
                on_code_clicked: move |_| {
                    code_error.set(None);
                    code_generated.set(None);
                    code_open.set(true);
                },
                on_revoke_invite: {
                    let api = api.clone();
                    let course_id = course_id.clone();
                    move |invitation_id: String| {
                        let api = api.clone();
                        let course_id = course_id.clone();
                        spawn(async move {
                            if api::revoke_course_invitation(&api, &course_id, &invitation_id).await.is_ok() {
                                invites.restart();
                            }
                        });
                    }
                },
                on_revoke_code: {
                    let api = api.clone();
                    let course_id = course_id.clone();
                    move |code_id: String| {
                        let api = api.clone();
                        let course_id = course_id.clone();
                        spawn(async move {
                            if api::revoke_enrollment_code(&api, &course_id, &code_id).await.is_ok() {
                                codes.restart();
                            }
                        });
                    }
                },
            }
        },
        (Some(Err(e)), _, _) => rsx! { p { class: "error", "Could not load members: {e}" } },
        (_, Some(Err(e)), _) => rsx! { p { class: "error", "Could not load invitations: {e}" } },
        (_, _, Some(Err(e))) => {
            rsx! { p { class: "error", "Could not load enrollment codes: {e}" } }
        }
        _ => rsx! { p { "Loading people…" } },
    };

    drop(codes_snap);
    drop(invites_snap);
    drop(members_snap);

    let invite_open_val = *invite_open.read();
    let code_open_val = *code_open.read();

    rsx! {
        { content }
        BulkImport { course_id: course_id.clone() }
        InviteModal {
            open: invite_open_val,
            on_close: move |_| {
                invite_open.set(false);
                invite_error.set(None);
            },
            on_send: {
                let api = api.clone();
                let course_id = course_id.clone();
                move |(email, role): (String, String)| {
                    let api = api.clone();
                    let course_id = course_id.clone();
                    let trimmed_email = email.trim().to_string();
                    if trimmed_email.is_empty() {
                        invite_error.set(Some("Email is required.".into()));
                        return;
                    }
                    invite_error.set(None);
                    invite_submitting.set(true);
                    let mut toast = toast;
                    spawn(async move {
                        let body = api::CreateInvitationBody {
                            email: trimmed_email.as_str(),
                            role: role.as_str(),
                        };
                        match api::create_course_invitation(&api, &course_id, &body).await {
                            Ok(_) => {
                                toast.push(
                                    ToastLevel::Success,
                                    "Invitation sent",
                                    format!("Invite emailed to {trimmed_email}."),
                                );
                                invite_submitting.set(false);
                                invite_open.set(false);
                                invites.restart();
                                members.restart();
                            }
                            Err(e) => {
                                let msg = format!("{e}");
                                invite_error.set(Some(msg.clone()));
                                toast.push(ToastLevel::Danger, "Invite failed", msg);
                                invite_submitting.set(false);
                            }
                        }
                    });
                }
            },
            submitting: *invite_submitting.read(),
            error: invite_error.read().clone(),
        }
        CodeModal {
            open: code_open_val,
            on_close: move |_| {
                code_open.set(false);
                code_error.set(None);
                code_generated.set(None);
                codes.restart();
            },
            on_generate: {
                let api = api.clone();
                let course_id = course_id.clone();
                move |max_uses: Option<i32>| {
                    let api = api.clone();
                    let course_id = course_id.clone();
                    code_error.set(None);
                    code_submitting.set(true);
                    let mut toast = toast;
                    spawn(async move {
                        let body = api::CreateCodeBody {
                            max_uses,
                            expires_at: None,
                        };
                        match api::create_enrollment_code(&api, &course_id, &body).await {
                            Ok(v) => {
                                let code = v
                                    .get("code")
                                    .and_then(|x| x.as_str())
                                    .map(str::to_owned);
                                if let Some(c) = code {
                                    code_generated.set(Some(c));
                                } else {
                                    code_generated.set(Some("(code created)".into()));
                                }
                                toast.push(
                                    ToastLevel::Success,
                                    "Enrollment code created",
                                    "Share this code with students.",
                                );
                                code_submitting.set(false);
                            }
                            Err(e) => {
                                let msg = format!("{e}");
                                code_error.set(Some(msg.clone()));
                                toast.push(ToastLevel::Danger, "Code generation failed", msg);
                                code_submitting.set(false);
                            }
                        }
                    });
                }
            },
            submitting: *code_submitting.read(),
            generated_code: code_generated.read().clone(),
            error: code_error.read().clone(),
        }
    }
}

#[component]
fn ScheduleTab(course_id: String, can_edit: bool) -> Element {
    let api = use_api();
    let mut sessions = use_resource({
        let api = api.clone();
        let course_id = course_id.clone();
        move || {
            let api = api.clone();
            let course_id = course_id.clone();
            async move { api::list_course_sessions(&api, &course_id).await }
        }
    });

    let mut submitting = use_signal(|| false);
    let mut submit_error = use_signal(|| Option::<String>::None);
    let toast = use_toast_sender();

    // Reschedule modal state
    let mut reschedule_open = use_signal(|| false);
    let mut reschedule_session_id = use_signal(String::new);
    let mut reschedule_starts_at = use_signal(String::new);
    let mut reschedule_submitting = use_signal(|| false);
    let mut reschedule_error = use_signal(|| Option::<String>::None);

    let snap = sessions.read_unchecked();
    let schedule = match snap.as_ref() {
        Some(Ok(sessions_data)) => rsx! {
            ScheduleView {
                entries: map_sessions(sessions_data.clone(), can_edit),
                on_cancel: {
                    let api = api.clone();
                    move |session_id: String| {
                        let api = api.clone();
                        let mut toast = toast;
                        spawn(async move {
                            let body = api::PatchOccurrenceBody {
                                status: Some("cancelled".to_string()),
                                ..Default::default()
                            };
                            match api::patch_occurrence(&api, &session_id, &body).await {
                                Ok(_) => {
                                    toast.push(
                                        ToastLevel::Success,
                                        "Session cancelled",
                                        "The session was cancelled.",
                                    );
                                    sessions.restart();
                                }
                                Err(e) => {
                                    toast.push(
                                        ToastLevel::Danger,
                                        "Cancel failed",
                                        format!("{e}"),
                                    );
                                }
                            }
                        });
                    }
                },
                on_reschedule: move |session_id: String| {
                    reschedule_session_id.set(session_id);
                    reschedule_starts_at.set(String::new());
                    reschedule_error.set(None);
                    reschedule_open.set(true);
                },
            }
        },
        Some(Err(e)) => rsx! { p { class: "error", "Could not load schedule: {e}" } },
        None => rsx! { p { "Loading schedule…" } },
    };
    drop(snap);

    rsx! {
        div { class: "course-schedule-tab",
            if can_edit {
                SeriesScheduler {
                    initial: SeriesDraft {
                        title: String::new(),
                        starts_at_iso: String::new(),
                        duration_minutes: 60,
                        frequency: "none".to_string(),
                        byweekday: Vec::new(),
                        end_kind: "count".to_string(),
                        occurrence_count: Some(1),
                        end_until_iso: None,
                        recording_enabled: Some(true),
                    },
                    preview: Vec::<String>::new(),
                    on_change: move |_| {},
                    on_submit: {
                        let api = api.clone();
                        let course_id = course_id.clone();
                        move |draft: SeriesDraft| {
                            let api = api.clone();
                            let course_id = course_id.clone();
                            // Validate required fields up-front so we don't
                            // bother the server with obviously-bad data.
                            let trimmed_title = draft.title.trim().to_string();
                            if trimmed_title.is_empty() {
                                submit_error.set(Some("Title is required.".into()));
                                return;
                            }
                            if draft.starts_at_iso.trim().is_empty() {
                                submit_error.set(Some("Start time is required.".into()));
                                return;
                            }
                            if draft.duration_minutes <= 0 {
                                submit_error.set(Some("Duration must be positive.".into()));
                                return;
                            }
                            submit_error.set(None);
                            submitting.set(true);
                            let mut toast = toast;
                            spawn(async move {
                                let byweekday = if draft.byweekday.is_empty() {
                                    None
                                } else {
                                    Some(draft.byweekday.clone())
                                };
                                let body = api::CreateSeriesBody {
                                    title: trimmed_title,
                                    starts_at: draft.starts_at_iso.clone(),
                                    duration_minutes: draft.duration_minutes,
                                    frequency: draft.frequency.clone(),
                                    byweekday,
                                    end_kind: draft.end_kind.clone(),
                                    occurrence_count: draft.occurrence_count,
                                    end_until: draft.end_until_iso.clone(),
                                    primary_teacher_id: None,
                                    recording_enabled: draft.recording_enabled,
                                    transport_mode: "webrtc".to_string(),
                                };
                                match api::create_series(&api, &course_id, &body).await {
                                    Ok(created) => {
                                        toast.push(
                                            ToastLevel::Success,
                                            "Session scheduled",
                                            format!(
                                                "{} occurrence(s) created.",
                                                created.occurrences.len()
                                            ),
                                        );
                                        submit_error.set(None);
                                        sessions.restart();
                                    }
                                    Err(e) => {
                                        let msg = format!("{e}");
                                        submit_error.set(Some(msg.clone()));
                                        toast.push(
                                            ToastLevel::Danger,
                                            "Schedule failed",
                                            msg,
                                        );
                                    }
                                }
                                submitting.set(false);
                            });
                        }
                    },
                    submitting: *submitting.read(),
                    error: submit_error.read().clone(),
                }
            }
            { schedule }
        }
        Modal {
            open: *reschedule_open.read(),
            title: "Reschedule session".to_string(),
            on_close: move |_| {
                reschedule_open.set(false);
                reschedule_error.set(None);
            },
            div { class: "reschedule-form",
                p { "Pick a new start time." }
                DateTimePicker {
                    value: reschedule_starts_at.read().clone(),
                    disabled: *reschedule_submitting.read(),
                    on_change: move |v: String| reschedule_starts_at.set(v),
                }
                if let Some(e) = reschedule_error.read().as_ref() {
                    div { class: "form-error", "{e}" }
                }
                div { class: "actions",
                    Button {
                        label: "Reschedule".to_string(),
                        variant: ButtonVariant::Primary,
                        disabled: *reschedule_submitting.read(),
                        on_click: {
                            let api = api.clone();
                            move |_| {
                                let new_starts = reschedule_starts_at.read().clone();
                                if new_starts.trim().is_empty() {
                                    reschedule_error.set(Some("Pick a date and time.".into()));
                                    return;
                                }
                                let session_id = reschedule_session_id.read().clone();
                                let api = api.clone();
                                let mut toast = toast;
                                reschedule_submitting.set(true);
                                reschedule_error.set(None);
                                spawn(async move {
                                    let body = api::PatchOccurrenceBody {
                                        starts_at: Some(new_starts),
                                        ..Default::default()
                                    };
                                    match api::patch_occurrence(&api, &session_id, &body).await {
                                        Ok(_) => {
                                            toast.push(
                                                ToastLevel::Success,
                                                "Session rescheduled",
                                                "Learners will see the new time.",
                                            );
                                            reschedule_submitting.set(false);
                                            reschedule_open.set(false);
                                            sessions.restart();
                                        }
                                        Err(e) => {
                                            let msg = format!("{e}");
                                            reschedule_error.set(Some(msg.clone()));
                                            toast.push(ToastLevel::Danger, "Reschedule failed", msg);
                                            reschedule_submitting.set(false);
                                        }
                                    }
                                });
                            }
                        },
                    }
                }
            }
        }
    }
}

/// Format a duration in seconds as "Xm Ys" (e.g. 185.0 → "3m 5s"). Negative
/// or NaN inputs clamp to "0m 0s".
fn fmt_attendance(seconds: f64) -> String {
    let total = if seconds.is_finite() && seconds > 0.0 {
        seconds.round() as i64
    } else {
        0
    };
    format!("{}m {}s", total / 60, total % 60)
}

/// Render the assignment-progress table from a course analytics rollup. Pure so
/// it's SSR-testable. Row id is the stable assignment_id; a None average grade
/// renders as "—", otherwise as a 1-decimal value.
fn analytics_assignments_table(rows: &[api::AssignmentProgressDto]) -> Element {
    let columns = vec![
        DataTableColumn::new("assignment", "Assignment"),
        DataTableColumn::new("status", "Status"),
        DataTableColumn::new("submitted", "Submitted"),
        DataTableColumn::new("graded", "Graded"),
        DataTableColumn::new("avg", "Avg grade"),
    ];
    let table_rows: Vec<DataTableRow> = rows
        .iter()
        .map(|a| {
            let avg = match a.avg_numeric_grade {
                Some(v) => format!("{v:.1}"),
                None => "—".to_string(),
            };
            DataTableRow::new(
                a.assignment_id.clone(),
                vec![
                    a.title.clone(),
                    a.status.clone(),
                    a.submitted_count.to_string(),
                    a.graded_count.to_string(),
                    avg,
                ],
            )
        })
        .collect();
    rsx! {
        DataTable {
            columns,
            rows: table_rows,
            caption: "Assignment progress".to_string(),
        }
    }
}

/// Truncate an assignment title for a chart axis label.
fn axis_label(title: &str) -> String {
    if title.chars().count() > 12 {
        let cut: String = title.chars().take(11).collect();
        format!("{cut}…")
    } else {
        title.to_string()
    }
}

/// Assignment activity chart (Submitted vs Graded per assignment) with the
/// detailed table in a "View data" expander. Pure so it's SSR-testable.
fn analytics_assignments_chart(rows: &[api::AssignmentProgressDto]) -> Element {
    if rows.is_empty() {
        return rsx! {
            p { class: "muted", "No assignments yet — charts appear once coursework exists." }
        };
    }
    let series = vec![
        ChartSeries::new(
            "Submitted",
            rows.iter().map(|r| r.submitted_count as f32).collect(),
        ),
        ChartSeries::new(
            "Graded",
            rows.iter().map(|r| r.graded_count as f32).collect(),
        ),
    ];
    let x_labels: Vec<String> = rows.iter().map(|r| axis_label(&r.title)).collect();
    rsx! {
        BarChart {
            label: "Assignment activity".to_string(),
            series,
            x_labels,
        }
        details { class: "chart-data-expander",
            summary { "View data" }
            { analytics_assignments_table(rows) }
        }
    }
}

/// Lesson-progress funnel chart + completion gauge. Pure so it's SSR-testable.
fn analytics_funnel(f: &api::ProgressFunnelDto) -> Element {
    let series = vec![ChartSeries::new(
        "Students",
        vec![
            f.enrolled as f32,
            f.started as f32,
            f.half as f32,
            f.completed as f32,
        ],
    )];
    let completion = if f.enrolled > 0 {
        f.completed as f32 / f.enrolled as f32
    } else {
        0.0
    };
    rsx! {
        div { class: "analytics-funnel-row",
            BarChart {
                label: "Progress funnel".to_string(),
                series,
                x_labels: vec![
                    "Enrolled".to_string(),
                    "Started".to_string(),
                    "50%+".to_string(),
                    "Completed".to_string(),
                ],
                show_legend: false,
            }
            DonutGauge {
                label: "Course completion".to_string(),
                value: completion,
                description: format!("{} of {} students finished all lessons", f.completed, f.enrolled),
                tone: ChartTone::Success,
            }
        }
    }
}

/// Graded-quiz score distribution chart. Renders nothing when no graded
/// attempts exist. Pure so it's SSR-testable.
fn analytics_quiz_distribution(q: &api::QuizScoreDistributionDto) -> Element {
    let total = q.under_50 + q.from_50_to_69 + q.from_70_to_89 + q.from_90_up;
    if total == 0 {
        return rsx! {};
    }
    let series = vec![ChartSeries::new(
        "Attempts",
        vec![
            q.under_50 as f32,
            q.from_50_to_69 as f32,
            q.from_70_to_89 as f32,
            q.from_90_up as f32,
        ],
    )];
    rsx! {
        BarChart {
            label: "Quiz score distribution".to_string(),
            series,
            x_labels: vec![
                "<50%".to_string(),
                "50–69%".to_string(),
                "70–89%".to_string(),
                "90%+".to_string(),
            ],
            show_legend: false,
        }
    }
}

/// Render the loaded analytics body: metric cards, then the chart sections
/// (each with an accessible data fallback). Pure so it's SSR-testable.
fn analytics_body(a: &api::CourseAnalyticsDto) -> Element {
    use features_courses::analytics_panel::{
        analytics_at_risk, analytics_grade_distribution, analytics_grade_trend,
    };
    let assignments = analytics_assignments_chart(&a.assignments);
    let funnel = analytics_funnel(&a.funnel);
    let quiz_distribution = analytics_quiz_distribution(&a.quiz_scores);
    let grade_trend = analytics_grade_trend(&a.grade_trend);
    let grade_distribution = analytics_grade_distribution(&a.grade_distribution);
    let at_risk = analytics_at_risk(&a.at_risk, a.at_risk_threshold);
    rsx! {
        div { class: "course-analytics-tab",
            div { class: "course-analytics-metrics",
                MetricCard {
                    label: "Enrolled students".to_string(),
                    value: a.enrolled_students.to_string(),
                    tone: MetricTone::Info,
                }
                MetricCard {
                    label: "Sessions".to_string(),
                    value: format!("{}/{}", a.sessions_ended, a.sessions_total),
                    delta: "ended / total".to_string(),
                    tone: MetricTone::Neutral,
                }
                MetricCard {
                    label: "Unique attendees".to_string(),
                    value: a.unique_attendees.to_string(),
                    tone: MetricTone::Success,
                }
                MetricCard {
                    label: "Avg attendance".to_string(),
                    value: fmt_attendance(a.avg_attendance_seconds),
                    tone: MetricTone::Neutral,
                }
            }
            { funnel }
            { quiz_distribution }
            { grade_trend }
            { grade_distribution }
            { at_risk }
            { assignments }
        }
    }
}

/// Skeleton placeholder shown while the course analytics load.
fn analytics_loading() -> Element {
    rsx! {
        div { class: "course-analytics-tab",
            div { class: "course-analytics-metrics",
                for _ in 0..4 {
                    SkeletonCard { height: "110px".to_string() }
                }
            }
            SkeletonCard { height: "160px".to_string() }
        }
    }
}

#[component]
fn AnalyticsTab(course_id: String) -> Element {
    let api = use_api();
    let analytics = use_resource({
        let api = api.clone();
        let course_id = course_id.clone();
        move || {
            let api = api.clone();
            let course_id = course_id.clone();
            async move { api::get_course_analytics(&api, &course_id).await }
        }
    });

    let snap = analytics.read_unchecked();
    let content = match snap.as_ref() {
        Some(Ok(a)) => analytics_body(a),
        // Staff-only endpoint: 403 means the current user isn't course staff.
        // Show a gentle note rather than a hard error.
        Some(Err(api::ApiError::Status(403, _))) => rsx! {
            p { class: "muted", "Analytics are visible to course staff." }
        },
        Some(Err(e)) => rsx! { p { class: "error", "Could not load analytics: {e}" } },
        None => analytics_loading(),
    };
    drop(snap);

    content
}

/// "Jun 17, 2026 10:39 PM" from RFC3339; the raw string when parsing fails.
fn format_session_ts(raw: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(raw) {
        Ok(dt) => dt.format("%b %-d, %Y %-I:%M %p").to_string(),
        Err(_) => raw.to_string(),
    }
}

/// Human label for a recording's processing status.
fn recording_status_label(status: &str) -> &'static str {
    match status {
        "available" => "Ready",
        "processing" => "Processing…",
        "failed" => "Processing failed — contact your administrator",
        _ => "Pending",
    }
}

/// "1h 12m" / "45m" / "32s" from seconds — recordings can be hours long, so
/// a bare minutes count ("452m") misreads.
fn format_recording_duration(total_seconds: i32) -> String {
    let s = total_seconds.max(0);
    if s >= 3600 {
        format!("{}h {}m", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}m", s / 60)
    } else {
        format!("{s}s")
    }
}

/// One recording row. Pure so it's SSR-testable.
fn recording_row(r: &api::CourseRecordingListItemDto, course_slug: &str) -> Element {
    let duration = format_recording_duration(r.duration_seconds);
    let when = format_session_ts(&r.starts_at);
    let status = recording_status_label(&r.processing_status);
    let is_available = r.has_playback && r.processing_status == "available";
    let audio_only = r.has_video == Some(false);
    let href = format!("/courses/{course_slug}/sessions/{}", r.session_id);
    rsx! {
        li { class: "recording-item",
            div { class: "row-1",
                span { class: "title", "{r.session_title}" }
                span { class: "muted", "{when}" }
            }
            div { class: "row-2",
                span { class: "muted", "{duration} · {status}" }
                // Flag it in the LIST, not just the player: the point is to set
                // the expectation before someone clicks "Watch replay" and
                // finds a black rectangle. Only an explicit false qualifies --
                // an unprobed recording must not be labelled.
                if audio_only {
                    span { class: "recording-badge recording-badge--audio-only",
                        title: "This recording has no video track - only the audio was saved.",
                        "Audio only"
                    }
                }
                if is_available {
                    a { class: "ds-button ds-button--secondary", href: "{href}",
                        if audio_only { "Listen to replay" } else { "Watch replay" }
                    }
                }
            }
        }
    }
}

#[component]
fn RecordingsTab(course_id: String, course_slug: String) -> Element {
    let api = use_api();
    let recordings = use_resource({
        let api = api.clone();
        let course_id = course_id.clone();
        move || {
            let api = api.clone();
            let course_id = course_id.clone();
            async move { api::list_course_recordings(&api, &course_id).await }
        }
    });

    let snap = recordings.read_unchecked();
    let content: Element = match snap.as_ref() {
        Some(Ok(rows)) if rows.is_empty() => rsx! {
            design_system::EmptyState {
                title: "No recordings yet.".to_string(),
                description: "Recordings appear here after live classes end and finish processing.".to_string(),
            }
        },
        Some(Ok(rows)) => {
            let rows = rows.clone();
            rsx! {
                ul { class: "recordings-list",
                    for r in rows.iter() {
                        { recording_row(r, &course_slug) }
                    }
                }
            }
        }
        Some(Err(e)) => rsx! { p { class: "error", "Could not load recordings: {e}" } },
        None => rsx! { SkeletonCard { height: "160px".to_string() } },
    };
    drop(snap);
    content
}

#[component]
pub fn CourseDetail(slug: String) -> Element {
    render_with_tab(slug, "outline")
}

#[component]
pub fn CourseAnalytics(slug: String) -> Element {
    render_with_tab(slug, "analytics")
}

#[component]
pub fn CoursePeople(slug: String) -> Element {
    render_with_tab(slug, "people")
}

#[component]
pub fn CourseEdit(slug: String) -> Element {
    render_with_tab(slug, "edit")
}

#[component]
pub fn CourseSchedule(slug: String) -> Element {
    render_with_tab(slug, "schedule")
}

#[component]
pub fn CourseLeaderboardPage(slug: String) -> Element {
    render_with_tab(slug, "leaderboard")
}

#[component]
pub fn CourseAnnouncementsPage(slug: String) -> Element {
    render_with_tab(slug, "announcements")
}

#[component]
pub fn CourseDiscussionsPage(slug: String) -> Element {
    render_with_tab(slug, "discussions")
}

#[component]
pub fn CourseScormPage(slug: String) -> Element {
    render_with_tab(slug, "scorm")
}

#[component]
pub fn CourseSyllabusPage(slug: String) -> Element {
    render_with_tab(slug, "syllabus")
}

#[component]
pub fn CourseGradebookPage(slug: String) -> Element {
    render_with_tab(slug, "gradebook")
}

#[component]
pub fn CourseCertificatesPage(slug: String) -> Element {
    render_with_tab(slug, "certificates")
}

#[component]
pub fn CourseFlashcardsPage(slug: String) -> Element {
    render_with_tab(slug, "flashcards")
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_types::TenantRole;

    fn course(owner_user_id: &str) -> api::CourseDto {
        api::CourseDto {
            id: "course-1".to_string(),
            slug: "math".to_string(),
            title: "Math".to_string(),
            description: None,
            status: "draft".to_string(),
            cover_asset_id: None,
            owner_user_id: owner_user_id.to_string(),
            caller_course_role: Some("teacher".to_string()),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            syllabus_md: None,
            grading_policy_md: None,
            self_enrollment_enabled: false,
        }
    }

    fn user(
        user_id: &str,
        tenant_role: Option<TenantRole>,
        is_platform_admin: bool,
    ) -> UserContext {
        UserContext {
            user_id: user_id.to_string(),
            display_name: "Test User".to_string(),
            email: "test@example.test".to_string(),
            tenant_id: Some("00000000-0000-0000-0000-000000000010".to_string()),
            tenant_role,
            is_platform_admin,
        }
    }

    #[component]
    fn CourseEditTabHarness(course: api::CourseDto) -> Element {
        use design_system::ToastQueue;

        let api_signal = use_signal(|| api::ApiContext {
            base_url: "http://localhost:8080".to_string(),
            id_token: String::new(),
        });
        use_context_provider::<Signal<api::ApiContext>>(|| api_signal);
        use_context_provider::<Signal<ToastQueue>>(|| Signal::new(ToastQueue::new()));

        rsx! {
            CourseEditTab {
                course,
            }
        }
    }

    #[test]
    fn assigned_tenant_teacher_is_course_admin_for_a_visible_course() {
        let user = user("teacher-1", Some(TenantRole::Teacher), false);
        assert!(can_admin_course(&user, &course("teacher-2")));

        let mut assistant_assignment = course("teacher-2");
        assistant_assignment.caller_course_role = Some("ta".to_string());
        assert!(!can_admin_course(&user, &assistant_assignment));
    }

    #[test]
    fn active_teacher_org_owner_org_admin_and_contextual_platform_admin_are_course_admins() {
        assert!(can_admin_course(
            &user("owner-1", Some(TenantRole::Teacher), false),
            &course("owner-1")
        ));
        assert!(can_admin_course(
            &user("org-owner-1", Some(TenantRole::OrgOwner), false),
            &course("owner-1")
        ));
        assert!(can_admin_course(
            &user("admin-1", Some(TenantRole::OrgAdmin), false),
            &course("owner-1")
        ));
        assert!(can_admin_course(
            &user("platform-1", Some(TenantRole::Student), true),
            &course("owner-1")
        ));
    }

    #[test]
    fn historical_owner_loses_admin_ui_after_demotion() {
        assert!(!can_admin_course(
            &user("owner-1", Some(TenantRole::Student), false),
            &course("owner-1")
        ));
    }

    #[test]
    fn default_lesson_type_matches_backend_contract() {
        assert_eq!(DEFAULT_LESSON_TYPE, "rich_text");
    }

    #[test]
    fn course_edit_tab_renders_existing_course_values() {
        let mut dom = VirtualDom::new_with_props(
            CourseEditTabHarness,
            CourseEditTabHarnessProps {
                course: api::CourseDto {
                    description: Some("Introductory algebra".to_string()),
                    status: "published".to_string(),
                    ..course("owner-1")
                },
            },
        );
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);

        assert!(
            html.contains("Course settings"),
            "html missing heading: {html}"
        );
        assert!(html.contains("Math"), "html missing title: {html}");
        assert!(
            html.contains("Introductory algebra"),
            "html missing description: {html}"
        );
        assert!(
            html.contains("<select"),
            "html missing status select: {html}"
        );
        assert!(
            html.contains(r#"option value="draft""#),
            "html missing draft status option: {html}"
        );
        assert!(
            html.contains(r#"option value="published""#),
            "html missing published status option: {html}"
        );
        assert!(
            html.contains(r#"option value="archived""#),
            "html missing archived status option: {html}"
        );
    }

    #[test]
    fn read_only_outline_omits_builder_edit_controls() {
        let modules = vec![ModuleNode {
            id: "module-1".to_string(),
            title: "Week 1".to_string(),
            lessons: vec![LessonNode {
                id: "lesson-1".to_string(),
                title: "Limits".to_string(),
                r#type: "rich_text".to_string(),
            }],
        }];
        let mut dom = VirtualDom::new_with_props(ReadOnlyOutline, ReadOnlyOutlineProps { modules });
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);

        assert!(html.contains("Week 1"), "html missing module: {html}");
        assert!(html.contains("Limits"), "html missing lesson: {html}");
        assert!(
            !html.contains("+ Add lesson"),
            "html has edit control: {html}"
        );
        assert!(!html.contains("+ Module"), "html has edit control: {html}");
    }

    #[test]
    fn course_detail_header_shows_edit_affordance_for_admin() {
        fn app() -> Element {
            rsx! {
                features_courses::course_detail::CourseDetail {
                    course_title: "Math".to_string(),
                    course_status: "draft".to_string(),
                    course_cover_asset_id: None::<String>,
                    can_admin: true,
                    active_tab: "outline".to_string(),
                    on_tab_change: move |_: String| {},
                    div { "body" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-page-header-actions"),
            "expected page-header actions slot for admin: {html}"
        );
        assert!(
            html.contains(">Edit</button>"),
            "expected admin Edit button: {html}"
        );
    }

    fn analytics_sample() -> api::CourseAnalyticsDto {
        api::CourseAnalyticsDto {
            course_id: "course-1".to_string(),
            enrolled_students: 42,
            sessions_total: 10,
            sessions_ended: 7,
            unique_attendees: 38,
            avg_attendance_seconds: 185.0,
            assignments: vec![
                api::AssignmentProgressDto {
                    assignment_id: "a-1".to_string(),
                    title: "Essay 1".to_string(),
                    status: "published".to_string(),
                    submitted_count: 30,
                    graded_count: 25,
                    avg_numeric_grade: Some(87.456),
                },
                api::AssignmentProgressDto {
                    assignment_id: "a-2".to_string(),
                    title: "Quiz".to_string(),
                    status: "draft".to_string(),
                    submitted_count: 0,
                    graded_count: 0,
                    avg_numeric_grade: None,
                },
            ],
            funnel: api::ProgressFunnelDto {
                enrolled: 42,
                started: 30,
                half: 18,
                completed: 9,
            },
            quiz_scores: api::QuizScoreDistributionDto {
                under_50: 3,
                from_50_to_69: 6,
                from_70_to_89: 12,
                from_90_up: 8,
            },
            grade_distribution: vec![],
            grade_trend: vec![],
            at_risk: vec![],
            at_risk_threshold: 0.60,
        }
    }

    #[test]
    fn fmt_attendance_formats_minutes_and_seconds() {
        assert_eq!(fmt_attendance(185.0), "3m 5s");
        assert_eq!(fmt_attendance(0.0), "0m 0s");
        assert_eq!(fmt_attendance(-12.0), "0m 0s");
        assert_eq!(fmt_attendance(f64::NAN), "0m 0s");
        assert_eq!(fmt_attendance(59.6), "1m 0s");
    }

    #[test]
    fn analytics_body_renders_metric_cards_and_data_table() {
        fn app_inner(a: api::CourseAnalyticsDto) -> Element {
            analytics_body(&a)
        }
        let mut vdom = VirtualDom::new_with_props(app_inner, analytics_sample());
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ui-metric-card"), "got: {html}");
        assert!(html.contains("ui-data-table"), "got: {html}");
        // Metric values.
        assert!(html.contains("Enrolled students"));
        assert!(html.contains("42"));
        assert!(html.contains("7/10"));
        assert!(html.contains("3m 5s"));
        // Table content: rounded average + the em-dash for None.
        assert!(html.contains("Essay 1"));
        assert!(html.contains("87.5"));
        assert!(html.contains("—"));
        assert!(html.contains("Assignment"));
        // Charts: assignment bars + funnel + quiz distribution, with the
        // detailed table behind a "View data" expander.
        assert!(html.contains("ui-chart--bar"), "bar charts missing: {html}");
        assert!(html.contains("Progress funnel"));
        assert!(html.contains("Quiz score distribution"));
        assert!(
            html.contains("ui-donut-gauge"),
            "completion gauge missing: {html}"
        );
        assert!(html.contains("9 of 42 students finished all lessons"));
        assert!(
            html.contains("chart-data-expander"),
            "expander missing: {html}"
        );
    }

    #[test]
    fn quiz_distribution_hidden_when_no_graded_attempts() {
        fn app_inner(q: api::QuizScoreDistributionDto) -> Element {
            analytics_quiz_distribution(&q)
        }
        let mut vdom =
            VirtualDom::new_with_props(app_inner, api::QuizScoreDistributionDto::default());
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            !html.contains("Quiz score distribution"),
            "should hide with zero attempts: {html}"
        );
    }

    #[test]
    fn axis_label_truncates_long_titles() {
        assert_eq!(axis_label("Short"), "Short");
        assert_eq!(axis_label("A very long assignment title"), "A very long…");
    }

    #[test]
    fn course_detail_header_hides_edit_affordance_for_non_admin() {
        fn app() -> Element {
            rsx! {
                features_courses::course_detail::CourseDetail {
                    course_title: "Math".to_string(),
                    course_status: "draft".to_string(),
                    course_cover_asset_id: None::<String>,
                    can_admin: false,
                    active_tab: "outline".to_string(),
                    on_tab_change: move |_: String| {},
                    div { "body" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            !html.contains(">Edit</button>"),
            "non-admin should not see edit button: {html}"
        );
    }
}
