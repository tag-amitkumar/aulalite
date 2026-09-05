// crates/shell-web/src/routes/admin_billing.rs
//
// Billing admin surface. Restricted to organization owners / contextual
// platform operators (the
// backend `/v1/admin/billing` family 403s everyone else). Renders the current
// plan + subscription status, a row of usage `MetricCard` tiles (Seats, Class
// monthly class minutes, total recording storage) with protected-limit tones
// (Warning >= ~80% of cap, Danger
// over cap), an upgrade section that starts a Stripe Checkout session and
// redirects the browser to the returned URL, and an overage-behavior toggle.
//
// Follows the project-wide four-state UX: loading, error, empty (no plans to
// upgrade to), and loaded.
use design_system::kinetics_ui::{MetricCard, MetricTone};
use design_system::{
    use_toast_sender, Button, ButtonVariant, Card, PageHeader, SkeletonCard, ToastLevel,
};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::api::{PlanDto, SubscriptionDto, UsageDto};
use features_courses::app_shell::{AppShell, ShellUser};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

/// Soft-cap tone for a single usage metric. Warning at >= 80% of the cap,
/// Danger once over the cap, Success otherwise. A non-positive cap is treated
/// as "no cap" → Neutral. Pure so it's unit-testable.
fn usage_tone(used: f64, included: f64) -> MetricTone {
    if included <= 0.0 {
        return MetricTone::Neutral;
    }
    let ratio = used / included;
    if ratio > 1.0 {
        MetricTone::Danger
    } else if ratio >= 0.8 {
        MetricTone::Warning
    } else {
        MetricTone::Success
    }
}

/// Human-readable delta string for a usage metric: how much headroom is left,
/// or how far over the cap. Pure so it's unit-testable.
fn usage_delta(used: f64, included: f64, unit: &str) -> String {
    if included <= 0.0 {
        return "no cap".to_string();
    }
    let remaining = included - used;
    if remaining < 0.0 {
        format!("{} {unit} over cap", fmt_num(-remaining))
    } else {
        format!("{} {unit} left", fmt_num(remaining))
    }
}

/// Format a number without a trailing ".0" for whole values (so integer caps
/// like seats/minutes read cleanly while GB keeps one decimal).
fn fmt_num(n: f64) -> String {
    if (n.fract()).abs() < f64::EPSILON {
        format!("{}", n as i64)
    } else {
        format!("{n:.1}")
    }
}

/// A subscription-status badge with a tone-mapped class. Pure/presentational.
fn status_badge(status: &str) -> Element {
    let tone = match status {
        "active" => "badge--success",
        "trialing" => "badge--info",
        "past_due" | "unpaid" => "badge--danger",
        "canceled" | "incomplete_expired" => "badge--neutral",
        _ => "badge--neutral",
    };
    rsx! {
        span { class: "billing-status-badge {tone}", "{status}" }
    }
}

/// "X / cap" value text where a missing cap reads as "no cap".
fn usage_value(used: String, included: Option<f64>) -> String {
    match included {
        Some(cap) => format!("{used} / {}", fmt_num(cap)),
        None => format!("{used} / no cap"),
    }
}

/// The row of usage MetricCards. Caps are `None` for plan-less tenants —
/// rendered as "no cap" with a Neutral tone. Pure so it's SSR-testable.
fn usage_grid(u: &UsageDto) -> Element {
    let seats_used = u.active_seats as f64;
    let seats_inc = u.included_seats.map(|v| v as f64);
    let mins_used = u.class_minutes_used as f64;
    let mins_inc = u.included_class_minutes.map(|v| v as f64);
    let gb_used = u.recording_gb_used;
    let gb_inc = u.included_recording_gb;
    rsx! {
        div { class: "admin-billing-usage-grid",
            MetricCard {
                label: "Seats".to_string(),
                value: usage_value(u.active_seats.to_string(), seats_inc),
                delta: usage_delta(seats_used, seats_inc.unwrap_or(0.0), "seats"),
                tone: usage_tone(seats_used, seats_inc.unwrap_or(0.0)),
            }
            MetricCard {
                label: "Monthly class minutes".to_string(),
                value: usage_value(u.class_minutes_used.to_string(), mins_inc),
                delta: usage_delta(mins_used, mins_inc.unwrap_or(0.0), "min"),
                tone: usage_tone(mins_used, mins_inc.unwrap_or(0.0)),
            }
            MetricCard {
                label: "Recording storage".to_string(),
                value: usage_value(fmt_num(gb_used), gb_inc),
                delta: usage_delta(gb_used, gb_inc.unwrap_or(0.0), "GB"),
                tone: usage_tone(gb_used, gb_inc.unwrap_or(0.0)),
            }
        }
    }
}

