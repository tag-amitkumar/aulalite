// crates/features-courses/src/onboarding_wizard.rs
//
// Guided onboarding wizard for a brand-new organization. An org-admin who has
// just provisioned an empty workspace is walked through four steps:
//
//   1. Workspace  — name the workspace + pick a primary brand color
//                    (`patch_my_tenant` + `patch_admin_branding`).
//   2. Invite     — invite one or more teachers by email
//                    (`create_member_invitation(email, "teacher")`).
//   3. Course     — create the first course (`create_course`).
//   4. Done       — summary + navigate to the new course or the dashboard.
//
// The wizard composes EXISTING `api` client fns only — no new endpoints. Every
// network step is best-effort: buttons disable while submitting and failures
// surface via a toast rather than blocking the flow (Skip is allowed on the
// invite + course steps). The current step lives in a signal; Next/Back move
// between steps and the kinetics `Stepper` reflects progress.
//
// The component is parameterized by `EventHandler` callbacks (`save_workspace`,
// `invite_teacher`, `create_course`, `go_to_course`, `go_to_dashboard`) so the
// shell route owns the live `ApiContext` / router and this component stays
// pure-enough to SSR-test without a live backend.

use design_system::kinetics_ui::{Stepper, StepperStep};
use design_system::{
    use_toast_sender, Button, ButtonVariant, Card, CardVariant, Field, Input, PageHeader,
    ToastLevel,
};
use dioxus::prelude::*;

/// Default brand color the picker opens on (mirrors the design-system green-600
/// primary token, matching `admin_branding`). Pure so it stays in one place.
pub const DEFAULT_PRIMARY: &str = "#244f43";

/// The four wizard steps, in order. Stable string ids drive the `Stepper`
/// `current` prop and the internal `current_step` signal.
pub const STEP_WORKSPACE: &str = "workspace";
pub const STEP_INVITE: &str = "invite";
pub const STEP_COURSE: &str = "course";
pub const STEP_DONE: &str = "done";
pub const STEP_ORDER: [&str; 4] = [STEP_WORKSPACE, STEP_INVITE, STEP_COURSE, STEP_DONE];

/// Build the ordered `Stepper` steps. Pure so SSR tests can assert the labels.
pub fn wizard_steps() -> Vec<StepperStep> {
    vec![
        StepperStep::new(STEP_WORKSPACE, "Workspace").with_description("Name & brand color"),
        StepperStep::new(STEP_INVITE, "Invite teachers").with_description("Add your staff"),
        StepperStep::new(STEP_COURSE, "First course").with_description("Create a course"),
        StepperStep::new(STEP_DONE, "Done").with_description("You're set up"),
    ]
}

/// Normalize a free-text hex value for the native color picker, which only
/// accepts `#rrggbb`. Anything else is coerced to `fallback`. Pure/testable.
pub fn picker_value(raw: &str, fallback: &str) -> String {
    let t = raw.trim();
    let is_hex6 =
        t.len() == 7 && t.starts_with('#') && t[1..].chars().all(|c| c.is_ascii_hexdigit());
    if is_hex6 {
        t.to_string()
    } else {
        fallback.to_string()
    }
}

