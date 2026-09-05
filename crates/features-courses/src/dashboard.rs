// crates/features-courses/src/dashboard.rs
use crate::schedule_view::format_session_ts;
use core_types::TenantRole;
use design_system::kinetics_ui::{MetricCard, MetricTone};
use design_system::{
    Card, CardVariant, EmptyState, EmptyStateVariant, PageHeader, PageHeaderVariant,
};
use dioxus::prelude::*;

/// Tasteful on-brand hero accent: a faint orbital motif + a gold "live" play
/// glyph, echoing the auth hero / generated course covers. Self-contained inline
/// SVG (no external text → no injection surface); strokes use `currentColor` so
/// it recolors for light + dark, with the gold (#b08842) brand accent inline.
const DASHBOARD_HERO_SVG: &str = "<svg xmlns='http://www.w3.org/2000/svg' \
viewBox='0 0 220 160' fill='none' role='img' aria-hidden='true' width='100%' \
height='100%' preserveAspectRatio='xMidYMid meet'>\
<g stroke='currentColor' stroke-opacity='0.14' stroke-width='1.5'>\
<circle cx='150' cy='66' r='86'/><circle cx='150' cy='66' r='58'/>\
<circle cx='150' cy='66' r='30'/></g>\
<circle cx='150' cy='66' r='30' fill='#b08842' fill-opacity='0.12' \
stroke='#b08842' stroke-width='2'/>\
<path d='M143 53 L166 66 L143 79 Z' fill='#b08842'/>\
<g stroke='currentColor' stroke-opacity='0.28' stroke-width='3' \
stroke-linecap='round'>\
<line x1='24' y1='52' x2='84' y2='52'/><line x1='24' y1='72' x2='104' y2='72'/>\
<line x1='24' y1='92' x2='72' y2='92'/></g>\
<line x1='24' y1='52' x2='60' y2='52' stroke='#b08842' stroke-width='3' \
stroke-linecap='round'/></svg>";

#[derive(Clone, PartialEq)]
pub struct EnrolledCourse {
    pub course_id: String,
    pub slug: String,
    pub title: String,
    pub status: String,
    pub role: String,
    pub next_session_at: Option<String>,
}