/// The current-plan summary card (plan name, status badge, trial/period
/// dates), or the designed "no plan yet" state for fresh tenants. Pure so
/// it's SSR-testable.
fn plan_summary(
    plan: Option<&PlanDto>,
    sub: Option<&SubscriptionDto>,
    overage_behavior: &str,
) -> Element {
    let Some(sub) = sub else {
        return rsx! {
            Card {
                div { class: "billing-plan-summary billing-plan-summary--empty",
                    div { class: "billing-plan-summary-head",
                        h2 { class: "billing-plan-name", "No active plan" }
                    }
                    p { class: "muted",
                        "This workspace isn't subscribed yet. Pick a plan below to start — usage shows with no caps until then."
                    }
                }
            }
        };
    };
    let plan_name = plan
        .map(|p| p.name.clone())
        .unwrap_or_else(|| sub.plan_id.clone());
    let trial = sub.trial_ends_at.clone();
    let period_end = sub.current_period_end.clone();
    rsx! {
        Card {
            div { class: "billing-plan-summary",
                div { class: "billing-plan-summary-head",
                    h2 { class: "billing-plan-name", "{plan_name}" }
                    {status_badge(&sub.status)}
                }
                dl { class: "billing-plan-meta",
                    if let Some(t) = trial {
                        dt { "Trial ends" }
                        dd { "{t}" }
                    }
                    if let Some(p) = period_end {
                        dt { "Current period ends" }
                        dd { "{p}" }
                    }
                    dt { "Overage behavior" }
                    dd { "{overage_behavior}" }
                }
            }
        }
    }
}

/// Checkout targets for a workspace that does not yet have a provider-managed
/// subscription. Existing plans only move upward by price; provider-backed
/// subscriptions change plans in Stripe's portal instead.
fn upgrade_targets(plans: &[PlanDto], current_plan: Option<&PlanDto>) -> Vec<PlanDto> {
    plans
        .iter()
        .filter(|candidate| {
            current_plan.is_none_or(|current| {
                candidate.id != current.id
                    && candidate.monthly_price_cents > current.monthly_price_cents
            })
        })
        .cloned()
        .collect()
}

fn plan_quota(value: i64, unit: &str) -> String {
    if value < 0 {
        format!("Unlimited {unit}")
    } else {
        format!("{value} {unit}")
    }
}

/// Only offer the Customer Portal after Stripe has acknowledged a concrete
/// subscription. Fresh/self-hosted rows without a provider id keep the plan
/// picker instead of leading administrators to a guaranteed error.
fn can_manage_subscription(subscription: Option<&SubscriptionDto>) -> bool {
    subscription
        .and_then(|sub| sub.stripe_subscription_id.as_deref())
        .is_some_and(|id| !id.trim().is_empty())
}

/// Format integer cents as "$X.YY / mo". Pure so it's unit-testable.
fn fmt_price(cents: i64) -> String {
    format!("${}.{:02} / mo", cents / 100, (cents % 100).abs())
}

/// Skeleton placeholder grid shown while billing loads.
fn loading_grid() -> Element {
    rsx! {
        div { class: "admin-billing-usage-grid",
            for _ in 0..3 {
                SkeletonCard { height: "120px".to_string() }
            }
        }
    }
}

