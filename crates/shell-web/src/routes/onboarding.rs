// crates/shell-web/src/routes/onboarding.rs
//
// Guided onboarding wizard route (`/onboarding`). Restricted to org-admin /
// platform-admin users — only admins set up a workspace. Anyone unauthenticated
// is bounced to Login; non-admins are redirected to the dashboard.
//
// The route owns the live `ApiContext` + router and wires the wizard's
// `EventHandler` callbacks to the existing `api` client fns:
//   * `save_workspace` → `patch_my_tenant` (name) + `patch_admin_branding`
//     (primary_color) — best-effort, each leg independent.
//   * `invite_teacher` → `create_member_invitation(email, "teacher")`.
//   * `create_course`  → `create_course`.
// Navigation on the final step uses the router (CourseDetail / Dashboard).
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api::{self, CreateCourseBody, PatchTenantBody};
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::onboarding_wizard::{OnboardingWizard, WorkspaceSaveOutcome};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

#[component]
pub fn Onboarding() -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();
    let mut signout = crate::routes::use_signout_action();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => {
            nav.push(Route::Login {});
            return rsx! { p { "Redirecting…" } };
        }
    };

    // Only admins onboard a workspace. Non-admins → dashboard.
    let is_admin = user.can_manage_organization();
    if !is_admin {
        nav.push(Route::Dashboard {});
        return rsx! { p { "Redirecting…" } };
    }

    // --- Step 1: workspace name + primary color (best-effort, two legs). ---
    let save_workspace = {
        let api = api.clone();
        move |(name, primary, cb): (String, String, EventHandler<WorkspaceSaveOutcome>)| {
            let api = api.clone();
            spawn(async move {
                // Leg 1: tenant name.
                let body = PatchTenantBody {
                    name: Some(name.clone()),
                    ..Default::default()
                };
                let name_res = api::patch_my_tenant(&api, &body).await;
                // Leg 2: branding primary color.
                let brand_res =
                    api::patch_admin_branding(&api, None, Some(primary.clone()), None).await;

                let name_ok = name_res.is_ok();
                let branding_ok = brand_res.is_ok();
                let error = match (&name_res, &brand_res) {
                    (Err(e), _) => Some(format!("Workspace name: {e}")),
                    (_, Err(e)) => Some(format!("Brand color: {e}")),
                    _ => None,
                };
                cb.call(WorkspaceSaveOutcome {
                    name_ok,
                    branding_ok,
                    error,
                });
            });
        }
    };

    // --- Step 2: invite a teacher. ---
    let invite_teacher = {
        let api = api.clone();
        move |(email, cb): (String, EventHandler<Result<(), String>>)| {
            let api = api.clone();
            spawn(async move {
                match api::create_member_invitation(&api, &email, "teacher").await {
                    Ok(_) => cb.call(Ok(())),
                    Err(e) => cb.call(Err(format!("{e}"))),
                }
            });
        }
    };

    // --- Step 3: create the first course. ---
    let create_course = {
        let api = api.clone();
        move |(title, description, cb): (String, String, EventHandler<Result<String, String>>)| {
            let api = api.clone();
            spawn(async move {
                let body = CreateCourseBody {
                    title: &title,
                    description: if description.is_empty() {
                        None
                    } else {
                        Some(&description)
                    },
                };
                match api::create_course(&api, &body).await {
                    Ok(c) => cb.call(Ok(c.slug)),
                    Err(e) => cb.call(Err(format!("{e}"))),
                }
            });
        }
    };

    // --- Step 4: navigation. ---
    let go_to_course = {
        move |slug: String| {
            nav.push(Route::CourseDetail { slug });
        }
    };
    let go_to_dashboard = {
        move |_| {
            nav.push(Route::Dashboard {});
        }
    };

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| signout(),
            OnboardingWizard {
                save_workspace: save_workspace,
                invite_teacher: invite_teacher,
                create_course: create_course,
                go_to_course: go_to_course,
                go_to_dashboard: go_to_dashboard,
            }
        }
    }
}
