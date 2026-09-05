// crates/shell-web/src/routes/platform.rs
//
// Platform SUPER-ADMIN console. Restricted to PLATFORM admins only — workspace admins
// are explicitly NOT allowed here (unlike the per-tenant /admin surfaces, which
// also admit OrgAdmin). The backend `/v1/platform/*` endpoints 403 everyone who
// is not a platform admin; this page gates client-side too so a non-platform
// user sees a Forbidden panel rather than a wall of failed fetches.
//
// Body: a kinetics `DataTable` of every tenant (Name / Slug / Status / Members
// / Plan / Created) with a per-row Suspend/Activate button below it (PATCH
// status active<->suspended, refreshing the list on success), plus a
// "Provision tenant" form (slug / name / admin_email) that POSTs a new tenant
// and refreshes the list. Follows the project-wide four-state UX: loading,
// error, empty, and loaded.
use design_system::kinetics_ui::{DataTable, DataTableColumn, DataTableRow};
use design_system::{
    use_toast_sender, Badge, BadgeTone, Button, ButtonVariant, Input, Modal, PageHeader, Select,
    SelectOption, ToastLevel,
};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::api::TenantSummaryDto;
use features_courses::app_shell::{AppShell, ShellUser};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

/// Build the read-only tenant directory DataTable. Pure so it's SSR-testable
/// without a live ApiContext.
fn tenant_table(tenants: &[TenantSummaryDto]) -> Element {
    let columns = vec![
        DataTableColumn::new("name", "Name"),
        DataTableColumn::new("slug", "Slug"),
        DataTableColumn::new("status", "Status"),
        DataTableColumn::new("members", "Members"),
        DataTableColumn::new("plan", "Plan"),
        DataTableColumn::new("created", "Created"),
    ];
    let rows: Vec<DataTableRow> = tenants
        .iter()
        .map(|t| {
            let plan = t
                .plan_id
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "—".into());
            DataTableRow::new(
                t.id.clone(),
                vec![
                    t.name.clone(),
                    t.slug.clone(),
                    t.status.clone(),
                    t.member_count.to_string(),
                    plan,
                    t.created_at.clone(),
                ],
            )
        })
        .collect();
    rsx! {
        DataTable {
            columns,
            rows,
            caption: format!("{} tenants", tenants.len()),
        }
    }
}

/// The target status for the per-row toggle button, given the current status.
/// Suspended tenants are re-activated; everything else (active / trialing) is
/// suspended. Pure so the button label + PATCH payload stay in lock-step and
/// are unit-testable.
fn toggle_target(status: &str) -> &'static str {
    if status == "suspended" {
        "active"
    } else {
        "suspended"
    }
}

/// The action label shown on the per-row toggle button for a given status.
fn toggle_label(status: &str) -> &'static str {
    if status == "suspended" {
        "Activate"
    } else {
        "Suspend"
    }
}

fn recovery_candidate_options(candidates: &[api::OwnershipCandidateDto]) -> Vec<SelectOption> {
    candidates
        .iter()
        // Initialized organizations expose administrators here. Legacy,
        // uninitialized organizations may have no administrator at all, so
        // the audited recovery API deliberately returns any active non-owner
        // member who can be promoted atomically. Never offer the current
        // owner as their own replacement.
        .filter(|candidate| candidate.role != "org_owner")
        .map(|candidate| {
            let role = match candidate.role.as_str() {
                "org_admin" => "Administrator",
                "teacher" => "Teacher",
                "ta" => "Teaching assistant",
                "student" => "Student",
                "parent" => "Parent",
                _ => "Member",
            };
            let label = candidate
                .display_name
                .clone()
                .filter(|name| !name.trim().is_empty())
                .map(|name| format!("{name} · {} · {role}", candidate.email))
                .unwrap_or_else(|| format!("{} · {role}", candidate.email));
            SelectOption {
                value: candidate.user_id.clone(),
                label,
            }
        })
        .collect()
}

fn looks_like_email(value: &str) -> bool {
    let value = value.trim();
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !value.chars().any(char::is_whitespace)
}

/// The Forbidden panel shown to non-platform-admins (org admins included).
/// Pure so the role gate is SSR-testable without a router/navigator.
fn forbidden_panel() -> Element {
    rsx! {
        div { class: "container",
            h1 { "Forbidden" }
            p { "You don't have permission to view the platform console." }
        }
    }
}