#[component]
pub fn AdminBilling() -> Element {
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

    let can_manage_billing = user.can_manage_billing();
    if !can_manage_billing {
        return rsx! {
            div { class: "container",
                h1 { "Owner access required" }
                p { "Billing and subscription changes are reserved for the organization owner." }
                a { href: "/admin", "Return to admin console" }
            }
        };
    }

    let billing = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::get_billing(&api).await }
        }
    });

    // Pending flags so the upgrade buttons / overage toggle can show progress
    // and avoid double-submits.
    let starting_checkout = use_signal(|| false);
    let starting_portal = use_signal(|| false);

    let snap = billing.read_unchecked();
    let content: Element = match snap.as_ref() {
        Some(Ok(b)) => {
            let plan = b.plan.clone();
            let sub = b.subscription.clone();
            let usage = b.usage.clone();
            let provider_managed = can_manage_subscription(sub.as_ref());
            let targets = upgrade_targets(&b.plans, plan.as_ref());
            let current_overage = b.overage_behavior.clone();

            // Stripe's portal is available once the subscription has a
            // provider id. The backend independently verifies the billing
            // capability and persisted customer before creating a session.
            let portal_action: Element = if provider_managed {
                let api_for_portal = api.clone();
                let mut starting = starting_portal;
                let mut toast = toast;
                rsx! {
                    div { class: "billing-plan-actions",
                        div {
                            strong { "Need to make an account change?" }
                            p { class: "muted",
                                "Update payment details, view invoices, change your plan, or cancel securely in Stripe."
                            }
                        }
                        Button {
                            label: "Manage subscription".to_string(),
                            variant: ButtonVariant::Secondary,
                            loading: *starting.read(),
                            on_click: move |_| {
                                let api = api_for_portal.clone();
                                starting.set(true);
                                spawn(async move {
                                    match api::start_billing_portal(&api).await {
                                        Ok(session) => {
                                            if let Err(error) = api::redirect_to(&session.url) {
                                                toast.push(
                                                    ToastLevel::Danger,
                                                    "Billing portal unavailable",
                                                    error,
                                                );
                                                starting.set(false);
                                            }
                                        }
                                        Err(err) => {
                                            toast.push(
                                                ToastLevel::Danger,
                                                "Billing portal unavailable",
                                                format!("{err}"),
                                            );
                                            starting.set(false);
                                        }
                                    }
                                });
                            },
                        }
                    }
                }
            } else {
                rsx! {}
            };

            // --- Upgrade section ---
            let api_for_checkout = api.clone();
            let upgrade_section: Element = if provider_managed {
                rsx! {
                    p { class: "muted",
                        "Use Manage subscription above to change or cancel this Stripe-managed plan."
                    }
                }
            } else if targets.is_empty() {
                rsx! {
                    p { class: "muted", "You're on the highest available plan." }
                }
            } else {
                rsx! {
                    div { class: "admin-billing-plans",
                        for plan in targets.iter() {
                            {
                                let plan_id = plan.id.clone();
                                let plan_name = plan.name.clone();
                                let price = fmt_price(plan.monthly_price_cents);
                                let class_quota =
                                    plan_quota(plan.included_class_minutes, "monthly min");
                                let recording_quota =
                                    plan_quota(plan.included_recording_gb, "recording GB");
                                let api_ck = api_for_checkout.clone();
                                let mut starting = starting_checkout;
                                let mut toast = toast;
                                rsx! {
                                    Card {
                                        div { class: "admin-billing-plan-card", key: "{plan_id}",
                                            h3 { class: "admin-billing-plan-card-name", "{plan_name}" }
                                            p { class: "admin-billing-plan-card-price", "{price}" }
                                            p { class: "muted",
                                                "{plan.included_seats} seats · {class_quota} · {recording_quota}"
                                            }
                                            Button {
                                                label: format!("Upgrade to {plan_name}"),
                                                variant: ButtonVariant::Premium,
                                                loading: *starting.read(),
                                                on_click: move |_| {
                                                    let api = api_ck.clone();
                                                    let plan_id = plan_id.clone();
                                                    starting.set(true);
                                                    spawn(async move {
                                                        match api::start_checkout(&api, &plan_id).await {
                                                            Ok(session) => {
                                                                // Redirect the browser to the hosted
                                                                // Stripe Checkout URL.
                                                                if let Err(error) = api::redirect_to(&session.url) {
                                                                    toast.push(
                                                                        ToastLevel::Danger,
                                                                        "Checkout failed",
                                                                        error,
                                                                    );
                                                                    starting.set(false);
                                                                }
                                                            }
                                                            Err(err) => {
                                                                toast.push(
                                                                    ToastLevel::Danger,
                                                                    "Checkout failed",
                                                                    format!("{err}"),
                                                                );
                                                                starting.set(false);
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
            };

            // Metered overage stays unavailable until provider usage events
            // and invoice reconciliation are implemented end to end.
            let overage_section: Element = rsx! {
                div { class: "admin-billing-overage",
                    strong { "Plan limits are protected" }
                    p { class: "muted", "New usage is blocked at your cap. Contact support to raise limits or change plans." }
                }
            };

            rsx! {
                section { class: "admin-billing-section",
                    h2 { class: "admin-billing-section-title", "Current plan" }
                    {plan_summary(plan.as_ref(), sub.as_ref(), &current_overage)}
                    {portal_action}
                }
                section { class: "admin-billing-section",
                    h2 { class: "admin-billing-section-title", "Usage and limits" }
                    {usage_grid(&usage)}
                }
                section { class: "admin-billing-section",
                    h2 { class: "admin-billing-section-title", "Change plan" }
                    {upgrade_section}
                }
                section { class: "admin-billing-section",
                    h2 { class: "admin-billing-section-title", "Overage behavior" }
                    {overage_section}
                }
            }
        }
        Some(Err(e)) => rsx! {
            p { class: "error", "Could not load billing: {e}" }
        },
        None => rsx! {
            section { class: "admin-billing-section",
                {loading_grid()}
            }
        },
    };
    drop(snap);

    let body = rsx! {
        div { class: "admin-billing-page",
            PageHeader {
                title: "Billing".to_string(),
                kicker: "Workspace administration".to_string(),
                subtitle: "Review monthly class usage, total recording storage, and protected plan limits.".to_string(),
            }
            { content }
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
    fn unlimited_plan_quota_has_marketable_copy() {
        assert_eq!(plan_quota(-1, "recording GB"), "Unlimited recording GB");
        assert_eq!(plan_quota(50, "recording GB"), "50 recording GB");
    }

    fn sample_usage() -> UsageDto {
        UsageDto {
            active_seats: 12,
            included_seats: Some(25),
            class_minutes_used: 950,
            included_class_minutes: Some(1000),
            recording_gb_used: 12.5,
            included_recording_gb: Some(10.0),
        }
    }

    fn sample_plan(id: &str, name: &str, cents: i64) -> PlanDto {
        PlanDto {
            id: id.into(),
            name: name.into(),
            monthly_price_cents: cents,
            included_seats: 25,
            included_class_minutes: 1000,
            included_recording_gb: 10,
        }
    }

    fn sample_sub() -> SubscriptionDto {
        SubscriptionDto {
            plan_id: "plan_pro".into(),
            status: "trialing".into(),
            current_period_start: None,
            current_period_end: None,
            trial_ends_at: Some("2026-06-15T00:00:00Z".into()),
            stripe_subscription_id: None,
            overage_behavior: "block".into(),
        }
    }

    #[test]
    fn usage_tone_thresholds() {
        // Well under cap → Success.
        assert_eq!(usage_tone(12.0, 25.0), MetricTone::Success);
        // >= 80% of cap → Warning.
        assert_eq!(usage_tone(950.0, 1000.0), MetricTone::Warning);
        assert_eq!(usage_tone(800.0, 1000.0), MetricTone::Warning);
        // Over cap → Danger.
        assert_eq!(usage_tone(12.5, 10.0), MetricTone::Danger);
        // No cap → Neutral.
        assert_eq!(usage_tone(5.0, 0.0), MetricTone::Neutral);
    }

    #[test]
    fn usage_delta_words() {
        assert_eq!(usage_delta(12.0, 25.0, "seats"), "13 seats left");
        assert_eq!(usage_delta(12.5, 10.0, "GB"), "2.5 GB over cap");
        assert_eq!(usage_delta(5.0, 0.0, "GB"), "no cap");
    }

    #[test]
    fn upgrade_targets_excludes_current() {
        let plans = vec![
            sample_plan("plan_starter", "Starter", 0),
            sample_plan("plan_pro", "Pro", 9900),
        ];
        let targets = upgrade_targets(&plans, Some(&plans[0]));
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "plan_pro");
        // Plan-less tenant can move to anything.
        assert_eq!(upgrade_targets(&plans, None).len(), 2);
        // A Pro workspace must never be offered Starter as an "upgrade".
        assert!(upgrade_targets(&plans, Some(&plans[1])).is_empty());
    }

    #[test]
    fn portal_action_requires_provider_subscription_id() {
        let mut sub = sample_sub();
        assert!(!can_manage_subscription(Some(&sub)));
        sub.stripe_subscription_id = Some("sub_123".into());
        assert!(can_manage_subscription(Some(&sub)));
        sub.stripe_subscription_id = Some("  ".into());
        assert!(!can_manage_subscription(Some(&sub)));
        assert!(!can_manage_subscription(None));
    }

    #[test]
    fn fmt_price_formats_cents() {
        assert_eq!(fmt_price(9900), "$99.00 / mo");
        assert_eq!(fmt_price(9999), "$99.99 / mo");
        assert_eq!(fmt_price(0), "$0.00 / mo");
    }

    #[test]
    fn usage_grid_renders_metric_cards_with_used_over_included() {
        fn app_inner(u: UsageDto) -> Element {
            usage_grid(&u)
        }
        let mut vdom = VirtualDom::new_with_props(app_inner, sample_usage());
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ui-metric-card"), "got: {html}");
        assert!(html.contains("Seats"));
        assert!(html.contains("12 / 25"));
        assert!(html.contains("Monthly class minutes"));
        assert!(html.contains("950 / 1000"));
        assert!(html.contains("Recording storage"));
        assert!(html.contains("12.5 / 10"));
        // Class minutes at 95% → Warning tone.
        assert!(html.contains("ui-metric-card--warning"), "got: {html}");
        // Recording storage over cap → Danger tone.
        assert!(html.contains("ui-metric-card--danger"), "got: {html}");
    }

    #[test]
    fn plan_summary_renders_name_status_and_dates() {
        fn app_inner(args: (PlanDto, SubscriptionDto)) -> Element {
            plan_summary(Some(&args.0), Some(&args.1), "block")
        }
        let mut vdom = VirtualDom::new_with_props(
            app_inner,
            (sample_plan("plan_pro", "Pro", 9900), sample_sub()),
        );
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Pro"));
        assert!(html.contains("billing-status-badge"));
        assert!(html.contains("trialing"));
        assert!(html.contains("Trial ends"));
        assert!(html.contains("2026-06-15T00:00:00Z"));
        assert!(html.contains("block"));
    }

    #[test]
    fn plan_summary_renders_no_plan_state_for_fresh_tenants() {
        fn app_inner() -> Element {
            plan_summary(None, None, "block")
        }
        let mut vdom = VirtualDom::new(app_inner);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("No active plan"), "got: {html}");
        assert!(html.contains("Pick a plan below"), "got: {html}");
    }

    #[test]
    fn usage_grid_handles_plan_less_caps() {
        fn app_inner(u: UsageDto) -> Element {
            usage_grid(&u)
        }
        let u = UsageDto {
            active_seats: 3,
            included_seats: None,
            class_minutes_used: 10,
            included_class_minutes: None,
            recording_gb_used: 0.0,
            included_recording_gb: None,
        };
        let mut vdom = VirtualDom::new_with_props(app_inner, u);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("3 / no cap"), "got: {html}");
        assert!(!html.contains("danger"), "no caps must not alarm: {html}");
    }

    #[test]
    fn loading_grid_renders_skeletons() {
        fn app_inner() -> Element {
            loading_grid()
        }
        let mut vdom = VirtualDom::new(app_inner);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("ds-skeleton-card"), "got: {html}");
    }
}