/// Return true when a value can be persisted as a six-digit CSS hex color.
/// The API remains authoritative, but validating here gives immediate feedback
/// instead of saving a value the browser cannot preview.
pub fn is_valid_brand_color(raw: &str) -> bool {
    let t = raw.trim();
    t.len() == 7 && t.starts_with('#') && t[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Very light email sanity check used only to skip obviously-empty rows before
/// firing an invite. The backend is the source of truth. Pure/testable.
pub fn looks_like_email(s: &str) -> bool {
    let t = s.trim();
    let Some((local, domain)) = t.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && !local.chars().any(char::is_whitespace)
        && !domain.chars().any(char::is_whitespace)
        && domain.split('.').all(|segment| {
            !segment.is_empty() && !segment.starts_with('-') && !segment.ends_with('-')
        })
        && domain.contains('.')
}

/// Result of the workspace step's combined save: each leg is independent so a
/// partial failure still lets the admin proceed (the toast names what failed).
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceSaveOutcome {
    pub name_ok: bool,
    pub branding_ok: bool,
    pub error: Option<String>,
}

#[derive(Props, Clone, PartialEq)]
pub struct OnboardingWizardProps {
    /// Persist the workspace name + primary color. Args: `(name, primary_color,
    /// cb)`. The callback reports the per-leg outcome so the wizard can toast
    /// and advance on success.
    pub save_workspace: EventHandler<(String, String, EventHandler<WorkspaceSaveOutcome>)>,
    /// Invite a single teacher by email. Args: `(email, cb)`. The callback
    /// receives `Ok(())` or `Err(message)`.
    pub invite_teacher: EventHandler<(String, EventHandler<Result<(), String>>)>,
    /// Create the first course. Args: `(title, description, cb)`. The callback
    /// receives `Ok(slug)` or `Err(message)`.
    pub create_course: EventHandler<(String, String, EventHandler<Result<String, String>>)>,
    /// Navigate to the freshly-created course (carries the slug).
    pub go_to_course: EventHandler<String>,
    /// Navigate to the dashboard.
    pub go_to_dashboard: EventHandler<()>,
}

#[component]
pub fn OnboardingWizard(props: OnboardingWizardProps) -> Element {
    let mut toast = use_toast_sender();

    // --- Wizard navigation ---
    let mut current_step = use_signal(|| STEP_WORKSPACE.to_string());

    // --- Step 1: Workspace ---
    let mut workspace_name = use_signal(String::new);
    let mut primary_color = use_signal(|| DEFAULT_PRIMARY.to_string());
    let mut ws_saving = use_signal(|| false);

    // --- Step 2: Invite teachers ---
    // One editable row per email; starts with a single blank row. `sent` is the
    // running list of emails the backend accepted (shown back to the admin).
    let mut invite_rows = use_signal(|| vec![String::new()]);
    let sent_invites = use_signal(Vec::<String>::new);
    let mut inviting = use_signal(|| false);
    // Counter for the in-flight invite batch (declared at the top level — hooks
    // must not be called inside event handlers). Reset to 0 before each batch.
    let mut invite_done = use_signal(|| 0usize);
    let mut invite_failures = use_signal(|| 0usize);

    // --- Step 3: First course ---
    let mut course_title = use_signal(String::new);
    let mut course_desc = use_signal(String::new);
    let mut creating = use_signal(|| false);
    let created_slug = use_signal(|| None::<String>);

    let steps = wizard_steps();
    let current = current_step.read().clone();
    let current_index = STEP_ORDER
        .iter()
        .position(|step| *step == current)
        .unwrap_or(0);
    let current_step_number = current_index + 1;
    let progress_percent = (current_index + 1) * 25;
    let time_hint = match current_index {
        0 => "About 3 minutes left",
        1 => "About 2 minutes left",
        2 => "About 1 minute left",
        _ => "Ready to launch",
    };

    // ---- Step 1 body ----
    let save_workspace = props.save_workspace;
    let workspace_body = {
        let name_v = workspace_name.read().clone();
        let primary_v = primary_color.read().clone();
        let primary_picker = picker_value(&primary_v, DEFAULT_PRIMARY);
        let busy = *ws_saving.read();
        rsx! {
            Card {
                variant: CardVariant::Accent,
                div { class: "onboarding-step onboarding-step--workspace",
                    div { class: "onboarding-step-intro",
                        span { class: "onboarding-step-number", "Step 1" }
                        h2 { class: "onboarding-step-title", "Make the space feel like yours" }
                        p { class: "muted", "Choose the academy name your members will see and set a recognizable brand accent." }
                    }
                    div { class: "onboarding-workspace-layout",
                    div { class: "onboarding-step-fields",
                    Field {
                        label: "Workspace name".to_string(),
                        for_id: "onboarding-workspace-name".to_string(),
                        helper: "Use the name learners and teachers already know.".to_string(),
                        Input {
                            id: "onboarding-workspace-name".to_string(),
                            name: "workspace_name".to_string(),
                            value: name_v.clone(),
                            placeholder: "e.g. Northgate Academy".to_string(),
                            disabled: busy,
                            on_input: move |v: String| workspace_name.set(v),
                        }
                    }
                    Field {
                        label: "Primary brand color".to_string(),
                        for_id: "onboarding-brand-color".to_string(),
                        helper: "Enter a six-digit hex color, such as #244f43.".to_string(),
                        div { class: "onboarding-color-row",
                            input {
                                id: "onboarding-color-picker",
                                class: "onboarding-color-picker",
                                r#type: "color",
                                value: "{primary_picker}",
                                disabled: busy,
                                "aria-label": "Choose primary brand color",
                                oninput: move |e| primary_color.set(e.value()),
                            }
                            Input {
                                id: "onboarding-brand-color".to_string(),
                                name: "brand_color".to_string(),
                                value: primary_v.clone(),
                                placeholder: DEFAULT_PRIMARY.to_string(),
                                disabled: busy,
                                error: !primary_v.trim().is_empty() && !is_valid_brand_color(&primary_v),
                                on_input: move |v: String| primary_color.set(v),
                            }
                        }
                    }
                    }
                    aside { class: "onboarding-brand-preview", "aria-label": "Brand preview",
                        div {
                            class: "onboarding-brand-preview-mark",
                            style: "--onboarding-preview-color: {primary_picker};",
                            span { "A" }
                        }
                        div { class: "onboarding-brand-preview-copy",
                            span { "Workspace preview" }
                            strong {
                                if name_v.trim().is_empty() { "Your academy" } else { "{name_v}" }
                            }
                            small { "Live learning, beautifully organized" }
                        }
                    }
                    }
                    div { class: "onboarding-actions",
                        Button {
                            label: if busy { "Saving workspace…".to_string() } else { "Save & continue".to_string() },
                            variant: ButtonVariant::Primary,
                            loading: busy,
                            disabled: busy,
                            on_click: move |_| {
                                let name = workspace_name.read().trim().to_string();
                                if name.is_empty() {
                                    toast.push(
                                        ToastLevel::Warning,
                                        "Name required",
                                        "Enter a workspace name to continue.",
                                    );
                                    return;
                                }
                                let primary = {
                                    let v = primary_color.read().trim().to_string();
                                    if v.is_empty() { DEFAULT_PRIMARY.to_string() } else { v }
                                };
                                if !is_valid_brand_color(&primary) {
                                    toast.push(
                                        ToastLevel::Warning,
                                        "Check the brand color",
                                        "Use a six-digit hex color such as #244f43.",
                                    );
                                    return;
                                }
                                ws_saving.set(true);
                                let mut toast = toast;
                                let mut ws_saving = ws_saving;
                                let mut current_step = current_step;
                                let cb = EventHandler::new(move |out: WorkspaceSaveOutcome| {
                                    ws_saving.set(false);
                                    if !out.name_ok {
                                        toast.push(
                                            ToastLevel::Danger,
                                            "Workspace was not saved",
                                            out.error.unwrap_or_else(|| "Try again in a moment.".to_string()),
                                        );
                                        return;
                                    } else if !out.branding_ok {
                                        toast.push(
                                            ToastLevel::Warning,
                                            "Workspace saved",
                                            "The name is ready, but the brand color needs another try in Settings.",
                                        );
                                    } else {
                                        toast.push(
                                            ToastLevel::Success,
                                            "Workspace saved",
                                            "Your academy identity is ready.",
                                        );
                                    }
                                    current_step.set(STEP_INVITE.to_string());
                                });
                                save_workspace.call((name, primary, cb));
                            },
                        }
                    }
                }
            }
        }
    };

    // ---- Step 2 body ----
    let invite_teacher = props.invite_teacher;
    let rows = invite_rows.read().clone();
    let sent = sent_invites.read().clone();
    let inviting_now = *inviting.read();
    let invite_body = rsx! {
        Card {
            variant: CardVariant::Accent,
            div { class: "onboarding-step onboarding-step--invite",
                div { class: "onboarding-step-intro",
                    span { class: "onboarding-step-number", "Step 2" }
                    h2 { class: "onboarding-step-title", "Bring in your teaching team" }
                    p { class: "muted", "Invite teachers now, or skip this step and add the team later from Academy settings." }
                }
                div { class: "onboarding-invite-rows",
                    for (idx, email) in rows.iter().cloned().enumerate() {
                        div { class: "onboarding-invite-row", key: "{idx}",
                            Field {
                                label: format!("Teacher email {}", idx + 1),
                                for_id: format!("onboarding-teacher-email-{idx}"),
                                error: if !email.trim().is_empty() && !looks_like_email(&email) {
                                    Some("Enter a complete email address.".to_string())
                                } else {
                                    None
                                },
                                Input {
                                    id: format!("onboarding-teacher-email-{idx}"),
                                    name: format!("teacher_email_{idx}"),
                                    value: email.clone(),
                                    input_type: "email".to_string(),
                                    placeholder: "teacher@example.com".to_string(),
                                    disabled: inviting_now,
                                    error: !email.trim().is_empty() && !looks_like_email(&email),
                                    on_input: move |v: String| {
                                        let mut r = invite_rows.read().clone();
                                        if idx < r.len() { r[idx] = v; invite_rows.set(r); }
                                    },
                                }
                            }
                            if rows.len() > 1 {
                                Button {
                                    label: "Remove".to_string(),
                                    variant: ButtonVariant::Ghost,
                                    disabled: inviting_now,
                                    on_click: move |_| {
                                        let mut r = invite_rows.read().clone();
                                        if idx < r.len() { r.remove(idx); }
                                        if r.is_empty() { r.push(String::new()); }
                                        invite_rows.set(r);
                                    },
                                }
                            }
                        }
                    }
                }
                div { class: "onboarding-invite-add",
                    Button {
                        label: "Add another".to_string(),
                        variant: ButtonVariant::Secondary,
                        disabled: inviting_now,
                        on_click: move |_| {
                            let mut r = invite_rows.read().clone();
                            r.push(String::new());
                            invite_rows.set(r);
                        },
                    }
                }
                if !sent.is_empty() {
                    div { class: "onboarding-invites-sent", role: "status", "aria-live": "polite",
                        p { class: "muted", "Ready to join" }
                        ul { class: "onboarding-invites-sent-list",
                            for e in sent.iter().cloned() {
                                li { key: "{e}", "{e}" }
                            }
                        }
                    }
                }
                div { class: "onboarding-actions",
                    Button {
                        label: "Back".to_string(),
                        variant: ButtonVariant::Ghost,
                        disabled: inviting_now,
                        on_click: move |_| current_step.set(STEP_WORKSPACE.to_string()),
                    }
                    Button {
                        label: "Skip".to_string(),
                        variant: ButtonVariant::Secondary,
                        disabled: inviting_now,
                        on_click: move |_| current_step.set(STEP_COURSE.to_string()),
                    }
                    Button {
                        label: if inviting_now { "Sending…".to_string() } else { "Send & continue".to_string() },
                        variant: ButtonVariant::Primary,
                        loading: inviting_now,
                        disabled: inviting_now,
                        on_click: move |_| {
                            let entered = invite_rows.read().clone();
                            let invalid_count = entered
                                .iter()
                                .filter(|email| !email.trim().is_empty() && !looks_like_email(email))
                                .count();
                            if invalid_count > 0 {
                                toast.push(
                                    ToastLevel::Warning,
                                    "Check the email addresses",
                                    format!("Fix {invalid_count} incomplete email address(es), or remove the row."),
                                );
                                return;
                            }
                            // Collect valid, not-yet-sent emails.
                            let already: Vec<String> = sent_invites.read().clone();
                            let pending: Vec<String> = entered
                                .iter()
                                .map(|s| s.trim().to_string())
                                .filter(|s| looks_like_email(s) && !already.contains(s))
                                .collect();
                            if pending.is_empty() {
                                // Nothing new to send — just move on.
                                current_step.set(STEP_COURSE.to_string());
                                return;
                            }
                            inviting.set(true);
                            // Fire each invite; advance once all have settled.
                            let total = pending.len();
                            invite_done.set(0);
                            invite_failures.set(0);
                            for email in pending {
                                let mut toast = toast;
                                let mut inviting = inviting;
                                let mut sent_invites = sent_invites;
                                let mut done = invite_done;
                                let mut failures = invite_failures;
                                let mut current_step = current_step;
                                let email_for_cb = email.clone();
                                let cb = EventHandler::new(move |res: Result<(), String>| {
                                    let failed_before = *failures.read();
                                    let failed_now = res.is_err();
                                    match res {
                                        Ok(()) => {
                                            let mut s = sent_invites.read().clone();
                                            if !s.contains(&email_for_cb) {
                                                s.push(email_for_cb.clone());
                                            }
                                            sent_invites.set(s);
                                            toast.push(
                                                ToastLevel::Success,
                                                "Invite sent",
                                                email_for_cb.clone(),
                                            );
                                        }
                                        Err(msg) => {
                                            failures.set(failed_before + 1);
                                            toast.push(
                                                ToastLevel::Danger,
                                                "Invite failed",
                                                format!("{email_for_cb}: {msg}"),
                                            );
                                        }
                                    }
                                    let n = *done.read() + 1;
                                    done.set(n);
                                    if n >= total {
                                        inviting.set(false);
                                        let failed_total = failed_before + failed_now as usize;
                                        if failed_total == 0 {
                                            current_step.set(STEP_COURSE.to_string());
                                        } else {
                                            toast.push(
                                                ToastLevel::Warning,
                                                "Some invites need attention",
                                                "Review the failed addresses, then try again or continue with Skip.",
                                            );
                                        }
                                    }
                                });
                                invite_teacher.call((email, cb));
                            }
                        },
                    }
                }
            }
        }
    };

    // ---- Step 3 body ----
    let create_course = props.create_course;
    let course_body = {
        let title_v = course_title.read().clone();
        let desc_v = course_desc.read().clone();
        let busy = *creating.read();
        rsx! {
            Card {
                variant: CardVariant::Accent,
                div { class: "onboarding-step onboarding-step--course",
                    div { class: "onboarding-step-intro",
                        span { class: "onboarding-step-number", "Step 3" }
                        h2 { class: "onboarding-step-title", "Create the first destination" }
                        p { class: "muted", "Start with a clear course shell. Modules, lessons, live sessions, and assignments come next." }
                    }
                    div { class: "onboarding-course-prompts", "aria-label": "Available course tools",
                        span { "Structured lessons" }
                        span { "Live sessions" }
                        span { "Assignments" }
                    }
                    Field {
                        label: "Course title".to_string(),
                        for_id: "onboarding-course-title".to_string(),
                        helper: "Keep it specific enough for learners to recognize instantly.".to_string(),
                        Input {
                            id: "onboarding-course-title".to_string(),
                            name: "course_title".to_string(),
                            value: title_v.clone(),
                            placeholder: "e.g. Intro to Calculus".to_string(),
                            disabled: busy,
                            on_input: move |v: String| course_title.set(v),
                        }
                    }
                    Field {
                        label: "Description (optional)".to_string(),
                        for_id: "onboarding-course-description".to_string(),
                        helper: "A one-sentence promise is enough for now.".to_string(),
                        textarea {
                            id: "onboarding-course-description",
                            name: "course_description",
                            rows: "4",
                            value: "{desc_v}",
                            placeholder: "What will learners be able to do after this course?",
                            disabled: busy,
                            oninput: move |e| course_desc.set(e.value()),
                        }
                    }
                    div { class: "onboarding-actions",
                        Button {
                            label: "Back".to_string(),
                            variant: ButtonVariant::Ghost,
                            disabled: busy,
                            on_click: move |_| current_step.set(STEP_INVITE.to_string()),
                        }
                        Button {
                            label: "Skip".to_string(),
                            variant: ButtonVariant::Secondary,
                            disabled: busy,
                            on_click: move |_| current_step.set(STEP_DONE.to_string()),
                        }
                        Button {
                            label: if busy { "Creating…".to_string() } else { "Create course".to_string() },
                            variant: ButtonVariant::Primary,
                            loading: busy,
                            disabled: busy,
                            on_click: move |_| {
                                let title = course_title.read().trim().to_string();
                                if title.is_empty() {
                                    toast.push(
                                        ToastLevel::Warning,
                                        "Title required",
                                        "Enter a course title, or Skip this step.",
                                    );
                                    return;
                                }
                                let desc = course_desc.read().trim().to_string();
                                creating.set(true);
                                let mut toast = toast;
                                let mut creating = creating;
                                let mut created_slug = created_slug;
                                let mut current_step = current_step;
                                let cb = EventHandler::new(move |res: Result<String, String>| {
                                    creating.set(false);
                                    match res {
                                        Ok(slug) => {
                                            created_slug.set(Some(slug));
                                            toast.push(
                                                ToastLevel::Success,
                                                "Course created",
                                                "Your first course is ready.",
                                            );
                                            current_step.set(STEP_DONE.to_string());
                                        }
                                        Err(msg) => {
                                            toast.push(ToastLevel::Danger, "Could not create course", msg);
                                        }
                                    }
                                });
                                create_course.call((title, desc, cb));
                            },
                        }
                    }
                }
            }
        }
    };

    // ---- Step 4 body ----
    let go_to_course = props.go_to_course;
    let go_to_dashboard = props.go_to_dashboard;
    let slug_opt = created_slug.read().clone();
    let sent_count = sent_invites.read().len();
    let ws_name = workspace_name.read().trim().to_string();
    let done_body = rsx! {
        Card {
            variant: CardVariant::Premium,
            div { class: "onboarding-step onboarding-step--done",
                div { class: "onboarding-complete-mark", "aria-hidden": "true", "✓" }
                span { class: "onboarding-step-number", "Setup complete" }
                h2 { class: "onboarding-step-title", "Your academy is ready to grow" }
                p { class: "muted", "You have the foundation. Open your course to add content, or head to the dashboard for the full picture." }
                ul { class: "onboarding-summary",
                    if !ws_name.is_empty() {
                        li { "Workspace named " strong { "{ws_name}" } "." }
                    }
                    if sent_count > 0 {
                        li { "Invited " strong { "{sent_count}" } " teacher(s)." }
                    }
                    if let Some(slug) = slug_opt.clone() {
                        li { "Created your first course (" code { "{slug}" } ")." }
                    }
                    if ws_name.is_empty() && sent_count == 0 && slug_opt.is_none() {
                        li { "Kept the starter workspace so you can configure it later." }
                    }
                }
                div { class: "onboarding-actions",
                    if let Some(slug) = slug_opt.clone() {
                        Button {
                            label: "Go to your course".to_string(),
                            variant: ButtonVariant::Primary,
                            on_click: move |_| go_to_course.call(slug.clone()),
                        }
                    }
                    Button {
                        label: "Go to dashboard".to_string(),
                        variant: if slug_opt.is_some() { ButtonVariant::Secondary } else { ButtonVariant::Primary },
                        on_click: move |_| go_to_dashboard.call(()),
                    }
                }
            }
        }
    };

    let step_body: Element = match current.as_str() {
        STEP_WORKSPACE => workspace_body,
        STEP_INVITE => invite_body,
        STEP_COURSE => course_body,
        STEP_DONE => done_body,
        _ => workspace_body,
    };

    rsx! {
        div { class: "onboarding-wizard page-stack motion-page",
            PageHeader {
                kicker: "Academy launch".to_string(),
                title: "Build a workspace people remember".to_string(),
                subtitle: "Four focused steps take you from a blank workspace to your first teachable course.".to_string(),
            }
            div { class: "onboarding-progress-card",
                div { class: "onboarding-progress-copy",
                    span { "Setup progress" }
                    strong { "Step {current_step_number} of 4" }
                }
                span { class: "onboarding-progress-time", "{time_hint}" }
                div {
                    class: "onboarding-progress-track",
                    role: "progressbar",
                    "aria-label": "Onboarding progress",
                    "aria-valuemin": "0",
                    "aria-valuemax": "100",
                    "aria-valuenow": "{progress_percent}",
                    span { style: "width: {progress_percent}%;" }
                }
            }
            div { class: "onboarding-stepper-shell",
                Stepper {
                    steps: steps,
                    current: current.clone(),
                    on_select: move |id: String| {
                        // Allow jumping back to earlier steps via the stepper, but
                        // not skipping ahead past the current step.
                        let cur_idx = STEP_ORDER.iter().position(|s| *s == current).unwrap_or(0);
                        let tgt_idx = STEP_ORDER.iter().position(|s| *s == id).unwrap_or(0);
                        if tgt_idx <= cur_idx {
                            current_step.set(id);
                        }
                    },
                }
            }
            section {
                class: "onboarding-step-stage",
                "aria-live": "polite",
                "aria-label": "Setup step {current_step_number}",
                {step_body}
            }
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    #[test]
    fn picker_value_coerces_invalid_hex() {
        assert_eq!(picker_value("#244f43", DEFAULT_PRIMARY), "#244f43");
        assert_eq!(picker_value("  #aabbcc ", DEFAULT_PRIMARY), "#aabbcc");
        assert_eq!(picker_value("", DEFAULT_PRIMARY), DEFAULT_PRIMARY);
        assert_eq!(picker_value("#abc", DEFAULT_PRIMARY), DEFAULT_PRIMARY);
        assert_eq!(picker_value("blue", DEFAULT_PRIMARY), DEFAULT_PRIMARY);
        assert!(is_valid_brand_color("#244f43"));
        assert!(is_valid_brand_color(" #AABBCC "));
        assert!(!is_valid_brand_color("#abc"));
        assert!(!is_valid_brand_color("green"));
    }

    #[test]
    fn looks_like_email_basic() {
        assert!(looks_like_email("a@b.co"));
        assert!(looks_like_email(" teacher@example.com "));
        assert!(!looks_like_email(""));
        assert!(!looks_like_email("nope"));
        assert!(!looks_like_email("a@b"));
        assert!(!looks_like_email("a @example.com"));
        assert!(!looks_like_email("@x"));
        assert!(!looks_like_email("x@"));
    }

    #[test]
    fn wizard_steps_are_ordered_and_labeled() {
        let s = wizard_steps();
        assert_eq!(s.len(), 4);
        assert_eq!(s[0].id, STEP_WORKSPACE);
        assert_eq!(s[0].label, "Workspace");
        assert_eq!(s[3].id, STEP_DONE);
    }

    #[test]
    fn wizard_renders_first_step_and_stepper() {
        fn app() -> Element {
            use design_system::toast::ToastQueue;
            // Provide a toast queue so `use_toast_sender` resolves in SSR.
            use_context_provider(|| Signal::new(ToastQueue::new()));
            rsx! {
                OnboardingWizard {
                    save_workspace: move |_| {},
                    invite_teacher: move |_| {},
                    create_course: move |_| {},
                    go_to_course: move |_| {},
                    go_to_dashboard: move |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        // Page header.
        assert!(
            html.contains("Build a workspace people remember"),
            "header missing: {html}"
        );
        // Stepper rendered with all four labels.
        assert!(html.contains("ui-stepper"), "stepper missing: {html}");
        assert!(html.contains("Workspace"));
        assert!(html.contains("Invite teachers"));
        assert!(html.contains("First course"));
        // First step (workspace) is the one shown initially.
        assert!(
            html.contains("Make the space feel like yours"),
            "first step body missing: {html}"
        );
        assert!(
            html.contains("onboarding-color-picker"),
            "color picker missing: {html}"
        );
        assert!(html.contains("role=\"progressbar\""));
        assert!(html.contains("Step 1 of 4"));
        assert!(html.contains("Workspace preview"));
    }
}
