//! Student-facing course progress surfaces (learning-suite Cycle 2): the
//! progress-aware course outline (kinetics `CourseOutline`), the course
//! progress card, and the dashboard "resume learning" strip.

use crate::api::{self, ApiContext, CourseProgressDto, ModuleWithLessonsDto, MyCourseProgressDto};
use design_system::kinetics_ui::{
    CourseLesson, CourseModule, CourseOutline, CourseProgressCard, LessonState, ResumeLearning,
};
use dioxus::prelude::*;

/// Per-lesson lock state from `GET /v1/courses/:cid/lessons/lock-state`
/// (drip release-date and/or unmet prerequisites). Defined locally so this
/// module stays self-contained; the backend returns
/// `{ lesson_id, locked, reason }`.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct LessonLock {
    pub lesson_id: String,
    pub locked: bool,
    pub reason: Option<String>,
}

/// `GET /v1/courses/:cid/lessons/lock-state` — the caller's per-lesson lock
/// state across one course. Staff receive an empty list (never gated).
pub async fn get_course_lock_state(
    cx: &ApiContext,
    course_id: &str,
) -> Result<Vec<LessonLock>, api::ApiError> {
    api::fetch_json(
        cx,
        "GET",
        &format!("/v1/courses/{course_id}/lessons/lock-state"),
        None::<&()>,
    )
    .await
}