#[derive(Props, Clone, PartialEq)]
pub struct DashboardProps {
    pub display_name: String,
    pub courses: Vec<EnrolledCourse>,
    pub upcoming_count: usize,
    /// When true, render the "Finish setting up your workspace" onboarding CTA
    /// above the dashboard. The route sets this only for an org-admin who has
    /// zero courses, so it disappears on its own once a course exists — no
    /// backend "onboarded" flag needed.
    #[props(default)]
    pub show_setup_banner: bool,
    /// The tenant-level role shapes language and next actions. Course-level
    /// roles still appear on individual course rows.
    #[props(default)]
    pub tenant_role: Option<TenantRole>,
    #[props(default)]
    pub is_platform_admin: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DashboardAudience {
    Owner,
    Administrator,
    Teacher,
    TeachingAssistant,
    Learner,
    Parent,
}

fn dashboard_audience(role: Option<TenantRole>, is_platform_admin: bool) -> DashboardAudience {
    if matches!(role, Some(TenantRole::OrgOwner)) {
        return DashboardAudience::Owner;
    }
    if is_platform_admin || matches!(role, Some(TenantRole::OrgAdmin)) {
        return DashboardAudience::Administrator;
    }
    match role {
        Some(TenantRole::Teacher) => DashboardAudience::Teacher,
        Some(TenantRole::Ta) => DashboardAudience::TeachingAssistant,
        Some(TenantRole::Parent) => DashboardAudience::Parent,
        _ => DashboardAudience::Learner,
    }
}

fn course_role_label(role: &str) -> &str {
    match role {
        "teacher" => "Teaching",
        "ta" => "Teaching assistant",
        "student" => "Learning",
        _ => "Member",
    }
}

fn status_label(status: &str) -> &str {
    match status {
        "draft" => "Draft",
        "published" => "Published",
        "archived" => "Archived",
        _ => "Course",
    }
}

/// The onboarding CTA banner shown above the dashboard for an org-admin with no
/// courses yet. Links to `/onboarding`. Pure so it's SSR-testable.
fn setup_banner() -> Element {
    rsx! {
        Card {
            variant: CardVariant::Premium,
            div { class: "onboarding-banner",
                div { class: "onboarding-banner-signal", "aria-hidden": "true",
                    span { "01" }
                }
                div { class: "onboarding-banner-copy",
                    p { class: "onboarding-banner-eyebrow", "2-minute guided setup" }
                    h2 { class: "onboarding-banner-title", "Turn your workspace into an academy" }
                    p { class: "muted", "Add your brand, invite your teaching team, and create the first course." }
                    ul { class: "onboarding-banner-checks", "aria-label": "Setup includes",
                        li { "Brand" }
                        li { "Team" }
                        li { "Course" }
                    }
                }
                a { class: "primary-link onboarding-banner-cta", href: "/onboarding",
                    "Continue setup"
                }
            }
        }
    }
}

#[component]
pub fn Dashboard(props: DashboardProps) -> Element {
    // Courses where this user teaches. Pure students always have zero, and a
    // "Teaching 0" tile reads as authoring vocabulary on a student surface —
    // so the Teaching metric only renders when the user actually teaches.
    let teaching_count = props
        .courses
        .iter()
        .filter(|c| matches!(c.role.as_str(), "teacher" | "ta"))
        .count();
    let audience = dashboard_audience(props.tenant_role, props.is_platform_admin);
    let (
        kicker,
        subtitle,
        courses_label,
        sessions_label,
        empty_title,
        empty_description,
        empty_cta_href,
        empty_cta_label,
        no_sessions_copy,
    ) = match audience {
        DashboardAudience::Owner => (
            "Owner command center",
            "Lead your academy at a glance, then jump into teaching, team, and commercial operations.",
            "Academy courses",
            "Upcoming sessions",
            "Build your first learning space",
            "Start with a course, then add lessons and schedule the first live session.",
            "/courses/new",
            "Create a course",
            "Your next live session will appear here as soon as it is scheduled.",
        ),
        DashboardAudience::Administrator => (
            "Academy operations",
            "Run the teaching team, courses, and day-to-day workspace experience from one focused view.",
            "Academy courses",
            "Upcoming sessions",
            "Build your first learning space",
            "Start with a course, then add lessons and schedule the first live session.",
            "/courses/new",
            "Create a course",
            "Your next live session will appear here as soon as it is scheduled.",
        ),
        DashboardAudience::Teacher => (
            "Teaching studio",
            "Your courses, live sessions, and teaching priorities are ready in one focused workspace.",
            "Teaching courses",
            "Upcoming sessions",
            "Create your first course",
            "Set up a course shell, add the outline, and invite learners when you are ready.",
            "/courses/new",
            "Create a course",
            "Plan your next live session from one of your teaching courses.",
        ),
        DashboardAudience::TeachingAssistant => (
            "Teaching workspace",
            "Stay close to the courses you support, upcoming sessions, and learner activity.",
            "Supported courses",
            "Upcoming sessions",
            "No assigned courses yet",
            "Once an academy owner or teacher adds you to a course, it will appear here.",
            "/schedule",
            "View schedule",
            "Assigned live sessions will appear here when your teaching team schedules them.",
        ),
        DashboardAudience::Learner => (
            "Learning home",
            "Pick up where you left off, see what is live next, and keep your learning momentum moving.",
            "Enrolled courses",
            "Upcoming live classes",
            "Your next course starts here",
            "Join with an enrollment code from your academy to unlock lessons and live classes.",
            "/redeem",
            "Join a course",
            "New live classes will appear here when your teachers schedule them.",
        ),
        DashboardAudience::Parent => (
            "Family overview",
            "See the learning spaces connected to your family and keep upcoming activity in view.",
            "Linked courses",
            "Upcoming sessions",
            "No learner activity linked yet",
            "Open the family view to connect a learner invitation and follow their progress.",
            "/parent",
            "Open family view",
            "Upcoming learner sessions will appear here once a learner is connected.",
        ),
    };
    let session_noun = match (audience, props.upcoming_count) {
        (DashboardAudience::Learner, 1) => "live class",
        (DashboardAudience::Learner, _) => "live classes",
        (_, 1) => "session",
        _ => "sessions",
    };
    let hero_actions = match audience {
        DashboardAudience::Owner => rsx! {
            a { class: "ds-button ds-button--primary", href: "/courses/new", "Create course" }
            a { class: "ds-button ds-button--secondary", href: "/admin/billing", "Plan & billing" }
        },
        DashboardAudience::Administrator => rsx! {
            a { class: "ds-button ds-button--primary", href: "/courses/new", "Create course" }
            a { class: "ds-button ds-button--secondary", href: "/admin", "Manage academy" }
        },
        DashboardAudience::Teacher => rsx! {
            a { class: "ds-button ds-button--primary", href: "/courses/new", "Create course" }
            a { class: "ds-button ds-button--secondary", href: "/schedule", "Teaching schedule" }
        },
        DashboardAudience::TeachingAssistant => rsx! {
            a { class: "ds-button ds-button--primary", href: "/courses", "Open teaching workspace" }
            a { class: "ds-button ds-button--secondary", href: "/schedule", "Teaching schedule" }
        },
        DashboardAudience::Learner => rsx! {
            a { class: "ds-button ds-button--primary", href: "/redeem", "Join a course" }
            a { class: "ds-button ds-button--secondary", href: "/schedule", "My schedule" }
        },
        DashboardAudience::Parent => rsx! {
            a { class: "ds-button ds-button--primary", href: "/parent", "Open family view" }
        },
    };
    let empty_cta = rsx! {
        a { class: "ds-button ds-button--primary", href: "{empty_cta_href}", "{empty_cta_label}" }
    };
    rsx! {
        div { class: "dashboard page-stack motion-page",
            if props.show_setup_banner {
                {setup_banner()}
            }
            // The hero PageHeader was text-only and the .dashboard-hero CSS
            // (gold-wash overlay) was orphaned — nothing carried the class. Wrap
            // it in .dashboard-hero so the styled hero actually renders, with a
            // tasteful on-brand inline-SVG "live academy" accent on the side.
            header { class: "dashboard-hero",
                div { class: "dashboard-hero-body",
                    PageHeader {
                        kicker: kicker.to_string(),
                        title: format!("Welcome back, {}", props.display_name),
                        subtitle: subtitle.to_string(),
                        variant: PageHeaderVariant::Hero,
                    }
                    nav { class: "dashboard-hero-actions", "aria-label": "Recommended actions",
                        {hero_actions}
                    }
                }
                div {
                    class: "dashboard-hero-art",
                    "aria-hidden": "true",
                    dangerous_inner_html: DASHBOARD_HERO_SVG,
                }
            }
            // KPI strip — kinetics MetricCard tiles (glass/elevation styling
            // inherits AulaLite's brand via the design_system token bridge).
            // Layout lives on .dashboard-metrics in components.css (Cycle 7
            // polish) so the strip participates in the .page-stack rhythm.
            div {
                class: "dashboard-metrics",
                MetricCard {
                    label: courses_label.to_string(),
                    value: props.courses.len().to_string(),
                    tone: MetricTone::Info,
                }
                MetricCard {
                    label: sessions_label.to_string(),
                    value: props.upcoming_count.to_string(),
                    tone: if props.upcoming_count > 0 { MetricTone::Success } else { MetricTone::Neutral },
                }
                if teaching_count > 0 {
                    MetricCard {
                        label: "Teaching".to_string(),
                        value: teaching_count.to_string(),
                        tone: MetricTone::Neutral,
                    }
                }
            }
            div { class: "dashboard-grid",
                Card {
                    variant: CardVariant::Default,
                    div { class: "card-header dashboard-card-header",
                        h2 { "Your courses" }
                        if !props.courses.is_empty() {
                            a { class: "dashboard-card-link", href: "/courses", "View all" }
                        }
                    }
                    if props.courses.is_empty() {
                        EmptyState {
                            title: empty_title.to_string(),
                            description: empty_description.to_string(),
                            variant: EmptyStateVariant::Accent,
                            cta: Some(empty_cta),
                        }
                    } else {
                        ul { class: "course-list-mini",
                            for course in &props.courses {
                                {
                                    let slug = course.slug.clone();
                                    let title = course.title.clone();
                                    let role = course.role.clone();
                                    let role_label = course_role_label(&role);
                                    let course_status = status_label(&course.status);
                                    // Humanize the raw backend timestamp (falls back to
                                    // the raw string if it isn't parseable).
                                    let next = course
                                        .next_session_at
                                        .as_deref()
                                        .map(format_session_ts);
                                    rsx! {
                                        li { class: "course-index-row",
                                            div { class: "course-index-main",
                                                a { href: "/courses/{slug}", "{title}" }
                                                span { class: "course-index-status", "{course_status}" }
                                            }
                                            span { class: "role-badge", "{role_label}" }
                                            if let Some(n) = next {
                                                span { class: "next-session", "Next: {n}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Card {
                    variant: CardVariant::Default,
                    div { class: "card-header dashboard-card-header",
                        h2 { "Coming up" }
                        a { class: "dashboard-card-link", href: "/schedule", "Full schedule" }
                    }
                    div { class: "dashboard-upcoming-readout",
                        strong { "{props.upcoming_count}" }
                        span { "{session_noun} in the next 30 days" }
                    }
                    if props.upcoming_count == 0 {
                        p { class: "dashboard-upcoming-note", "{no_sessions_copy}" }
                    } else {
                        p { class: "dashboard-upcoming-note", "Everything scheduled across your courses, all in one place." }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    #[test]
    fn empty_state_renders_when_no_courses() {
        fn app() -> Element {
            rsx! {
                Dashboard {
                    display_name: "Eve".to_string(),
                    courses: Vec::<EnrolledCourse>::new(),
                    upcoming_count: 0usize,
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Your next course starts here"));
        assert!(html.contains("/redeem"), "learner CTA missing: {html}");
        assert!(
            html.contains("empty-state--accent"),
            "accent empty-state class missing: {html}"
        );
    }

    #[test]
    fn courses_list_renders() {
        fn app() -> Element {
            rsx! {
                Dashboard {
                    display_name: "Tee".to_string(),
                    courses: vec![EnrolledCourse {
                        course_id: "abc".into(),
                        slug: "calc-1".into(),
                        title: "Calc 1".into(),
                        status: "draft".into(),
                        role: "teacher".into(),
                        next_session_at: Some("Tue 5:00 PM".into()),
                    }],
                    upcoming_count: 1usize,
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Calc 1"));
        assert!(html.contains("/courses/calc-1"));
        // Non-timestamp strings pass through the formatter untouched.
        assert!(html.contains("Next: Tue 5:00 PM"), "fallback lost: {html}");
    }

    #[test]
    fn next_session_timestamp_renders_humanized() {
        fn app() -> Element {
            rsx! {
                Dashboard {
                    display_name: "Tee".to_string(),
                    courses: vec![EnrolledCourse {
                        course_id: "abc".into(),
                        slug: "calc-1".into(),
                        title: "Calc 1".into(),
                        status: "published".into(),
                        role: "teacher".into(),
                        next_session_at: Some("2026-06-19T22:39:04.5996892".into()),
                    }],
                    upcoming_count: 1usize,
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Next: Jun 19, 2026 10:39 PM"),
            "humanized next-session missing: {html}"
        );
        assert!(
            !html.contains("2026-06-19T22:39"),
            "raw ISO timestamp leaked: {html}"
        );
    }

    #[test]
    fn hero_page_header_renders_kicker_and_title() {
        fn app() -> Element {
            rsx! {
                Dashboard {
                    display_name: "Tee".to_string(),
                    courses: Vec::<EnrolledCourse>::new(),
                    upcoming_count: 0usize,
                    tenant_role: Some(TenantRole::OrgOwner),
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("ds-page-header--hero"),
            "hero variant class missing: {html}"
        );
        // The hero is now wrapped in the (previously orphaned) .dashboard-hero
        // surface with an on-brand inline-SVG accent.
        assert!(
            html.contains("dashboard-hero"),
            "dashboard-hero wrapper missing: {html}"
        );
        assert!(
            html.contains("dashboard-hero-art"),
            "dashboard-hero artwork slot missing: {html}"
        );
        assert!(html.contains("<svg"), "hero accent svg missing: {html}");
        assert!(
            html.contains("Owner command center"),
            "kicker missing: {html}"
        );
        assert!(
            html.contains("Create course"),
            "owner action missing: {html}"
        );
        assert!(
            html.contains("Plan &#38; billing")
                || html.contains("Plan &amp; billing")
                || html.contains("Plan & billing"),
            "owner action missing: {html}"
        );
        assert!(html.contains("Welcome back, Tee"), "title missing: {html}");
    }

    #[test]
    fn organization_admin_gets_operations_not_owner_billing() {
        assert_eq!(
            dashboard_audience(Some(TenantRole::OrgAdmin), false),
            DashboardAudience::Administrator
        );
        assert_eq!(
            dashboard_audience(Some(TenantRole::OrgOwner), false),
            DashboardAudience::Owner
        );
    }

    #[test]
    fn setup_banner_renders_when_flagged_and_zero_courses() {
        fn app() -> Element {
            rsx! {
                Dashboard {
                    display_name: "Admin".to_string(),
                    courses: Vec::<EnrolledCourse>::new(),
                    upcoming_count: 0usize,
                    show_setup_banner: true,
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Turn your workspace into an academy"),
            "setup banner missing: {html}"
        );
        assert!(
            html.contains("/onboarding"),
            "onboarding link missing: {html}"
        );
        assert!(html.contains("onboarding-banner"));
    }

    #[test]
    fn setup_banner_absent_when_not_flagged() {
        // A non-admin (or admin with courses) never sets the flag → no banner.
        fn app() -> Element {
            rsx! {
                Dashboard {
                    display_name: "Tee".to_string(),
                    courses: vec![EnrolledCourse {
                        course_id: "abc".into(),
                        slug: "calc-1".into(),
                        title: "Calc 1".into(),
                        status: "published".into(),
                        role: "teacher".into(),
                        next_session_at: None,
                    }],
                    upcoming_count: 0usize,
                    show_setup_banner: false,
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            !html.contains("Finish setting up your workspace"),
            "setup banner leaked: {html}"
        );
        assert!(
            !html.contains("/onboarding"),
            "onboarding link leaked: {html}"
        );
    }

    #[test]
    fn dashboard_renders_kinetics_metric_cards() {
        fn app() -> Element {
            rsx! {
                Dashboard {
                    display_name: "Tee".to_string(),
                    courses: vec![EnrolledCourse {
                        course_id: "abc".into(),
                        slug: "calc-1".into(),
                        title: "Calc 1".into(),
                        status: "published".into(),
                        role: "teacher".into(),
                        next_session_at: None,
                    }],
                    upcoming_count: 2usize,
                    tenant_role: Some(TenantRole::Teacher),
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        // The kinetics MetricCard renders with its `.ui-metric-card` class
        // (styled via the design_system token bridge → AulaLite brand) and
        // carries our KPI labels — proving the library is wired end-to-end.
        assert!(
            html.contains("ui-metric-card"),
            "kinetics metric-card class missing: {html}"
        );
        assert!(
            html.contains("Upcoming sessions"),
            "metric label missing: {html}"
        );
        assert!(html.contains("Teaching"), "teaching metric missing: {html}");
    }

    #[test]
    fn teaching_metric_hidden_for_pure_students() {
        fn app() -> Element {
            rsx! {
                Dashboard {
                    display_name: "Stu".to_string(),
                    courses: vec![EnrolledCourse {
                        course_id: "abc".into(),
                        slug: "calc-1".into(),
                        title: "Calc 1".into(),
                        status: "published".into(),
                        role: "student".into(),
                        next_session_at: None,
                    }],
                    upcoming_count: 1usize,
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            !html.contains("Teaching"),
            "Teaching metric leaked onto a student dashboard: {html}"
        );
    }

    #[test]
    fn upcoming_copy_pluralizes() {
        fn one_session() -> Element {
            rsx! {
                Dashboard {
                    display_name: "Stu".to_string(),
                    courses: Vec::<EnrolledCourse>::new(),
                    upcoming_count: 1usize,
                    tenant_role: Some(TenantRole::Teacher),
                }
            }
        }
        fn two_sessions() -> Element {
            rsx! {
                Dashboard {
                    display_name: "Stu".to_string(),
                    courses: Vec::<EnrolledCourse>::new(),
                    upcoming_count: 2usize,
                    tenant_role: Some(TenantRole::Teacher),
                }
            }
        }
        let mut one = VirtualDom::new(one_session);
        one.rebuild_in_place();
        let html = dioxus_ssr::render(&one);
        assert!(
            html.contains("session in the next 30 days"),
            "singular copy missing: {html}"
        );
        assert!(html.contains(">1<"), "singular count missing: {html}");

        let mut two = VirtualDom::new(two_sessions);
        two.rebuild_in_place();
        let html = dioxus_ssr::render(&two);
        assert!(
            html.contains("sessions in the next 30 days"),
            "plural copy missing: {html}"
        );
        assert!(html.contains(">2<"), "plural count missing: {html}");
    }

    #[test]
    fn learner_and_ta_actions_are_role_appropriate() {
        fn learner() -> Element {
            rsx! {
                Dashboard {
                    display_name: "Stu".to_string(),
                    courses: Vec::<EnrolledCourse>::new(),
                    upcoming_count: 0usize,
                    tenant_role: Some(TenantRole::Student),
                }
            }
        }
        fn teaching_assistant() -> Element {
            rsx! {
                Dashboard {
                    display_name: "Alex".to_string(),
                    courses: Vec::<EnrolledCourse>::new(),
                    upcoming_count: 0usize,
                    tenant_role: Some(TenantRole::Ta),
                }
            }
        }

        let mut learner_dom = VirtualDom::new(learner);
        learner_dom.rebuild_in_place();
        let learner_html = dioxus_ssr::render(&learner_dom);
        assert!(learner_html.contains("Learning home"));
        assert!(learner_html.contains("Join a course"));
        assert!(!learner_html.contains("Manage academy"));

        let mut ta_dom = VirtualDom::new(teaching_assistant);
        ta_dom.rebuild_in_place();
        let ta_html = dioxus_ssr::render(&ta_dom);
        assert!(ta_html.contains("Teaching workspace"));
        assert!(ta_html.contains("No assigned courses yet"));
        assert!(!ta_html.contains("Create course"));
    }
}
