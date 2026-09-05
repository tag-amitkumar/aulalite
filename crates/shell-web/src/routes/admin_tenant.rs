// crates/shell-web/src/routes/admin_tenant.rs
//
// Admin tenant view: shows the current tenant's settings + memberships.
// Tenant settings are PATCHable via a form. Memberships render as a kinetics
// `DataTable` (read view) with an editor section below where role/status can be
// changed per-member via `Select` controls that call `patch_tenant_membership`
// and refresh the list.
use design_system::kinetics_ui::{
    DataTable, DataTableColumn, DataTableRow, MetricCard, MetricTone,
};
use design_system::{
    use_toast_sender, Badge, BadgeTone, Button, ButtonVariant, Card, Input, Modal, Select,
    SelectOption, ToastLevel,
};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::api::{BillingDto, InvitationDto, MembershipDto, ParentInvitationDto};
use features_courses::app_shell::{AppShell, ShellUser};

use crate::contexts::UserContext;
use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

/// Format an RFC3339 timestamp as `MMM D, YYYY h:mm AM/PM`, falling back to
/// the raw string when parsing fails (same pattern as parent_home::format_ts).
fn format_ts(raw: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(raw) {
        Ok(dt) => dt.format("%b %-d, %Y %-I:%M %p").to_string(),
        Err(_) => raw.to_string(),
    }
}