/// The body of the platform console: header + tenant directory (table + per-row
/// status toggles) + provision form. Pulled into its own component so it does
/// NOT call `use_navigator` (it reads only the context-provided ApiContext and
/// toast sender). That keeps it mountable in SSR tests with just the two context
/// providers — the parent `Platform` owns the navigator/redirect/gate. Only
/// rendered once the gate has confirmed the user is a platform admin.
#[component]
fn PlatformConsole() -> Element {
    let api = use_api();
    let toast = use_toast_sender();

    let mut tenants = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::platform_list_tenants(&api).await }
        }
    });

    // Provision form state.
    let mut new_slug = use_signal(String::new);
    let mut new_name = use_signal(String::new);
    let mut new_admin_email = use_signal(String::new);
    let mut provisioning = use_signal(|| false);

    // Break-glass ownership recovery state. Candidate discovery happens only
    // after the operator explicitly opens a tenant's recovery dialog.
    let mut recovery_open = use_signal(|| false);
    let mut recovery_tenant_id = use_signal(String::new);
    let mut recovery_tenant_name = use_signal(String::new);
    let mut recovery_candidates = use_signal(Vec::<api::OwnershipCandidateDto>::new);
    let mut recovery_target = use_signal(String::new);
    let mut recovery_confirmation = use_signal(String::new);
    let mut recovery_loading = use_signal(|| false);
    let mut recovering = use_signal(|| false);
    let mut recovery_error = use_signal(|| None::<String>);

    // Newly provisioned workspace typo recovery. This never creates a second
    // invitation: the backend only replaces the sole pending owner invitation
    // while no owner membership exists.
    let mut replacement_open = use_signal(|| false);
    let mut replacement_tenant_id = use_signal(String::new);
    let mut replacement_tenant_name = use_signal(String::new);
    let mut replacement_email = use_signal(String::new);
    let mut replacement_confirmation = use_signal(String::new);
    let mut replacing_invitation = use_signal(|| false);

    let snap = tenants.read_unchecked();
    let directory: Element = match snap.as_ref() {
        Some(Ok(list)) if list.is_empty() => rsx! {
            p { class: "muted", "No tenants yet. Provision the first one below." }
        },
        Some(Ok(list)) => {
            let table = tenant_table(list);
            let api_for_toggle = api.clone();
            rsx! {
                {table}
                div { class: "platform-tenant-actions",
                    for t in list.iter() {
                        {
                            let id = t.id.clone();
                            let name = t.name.clone();
                            let status = t.status.clone();
                            let target = toggle_target(&status).to_string();
                            let label = toggle_label(&status).to_string();
                            // Suspend is destructive; Activate is the primary recovery.
                            let variant = if status == "suspended" {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Destructive
                            };
                            let api_row = api_for_toggle.clone();
                            let api_recovery = api_for_toggle.clone();
                            let recovery_id = id.clone();
                            let recovery_name = name.clone();
                            let replacement_id = id.clone();
                            let replacement_name = name.clone();
                            rsx! {
                                div { class: "platform-tenant-action-row", key: "{id}",
                                    span { class: "platform-tenant-action-name", "{name}" }
                                    span { class: "platform-tenant-action-status muted", "{status}" }
                                    Button {
                                        label,
                                        variant,
                                        on_click: move |_| {
                                            let api = api_row.clone();
                                            let id = id.clone();
                                            let name = name.clone();
                                            let target = target.clone();
                                            let mut toast = toast;
                                            spawn(async move {
                                                match api::platform_set_tenant_status(&api, &id, &target).await {
                                                    Ok(_) => {
                                                        toast.push(
                                                            ToastLevel::Success,
                                                            "Tenant updated",
                                                            format!("{name} is now {target}."),
                                                        );
                                                        tenants.restart();
                                                    }
                                                    Err(err) => {
                                                        toast.push(
                                                            ToastLevel::Danger,
                                                            "Status change failed",
                                                            format!("{err}"),
                                                        );
                                                    }
                                                }
                                            });
                                        },
                                    }
                                    Button {
                                        label: "Fix owner invite".to_string(),
                                        variant: ButtonVariant::Ghost,
                                        on_click: move |_| {
                                            replacement_tenant_id.set(replacement_id.clone());
                                            replacement_tenant_name.set(replacement_name.clone());
                                            replacement_email.set(String::new());
                                            replacement_confirmation.set(String::new());
                                            replacement_open.set(true);
                                        },
                                    }
                                    Button {
                                        label: "Recover owner".to_string(),
                                        variant: ButtonVariant::Ghost,
                                        on_click: move |_| {
                                            let api = api_recovery.clone();
                                            let tenant_id = recovery_id.clone();
                                            let tenant_name = recovery_name.clone();
                                            recovery_tenant_id.set(tenant_id.clone());
                                            recovery_tenant_name.set(tenant_name);
                                            recovery_candidates.set(Vec::new());
                                            recovery_target.set(String::new());
                                            recovery_confirmation.set(String::new());
                                            recovery_error.set(None);
                                            recovery_loading.set(true);
                                            recovery_open.set(true);
                                            spawn(async move {
                                                match api::platform_list_ownership_candidates(&api, &tenant_id).await {
                                                    Ok(candidates) => {
                                                        let options = recovery_candidate_options(&candidates);
                                                        if let Some(first) = options.first() {
                                                            recovery_target.set(first.value.clone());
                                                        }
                                                        recovery_candidates.set(candidates);
                                                    }
                                                    Err(err) => recovery_error.set(Some(format!("{err}"))),
                                                }
                                                recovery_loading.set(false);
                                            });
                                        },
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Some(Err(e)) => rsx! {
            p { class: "error", "Could not load tenants: {e}" }
        },
        None => rsx! { p { "Loading tenants…" } },
    };
    drop(snap);

    let api_for_provision = api.clone();
    let provision_form: Element = rsx! {
        section { class: "platform-provision-section",
            h2 { "Provision tenant" }
            p { class: "muted",
                "Create a new workspace and designate its first organization owner. The owner joins automatically the first time they sign in with this email."
            }
            form {
                class: "platform-provision-form",
                onsubmit: move |e: FormEvent| {
                    e.prevent_default();
                    let slug = new_slug.read().trim().to_string();
                    let name = new_name.read().trim().to_string();
                    let admin_email = new_admin_email.read().trim().to_string();
                    if slug.is_empty() || name.is_empty() || admin_email.is_empty() {
                        return;
                    }
                    let api = api_for_provision.clone();
                    let mut toast = toast;
                    provisioning.set(true);
                    spawn(async move {
                        match api::platform_create_tenant(&api, &slug, &name, &admin_email).await {
                            Ok(t) => {
                                toast.push(
                                    ToastLevel::Success,
                                    "Tenant provisioned",
                                    format!("{} ({}) is ready.", t.name, t.slug),
                                );
                                new_slug.set(String::new());
                                new_name.set(String::new());
                                new_admin_email.set(String::new());
                                tenants.restart();
                            }
                            Err(err) => {
                                toast.push(
                                    ToastLevel::Danger,
                                    "Provision failed",
                                    format!("{err}"),
                                );
                            }
                        }
                        provisioning.set(false);
                    });
                },
                label { r#for: "platform-tenant-slug", "Slug" }
                Input {
                    id: "platform-tenant-slug".to_string(),
                    name: "tenant_slug".to_string(),
                    value: new_slug.read().clone(),
                    placeholder: "acme".to_string(),
                    autocomplete: "off".to_string(),
                    required: true,
                    disabled: *provisioning.read(),
                    on_input: move |v: String| new_slug.set(v),
                }
                label { r#for: "platform-tenant-name", "Name" }
                Input {
                    id: "platform-tenant-name".to_string(),
                    name: "tenant_name".to_string(),
                    value: new_name.read().clone(),
                    placeholder: "Acme Inc".to_string(),
                    autocomplete: "organization".to_string(),
                    required: true,
                    disabled: *provisioning.read(),
                    on_input: move |v: String| new_name.set(v),
                }
                label { r#for: "platform-admin-email", "Initial owner email" }
                Input {
                    id: "platform-admin-email".to_string(),
                    name: "admin_email".to_string(),
                    value: new_admin_email.read().clone(),
                    input_type: "email".to_string(),
                    placeholder: "admin@acme.com".to_string(),
                    autocomplete: "email".to_string(),
                    required: true,
                    disabled: *provisioning.read(),
                    on_input: move |v: String| new_admin_email.set(v),
                }
                Button {
                    label: if *provisioning.read() { "Provisioning…".to_string() } else { "Provision tenant".to_string() },
                    button_type: "submit".to_string(),
                    disabled: *provisioning.read(),
                    on_click: move |_| {},
                }
            }
        }
    };

    let recovery_options = recovery_candidate_options(&recovery_candidates.read());
    let recovery_phrase = api::OWNERSHIP_RECOVERY_CONFIRMATION;
    let recovery_ready = !recovery_target.read().is_empty()
        && recovery_confirmation.read().trim() == recovery_phrase
        && !*recovery_loading.read();
    let api_for_recovery = api.clone();
    let recovery_modal: Element = rsx! {
        Modal {
            open: *recovery_open.read(),
            title: format!("Recover owner · {}", recovery_tenant_name.read()),
            on_close: move |_| {
                if !*recovering.read() {
                    recovery_open.set(false);
                    recovery_confirmation.set(String::new());
                    recovery_error.set(None);
                }
            },
            div { class: "platform-owner-recovery",
                div { class: "platform-owner-recovery__heading",
                    Badge { label: "Break-glass action".to_string(), tone: BadgeTone::Danger }
                    p {
                        strong { "Use only when the recorded owner cannot exercise ownership." }
                        " This action replaces the current owner, is audit logged, and changes control of billing and administrators."
                    }
                }
                if *recovery_loading.read() {
                    p { class: "muted", role: "status", "Loading eligible administrators…" }
                } else if let Some(error) = recovery_error.read().as_ref() {
                    div { class: "form-error", role: "alert", "Could not load candidates: {error}" }
                } else if recovery_options.is_empty() {
                    div { class: "ownership-section__notice",
                        strong { "No eligible organization administrator" }
                        p { class: "muted",
                            "Recovery requires an active Organization admin. Resolve membership access before attempting this action."
                        }
                    }
                } else {
                    label { r#for: "platform-recovery-owner", "New organization owner" }
                    Select {
                        id: "platform-recovery-owner".to_string(),
                        name: "recovery_owner_user_id".to_string(),
                        value: recovery_target.read().clone(),
                        options: recovery_options,
                        disabled: *recovering.read(),
                        on_change: move |value: String| {
                            recovery_target.set(value);
                            recovery_confirmation.set(String::new());
                        },
                    }
                    p { class: "muted",
                        "Verify the request out of band. Then type "
                        code { "{recovery_phrase}" }
                        " to continue."
                    }
                    label { r#for: "platform-recovery-confirmation", "Confirmation phrase" }
                    Input {
                        id: "platform-recovery-confirmation".to_string(),
                        name: "recovery_confirmation".to_string(),
                        value: recovery_confirmation.read().clone(),
                        placeholder: recovery_phrase.to_string(),
                        autocomplete: "off".to_string(),
                        disabled: *recovering.read(),
                        on_input: move |value: String| recovery_confirmation.set(value),
                    }
                    div { class: "actions platform-owner-recovery__actions",
                        Button {
                            label: "Cancel".to_string(),
                            variant: ButtonVariant::Ghost,
                            disabled: *recovering.read(),
                            on_click: move |_| {
                                recovery_open.set(false);
                                recovery_confirmation.set(String::new());
                            },
                        }
                        Button {
                            label: if *recovering.read() {
                                "Recovering…".to_string()
                            } else {
                                "Recover ownership".to_string()
                            },
                            variant: ButtonVariant::Destructive,
                            disabled: !recovery_ready || *recovering.read(),
                            on_click: move |_| {
                                let tenant_id = recovery_tenant_id.read().clone();
                                let tenant_name = recovery_tenant_name.read().clone();
                                let target = recovery_target.read().clone();
                                let confirmation = recovery_confirmation.read().clone();
                                if tenant_id.is_empty()
                                    || target.is_empty()
                                    || confirmation.trim() != api::OWNERSHIP_RECOVERY_CONFIRMATION
                                {
                                    return;
                                }
                                let api = api_for_recovery.clone();
                                let mut toast = toast;
                                recovering.set(true);
                                spawn(async move {
                                    match api::platform_recover_tenant_owner(
                                        &api,
                                        &tenant_id,
                                        &target,
                                        confirmation.trim(),
                                    )
                                    .await
                                    {
                                        Ok(_) => {
                                            toast.push(
                                                ToastLevel::Success,
                                                "Ownership recovered",
                                                format!("{tenant_name} now has the selected organization owner."),
                                            );
                                            recovery_open.set(false);
                                            recovery_confirmation.set(String::new());
                                            recovery_candidates.set(Vec::new());
                                        }
                                        Err(err) => {
                                            toast.push(
                                                ToastLevel::Danger,
                                                "Recovery failed",
                                                format!("{err}"),
                                            );
                                        }
                                    }
                                    recovering.set(false);
                                });
                            },
                        }
                    }
                }
            }
        }
    };

    let replacement_phrase = api::OWNER_INVITATION_REPLACEMENT_CONFIRMATION;
    let replacement_ready = looks_like_email(&replacement_email.read())
        && replacement_confirmation.read().trim() == replacement_phrase;
    let api_for_replacement = api.clone();
    let replacement_modal: Element = rsx! {
        Modal {
            open: *replacement_open.read(),
            title: format!("Set owner invitation · {}", replacement_tenant_name.read()),
            on_close: move |_| {
                if !*replacing_invitation.read() {
                    replacement_open.set(false);
                    replacement_email.set(String::new());
                    replacement_confirmation.set(String::new());
                }
            },
            div { class: "platform-owner-recovery",
                div { class: "platform-owner-recovery__heading",
                    Badge { label: "Provisioning recovery".to_string(), tone: BadgeTone::Warning }
                    p {
                        strong { "Create or correct the initial owner invitation." }
                        " This works only before anyone accepts ownership. It can initialize an empty legacy organization or replace its pending owner address; every change is audit logged and a fresh email is sent."
                    }
                }
                label { r#for: "platform-replacement-email", "Organization owner email" }
                Input {
                    id: "platform-replacement-email".to_string(),
                    name: "replacement_owner_email".to_string(),
                    value: replacement_email.read().clone(),
                    input_type: "email".to_string(),
                    placeholder: "owner@organization.com".to_string(),
                    autocomplete: "email".to_string(),
                    required: true,
                    disabled: *replacing_invitation.read(),
                    on_input: move |value: String| replacement_email.set(value),
                }
                p { class: "muted",
                    "Verify the corrected address with the organization. Then type "
                    code { "{replacement_phrase}" }
                    " to continue."
                }
                label { r#for: "platform-replacement-confirmation", "Confirmation phrase" }
                Input {
                    id: "platform-replacement-confirmation".to_string(),
                    name: "replacement_confirmation".to_string(),
                    value: replacement_confirmation.read().clone(),
                    placeholder: replacement_phrase.to_string(),
                    autocomplete: "off".to_string(),
                    disabled: *replacing_invitation.read(),
                    on_input: move |value: String| replacement_confirmation.set(value),
                }
                div { class: "actions platform-owner-recovery__actions",
                    Button {
                        label: "Cancel".to_string(),
                        variant: ButtonVariant::Ghost,
                        disabled: *replacing_invitation.read(),
                        on_click: move |_| {
                            replacement_open.set(false);
                            replacement_email.set(String::new());
                            replacement_confirmation.set(String::new());
                        },
                    }
                    Button {
                        label: if *replacing_invitation.read() {
                            "Saving…".to_string()
                        } else {
                            "Save owner invitation".to_string()
                        },
                        variant: ButtonVariant::Destructive,
                        disabled: !replacement_ready || *replacing_invitation.read(),
                        on_click: move |_| {
                            let tenant_id = replacement_tenant_id.read().clone();
                            let tenant_name = replacement_tenant_name.read().clone();
                            let email = replacement_email.read().trim().to_string();
                            let confirmation = replacement_confirmation.read().clone();
                            if tenant_id.is_empty()
                                || !looks_like_email(&email)
                                || confirmation.trim()
                                    != api::OWNER_INVITATION_REPLACEMENT_CONFIRMATION
                            {
                                return;
                            }
                            let api = api_for_replacement.clone();
                            let mut toast = toast;
                            replacing_invitation.set(true);
                            spawn(async move {
                                match api::platform_replace_pending_owner_invitation(
                                    &api,
                                    &tenant_id,
                                    &email,
                                    confirmation.trim(),
                                )
                                .await
                                {
                                    Ok(_) => {
                                        toast.push(
                                            ToastLevel::Success,
                                            "Owner invitation updated",
                                            format!("The owner invitation for {tenant_name} was sent to {email}."),
                                        );
                                        replacement_open.set(false);
                                        replacement_email.set(String::new());
                                        replacement_confirmation.set(String::new());
                                    }
                                    Err(err) => {
                                        toast.push(
                                            ToastLevel::Danger,
                                            "Invitation could not be replaced",
                                            format!("{err}"),
                                        );
                                    }
                                }
                                replacing_invitation.set(false);
                            });
                        },
                    }
                }
            }
        }
    };

    rsx! {
        div { class: "platform-page",
            PageHeader {
                title: "Elementors platform".to_string(),
                kicker: "Platform owner".to_string(),
                subtitle: "Provision organizations, govern workspace lifecycle, and use audited recovery controls across the platform.".to_string(),
            }
            section { class: "platform-tenants-section",
                h2 { "Tenants" }
                { directory }
            }
            { provision_form }
            { recovery_modal }
            { replacement_modal }
        }
    }
}

#[component]
pub fn Platform() -> Element {
    let nav = use_navigator();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => {
            nav.push(Route::Login {});
            return rsx! { p { "Redirecting…" } };
        }
    };

    // Platform-admin ONLY. Org admins are not allowed on this surface (unlike
    // the per-tenant /admin pages, which also admit OrgAdmin).
    if !user.is_platform_admin {
        return forbidden_panel();
    }

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
            PlatformConsole {}
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    fn sample() -> Vec<TenantSummaryDto> {
        vec![
            TenantSummaryDto {
                id: "t-1".into(),
                slug: "acme".into(),
                name: "Acme Inc".into(),
                status: "active".into(),
                created_at: "2026-01-01".into(),
                member_count: 42,
                plan_id: Some("plan_pro".into()),
            },
            TenantSummaryDto {
                id: "t-2".into(),
                slug: "globex".into(),
                name: "Globex".into(),
                status: "suspended".into(),
                created_at: "2026-02-02".into(),
                member_count: 3,
                plan_id: None,
            },
        ]
    }

    #[test]
    fn tenant_table_renders_columns_and_rows() {
        fn app_inner(tenants: Vec<TenantSummaryDto>) -> Element {
            tenant_table(&tenants)
        }
        let mut vdom = VirtualDom::new_with_props(app_inner, sample());
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ui-data-table"), "got: {html}");
        // Column headers.
        assert!(html.contains("Name"));
        assert!(html.contains("Slug"));
        assert!(html.contains("Status"));
        assert!(html.contains("Members"));
        assert!(html.contains("Plan"));
        assert!(html.contains("Created"));
        // Row data.
        assert!(html.contains("Acme Inc"));
        assert!(html.contains("acme"));
        assert!(html.contains("plan_pro"));
        assert!(html.contains("42"));
        // A tenant with no plan renders the placeholder.
        assert!(html.contains("Globex"));
        assert!(html.contains("—"));
    }

    #[test]
    fn toggle_target_and_label_flip_on_status() {
        assert_eq!(toggle_target("active"), "suspended");
        assert_eq!(toggle_target("trialing"), "suspended");
        assert_eq!(toggle_target("suspended"), "active");
        assert_eq!(toggle_label("active"), "Suspend");
        assert_eq!(toggle_label("suspended"), "Activate");
    }

    #[test]
    fn recovery_picker_excludes_owner_and_supports_legacy_members() {
        let candidates = vec![
            api::OwnershipCandidateDto {
                user_id: "owner-1".into(),
                email: "owner@example.test".into(),
                display_name: Some("Current Owner".into()),
                role: "org_owner".into(),
            },
            api::OwnershipCandidateDto {
                user_id: "admin-1".into(),
                email: "admin@example.test".into(),
                display_name: Some("Recovery Admin".into()),
                role: "org_admin".into(),
            },
            api::OwnershipCandidateDto {
                user_id: "teacher-1".into(),
                email: "teacher@example.test".into(),
                display_name: Some("Legacy Teacher".into()),
                role: "teacher".into(),
            },
        ];
        let options = recovery_candidate_options(&candidates);
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].value, "admin-1");
        assert!(options[0].label.contains("Recovery Admin"));
        assert!(options[0].label.contains("admin@example.test"));
        assert!(options[0].label.contains("Administrator"));
        assert_eq!(options[1].value, "teacher-1");
        assert!(options[1].label.contains("Legacy Teacher"));
        assert!(options[1].label.contains("Teacher"));
    }

    #[test]
    fn replacement_email_requires_a_plausible_complete_address() {
        assert!(looks_like_email("owner@example.test"));
        assert!(looks_like_email(" owner+ops@school.example "));
        assert!(!looks_like_email("owner"));
        assert!(!looks_like_email("@example.test"));
        assert!(!looks_like_email("owner@example"));
        assert!(!looks_like_email("owner @example.test"));
    }

    // --- Page-level role gating ---
    // The platform-admin path renders the console body (`PlatformConsole`),
    // which reads only the context-provided ApiContext + toast sender — no
    // navigator — so it mounts cleanly in SSR. The `use_resource` fetch yields
    // the native stub error (api::fetch_json only works on wasm32), so the
    // directory body shows its error state, but the page chrome, provision form,
    // and the "Tenants" section header all render regardless.
    //
    // The Forbidden path is exercised via the pure `forbidden_panel()` helper +
    // the `is_platform_admin` gate decision, avoiding the navigator the full
    // `Platform` component uses (which panics outside a Router in unit tests).

    use crate::contexts::UserContext;
    use design_system::ToastProvider;
    use features_courses::api::ApiContext;

    fn platform_admin() -> UserContext {
        UserContext {
            user_id: "pa-1".into(),
            display_name: "Platform Owner".into(),
            email: "pa@x".into(),
            tenant_id: Some("00000000-0000-0000-0000-000000000010".into()),
            tenant_role: Some(core_types::TenantRole::Teacher),
            is_platform_admin: true,
        }
    }

    fn org_admin() -> UserContext {
        UserContext {
            user_id: "oa-1".into(),
            display_name: "Org Admin".into(),
            email: "oa@x".into(),
            tenant_id: Some("00000000-0000-0000-0000-000000000010".into()),
            tenant_role: Some(core_types::TenantRole::OrgAdmin),
            is_platform_admin: false,
        }
    }

    /// Mount `PlatformConsole` with the context providers it reads (ApiContext +
    /// the toast sender provided by `ToastProvider`).
    fn mount_console() -> String {
        fn app() -> Element {
            use_context_provider(|| {
                Signal::new(ApiContext {
                    base_url: String::new(),
                    id_token: String::new(),
                })
            });
            rsx! {
                ToastProvider {
                    PlatformConsole {}
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        dioxus_ssr::render(&vdom)
    }

    #[test]
    fn platform_page_renders_provision_form_and_table_for_platform_admin() {
        // The console body is what a platform admin sees once past the gate.
        assert!(platform_admin().is_platform_admin);
        let html = mount_console();
        // Page chrome.
        assert!(html.contains("Elementors platform"), "got: {html}");
        assert!(html.contains("Platform owner"), "got: {html}");
        // Provision form fields render.
        assert!(html.contains("platform-provision-form"), "got: {html}");
        assert!(html.contains("Provision tenant"));
        assert!(html.contains("Initial owner email"));
        // The tenants section header is always present.
        assert!(html.contains("platform-tenants-section"));
        assert!(html.contains(">Tenants<"));
        // Not the Forbidden panel.
        assert!(!html.contains(">Forbidden<"), "got: {html}");
    }

    #[test]
    fn platform_page_forbidden_for_org_admin() {
        // Org admins are NOT platform admins → the gate fails and the page shows
        // the Forbidden panel rather than the console body.
        let user = org_admin();
        assert!(
            !user.is_platform_admin,
            "org admin must not be platform admin"
        );

        fn app() -> Element {
            forbidden_panel()
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains(">Forbidden<"), "got: {html}");
        assert!(!html.contains("platform-provision-form"), "got: {html}");
        assert!(!html.contains("Elementors platform"));
    }
}