/// Map the course outline + the caller's completions + lock state onto the
/// kinetics outline vocabulary. A locked lesson (drip/prereq) becomes
/// `Locked` (rendered disabled) with its reason shown in the duration slot;
/// otherwise completed lessons are `Completed`, the first incomplete unlocked
/// lesson is `Current`, and the rest are `Available`.
pub fn to_outline_modules(
    modules: &[ModuleWithLessonsDto],
    completed_lesson_ids: &[String],
    locks: &[LessonLock],
) -> Vec<CourseModule> {
    let mut current_marked = false;
    modules
        .iter()
        .map(|m| CourseModule {
            id: m.id.clone(),
            title: m.title.clone(),
            lessons: m
                .lessons
                .iter()
                .map(|l| {
                    let lock = locks.iter().find(|x| x.lesson_id == l.id && x.locked);
                    let (state, duration) = if let Some(lock) = lock {
                        let reason = lock.reason.clone().unwrap_or_else(|| "Locked".to_string());
                        (LessonState::Locked, format!("🔒 {reason}"))
                    } else if completed_lesson_ids.contains(&l.id) {
                        (
                            LessonState::Completed,
                            lesson_type_label(&l.r#type).to_string(),
                        )
                    } else if !current_marked {
                        current_marked = true;
                        (
                            LessonState::Current,
                            lesson_type_label(&l.r#type).to_string(),
                        )
                    } else {
                        (
                            LessonState::Available,
                            lesson_type_label(&l.r#type).to_string(),
                        )
                    };
                    CourseLesson {
                        id: l.id.clone(),
                        title: l.title.clone(),
                        duration,
                        state,
                    }
                })
                .collect(),
        })
        .collect()
}

fn lesson_type_label(lesson_type: &str) -> &'static str {
    match lesson_type {
        "rich_text" => "Reading",
        "video" => "Video",
        "live_session" => "Live session",
        "file_bundle" => "Materials",
        _ => "Lesson",
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct StudentCourseOutlineProps {
    pub modules: Vec<ModuleWithLessonsDto>,
    pub progress: CourseProgressDto,
    pub on_select: EventHandler<String>,
}

/// Progress-aware outline for enrolled students: a course progress card on
/// top of the kinetics outline tree. Selecting a lesson bubbles its id up.
///
/// Lock state (drip release-dates + unmet prerequisites) is fetched here so
/// callers don't have to thread it through; locked lessons render disabled in
/// the kinetics outline with their reason in the duration slot. A failed
/// lock-state fetch degrades gracefully to "nothing locked".
#[component]
pub fn StudentCourseOutline(props: StudentCourseOutlineProps) -> Element {
    let cx = api::use_api();
    let course_id = props.progress.course_id.clone();
    let locks = use_resource(move || {
        let cx = cx.clone();
        let course_id = course_id.clone();
        async move {
            get_course_lock_state(&cx, &course_id)
                .await
                .unwrap_or_default()
        }
    });
    let lock_states = locks.read_unchecked().clone().unwrap_or_default();
    let outline = to_outline_modules(
        &props.modules,
        &props.progress.completed_lesson_ids,
        &lock_states,
    );
    rsx! {
        div { class: "student-outline",
            CourseProgressCard {
                title: "Your progress".to_string(),
                completed: props.progress.completed.max(0) as usize,
                total: props.progress.total.max(0) as usize,
            }
            CourseOutline {
                label: "Course outline".to_string(),
                modules: outline,
                on_select: props.on_select,
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ResumeLearningStripProps {
    pub progress: Vec<MyCourseProgressDto>,
    /// Called with `(course_slug, lesson_id)` when the learner hits resume.
    pub on_resume: EventHandler<(String, String)>,
}

/// Dashboard strip: "pick up where you left off" for the most recently
/// active course that still has an incomplete lesson. Renders nothing when
/// there is nothing to resume (all done, or no enrollments).
#[component]
pub fn ResumeLearningStrip(props: ResumeLearningStripProps) -> Element {
    // `/v1/me/progress` is ordered by recent activity; pick the first course
    // with a resume point.
    let target = props
        .progress
        .iter()
        .find(|p| p.resume_lesson_id.is_some() && p.total > 0)
        .cloned();
    let Some(course) = target else {
        return rsx! {};
    };
    let lesson_title = course.resume_lesson_title.clone().unwrap_or_default();
    let lesson_id = course.resume_lesson_id.clone().unwrap_or_default();
    let slug = course.slug.clone();
    let fraction = if course.total > 0 {
        course.completed as f32 / course.total as f32
    } else {
        0.0
    };
    rsx! {
        div { class: "resume-learning-strip",
            ResumeLearning {
                course: course.title.clone(),
                lesson: lesson_title,
                progress: fraction,
                on_resume: move |_| props.on_resume.call((slug.clone(), lesson_id.clone())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::LessonSummaryDto;

    fn lesson(id: &str, title: &str) -> LessonSummaryDto {
        LessonSummaryDto {
            id: id.into(),
            course_id: "c".into(),
            module_id: "m".into(),
            r#type: "rich_text".into(),
            title: title.into(),
            body_md: None,
            video_asset_id: None,
            live_session_id: None,
            sort_order: 0,
        }
    }

    fn module(id: &str, lessons: Vec<LessonSummaryDto>) -> ModuleWithLessonsDto {
        ModuleWithLessonsDto {
            id: id.into(),
            course_id: "c".into(),
            title: format!("Module {id}"),
            sort_order: 0,
            lessons,
        }
    }

    #[test]
    fn outline_marks_completed_then_first_incomplete_as_current() {
        let modules = vec![
            module("m1", vec![lesson("l1", "One"), lesson("l2", "Two")]),
            module("m2", vec![lesson("l3", "Three")]),
        ];
        let completed = vec!["l1".to_string()];

        let outline = to_outline_modules(&modules, &completed, &[]);

        assert_eq!(outline[0].lessons[0].state, LessonState::Completed);
        assert_eq!(outline[0].lessons[1].state, LessonState::Current);
        assert_eq!(outline[1].lessons[0].state, LessonState::Available);
    }

    #[test]
    fn outline_with_no_completions_marks_first_lesson_current() {
        let modules = vec![module("m1", vec![lesson("l1", "One"), lesson("l2", "Two")])];
        let outline = to_outline_modules(&modules, &[], &[]);
        assert_eq!(outline[0].lessons[0].state, LessonState::Current);
        assert_eq!(outline[0].lessons[1].state, LessonState::Available);
    }

    #[test]
    fn outline_locks_lesson_with_unmet_prereq_and_skips_it_for_current() {
        let modules = vec![module(
            "m1",
            vec![
                lesson("l1", "One"),
                lesson("l2", "Two"),
                lesson("l3", "Three"),
            ],
        )];
        let locks = vec![LessonLock {
            lesson_id: "l2".into(),
            locked: true,
            reason: Some("Finish \"One\" first".into()),
        }];
        let outline = to_outline_modules(&modules, &[], &locks);
        assert_eq!(outline[0].lessons[0].state, LessonState::Current);
        assert_eq!(outline[0].lessons[1].state, LessonState::Locked);
        assert!(outline[0].lessons[1].duration.contains("Finish"));
        // The locked l2 must not consume the "current" marker.
        assert_eq!(outline[0].lessons[2].state, LessonState::Available);
    }

    #[test]
    fn resume_strip_picks_first_course_with_a_resume_point() {
        fn app() -> Element {
            rsx! {
                ResumeLearningStrip {
                    progress: vec![
                        MyCourseProgressDto {
                            course_id: "c1".into(),
                            slug: "done".into(),
                            title: "Finished course".into(),
                            completed: 3,
                            total: 3,
                            last_activity_at: None,
                            resume_lesson_id: None,
                            resume_lesson_title: None,
                        },
                        MyCourseProgressDto {
                            course_id: "c2".into(),
                            slug: "algebra".into(),
                            title: "Algebra".into(),
                            completed: 1,
                            total: 4,
                            last_activity_at: None,
                            resume_lesson_id: Some("l2".into()),
                            resume_lesson_title: Some("Linear equations".into()),
                        },
                    ],
                    on_resume: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Algebra"), "{html}");
        assert!(html.contains("Linear equations"));
        assert!(!html.contains("Finished course"));
    }

    #[test]
    fn resume_strip_renders_nothing_when_all_complete() {
        fn app() -> Element {
            rsx! {
                ResumeLearningStrip {
                    progress: vec![MyCourseProgressDto {
                        course_id: "c1".into(),
                        slug: "done".into(),
                        title: "Finished".into(),
                        completed: 2,
                        total: 2,
                        last_activity_at: None,
                        resume_lesson_id: None,
                        resume_lesson_title: None,
                    }],
                    on_resume: |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(!html.contains("Finished"), "{html}");
    }
}