/// Capitalize-and-space fallback for unknown snake_case enum values.
fn humanize_value(raw: &str) -> String {
    let spaced = raw.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Human label for a membership role enum.
fn role_label(role: &str) -> String {
    match role {
        "org_owner" => "Organization owner".to_string(),
        "org_admin" => "Organization admin".to_string(),
        "teacher" => "Teacher".to_string(),
        "ta" => "TA".to_string(),
        "student" => "Student".to_string(),
        "parent" => "Parent".to_string(),
        other => humanize_value(other),
    }
}

/// Human label for a membership status enum ("active" → "Active").
fn status_label(status: &str) -> String {
    match status {
        "active" => "Active".to_string(),
        "invited" => "Invited".to_string(),
        "suspended" => "Suspended".to_string(),
        other => humanize_value(other),
    }
}

/// Ownership is the highest-trust workspace role and receives the premium
/// treatment; administrators remain visually distinct from ordinary members.
fn role_tone(role: &str) -> BadgeTone {
    match role {
        "org_owner" => BadgeTone::Premium,
        "org_admin" => BadgeTone::Primary,
        _ => BadgeTone::Neutral,
    }
}

/// Chip tone for a status: active is healthy, invited is pending, suspended is
/// a problem state.
fn status_tone(status: &str) -> BadgeTone {
    match status {
        "active" => BadgeTone::Success,
        "invited" => BadgeTone::Warning,
        "suspended" => BadgeTone::Danger,
        _ => BadgeTone::Neutral,
    }
}

/// Ownership is never assigned through a role dropdown. It has a dedicated,
/// explicit transfer ceremony below. Only the owner may grant/revoke the
/// administrator role.
fn role_options(can_assign_admin: bool) -> Vec<SelectOption> {
    let roles: &[&str] = if can_assign_admin {
        &["org_admin", "teacher", "ta", "student", "parent"]
    } else {
        &["teacher", "ta", "student", "parent"]
    };
    roles
        .iter()
        .map(|v| SelectOption {
            value: v.to_string(),
            label: role_label(v),
        })
        .collect()
}

fn status_options(current: &str) -> Vec<SelectOption> {
    let values: &[&str] = if current == "invited" {
        &["invited", "active", "suspended"]
    } else {
        // The backend intentionally forbids active/suspended -> invited. New
        // invitations use the dedicated invitation flow and its audit trail.
        &["active", "suspended"]
    };
    values
        .iter()
        .map(|v| SelectOption {
            value: v.to_string(),
            label: status_label(v),
        })
        .collect()
}

fn can_edit_membership(member: &MembershipDto, viewer_is_owner: bool) -> bool {
    member.role != "org_owner" && (viewer_is_owner || member.role != "org_admin")
}

fn member_role_options(member: &MembershipDto, viewer_is_owner: bool) -> Vec<SelectOption> {
    if can_edit_membership(member, viewer_is_owner) {
        role_options(viewer_is_owner)
    } else {
        vec![SelectOption {
            value: member.role.clone(),
            label: role_label(&member.role),
        }]
    }
}

fn transfer_target_options(mems: &[MembershipDto]) -> Vec<SelectOption> {
    mems.iter()
        .filter(|member| member.role == "org_admin" && member.status == "active")
        .map(|member| SelectOption {
            value: member.user_id.clone(),
            label: member_label(member),
        })
        .collect()
}

/// Build the read-only membership table. Hand-rolled (kinetics table classes)
/// rather than a `DataTable` because role/status render as Badge chips, and
/// DataTable cells are plain strings. Pure so it's SSR-testable.
fn membership_table(mems: &[MembershipDto]) -> Element {
    rsx! {
        table { class: "ui-data-table membership-table",
            thead { class: "ui-data-table-head",
                tr {
                    th { scope: "col", class: "ui-data-table-th", "Member" }
                    th { scope: "col", class: "ui-data-table-th", "Role" }
                    th { scope: "col", class: "ui-data-table-th", "Status" }
                    th { scope: "col", class: "ui-data-table-th", "Joined" }
                }
            }
            tbody { class: "ui-data-table-body",
                for m in mems.iter() {
                    {
                        let member = m
                            .display_name
                            .clone()
                            .filter(|s| !s.is_empty())
                            .or_else(|| m.email.clone())
                            .unwrap_or_else(|| "(no name)".into());
                        rsx! {
                            tr { class: "ui-data-table-row", key: "{m.user_id}",
                                td { class: "ui-data-table-cell", "{member}" }
                                td { class: "ui-data-table-cell",
                                    Badge { label: role_label(&m.role), tone: role_tone(&m.role) }
                                }
                                td { class: "ui-data-table-cell",
                                    Badge { label: status_label(&m.status), tone: status_tone(&m.status) }
                                }
                                td { class: "ui-data-table-cell", "{format_ts(&m.joined_at)}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The label for a member option/row: display_name, else email, else id.
fn member_label(m: &MembershipDto) -> String {
    m.display_name
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| m.email.clone())
        .unwrap_or_else(|| m.user_id.clone())
}

/// Build the Select options for the student picker from memberships whose role
/// is "student". Pure so it's SSR-testable.
fn student_options(mems: &[MembershipDto]) -> Vec<SelectOption> {
    mems.iter()
        .filter(|m| m.role == "student")
        .map(|m| SelectOption {
            value: m.user_id.clone(),
            label: member_label(m),
        })
        .collect()
}

/// Build the pending parent-invitations DataTable. Pure so it's SSR-testable.
/// `student_name_of` maps a student_user_id to a friendly label.
fn invitations_table(
    invites: &[ParentInvitationDto],
    student_name_of: &dyn Fn(&str) -> String,
) -> Element {
    let columns = vec![
        DataTableColumn::new("parent", "Parent email"),
        DataTableColumn::new("student", "Student"),
        DataTableColumn::new("relationship", "Relationship"),
        DataTableColumn::new("status", "Status"),
    ];
    let rows: Vec<DataTableRow> = invites
        .iter()
        .map(|inv| {
            let relationship = inv
                .relationship
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "—".into());
            DataTableRow::new(
                inv.id.clone(),
                vec![
                    inv.parent_email.clone(),
                    student_name_of(&inv.student_user_id),
                    relationship,
                    status_label(&inv.status),
                ],
            )
        })
        .collect();
    rsx! {
        DataTable {
            columns,
            rows,
            caption: format!("{} parent invitations", invites.len()),
        }
    }
}

/// Role options for the member-invite Select. Same set as `role_options` but
/// kept separate so the two surfaces can diverge later (e.g. inviting a parent
/// without a student link is allowed here).
fn invite_role_options(viewer_is_owner: bool) -> Vec<SelectOption> {
    role_options(viewer_is_owner)
}

/// Seat availability derived from the billing usage + plan caps. Pure so the
/// indicator and the "is the form blocked?" decision are both unit-testable.
///
/// `included` is `None` when the tenant has no plan cap (then the form is never
/// blocked and we render just the active count). `blocked` is true only when
/// the tenant chose to BLOCK over cap AND active seats have reached the cap —
/// mirroring the backend `SeatUsage::would_block` rule (the UI uses
/// `active_seats` from billing, which is the best signal available here).
#[derive(Clone, Debug, PartialEq)]
struct SeatState {
    active: i64,
    included: Option<i64>,
    overage_behavior: String,
    blocked: bool,
}

impl SeatState {
    fn from_billing(b: &BillingDto) -> Self {
        let active = b.usage.active_seats;
        // A missing or non-positive cap is "no cap" (plan-less / unlimited).
        let included = b.usage.included_seats.filter(|cap| *cap > 0);
        let overage_behavior = b.overage_behavior.clone();
        let blocked = overage_behavior == "block" && matches!(included, Some(cap) if active >= cap);
        SeatState {
            active,
            included,
            overage_behavior,
            blocked,
        }
    }
}

/// The seat-availability indicator: a MetricCard showing active/included seats
/// plus a tone badge when the tenant is at/over a hard cap. Pure so it's
/// SSR-testable without a live ApiContext.
fn seat_indicator(state: &SeatState) -> Element {
    let value = match state.included {
        Some(cap) => format!("{} / {}", state.active, cap),
        None => format!("{}", state.active),
    };
    let (delta, tone) = match state.included {
        Some(cap) => {
            let remaining = cap - state.active;
            if remaining <= 0 {
                ("0 seats left".to_string(), MetricTone::Danger)
            } else if remaining as f64 <= (cap as f64) * 0.2 {
                (format!("{remaining} seats left"), MetricTone::Warning)
            } else {
                (format!("{remaining} seats left"), MetricTone::Success)
            }
        }
        None => ("no cap".to_string(), MetricTone::Neutral),
    };
    rsx! {
        div { class: "member-invite-seat-indicator",
            MetricCard {
                label: "Seats".to_string(),
                value,
                delta,
                tone,
            }
            if state.blocked {
                Badge { label: "Seat limit reached".to_string(), tone: BadgeTone::Danger }
            }
        }
    }
}

/// Build the pending member-invitations DataTable. Pure so it's SSR-testable.
fn member_invitations_table(invites: &[InvitationDto]) -> Element {
    let columns = vec![
        DataTableColumn::new("email", "Email"),
        DataTableColumn::new("role", "Role"),
        DataTableColumn::new("status", "Status"),
    ];
    let rows: Vec<DataTableRow> = invites
        .iter()
        .map(|inv| {
            DataTableRow::new(
                inv.id.clone(),
                vec![
                    inv.email.clone(),
                    role_label(&inv.role),
                    status_label(&inv.status),
                ],
            )
        })
        .collect();
    rsx! {
        DataTable {
            columns,
            rows,
            caption: format!("{} member invitations", invites.len()),
        }
    }
}

/// Map an `ApiError` from a member-invite create into a friendly message. The
/// 409 `seat_limit_reached` conflict gets a human upgrade prompt; everything
/// else falls through to the default `Display`.
fn invite_error_message(err: &api::ApiError) -> String {
    if let api::ApiError::Status(409, body) = err {
        if body.contains(api::SEAT_LIMIT_REACHED) {
            return "Seat limit reached — upgrade your plan.".to_string();
        }
    }
    format!("{err}")
}

fn recovery_reset_panel(member_name: &str, codes: &[String], acknowledged: bool) -> Element {
    rsx! {
        div { class: "admin-mfa-reset-panel",
            h3 { "Replacement recovery codes" }
            p { class: "muted", "Give these codes to {member_name}. They are shown only now." }
            ul { class: "security-recovery-list",
                for code in codes.iter() {
                    li { class: "security-recovery-code", "{code}" }
                }
            }
            label { class: "security-recovery-ack",
                input { r#type: "checkbox", checked: acknowledged, readonly: true }
                span { "Codes have been saved" }
            }
        }
    }
}

#[component]
pub fn AdminTenant() -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();
    let toast = use_toast_sender();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => {
            nav.push(Route::Login {});
            return rsx! { p { "Redirecting…" } };
        }
    };

    let is_admin = user.can_manage_members();
    if !is_admin {
        return rsx! {
            div { class: "container",
                h1 { "Forbidden" }
                p { "You don't have permission to manage workspace members or settings." }
            }
        };
    }
    let viewer_is_owner = user.can_own_organization();
    let viewer_can_manage_billing = user.can_manage_billing();

    let mut tenant = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::get_my_tenant(&api).await }
        }
    });
    let mut memberships = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::list_tenant_memberships(&api).await }
        }
    });

    // Parent invitations (all students). Refreshed after invite/revoke.
    let mut parent_invites = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::list_parent_invitations(&api, None).await }
        }
    });

    // Member invitations (tenant-wide). Refreshed after invite/revoke.
    let mut member_invites = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::list_member_invitations(&api).await }
        }
    });

    // Billing — read for the seat-availability indicator. Best-effort: if it
    // fails (e.g. no subscription) we fall back to a no-cap display.
    let mut billing = use_resource({
        let api = api.clone();
        let can_read_billing = viewer_can_manage_billing;
        move || {
            let api = api.clone();
            async move {
                if can_read_billing {
                    api::get_billing(&api).await.ok()
                } else {
                    None
                }
            }
        }
    });

    // Invite form state.
    let mut invite_email = use_signal(String::new);
    let mut invite_student = use_signal(String::new);
    let mut invite_relationship = use_signal(String::new);
    let mut inviting = use_signal(|| false);

    // Member-invite form state.
    let mut member_invite_email = use_signal(String::new);
    let mut member_invite_role = use_signal(|| "teacher".to_string());
    let mut member_inviting = use_signal(|| false);

    // Ownership transfer is intentionally a separate ceremony from ordinary
    // role editing: select an existing active administrator, review the
    // consequences, then type an exact confirmation phrase.
    let mut transfer_open = use_signal(|| false);
    let mut transfer_target = use_signal(String::new);
    let mut transfer_confirmation = use_signal(String::new);
    let mut transferring = use_signal(|| false);

    let mut reset_codes_for = use_signal(|| None::<String>);
    let mut reset_codes = use_signal(Vec::<String>::new);
    let mut reset_ack = use_signal(|| false);

    let mut editing_name = use_signal(String::new);
    let mut editing_rec_default = use_signal(|| true);
    let mut editing_retention = use_signal(|| 90i32);
    let mut name_loaded = use_signal(|| false);
    let mut saving_tenant = use_signal(|| false);

    let tenant_snap = tenant.read_unchecked();
    let mem_snap = memberships.read_unchecked();

    // Bootstrap edit signals from the loaded tenant once.
    if !*name_loaded.read() {
        if let Some(Ok(t)) = tenant_snap.as_ref() {
            editing_name.set(t.name.clone());
            editing_rec_default.set(t.recording_default);
            editing_retention.set(t.recording_retention_days);
            name_loaded.set(true);
        }
    }

    // 4-state UX for the membership editor section.
    let members_section: Element = match mem_snap.as_ref() {
        Some(Ok(m)) => {
            if m.memberships.is_empty() {
                rsx! {
                    h2 { "Members (0)" }
                    p { class: "muted", "No members in this tenant yet." }
                }
            } else {
                let mems = m.memberships.clone();
                let table = membership_table(&mems);
                let api_for_member = api.clone();
                rsx! {
                    h2 { "Members ({mems.len()})" }
                    {table}

                    h3 { "Edit roles & status" }
                    div { class: "membership-role-guide", aria_label: "Role capability guide",
                        p { strong { "Organization owner" } " · Billing, ownership, administrators, and all workspace controls" }
                        p { strong { "Organization admin" } " · Members, branding, integrations, courses, and daily operations" }
                        p { strong { "Teacher" } " · Course authoring, live classes, and grading" }
                        p { strong { "TA" } " · Assigned-course assistance and grading" }
                        p { strong { "Student / Parent" } " · Learning or linked-child read-only access" }
                    }
                    div { class: "membership-editor",
                        for m in mems.iter() {
                            {
                                let user_id = m.user_id.clone();
                                let role = m.role.clone();
                                let status = m.status.clone();
                                let name = m
                                    .display_name
                                    .clone()
                                    .filter(|s| !s.is_empty())
                                    .or_else(|| m.email.clone())
                                    .unwrap_or_else(|| "(no name)".into());
                                let api_role = api_for_member.clone();
                                let api_status = api_for_member.clone();
                                let api_reset = api_for_member.clone();
                                let uid_role = user_id.clone();
                                let uid_status = user_id.clone();
                                let uid_reset = user_id.clone();
                                let reset_name = name.clone();
                                let editable = can_edit_membership(m, viewer_is_owner);
                                let access_note = if m.role == "org_owner" {
                                    Some("Owner · use the transfer control below")
                                } else if m.role == "org_admin" && !viewer_is_owner {
                                    Some("Administrator role · owner managed")
                                } else {
                                    None
                                };
                                rsx! {
                                    div { class: "membership-editor-row", key: "{user_id}",
                                        span { class: "membership-editor-name",
                                            "{name}"
                                            if let Some(note) = access_note {
                                                small { "{note}" }
                                            }
                                        }
                                        label { class: "membership-editor-field",
                                            span { class: "membership-editor-field-label", "Role" }
                                            Select {
                                                value: role,
                                                options: member_role_options(m, viewer_is_owner),
                                                disabled: !editable,
                                                on_change: move |new_role: String| {
                                                    let api = api_role.clone();
                                                    let uid = uid_role.clone();
                                                    let mut toast = toast;
                                                    spawn(async move {
                                                        let body = api::PatchMembershipBody {
                                                            role: Some(new_role.clone()),
                                                            ..Default::default()
                                                        };
                                                        match api::patch_tenant_membership(&api, &uid, &body).await {
                                                            Ok(_) => {
                                                                toast.push(
                                                                    ToastLevel::Success,
                                                                    "Role updated",
                                                                    format!("New role: {}", role_label(&new_role)),
                                                                );
                                                                memberships.restart();
                                                            }
                                                            Err(err) => {
                                                                toast.push(
                                                                    ToastLevel::Danger,
                                                                    "Role change failed",
                                                                    format!("{err}"),
                                                                );
                                                            }
                                                        }
                                                    });
                                                },
                                            }
                                        }
                                        label { class: "membership-editor-field",
                                            span { class: "membership-editor-field-label", "Status" }
                                            Select {
                                                value: status.clone(),
                                                options: status_options(&status),
                                                disabled: !editable,
                                                on_change: move |new_status: String| {
                                                    let api = api_status.clone();
                                                    let uid = uid_status.clone();
                                                    let mut toast = toast;
                                                    spawn(async move {
                                                        let body = api::PatchMembershipBody {
                                                            status: Some(new_status.clone()),
                                                            ..Default::default()
                                                        };
                                                        match api::patch_tenant_membership(&api, &uid, &body).await {
                                                            Ok(_) => {
                                                                toast.push(
                                                                    ToastLevel::Success,
                                                                    "Status updated",
                                                                    format!("New status: {}", status_label(&new_status)),
                                                                );
                                                                memberships.restart();
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
                                        }
                                        Button {
                                            label: "Reset recovery codes".to_string(),
                                            variant: ButtonVariant::Secondary,
                                            disabled: !user.can_manage_platform(),
                                            on_click: {
                                                let api = api_reset.clone();
                                                let uid = uid_reset.clone();
                                                let member_name = reset_name.clone();
                                                move |_| {
                                                    let api = api.clone();
                                                    let uid = uid.clone();
                                                    let member_name = member_name.clone();
                                                    let mut toast = toast;
                                                    spawn(async move {
                                                        match api::admin_reset_mfa_recovery_codes(&api, &uid).await {
                                                            Ok(resp) => {
                                                                reset_codes_for.set(Some(member_name));
                                                                reset_codes.set(resp.recovery_codes);
                                                                reset_ack.set(false);
                                                                toast.push(
                                                                    ToastLevel::Success,
                                                                    "Recovery codes reset",
                                                                    "Share the replacement codes securely.",
                                                                );
                                                            }
                                                            Err(err) => {
                                                                toast.push(
                                                                    ToastLevel::Danger,
                                                                    "Reset failed",
                                                                    format!("{err}"),
                                                                );
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
                }
            }
        }
        Some(Err(e)) => rsx! {
            h2 { "Members" }
            p { class: "error", "Could not load memberships: {e}" }
        },
        None => rsx! {
            h2 { "Members" }
            p { "Loading members…" }
        },
    };

    let reset_panel: Element = if reset_codes.read().is_empty() {
        rsx! {}
    } else {
        let member_name = reset_codes_for
            .read()
            .clone()
            .unwrap_or_else(|| "this member".to_string());
        let codes = reset_codes.read().clone();
        let acknowledged = *reset_ack.read();
        rsx! {
            div { class: "admin-mfa-reset-panel-wrap",
                { recovery_reset_panel(&member_name, &codes, acknowledged) }
                label { class: "security-recovery-ack",
                    input {
                        r#type: "checkbox",
                        checked: acknowledged,
                        onchange: move |event| reset_ack.set(event.checked()),
                    }
                    span { "I saved these replacement codes" }
                }
                Button {
                    label: "Close".to_string(),
                    variant: ButtonVariant::Primary,
                    disabled: !acknowledged,
                    on_click: move |_| {
                        reset_codes.set(Vec::new());
                        reset_codes_for.set(None);
                        reset_ack.set(false);
                    },
                }
            }
        }
    };

    // --- Parent invitations section ---
    // Student picker is sourced from the loaded memberships (role == student).
    let all_members: Vec<MembershipDto> = match mem_snap.as_ref() {
        Some(Ok(m)) => m.memberships.clone(),
        _ => Vec::new(),
    };

    let transfer_targets = transfer_target_options(&all_members);
    let transfer_target_valid = transfer_targets
        .iter()
        .any(|option| option.value == *transfer_target.read());
    if !transfer_target_valid {
        transfer_target.set(
            transfer_targets
                .first()
                .map(|option| option.value.clone())
                .unwrap_or_default(),
        );
    }
    let selected_transfer_name = transfer_targets
        .iter()
        .find(|option| option.value == *transfer_target.read())
        .map(|option| option.label.clone())
        .unwrap_or_else(|| "the selected administrator".to_string());
    let transfer_ready = !transfer_target.read().is_empty()
        && transfer_confirmation.read().trim() == api::OWNERSHIP_TRANSFER_CONFIRMATION;
    let transfer_phrase = api::OWNERSHIP_TRANSFER_CONFIRMATION;
    let ownership_section: Element = if viewer_is_owner {
        let api_for_transfer = api.clone();
        let transfer_actor = user.clone();
        rsx! {
            section { class: "ownership-section", aria_label: "Organization ownership",
                Card {
                    div { class: "ownership-section__header",
                        div {
                            p { class: "eyebrow", "Highest-trust role" }
                            h2 { "Organization ownership" }
                        }
                        Badge { label: "You are the owner".to_string(), tone: BadgeTone::Premium }
                    }
                    p { class: "muted",
                        "The owner controls billing, appoints administrators, and is the final authority for this workspace. There is exactly one owner."
                    }
                    if transfer_targets.is_empty() {
                        div { class: "ownership-section__notice",
                            strong { "No eligible administrator yet" }
                            p { class: "muted",
                                "Promote a trusted active member to Organization admin first. Only an active administrator can receive ownership."
                            }
                        }
                    } else {
                        Button {
                            label: "Transfer ownership".to_string(),
                            variant: ButtonVariant::Secondary,
                            on_click: move |_| {
                                transfer_confirmation.set(String::new());
                                transfer_open.set(true);
                            },
                        }
                    }
                }
            }
            Modal {
                open: *transfer_open.read(),
                title: "Transfer organization ownership".to_string(),
                on_close: move |_| {
                    if !*transferring.read() {
                        transfer_open.set(false);
                        transfer_confirmation.set(String::new());
                    }
                },
                div { class: "ownership-transfer-dialog",
                    div { class: "ownership-transfer-dialog__warning", role: "alert",
                        strong { "This changes who controls the organization." }
                        p {
                            "The new owner will control billing and administrators. You will immediately become an Organization admin."
                        }
                    }
                    label { r#for: "ownership-transfer-target", "New owner" }
                    Select {
                        id: "ownership-transfer-target".to_string(),
                        name: "new_owner_user_id".to_string(),
                        value: transfer_target.read().clone(),
                        options: transfer_targets.clone(),
                        disabled: *transferring.read(),
                        on_change: move |value: String| {
                            transfer_target.set(value);
                            transfer_confirmation.set(String::new());
                        },
                    }
                    p { class: "muted",
                        "To transfer ownership to {selected_transfer_name}, type "
                        code { "{transfer_phrase}" }
                        "."
                    }
                    label { r#for: "ownership-transfer-confirmation", "Confirmation phrase" }
                    Input {
                        id: "ownership-transfer-confirmation".to_string(),
                        name: "ownership_confirmation".to_string(),
                        value: transfer_confirmation.read().clone(),
                        placeholder: api::OWNERSHIP_TRANSFER_CONFIRMATION.to_string(),
                        autocomplete: "off".to_string(),
                        disabled: *transferring.read(),
                        on_input: move |value: String| transfer_confirmation.set(value),
                    }
                    div { class: "actions ownership-transfer-dialog__actions",
                        Button {
                            label: "Cancel".to_string(),
                            variant: ButtonVariant::Ghost,
                            disabled: *transferring.read(),
                            on_click: move |_| {
                                transfer_open.set(false);
                                transfer_confirmation.set(String::new());
                            },
                        }
                        Button {
                            label: if *transferring.read() {
                                "Transferring…".to_string()
                            } else {
                                "Transfer ownership".to_string()
                            },
                            variant: ButtonVariant::Destructive,
                            disabled: !transfer_ready || *transferring.read(),
                            on_click: move |_| {
                                let new_owner_user_id = transfer_target.read().clone();
                                let confirmation = transfer_confirmation.read().clone();
                                if new_owner_user_id.is_empty()
                                    || confirmation.trim() != api::OWNERSHIP_TRANSFER_CONFIRMATION
                                {
                                    return;
                                }
                                let api = api_for_transfer.clone();
                                let mut refreshed_user_ctx = user_ctx;
                                let mut demoted_user = transfer_actor.clone();
                                demoted_user.tenant_role = Some(core_types::TenantRole::OrgAdmin);
                                let nav_after_transfer = nav;
                                let mut toast = toast;
                                transferring.set(true);
                                spawn(async move {
                                    match api::transfer_tenant_ownership(
                                        &api,
                                        &new_owner_user_id,
                                        confirmation.trim(),
                                    )
                                    .await
                                    {
                                        Ok(_) => {
                                            // Apply the guaranteed postcondition immediately on
                                            // every client, then replace it with the authoritative
                                            // `/v1/me` payload when available. Native shells do not
                                            // have a browser reload to repair stale permissions.
                                            refreshed_user_ctx.set(Some(demoted_user));
                                            if let Ok(dto) = api::get_me(&api).await {
                                                refreshed_user_ctx.set(Some(UserContext::from_dto(dto)));
                                            }
                                            toast.push(
                                                ToastLevel::Success,
                                                "Ownership transferred",
                                                "You are now an Organization admin. Reloading your workspace permissions…",
                                            );
                                            transfer_open.set(false);
                                            transfer_confirmation.set(String::new());
                                            memberships.restart();
                                            nav_after_transfer.push(Route::AdminHome {});
                                            #[cfg(target_arch = "wasm32")]
                                            if let Some(window) = web_sys::window() {
                                                let _ = window.location().reload();
                                            }
                                        }
                                        Err(err) => {
                                            toast.push(
                                                ToastLevel::Danger,
                                                "Transfer failed",
                                                format!("{err}"),
                                            );
                                        }
                                    }
                                    transferring.set(false);
                                });
                            },
                        }
                    }
                }
            }
        }
    } else {
        rsx! {}
    };

    let student_opts = student_options(&all_members);
    // Seed the picker with the first student once options are available.
    if invite_student.read().is_empty() {
        if let Some(first) = student_opts.first() {
            invite_student.set(first.value.clone());
        }
    }

    // Map student id -> friendly label for the invitations table.
    let name_lookup: std::collections::HashMap<String, String> = all_members
        .iter()
        .map(|m| (m.user_id.clone(), member_label(m)))
        .collect();

    let invites_snap = parent_invites.read_unchecked();
    let invites_list: Element = match invites_snap.as_ref() {
        Some(Ok(invs)) if invs.is_empty() => rsx! {
            p { class: "muted", "No parent invitations yet." }
        },
        Some(Ok(invs)) => {
            let table = {
                let lookup = name_lookup.clone();
                invitations_table(invs, &move |id: &str| {
                    lookup.get(id).cloned().unwrap_or_else(|| id.to_string())
                })
            };
            let api_for_revoke = api.clone();
            rsx! {
                {table}
                div { class: "parent-invite-revoke-list",
                    for inv in invs.iter() {
                        {
                            let id = inv.id.clone();
                            let parent_email = inv.parent_email.clone();
                            let api_rev = api_for_revoke.clone();
                            rsx! {
                                div { class: "parent-invite-revoke-row", key: "{id}",
                                    span { class: "parent-invite-revoke-email", "{parent_email}" }
                                    Button {
                                        label: "Revoke".to_string(),
                                        variant: ButtonVariant::Destructive,
                                        on_click: move |_| {
                                            let api = api_rev.clone();
                                            let id = id.clone();
                                            let mut toast = toast;
                                            spawn(async move {
                                                match api::revoke_parent_invitation(&api, &id).await {
                                                    Ok(()) => {
                                                        toast.push(
                                                            ToastLevel::Success,
                                                            "Invitation revoked",
                                                            "The parent invitation was removed.",
                                                        );
                                                        parent_invites.restart();
                                                    }
                                                    Err(err) => {
                                                        toast.push(
                                                            ToastLevel::Danger,
                                                            "Revoke failed",
                                                            format!("{err}"),
                                                        );
                                                    }
                                                }
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
            p { class: "error", "Could not load parent invitations: {e}" }
        },
        None => rsx! { p { "Loading invitations…" } },
    };
    drop(invites_snap);

    let has_students = !student_opts.is_empty();
    let api_for_invite = api.clone();
    let parent_invites_section: Element = rsx! {
        section { class: "parent-invitations-section",
            h2 { "Parent invitations" }
            p { class: "muted",
                "Link a parent or guardian to a student so they can view that child's grades, attendance, and schedule."
            }
            if has_students {
                form {
                    class: "parent-invite-form",
                    onsubmit: move |e: FormEvent| {
                        e.prevent_default();
                        let email = invite_email.read().trim().to_string();
                        let student = invite_student.read().clone();
                        let relationship = invite_relationship.read().trim().to_string();
                        if email.is_empty() || student.is_empty() {
                            return;
                        }
                        let api = api_for_invite.clone();
                        let mut toast = toast;
                        inviting.set(true);
                        spawn(async move {
                            let rel = if relationship.is_empty() { None } else { Some(relationship.as_str()) };
                            match api::create_parent_invitation(&api, &email, &student, rel).await {
                                Ok(_) => {
                                    toast.push(
                                        ToastLevel::Success,
                                        "Parent invited",
                                        format!("Invitation sent to {email}."),
                                    );
                                    invite_email.set(String::new());
                                    invite_relationship.set(String::new());
                                    parent_invites.restart();
                                }
                                Err(err) => {
                                    toast.push(
                                        ToastLevel::Danger,
                                        "Invite failed",
                                        format!("{err}"),
                                    );
                                }
                            }
                            inviting.set(false);
                        });
                    },
                    label { r#for: "parent-invite-email", "Parent email" }
                    Input {
                        id: "parent-invite-email".to_string(),
                        name: "parent_email".to_string(),
                        value: invite_email.read().clone(),
                        input_type: "email".to_string(),
                        placeholder: "parent@example.com".to_string(),
                        autocomplete: "email".to_string(),
                        required: true,
                        disabled: *inviting.read(),
                        on_input: move |v: String| invite_email.set(v),
                    }
                    label { r#for: "parent-invite-student", "Student" }
                    Select {
                        id: "parent-invite-student".to_string(),
                        name: "student_id".to_string(),
                        value: invite_student.read().clone(),
                        options: student_opts.clone(),
                        disabled: *inviting.read(),
                        on_change: move |v: String| invite_student.set(v),
                    }
                    label { r#for: "parent-invite-relationship", "Relationship (optional)" }
                    Input {
                        id: "parent-invite-relationship".to_string(),
                        name: "relationship".to_string(),
                        value: invite_relationship.read().clone(),
                        placeholder: "e.g. mother, guardian".to_string(),
                        disabled: *inviting.read(),
                        on_input: move |v: String| invite_relationship.set(v),
                    }
                    Button {
                        label: if *inviting.read() { "Inviting…".to_string() } else { "Invite parent".to_string() },
                        button_type: "submit".to_string(),
                        disabled: *inviting.read(),
                        on_click: move |_| {},
                    }
                }
            } else {
                p { class: "muted", "Add a student to this tenant first to invite their parent." }
            }

            h3 { "Pending invitations" }
            {invites_list}
        }
    };

    // --- Member invitations section ---
    // Seat availability is read from billing; an error / no-cap plan falls back
    // to a never-blocked "no cap" indicator showing just the active count.
    let billing_snap = billing.read_unchecked();
    let seat_state: SeatState = match billing_snap.as_ref() {
        Some(Some(b)) => SeatState::from_billing(b),
        _ => SeatState {
            active: 0,
            included: None,
            overage_behavior: "metered".to_string(),
            blocked: false,
        },
    };
    drop(billing_snap);
    let seat_blocked = viewer_can_manage_billing && seat_state.blocked;
    let seat_indicator_el = if viewer_can_manage_billing {
        seat_indicator(&seat_state)
    } else {
        rsx! {
            p { class: "member-invite-help muted",
                "Your organization owner manages seat limits and billing. Invitations are still validated against the active plan when sent."
            }
        }
    };

    let member_invites_snap = member_invites.read_unchecked();
    let member_invites_list: Element = match member_invites_snap.as_ref() {
        Some(Ok(invs)) if invs.is_empty() => rsx! {
            p { class: "muted", "No member invitations yet." }
        },
        Some(Ok(invs)) => {
            let table = member_invitations_table(invs);
            let api_for_revoke = api.clone();
            rsx! {
                {table}
                div { class: "member-invite-revoke-list",
                    for inv in invs.iter() {
                        {
                            let id = inv.id.clone();
                            let email = inv.email.clone();
                            let can_revoke = viewer_is_owner || inv.role != "org_admin";
                            let api_rev = api_for_revoke.clone();
                            rsx! {
                                div { class: "member-invite-revoke-row", key: "{id}",
                                    span { class: "member-invite-revoke-email", "{email}" }
                                    Button {
                                        label: "Revoke".to_string(),
                                        variant: ButtonVariant::Destructive,
                                        disabled: !can_revoke,
                                        on_click: move |_| {
                                            let api = api_rev.clone();
                                            let id = id.clone();
                                            let mut toast = toast;
                                            spawn(async move {
                                                match api::revoke_member_invitation(&api, &id).await {
                                                    Ok(()) => {
                                                        toast.push(
                                                            ToastLevel::Success,
                                                            "Invitation revoked",
                                                            "The member invitation was removed.",
                                                        );
                                                        member_invites.restart();
                                                    }
                                                    Err(err) => {
                                                        toast.push(
                                                            ToastLevel::Danger,
                                                            "Revoke failed",
                                                            format!("{err}"),
                                                        );
                                                    }
                                                }
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
            p { class: "error", "Could not load member invitations: {e}" }
        },
        None => rsx! { p { "Loading invitations…" } },
    };
    drop(member_invites_snap);

    let api_for_member_invite = api.clone();
    let member_role_opts = invite_role_options(viewer_is_owner);
    let member_invites_section: Element = rsx! {
        section { class: "member-invitations-section",
            h2 { "Invite members" }
            p { class: "muted",
                if viewer_is_owner {
                    "Invite an organization admin, teacher, TA, student, or parent. They join automatically the first time they sign in with the invited email."
                } else {
                    "Invite a teacher, TA, student, or parent. Organization administrators are appointed by the owner."
                }
            }
            {seat_indicator_el}
            if seat_blocked {
                div { class: "member-invite-seat-warning",
                    p { class: "error",
                        "Seat limit reached — upgrade to add more."
                    }
                    Button {
                        label: "Upgrade plan".to_string(),
                        variant: ButtonVariant::Premium,
                        on_click: move |_| {
                            nav.push(Route::AdminBilling {});
                        },
                    }
                }
            }
            form {
                class: "member-invite-form",
                onsubmit: move |e: FormEvent| {
                    e.prevent_default();
                    if seat_blocked {
                        return;
                    }
                    let email = member_invite_email.read().trim().to_string();
                    let role = member_invite_role.read().clone();
                    if email.is_empty() {
                        return;
                    }
                    let api = api_for_member_invite.clone();
                    let mut toast = toast;
                    member_inviting.set(true);
                    spawn(async move {
                        match api::create_member_invitation(&api, &email, &role).await {
                            Ok(_) => {
                                toast.push(
                                    ToastLevel::Success,
                                    "Member invited",
                                    format!("Invitation sent to {email}."),
                                );
                                member_invite_email.set(String::new());
                                member_invites.restart();
                                billing.restart();
                            }
                            Err(err) => {
                                toast.push(
                                    ToastLevel::Danger,
                                    "Invite failed",
                                    invite_error_message(&err),
                                );
                            }
                        }
                        member_inviting.set(false);
                    });
                },
                label { r#for: "member-invite-email", "Email" }
                Input {
                    id: "member-invite-email".to_string(),
                    name: "member_email".to_string(),
                    value: member_invite_email.read().clone(),
                    input_type: "email".to_string(),
                    placeholder: "member@example.com".to_string(),
                    autocomplete: "email".to_string(),
                    required: true,
                    disabled: *member_inviting.read() || seat_blocked,
                    on_input: move |v: String| member_invite_email.set(v),
                }
                label { r#for: "member-invite-role", "Role" }
                Select {
                    id: "member-invite-role".to_string(),
                    name: "member_role".to_string(),
                    value: member_invite_role.read().clone(),
                    options: member_role_opts,
                    disabled: *member_inviting.read() || seat_blocked,
                    on_change: move |v: String| member_invite_role.set(v),
                }
                Button {
                    label: if *member_inviting.read() { "Sending…".to_string() } else { "Send invite".to_string() },
                    button_type: "submit".to_string(),
                    disabled: *member_inviting.read() || seat_blocked,
                    on_click: move |_| {},
                }
                if seat_blocked {
                    p { class: "member-invite-help muted",
                        "Seat limit reached — upgrade to add more."
                    }
                }
            }

            h3 { "Pending member invitations" }
            {member_invites_list}
        }
    };

    let body: Element = match tenant_snap.as_ref() {
        Some(Ok(t)) => {
            let api_for_save = api.clone();
            let t_slug = t.slug.clone();
            let t_status = t.status.clone();
            let t_created = format_ts(&t.created_at);
            rsx! {
                div { class: "admin-tenant-page",
                    h1 { "Members & workspace settings" }
                    form {
                        class: "tenant-settings-form",
                        onsubmit: move |e: FormEvent| {
                            e.prevent_default();
                            let api = api_for_save.clone();
                            let mut toast = toast;
                            let body = api::PatchTenantBody {
                                name: Some(editing_name.read().clone()),
                                recording_default: Some(*editing_rec_default.read()),
                                recording_retention_days: Some(*editing_retention.read()),
                            };
                            saving_tenant.set(true);
                            spawn(async move {
                                match api::patch_my_tenant(&api, &body).await {
                                    Ok(_) => {
                                        toast.push(
                                            ToastLevel::Success,
                                            "Tenant updated",
                                            "Settings saved.",
                                        );
                                        tenant.restart();
                                    }
                                    Err(err) => {
                                        toast.push(
                                            ToastLevel::Danger,
                                            "Save failed",
                                            format!("{err}"),
                                        );
                                    }
                                }
                                saving_tenant.set(false);
                            });
                        },
                        label { "Display name" }
                        input {
                            value: "{editing_name}",
                            disabled: *saving_tenant.read(),
                            oninput: move |e| editing_name.set(e.value()),
                        }
                        label {
                            input {
                                r#type: "checkbox",
                                checked: *editing_rec_default.read(),
                                disabled: *saving_tenant.read(),
                                onchange: move |e| {
                                    editing_rec_default.set(e.value() == "true");
                                },
                            }
                            " Default new sessions to record"
                        }
                        label { "Recording retention (days, 1-3650)" }
                        input {
                            r#type: "number",
                            min: "1",
                            max: "3650",
                            value: "{editing_retention}",
                            disabled: *saving_tenant.read(),
                            oninput: move |e| {
                                if let Ok(n) = e.value().parse::<i32>() {
                                    editing_retention.set(n);
                                }
                            },
                        }
                        button {
                            r#type: "submit",
                            disabled: *saving_tenant.read(),
                            if *saving_tenant.read() { "Saving…" } else { "Save" }
                        }
                    }

                    h2 { "Read-only meta" }
                    dl { class: "tenant-meta",
                        dt { "Slug" }
                        dd { "{t_slug}" }
                        dt { "Status" }
                        dd { "{t_status}" }
                        dt { "Created" }
                        dd { "{t_created}" }
                    }

                    {members_section}

                    {ownership_section}

                    {reset_panel}

                    {member_invites_section}

                    {parent_invites_section}
                }
            }
        }
        Some(Err(e)) => rsx! { p { class: "error", "Could not load tenant: {e}" } },
        None => rsx! { p { "Loading tenant…" } },
    };
    drop(tenant_snap);
    drop(mem_snap);

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

    fn sample() -> Vec<MembershipDto> {
        vec![MembershipDto {
            user_id: "u-1".into(),
            email: Some("a@x".into()),
            display_name: Some("Ada".into()),
            role: "teacher".into(),
            status: "active".into(),
            joined_at: "2026-01-01T09:00:00Z".into(),
        }]
    }

    #[test]
    fn membership_table_renders_chips_and_formatted_date() {
        let mems = sample();
        fn app_inner(mems: Vec<MembershipDto>) -> Element {
            membership_table(&mems)
        }
        let mut vdom = VirtualDom::new_with_props(app_inner, mems);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ui-data-table"), "got: {html}");
        assert!(html.contains("Ada"));
        // Role/status render as human labels inside Badge chips.
        assert!(html.contains("Teacher"), "got: {html}");
        assert!(html.contains("Active"), "got: {html}");
        assert!(html.contains("badge-neutral"), "role chip missing: {html}");
        assert!(
            html.contains("badge-success"),
            "status chip missing: {html}"
        );
        // Joined renders via the shared chrono pattern.
        assert!(html.contains("Jan 1, 2026 9:00 AM"), "got: {html}");
        assert!(html.contains("Member"));
    }

    #[test]
    fn role_and_status_labels_humanize_known_and_unknown_values() {
        assert_eq!(role_label("org_owner"), "Organization owner");
        assert_eq!(role_label("org_admin"), "Organization admin");
        assert_eq!(role_label("teacher"), "Teacher");
        assert_eq!(role_label("ta"), "TA");
        assert_eq!(role_label("student"), "Student");
        assert_eq!(role_label("parent"), "Parent");
        assert_eq!(role_label("site_owner"), "Site owner");
        assert_eq!(status_label("active"), "Active");
        assert_eq!(status_label("invited"), "Invited");
        assert_eq!(status_label("suspended"), "Suspended");
        assert_eq!(status_label("pending"), "Pending");
    }

    #[test]
    fn ownership_is_never_a_dropdown_role_and_admin_assignment_is_owner_only() {
        let owner_options = role_options(true);
        assert!(owner_options
            .iter()
            .any(|option| option.value == "org_admin"));
        assert!(!owner_options
            .iter()
            .any(|option| option.value == "org_owner"));

        let admin_options = role_options(false);
        assert!(!admin_options
            .iter()
            .any(|option| option.value == "org_admin"));
        assert!(!admin_options
            .iter()
            .any(|option| option.value == "org_owner"));
    }

    #[test]
    fn transfer_targets_only_active_organization_admins() {
        let member = |id: &str, role: &str, status: &str| MembershipDto {
            user_id: id.into(),
            email: Some(format!("{id}@x.test")),
            display_name: Some(id.into()),
            role: role.into(),
            status: status.into(),
            joined_at: "2026-01-01T09:00:00Z".into(),
        };
        let members = vec![
            member("owner", "org_owner", "active"),
            member("ready", "org_admin", "active"),
            member("suspended", "org_admin", "suspended"),
            member("teacher", "teacher", "active"),
        ];
        let options = transfer_target_options(&members);
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].value, "ready");
    }

    #[test]
    fn administrators_cannot_edit_privileged_members() {
        let member = |role: &str| MembershipDto {
            user_id: role.into(),
            email: None,
            display_name: None,
            role: role.into(),
            status: "active".into(),
            joined_at: "2026-01-01T09:00:00Z".into(),
        };
        assert!(!can_edit_membership(&member("org_owner"), true));
        assert!(!can_edit_membership(&member("org_owner"), false));
        let locked_owner_options = member_role_options(&member("org_owner"), true);
        assert_eq!(locked_owner_options.len(), 1);
        assert_eq!(locked_owner_options[0].value, "org_owner");
        assert!(can_edit_membership(&member("org_admin"), true));
        assert!(!can_edit_membership(&member("org_admin"), false));
        assert!(can_edit_membership(&member("teacher"), false));
    }

    #[test]
    fn active_members_cannot_be_moved_back_to_invited() {
        let active = status_options("active");
        assert!(!active.iter().any(|option| option.value == "invited"));
        assert!(status_options("invited")
            .iter()
            .any(|option| option.value == "invited"));
    }

    #[test]
    fn student_options_filters_to_students_only() {
        let mems = vec![
            MembershipDto {
                user_id: "t-1".into(),
                email: Some("t@x".into()),
                display_name: Some("Teach".into()),
                role: "teacher".into(),
                status: "active".into(),
                joined_at: "2026-01-01".into(),
            },
            MembershipDto {
                user_id: "s-1".into(),
                email: Some("s@x".into()),
                display_name: Some("Stu".into()),
                role: "student".into(),
                status: "active".into(),
                joined_at: "2026-01-01".into(),
            },
        ];
        let opts = student_options(&mems);
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].value, "s-1");
        assert_eq!(opts[0].label, "Stu");
    }

    #[test]
    fn invitations_table_renders_rows_with_student_name() {
        let invites = vec![ParentInvitationDto {
            id: "inv-1".into(),
            parent_email: "parent@x".into(),
            student_user_id: "s-1".into(),
            relationship: Some("mother".into()),
            status: "pending".into(),
            created_at: "2026-01-01".into(),
        }];
        fn app_inner(invites: Vec<ParentInvitationDto>) -> Element {
            invitations_table(&invites, &|id: &str| {
                if id == "s-1" {
                    "Stu".to_string()
                } else {
                    id.to_string()
                }
            })
        }
        let mut vdom = VirtualDom::new_with_props(app_inner, invites);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ui-data-table"), "got: {html}");
        assert!(html.contains("parent@x"));
        assert!(html.contains("Stu"));
        assert!(html.contains("mother"));
        assert!(html.contains("Pending"));
        assert!(html.contains("Parent email"));
    }

    use features_courses::api::{PlanDto, SubscriptionDto, UsageDto};

    fn billing_with(active_seats: i64, included_seats: i64, overage: &str) -> BillingDto {
        let plan = PlanDto {
            id: "plan_pro".into(),
            name: "Pro".into(),
            monthly_price_cents: 9900,
            included_seats,
            included_class_minutes: 1000,
            included_recording_gb: 10,
        };
        BillingDto {
            plan: Some(plan.clone()),
            subscription: Some(SubscriptionDto {
                plan_id: "plan_pro".into(),
                status: "active".into(),
                current_period_start: None,
                current_period_end: None,
                trial_ends_at: None,
                stripe_subscription_id: None,
                overage_behavior: overage.into(),
            }),
            usage: UsageDto {
                active_seats,
                included_seats: Some(included_seats),
                class_minutes_used: 0,
                included_class_minutes: Some(0),
                recording_gb_used: 0.0,
                included_recording_gb: Some(0.0),
            },
            overage_behavior: overage.into(),
            plans: vec![plan],
        }
    }

    #[test]
    fn seat_state_blocks_only_when_block_and_at_cap() {
        // At cap + block → blocked.
        let s = SeatState::from_billing(&billing_with(5, 5, "block"));
        assert_eq!(s.active, 5);
        assert_eq!(s.included, Some(5));
        assert!(s.blocked);

        // At cap but metered → not blocked.
        let s = SeatState::from_billing(&billing_with(5, 5, "metered"));
        assert!(!s.blocked);

        // Under cap + block → not blocked.
        let s = SeatState::from_billing(&billing_with(3, 5, "block"));
        assert!(!s.blocked);

        // No cap (included <= 0) → no cap, never blocked.
        let s = SeatState::from_billing(&billing_with(99, 0, "block"));
        assert_eq!(s.included, None);
        assert!(!s.blocked);
    }

    #[test]
    fn seat_indicator_shows_used_over_included() {
        fn app_inner(state: SeatState) -> Element {
            seat_indicator(&state)
        }
        let state = SeatState::from_billing(&billing_with(12, 25, "metered"));
        let mut vdom = VirtualDom::new_with_props(app_inner, state);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ui-metric-card"), "got: {html}");
        assert!(html.contains("Seats"));
        assert!(html.contains("12 / 25"));
    }

    #[test]
    fn seat_indicator_shows_danger_badge_at_cap() {
        fn app_inner(state: SeatState) -> Element {
            seat_indicator(&state)
        }
        let state = SeatState::from_billing(&billing_with(5, 5, "block"));
        let mut vdom = VirtualDom::new_with_props(app_inner, state);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("5 / 5"), "got: {html}");
        assert!(html.contains("badge-danger"), "got: {html}");
        assert!(html.contains("Seat limit reached"));
    }

    #[test]
    fn seat_indicator_no_cap_shows_active_count_only() {
        fn app_inner(state: SeatState) -> Element {
            seat_indicator(&state)
        }
        let state = SeatState::from_billing(&billing_with(7, 0, "metered"));
        let mut vdom = VirtualDom::new_with_props(app_inner, state);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        // No "/ included" segment, just the active count + "no cap" delta.
        assert!(html.contains("no cap"), "got: {html}");
        assert!(!html.contains(" / "), "should not show a cap: {html}");
    }

    #[test]
    fn member_invitations_table_renders_rows() {
        let invites = vec![InvitationDto {
            id: "mi-1".into(),
            email: "teach@example.com".into(),
            role: "teacher".into(),
            status: "pending".into(),
            created_at: "2026-05-29T00:00:00Z".into(),
        }];
        fn app_inner(invites: Vec<InvitationDto>) -> Element {
            member_invitations_table(&invites)
        }
        let mut vdom = VirtualDom::new_with_props(app_inner, invites);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ui-data-table"), "got: {html}");
        assert!(html.contains("teach@example.com"));
        assert!(html.contains("Teacher"));
        assert!(html.contains("Pending"));
        assert!(html.contains("Email"));
    }

    #[test]
    fn recovery_reset_panel_requires_acknowledgement_copy() {
        fn app_inner() -> Element {
            recovery_reset_panel(
                "Ada",
                &["abcde-23456".to_string(), "fghjk-789pq".to_string()],
                false,
            )
        }
        let mut vdom = VirtualDom::new(app_inner);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Replacement recovery codes"), "{html}");
        assert!(html.contains("Ada"), "{html}");
        assert!(html.contains("abcde-23456"), "{html}");
        assert!(html.contains("Codes have been saved"), "{html}");
    }

    #[test]
    fn invite_error_message_maps_seat_limit_conflict() {
        let err = api::ApiError::Status(409, "conflict: seat_limit_reached".into());
        assert_eq!(
            invite_error_message(&err),
            "Seat limit reached — upgrade your plan."
        );
        // Other errors fall through to Display.
        let other = api::ApiError::Status(500, "boom".into());
        assert_eq!(invite_error_message(&other), "status 500: boom");
    }
}
