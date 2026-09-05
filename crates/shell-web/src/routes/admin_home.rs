// crates/shell-web/src/routes/admin_home.rs
//
// Admin landing page. Restricted to organization owners/admins and contextual
// platform operators. Commercial controls remain owner-only.
// Renders a PageHeader plus a `design_system::Card` grid linking out to every
// admin surface (Analytics, Billing, Branding, Members, Audit log, Files,
// Onboarding). Shows a quick member count when memberships resolve.
use design_system::{Card, CardDescription, CardHeader, CardTitle, PageHeader};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

/// One admin destination card: title + one-line description (+ optional meta)
/// inside a clickable Card. Pure/presentational so it's SSR-testable.
fn admin_card(href: &str, title: &str, blurb: &str, meta: Option<String>) -> Element {
    rsx! {
        a { class: "admin-hub-card", href: "{href}",
            Card {
                interactive: true,
                CardHeader {
                    CardTitle { "{title}" }
                    CardDescription { "{blurb}" }
                }
                if let Some(m) = meta {
                    p { class: "admin-hub-card-meta muted", "{m}" }
                }
            }
        }
    }
}

#[component]
pub fn AdminHome() -> Element {
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

    let is_admin = user.can_manage_organization();
    if !is_admin {
        return rsx! {
            div { class: "container",
                h1 { "Forbidden" }
                p { "You don't have permission to view the admin console." }
            }
        };
    }

    // Quick member count — best-effort. Failure to load doesn't break the page;
    // the card simply omits the count.
    let memberships = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::list_tenant_memberships(&api).await }
        }
    });

    let member_meta: Option<String> = {
        let snap = memberships.read_unchecked();
        match snap.as_ref() {
            Some(Ok(m)) => Some(format!("{} members", m.memberships.len())),
            _ => None,
        }
    };

    let billing_card = if user.can_manage_billing() {
        admin_card(
            "/admin/billing",
            "Billing",
            "Review your plan, seats, invoices, and usage limits.",
            Some("Owner access".to_string()),
        )
    } else {
        rsx! {}
    };
    let subtitle = if user.can_own_organization() {
        "You hold organization ownership. Manage the team, commercial plan, security, and workspace operations."
    } else {
        "Manage the team and day-to-day workspace operations. Ownership and billing stay with your organization owner."
    };

    let body = rsx! {
        div { class: "admin-home-page",
            PageHeader {
                title: "Admin console".to_string(),
                kicker: "Workspace administration".to_string(),
                subtitle: subtitle.to_string(),
            }
            div { class: "admin-hub-grid",
                {admin_card(
                    "/admin/analytics",
                    "Analytics",
                    "Tenant-wide engagement, attendance, and grading metrics.",
                    None,
                )}
                {billing_card}
                {admin_card(
                    "/admin/branding",
                    "Branding",
                    "Set your workspace logo and brand colors.",
                    None,
                )}
                {admin_card(
                    "/admin/tenant",
                    "Members & settings",
                    "Edit workspace settings and manage member roles and status.",
                    member_meta,
                )}
                {admin_card(
                    "/admin/integrations",
                    "Integrations",
                    "Manage API keys, webhooks, SSO, and LTI setup.",
                    None,
                )}
                {admin_card(
                    "/admin/audit",
                    "Audit log",
                    "Review significant actions across the tenant.",
                    None,
                )}
                {admin_card(
                    "/admin/files",
                    "Files",
                    "Browse file assets uploaded in this tenant.",
                    None,
                )}
                {admin_card(
                    "/onboarding",
                    "Onboarding",
                    "Walk through the guided setup checklist for this workspace.",
                    None,
                )}
            }
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
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                {
                    use platform_bridge::PlatformBridge;
                    spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                }
                nav.push(Route::Login {});
            },
            { body }
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    #[test]
    fn admin_home_cards_render_card_links() {
        fn app() -> Element {
            rsx! {
                div { class: "admin-hub-grid",
                    {admin_card("/admin/tenant", "Members & settings", "blurb", Some("3 members".to_string()))}
                    {admin_card("/admin/integrations", "Integrations", "blurb", None)}
                    {admin_card("/admin/audit", "Audit log", "blurb", None)}
                    {admin_card("/admin/files", "Files", "blurb", None)}
                    {admin_card("/onboarding", "Onboarding", "blurb", None)}
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("admin-hub-grid"));
        assert!(html.contains("/admin/tenant"));
        assert!(html.contains("/admin/integrations"));
        assert!(html.contains("/admin/audit"));
        assert!(html.contains("/admin/files"));
        assert!(html.contains("/onboarding"));
        // Destinations render as design-system Cards, not bare links.
        assert!(html.contains("ds-card"), "got: {html}");
        assert!(html.contains("ds-card--interactive"), "got: {html}");
        assert!(html.contains("ds-card-title"), "got: {html}");
        assert!(html.contains("Audit log"));
        assert!(html.contains("3 members"));
    }
}
