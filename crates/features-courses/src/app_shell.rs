// crates/features-courses/src/app_shell.rs
use crate::a11y::{SkipToContent, MAIN_CONTENT_ID};
use crate::api::{self, ApiContext, BrandingDto, WorkspaceDto};
use crate::command_palette::CommandPalette;
use crate::locale_switcher::LocaleSwitcher;
use crate::notification_bell::NotificationBell;
use design_system::kinetics_ui::{Tour, TourStep};
use design_system::{
    apply_density_preference, apply_theme_preference, AulaLogo, ThemePreference, ThemeToggle,
    UiIcon, UiIconView,
};
use dioxus::prelude::*;

/// DOM id for a sidebar nav link, derived from its label ("My Courses" →
/// `nav-my-courses`). Stable ids let the guided tour spotlight nav entries.
pub fn nav_link_id(label: &str) -> String {
    let slug: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    format!("nav-{slug}")
}

/// First-run tour steps for teaching/admin roles. Admins additionally get the
/// invite-members step. Pure so it's SSR-testable.
pub fn tour_steps(is_admin: bool) -> Vec<TourStep> {
    let mut steps = vec![
        TourStep::new(
            "welcome",
            "Welcome to AulaLite",
            "A quick tour of the essentials — it takes about 30 seconds. You can dismiss it at any time.",
        ),
        TourStep::new(
            "courses",
            "Create your courses",
            "Build courses with modules, lessons, quizzes, and assignments under My Courses.",
        )
        .with_target(nav_link_id("My Courses")),
    ];
    if is_admin {
        steps.push(
            TourStep::new(
                "members",
                "Invite your people",
                "Add teachers and students to the workspace from the Members page.",
            )
            .with_target(nav_link_id("Members")),
        );
    }
    steps.push(TourStep::new(
        "go-live",
        "Go live",
        "Start or schedule a live class from any course's Schedule tab — students join right from their dashboard.",
    ));
    steps.push(
        TourStep::new(
            "cmdk",
            "Move faster",
            "Press Ctrl+K (Cmd+K on Mac) anywhere to jump to courses, admin pages, and actions.",
        )
        .with_target("topbar-cmdk"),
    );
    steps
}

/// Build the `:root` token-override CSS for a tenant's branding, or `None` when
/// nothing is set (so the default theme is left untouched). Only fields that
/// ARE set emit overrides:
///   * `primary_color` drives `--color-primary` / `--color-primary-hover`,
///     `--brand-primary` / `--brand-primary-hover`, and kinetics `--ui-primary`.
///   * `accent_color` drives `--color-accent` / `--color-accent-strong` and
///     kinetics `--ui-accent`.
/// Pure so it's unit-/SSR-testable without a live ApiContext. Injected as a
/// `<style>` via `dangerous_inner_html`, mirroring `KineticsStyles`.
pub fn branding_override_css(b: &BrandingDto) -> Option<String> {
    let primary = b
        .primary_color
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let accent = b
        .accent_color
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if primary.is_none() && accent.is_none() {
        return None;
    }
    let mut decls = String::new();
    if let Some(p) = primary {
        decls.push_str(&format!("  --color-primary: {p};\n"));
        decls.push_str(&format!("  --color-primary-hover: {p};\n"));
        decls.push_str(&format!("  --brand-primary: {p};\n"));
        decls.push_str(&format!("  --brand-primary-hover: {p};\n"));
        decls.push_str(&format!("  --ui-primary: {p};\n"));
    }
    if let Some(a) = accent {
        decls.push_str(&format!("  --color-accent: {a};\n"));
        decls.push_str(&format!("  --color-accent-strong: {a};\n"));
        decls.push_str(&format!("  --ui-accent: {a};\n"));
    }
    Some(format!(":root {{\n{decls}}}\n"))
}

#[derive(Clone, PartialEq)]
pub struct ShellUser {
    pub display_name: String,
    pub email: String,
    pub tenant_role: Option<core_types::TenantRole>,
    pub is_platform_admin: bool,
}

#[derive(Props, Clone, PartialEq)]
pub struct AppShellProps {
    pub user: ShellUser,
    pub on_signout: EventHandler<()>,
    pub children: Element,
}

/// Shell-level sign-out supplied by the root route layout. It clears the
/// renderer's credential store and live API/user contexts before navigating,
/// preventing individual routes from accidentally implementing web-only
/// logout. `AppShellProps::on_signout` remains a fallback for isolated SSR
/// component tests that intentionally mount no application layout.
#[derive(Clone, Copy)]
pub struct AppShellSignOut(pub EventHandler<()>);

/// Root-level workspace transition supplied by the application shell. Native
/// renderers cannot refresh a browser document, so the root uses this event to
/// refresh the authenticated user context and remount workspace-scoped routes.
#[derive(Clone, Copy)]
pub struct AppShellWorkspaceSwitch(pub EventHandler<String>);

fn workspace_role_label(role: &str) -> &'static str {
    match role {
        "org_owner" => "Organization owner",
        "org_admin" => "Organization admin",
        "teacher" => "Teacher",
        "ta" => "Teaching assistant",
        "student" => "Student",
        "parent" => "Parent",
        _ => "Member",
    }
}

#[component]
fn WorkspaceSwitcher(workspaces: Vec<WorkspaceDto>) -> Element {
    if workspaces.is_empty() {
        return rsx! {};
    }

    if workspaces.len() == 1 {
        let workspace = &workspaces[0];
        return rsx! {
            div {
                class: "workspace-context workspace-context--single",
                title: "Current workspace",
                span { class: "workspace-context__mark", aria_hidden: "true", "W" }
                span { class: "workspace-context__copy",
                    span { class: "workspace-context__name", "{workspace.name}" }
                    span { class: "workspace-context__role", "{workspace_role_label(&workspace.role)}" }
                }
            }
        };
    }

    let selected = workspaces
        .iter()
        .find(|workspace| workspace.current)
        .map(|workspace| workspace.tenant_id.clone())
        .unwrap_or_else(|| workspaces[0].tenant_id.clone());
    let contextual_switch = try_consume_context::<AppShellWorkspaceSwitch>();

    rsx! {
        div { class: "workspace-switcher",
            span { class: "workspace-context__mark", aria_hidden: "true", "W" }
            label { class: "workspace-switcher__label", r#for: "workspace-switcher-select", "Workspace" }
            select {
                id: "workspace-switcher-select",
                class: "workspace-switcher__select",
                value: "{selected}",
                "aria-label": "Switch workspace",
                onchange: move |event| {
                    let tenant_id = event.value();
                    if tenant_id != selected {
                        match contextual_switch {
                            Some(handler) => handler.0.call(tenant_id),
                            None => {
                                api::set_selected_workspace_id(Some(&tenant_id));
                                reload_current_page();
                            }
                        }
                    }
                },
                for workspace in &workspaces {
                    option {
                        value: "{workspace.tenant_id}",
                        selected: workspace.current,
                        "{workspace.name} · {workspace_role_label(&workspace.role)}"
                    }
                }
            }
        }
    }
}

#[component]
pub fn AppShell(props: AppShellProps) -> Element {
    let router = dioxus_router::try_router();
    let role = props.user.tenant_role;
    let contextual_signout = try_consume_context::<AppShellSignOut>();
    let fallback_signout = props.on_signout;
    let on_signout = move || match contextual_signout {
        Some(handler) => handler.0.call(()),
        None => fallback_signout.call(()),
    };
    let has_tenant_context = role.is_some();
    let show_workspace_tools = has_tenant_context || !props.user.is_platform_admin;
    let is_parent = role == Some(core_types::TenantRole::Parent);
    let show_discovery_tools = show_workspace_tools && !is_parent;

    let mut primary_menu = if props.user.is_platform_admin && !has_tenant_context {
        // Platform operators are global until they deliberately enter one of
        // their own active workspace memberships. Do not advertise tenant or
        // learner actions that the backend will correctly refuse without that
        // context.
        Vec::new()
    } else {
        match role {
            Some(core_types::TenantRole::OrgOwner) => vec![
                ("Dashboard", "/", UiIcon::Dashboard),
                ("My Courses", "/courses", UiIcon::Courses),
            ],
            Some(core_types::TenantRole::OrgAdmin) => vec![
                ("Dashboard", "/", UiIcon::Dashboard),
                ("My Courses", "/courses", UiIcon::Courses),
            ],
            Some(core_types::TenantRole::Teacher) => vec![
                ("Dashboard", "/", UiIcon::Dashboard),
                ("My Courses", "/courses", UiIcon::Courses),
            ],
            Some(core_types::TenantRole::Ta) => vec![
                ("Dashboard", "/", UiIcon::Dashboard),
                ("My Courses", "/courses", UiIcon::Courses),
            ],
            Some(core_types::TenantRole::Student) | None => vec![
                ("Dashboard", "/", UiIcon::Dashboard),
                ("My Courses", "/courses", UiIcon::Courses),
                ("Catalog", "/catalog", UiIcon::Courses),
                ("My Schedule", "/schedule", UiIcon::Schedule),
                ("My Transcript", "/transcript", UiIcon::File),
                ("My Certificates", "/certificates", UiIcon::File),
                ("Redeem Code", "/redeem", UiIcon::Redeem),
            ],
            Some(core_types::TenantRole::Parent) => {
                vec![("Family overview", "/parent", UiIcon::People)]
            }
        }
    };

    // Common items shown to every role (appended after the role-specific links
    // so existing per-role entries are preserved). Calendar is a personal agenda
    // of the caller's sessions + assignment due dates (with an .ics feed). The
    // Notifications link is the discoverable entry point to the
    // notification-preferences settings page.
    if show_workspace_tools && !is_parent {
        primary_menu.push(("Calendar", "/calendar", UiIcon::Schedule));
    }
    if show_workspace_tools {
        primary_menu.push(("Notifications", "/settings/notifications", UiIcon::Settings));
    }

    // Workspace administration is shared by owners and administrators. The
    // billing destination remains owner-only because it can change the legal
    // subscription and payment relationship for the organization.
    let is_admin = matches!(
        role,
        Some(core_types::TenantRole::OrgOwner | core_types::TenantRole::OrgAdmin)
    ) || (props.user.is_platform_admin && has_tenant_context);
    let is_owner = role == Some(core_types::TenantRole::OrgOwner)
        || (props.user.is_platform_admin && has_tenant_context);
    let admin_menu = if is_admin {
        let mut items = vec![
            ("Set up workspace", "/onboarding", UiIcon::Settings),
            ("Admin home", "/admin", UiIcon::Settings),
            ("Workspace courses", "/courses?scope=all", UiIcon::Courses),
            ("Analytics", "/admin/analytics", UiIcon::Dashboard),
            ("Branding", "/admin/branding", UiIcon::Settings),
            ("Members", "/admin/tenant", UiIcon::People),
        ];
        if is_owner {
            items.insert(4, ("Billing", "/admin/billing", UiIcon::Redeem));
        }
        items
    } else {
        Vec::new()
    };
    let operations_menu = if is_admin {
        vec![
            ("Integrations", "/admin/integrations", UiIcon::Settings),
            ("Audit log", "/admin/audit", UiIcon::File),
            ("Delivery log", "/admin/notifications", UiIcon::File),
            ("Files", "/admin/files", UiIcon::File),
        ]
    } else {
        Vec::new()
    };

    // Platform super-admin section: a single top-level entry, shown ONLY to
    // platform admins. Distinct from the per-tenant "Admin" section above (which
    // also admits org admins) — the platform console manages tenants across the
    // whole deployment, so org admins must NOT see this link.
    let platform_menu = props.user.is_platform_admin.then_some((
        "Platform owner console",
        "/platform",
        UiIcon::Settings,
    ));

    let current_path = current_browser_path(router);
    let user_initials = display_initials(&props.user.display_name);
    let operations_expanded = operations_menu
        .iter()
        .any(|(_, href, _)| nav_link_is_active(&current_path, href));
    let current_section = primary_menu
        .iter()
        .chain(admin_menu.iter())
        .chain(operations_menu.iter())
        .find(|(_, href, _)| nav_link_is_active(&current_path, href))
        .map(|(label, _, _)| *label)
        .or_else(|| {
            platform_menu.as_ref().and_then(|(label, href, _)| {
                nav_link_is_active(&current_path, href).then_some(*label)
            })
        })
        .unwrap_or("Workspace");

    // Role info handed to the global command palette so its Admin group is
    // gated exactly like the sidebar nav above.
    let palette_role = role;
    let palette_is_platform_admin = props.user.is_platform_admin && has_tenant_context;

    // Global search box state. On submit we navigate to `/search?q=<encoded>`
    // by setting `window.location.href`, matching the AppShell `<a href>` nav
    // convention (same approach the command palette uses). Visible to every
    // authed role.
    let mut search_query = use_signal(String::new);
    let submit_search = move || {
        let q = search_query.read().trim().to_string();
        if q.is_empty() {
            return;
        }
        let href = format!("/search?q={}", urlencoding::encode(&q));
        navigate_to(router, &href);
    };

    // --- Runtime tenant theming ---
    // Resolve ApiContext defensively (the AppShell renders on every authed
    // route where the signal is provided, but tolerating its absence keeps the
    // shell panic-free in SSR tests / unusual mount sites). A missing context
    // yields an empty token; the branding fetch then short-circuits on the
    // resulting 401/network error and we fall back to the default theme + logo.
    let api = use_hook(|| {
        try_consume_context::<Signal<ApiContext>>().unwrap_or_else(|| {
            Signal::new(ApiContext {
                base_url: String::new(),
                id_token: String::new(),
            })
        })
    });
    let branding = use_resource(move || {
        let api = api;
        async move {
            let ctx = api.read().clone();
            api::get_my_branding(&ctx).await
        }
    });
    let workspace_directory = use_resource(move || {
        let api = api;
        async move {
            let ctx = api.read().clone();
            api::get_my_workspaces(&ctx).await
        }
    });

    // --- Theme / density preference ---
    // The boot script in index.html already painted the localStorage-mirrored
    // theme; once authed, the backend preference is authoritative. Fetch it,
    // apply it to <html>, and keep the toggle in sync. Changing the toggle
    // applies instantly and persists via PATCH (best-effort: a failed PATCH
    // leaves the local theme applied; next load falls back to the stored one).
    let mut theme_pref = use_signal(ThemePreference::default);
    // First-run guided tour: opens once the preference fetch confirms it has
    // not been dismissed, for teaching/admin roles only.
    let tour_eligible = props.user.is_platform_admin
        || matches!(
            role,
            Some(core_types::TenantRole::OrgOwner)
                | Some(core_types::TenantRole::OrgAdmin)
                | Some(core_types::TenantRole::Teacher)
                | Some(core_types::TenantRole::Ta)
        );
    let mut tour_open = use_signal(|| false);
    let mut tour_active = use_signal(|| 0usize);
    use_future(move || {
        let api = api;
        async move {
            let ctx = api.read().clone();
            if let Ok(prefs) = api::get_my_preferences(&ctx).await {
                let pref = ThemePreference::parse(&prefs.theme);
                theme_pref.set(pref);
                apply_theme_preference(pref);
                apply_density_preference(&prefs.density);
                if tour_eligible && !prefs.tour_dismissed {
                    tour_open.set(true);
                }
            }
        }
    });
    let dismiss_tour = move |_: ()| {
        tour_open.set(false);
        let api = api;
        spawn(async move {
            let ctx = api.read().clone();
            let _ = api::dismiss_tour(&ctx).await;
        });
    };
    let on_theme_change = move |pref: ThemePreference| {
        theme_pref.set(pref);
        apply_theme_preference(pref);
        let api = api;
        spawn(async move {
            let ctx = api.read().clone();
            let _ = api::patch_my_preferences(&ctx, Some(pref.as_str().to_string()), None).await;
        });
    };

    // On fetch error / no branding → default theme + logo unchanged.
    let branding_dto: Option<BrandingDto> = {
        let snap = branding.read_unchecked();
        match snap.as_ref() {
            Some(Ok(b)) => Some(b.clone()),
            _ => None,
        }
    };
    let override_css: Option<String> = branding_dto.as_ref().and_then(branding_override_css);
    let logo_url: Option<String> = branding_dto.as_ref().and_then(|b| {
        b.logo_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });
    let workspaces: Vec<WorkspaceDto> = {
        let snap = workspace_directory.read_unchecked();
        match snap.as_ref() {
            Some(Ok(directory)) => directory.workspaces.clone(),
            _ => Vec::new(),
        }
    };

    let brand_mark: Element = match logo_url {
        Some(url) => rsx! {
            span { class: "app-brand-logo",
                img { class: "aula-logo__image", src: "{url}", alt: "Workspace logo" }
            }
        },
        None => rsx! {
            AulaLogo { class: "app-brand-logo".to_string(), compact: false }
        },
    };

    rsx! {
        div { class: "app-shell-layout",
            // First focusable element on the page: lets keyboard / screen-reader
            // users bypass the nav chrome and jump straight to main content.
            SkipToContent {}
            // Per-tenant token overrides. Injected only when at least one
            // accent field is set; otherwise the default theme is untouched.
            if let Some(css) = override_css {
                style { dangerous_inner_html: "{css}" }
            }
            // Global command palette (Cmd-K / Ctrl-K). Mounted once here so the
            // shortcut and the palette are available on every authed screen.
            if show_discovery_tools {
                CommandPalette {
                    tenant_role: palette_role,
                    is_platform_admin: palette_is_platform_admin,
                }
            }
            // First-run spotlight tour (teacher/admin). Controlled: dismissal
            // persists via PATCH /v1/me/preferences so it shows exactly once.
            // NOTE: the tour's root div is neutralized to `display: contents`
            // in components.css — its visuals are all position:fixed, but the
            // root would otherwise become a grid child of .app-shell-layout
            // and push the main column into a new row below the sidebar.
            if show_workspace_tools && *tour_open.read() {
                Tour {
                    id: "first-run-tour".to_string(),
                    open: true,
                    steps: tour_steps(is_admin),
                    active: *tour_active.read(),
                    on_change: move |step: usize| tour_active.set(step),
                    on_dismiss: dismiss_tour,
                }
            }
            aside { class: "app-side",
                div { class: "app-brand",
                    {brand_mark}
                }
                nav { class: "app-nav", aria_label: "Primary navigation",
                    div { class: "app-nav-section",
                        p { class: "app-nav-section__label", "Your work" }
                        for (label, href, icon) in &primary_menu {
                            {
                                let active = nav_link_is_active(&current_path, href);
                                rsx! {
                                    a {
                                        class: if active { "nav-link is-active" } else { "nav-link" },
                                        id: "{nav_link_id(label)}",
                                        href: "{href}",
                                        "aria-current": if active { "page" } else { "false" },
                                        UiIconView { icon: icon.clone(), title: Some(label.to_string()) }
                                        span { "{label}" }
                                    }
                                }
                            }
                        }
                    }
                    if !admin_menu.is_empty() {
                        div { class: "app-nav-section",
                            p { class: "app-nav-section__label", "Manage" }
                            for (label, href, icon) in &admin_menu {
                                {
                                    let active = nav_link_is_active(&current_path, href);
                                    rsx! {
                                        a {
                                            class: if active { "nav-link is-active" } else { "nav-link" },
                                            id: "{nav_link_id(label)}",
                                            href: "{href}",
                                            "aria-current": if active { "page" } else { "false" },
                                            UiIconView { icon: icon.clone(), title: Some(label.to_string()) }
                                            span { "{label}" }
                                        }
                                    }
                                }
                            }
                            if !operations_menu.is_empty() {
                                details { class: "app-nav-more", open: operations_expanded,
                                    summary { "More tools" }
                                    div { class: "app-nav-more__items",
                                        for (label, href, icon) in &operations_menu {
                                            {
                                                let active = nav_link_is_active(&current_path, href);
                                                rsx! {
                                                    a {
                                                        class: if active { "nav-link is-active" } else { "nav-link" },
                                                        id: "{nav_link_id(label)}",
                                                        href: "{href}",
                                                        "aria-current": if active { "page" } else { "false" },
                                                        UiIconView { icon: icon.clone(), title: Some(label.to_string()) }
                                                        span { "{label}" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Some((label, href, icon)) = &platform_menu {
                        div { class: "app-nav-section app-nav-section--platform",
                            p { class: "app-nav-section__label", "Elementors" }
                            {
                                let active = nav_link_is_active(&current_path, href);
                                rsx! {
                                    a {
                                        class: if active { "nav-link is-active" } else { "nav-link" },
                                        id: "{nav_link_id(label)}",
                                        href: "{href}",
                                        "aria-current": if active { "page" } else { "false" },
                                        UiIconView { icon: icon.clone(), title: Some(label.to_string()) }
                                        span { "{label}" }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "app-side-footer",
                    span { class: "app-side-footer__dot", aria_hidden: "true" }
                    a { href: "mailto:hello@elementors.guru", "Help & support" }
                }
            }
            main { class: "app-main",
                header { class: "app-topbar",
                    div { class: "app-topbar-title",
                        span { class: "topbar-kicker", "{current_section}" }
                        // Discoverability hint for the global command palette.
                        if show_discovery_tools {
                            kbd { id: "topbar-cmdk", class: "topbar-cmdk-hint", title: "Open command palette", "Ctrl K" }
                        }
                    }
                    WorkspaceSwitcher { workspaces }
                    // Global search box. Submitting (Enter or the button)
                    // navigates to /search?q=<encoded>. Shown to all authed roles.
                    if show_discovery_tools {
                        form {
                            class: "topbar-search",
                            role: "search",
                            onsubmit: move |evt| {
                                evt.prevent_default();
                                submit_search();
                            },
                            input {
                                class: "topbar-search-input",
                                r#type: "search",
                                name: "q",
                                placeholder: "Search…",
                                "aria-label": "Search",
                                value: "{search_query}",
                                oninput: move |evt| search_query.set(evt.value()),
                            }
                            button { class: "topbar-search-submit", r#type: "submit", "Search" }
                        }
                    }
                    div { class: "user-menu",
                        LocaleSwitcher {}
                        ThemeToggle { value: theme_pref(), on_change: on_theme_change }
                        if show_workspace_tools {
                            NotificationBell {}
                        }
                        span { class: "user-avatar", aria_hidden: "true", "{user_initials}" }
                        div { class: "user-meta",
                            span { class: "user-name", "{props.user.display_name}" }
                            span { class: "user-email", "{props.user.email}" }
                        }
                        button { class: "linkish", onclick: move |_| on_signout(), "Sign out" }
                    }
                }
                section {
                    class: "app-content",
                    id: MAIN_CONTENT_ID,
                    // `tabindex=-1` lets the skip link move focus here (without
                    // adding it to the natural tab order).
                    tabindex: "-1",
                    {props.children}
                }
            }
        }
    }
}

/// Navigate to a same-origin path. Browsers preserve the established full-page
/// behavior; native WebViews use the Dioxus router because they have no web
/// origin to reload.
fn navigate_to(router: Option<dioxus_router::RouterContext>, href: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = router;
        if let Some(win) = web_sys::window() {
            let _ = win.location().set_href(href);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let (Some(router), Some(path)) = (
            router,
            platform_bridge::navigation::internal_route_path(href),
        ) {
            let _ = router.push(path);
        }
    }
}

fn reload_current_page() {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(window) = web_sys::window() {
            let _ = window.location().reload();
        }
    }
}

/// Return the browser-visible route including its query string. AppShell links
/// currently perform full same-origin navigations, so this value is stable for
/// the lifetime of the mounted shell and does not need a router subscription.
fn current_browser_path(_router: Option<dioxus_router::RouterContext>) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(window) = web_sys::window() else {
            return String::new();
        };
        let location = window.location();
        let pathname = location.pathname().unwrap_or_default();
        let search = location.search().unwrap_or_default();
        format!("{pathname}{search}")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        _router
            .map(|router| router.full_route_string())
            .unwrap_or_default()
    }
}

/// Navigation matching is deliberately conservative: only the course index
/// owns course descendants; admin home does not light up alongside every
/// specific admin tool, and the workspace-wide course query stays distinct
/// from the caller's regular course list.
pub fn nav_link_is_active(current: &str, href: &str) -> bool {
    if current.is_empty() {
        return false;
    }
    if href.contains('?') {
        return current.trim_end_matches('/') == href.trim_end_matches('/');
    }

    let current_path = current.split('?').next().unwrap_or(current);
    let current_path = if current_path.len() > 1 {
        current_path.trim_end_matches('/')
    } else {
        current_path
    };
    match href {
        "/" => current_path == "/",
        "/courses" => {
            !current.contains("scope=all")
                && (current_path == "/courses" || current_path.starts_with("/courses/"))
        }
        _ => current_path == href.trim_end_matches('/'),
    }
}

pub fn display_initials(name: &str) -> String {
    let words: Vec<&str> = name
        .split_whitespace()
        .filter(|word| !word.is_empty())
        .collect();
    let chars: Vec<char> = if words.len() > 1 {
        words
            .iter()
            .take(2)
            .filter_map(|word| word.chars().next())
            .collect()
    } else {
        words
            .first()
            .map(|word| word.chars().take(2).collect())
            .unwrap_or_default()
    };
    let initials: String = chars.into_iter().flat_map(char::to_uppercase).collect();
    if initials.is_empty() {
        "A".to_string()
    } else {
        initials
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    #[test]
    fn workspace_role_labels_distinguish_owner_and_admin() {
        assert_eq!(workspace_role_label("org_owner"), "Organization owner");
        assert_eq!(workspace_role_label("org_admin"), "Organization admin");
    }

    #[test]
    fn app_shell_renders_admin_menu_without_owner_only_billing() {
        fn app() -> Element {
            rsx! {
                AppShell {
                    user: ShellUser {
                        display_name: "Admin".into(),
                        email: "a@x".into(),
                        tenant_role: Some(core_types::TenantRole::OrgAdmin),
                        is_platform_admin: false,
                    },
                    on_signout: |_| {},
                    div { "child" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Workspace courses"));
        assert!(html.contains("child"));
        // Admin section links are present for an org-admin.
        assert!(html.contains("Admin home"));
        assert!(html.contains("/admin\""));
        // The onboarding "Set up workspace" link is in the admin section.
        assert!(
            html.contains("/onboarding\""),
            "onboarding nav link missing: {html}"
        );
        assert!(html.contains("Set up workspace"));
        assert!(html.contains("Audit log"));
        // Analytics link is in the admin section.
        assert!(html.contains("/admin/analytics\""));
        assert!(html.contains(">Analytics<"));
        // Billing changes the organization's commercial relationship and is
        // intentionally not exposed to an ordinary workspace administrator.
        assert!(!html.contains("/admin/billing\""));
        assert!(!html.contains(">Billing<"));
        // Branding link is in the admin section.
        assert!(html.contains("/admin/branding\""));
        assert!(html.contains(">Branding<"));
        // Integrations link is in the admin section.
        assert!(html.contains("/admin/integrations\""));
        assert!(html.contains(">Integrations<"));
        // An org admin who is NOT a platform admin must NOT see the Platform link.
        assert!(
            !html.contains("/platform\""),
            "org admin leaked platform link: {html}"
        );
        assert!(!html.contains("Platform owner console"));
    }

    #[test]
    fn app_shell_owner_sees_billing_and_owner_role() {
        fn app() -> Element {
            rsx! {
                AppShell {
                    user: ShellUser {
                        display_name: "Owner".into(),
                        email: "owner@x".into(),
                        tenant_role: Some(core_types::TenantRole::OrgOwner),
                        is_platform_admin: false,
                    },
                    on_signout: |_| {},
                    div { "owner dashboard" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("/admin/billing\""), "{html}");
        assert!(html.contains(">Billing<"), "{html}");
        assert!(html.contains("Admin home"), "{html}");
    }

    #[test]
    fn app_shell_renders_admin_menu_for_platform_admin() {
        // A platform admin whose tenant role is not OrgAdmin still sees Admin.
        fn app() -> Element {
            rsx! {
                AppShell {
                    user: ShellUser {
                        display_name: "PA".into(),
                        email: "pa@x".into(),
                        tenant_role: Some(core_types::TenantRole::Teacher),
                        is_platform_admin: true,
                    },
                    on_signout: |_| {},
                    div {}
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Admin home"));
        assert!(html.contains("Audit log"));
        // The platform super-admin link is shown to a platform admin.
        assert!(html.contains("Platform owner console"));
        assert!(html.contains("/platform\""));
    }

    #[test]
    fn platform_operator_without_workspace_sees_only_global_navigation() {
        fn app() -> Element {
            rsx! {
                AppShell {
                    user: ShellUser {
                        display_name: "Platform Operator".into(),
                        email: "operator@x".into(),
                        tenant_role: None,
                        is_platform_admin: true,
                    },
                    on_signout: |_| {},
                    div {}
                }
            }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Platform owner console"));
        assert!(!html.contains("Admin home"));
        assert!(!html.contains("My Courses"));
        assert!(!html.contains("Redeem Code"));
        assert!(!html.contains("href=\"/calendar\""));
        assert!(!html.contains("topbar-search"));
        assert!(!html.contains("topbar-cmdk"));
    }

    #[test]
    fn app_shell_renders_parent_menu() {
        fn app() -> Element {
            rsx! {
                AppShell {
                    user: ShellUser {
                        display_name: "Parent".into(),
                        email: "p@x".into(),
                        tenant_role: Some(core_types::TenantRole::Parent),
                        is_platform_admin: false,
                    },
                    on_signout: |_| {},
                    div {}
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        // Parent navigation stays focused on family information.
        assert!(html.contains("Family overview"));
        assert!(html.contains("/parent\""));
        assert!(!html.contains(">Dashboard<"));
        assert!(!html.contains("href=\"/calendar\""));
        assert!(!html.contains("topbar-search"));
        assert!(!html.contains("topbar-cmdk"));
        // A parent must NOT see the Admin section or learner-only links.
        assert!(!html.contains("/admin\""));
        assert!(!html.contains("Redeem Code"));
    }

    #[test]
    fn app_shell_renders_student_menu() {
        fn app() -> Element {
            rsx! {
                AppShell {
                    user: ShellUser {
                        display_name: "Stu".into(),
                        email: "s@x".into(),
                        tenant_role: Some(core_types::TenantRole::Student),
                        is_platform_admin: false,
                    },
                    on_signout: |_| {},
                    div {}
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Redeem Code"));
        assert!(!html.contains("Workspace courses"));
        // A student must NOT see the Admin section.
        assert!(!html.contains("/admin\""));
        assert!(!html.contains("Audit log"));
        // …nor the platform super-admin link.
        assert!(!html.contains("/platform\""));
    }

    #[test]
    fn app_shell_renders_global_search_box() {
        // The topbar search box is shown to every authed role.
        fn app() -> Element {
            rsx! {
                AppShell {
                    user: ShellUser {
                        display_name: "Stu".into(),
                        email: "s@x".into(),
                        tenant_role: Some(core_types::TenantRole::Student),
                        is_platform_admin: false,
                    },
                    on_signout: |_| {},
                    div {}
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("topbar-search"),
            "search form missing: {html}"
        );
        assert!(
            html.contains("topbar-search-input"),
            "search input missing: {html}"
        );
        assert!(
            html.contains("placeholder=\"Search…\""),
            "search placeholder missing: {html}"
        );
    }

    #[test]
    fn branding_override_css_emits_set_tokens_only() {
        // Primary set, accent unset → only the primary/brand/ui-primary tokens.
        let css = branding_override_css(&BrandingDto {
            logo_url: None,
            primary_color: Some("#112233".into()),
            accent_color: None,
        })
        .expect("override expected when primary is set");
        assert!(css.contains("--color-primary: #112233"), "got: {css}");
        assert!(css.contains("--color-primary-hover: #112233"));
        assert!(css.contains("--brand-primary: #112233"));
        assert!(css.contains("--brand-primary-hover: #112233"));
        assert!(css.contains("--ui-primary: #112233"));
        // Accent tokens must NOT appear when accent is unset.
        assert!(!css.contains("--color-accent"), "accent leaked: {css}");
        assert!(!css.contains("--ui-accent"), "accent leaked: {css}");

        // Accent set → accent tokens present.
        let css2 = branding_override_css(&BrandingDto {
            logo_url: None,
            primary_color: None,
            accent_color: Some("#aabbcc".into()),
        })
        .expect("override expected when accent is set");
        assert!(css2.contains("--color-accent: #aabbcc"));
        assert!(css2.contains("--color-accent-strong: #aabbcc"));
        assert!(css2.contains("--ui-accent: #aabbcc"));
        assert!(!css2.contains("--color-primary"), "primary leaked: {css2}");
    }

    #[test]
    fn branding_override_css_none_when_no_colors() {
        // No colors (logo only, or fully empty) → no override at all.
        assert!(branding_override_css(&BrandingDto {
            logo_url: Some("https://cdn.example.com/logo.svg".into()),
            primary_color: None,
            accent_color: None,
        })
        .is_none());
        // Whitespace-only values are treated as unset.
        assert!(branding_override_css(&BrandingDto {
            logo_url: None,
            primary_color: Some("   ".into()),
            accent_color: Some(String::new()),
        })
        .is_none());
    }

    #[test]
    fn app_shell_with_branding_override_emits_primary_override() {
        // Provide an ApiContext + a branding override directly via the pure
        // helper to assert the injected <style> carries the token override.
        // (The use_resource fetch yields None in SSR, so the shell relies on the
        // default theme; this asserts the override CSS the shell injects when a
        // branding DTO is present.)
        fn app() -> Element {
            let css = branding_override_css(&BrandingDto {
                logo_url: None,
                primary_color: Some("#7a00ff".into()),
                accent_color: None,
            });
            rsx! {
                if let Some(css) = css {
                    style { dangerous_inner_html: "{css}" }
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("<style"), "expected a style tag: {html}");
        assert!(
            html.contains("--color-primary: #7a00ff"),
            "expected primary override: {html}"
        );
    }

    #[test]
    fn app_shell_without_branding_renders_default_logo() {
        // No ApiContext provided → branding fetch fails → default AulaLogo and
        // no injected override style.
        fn app() -> Element {
            rsx! {
                AppShell {
                    user: ShellUser {
                        display_name: "Stu".into(),
                        email: "s@x".into(),
                        tenant_role: Some(core_types::TenantRole::Student),
                        is_platform_admin: false,
                    },
                    on_signout: |_| {},
                    div {}
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        // Default wordmark logo renders. AulaLogo now renders the emblem mark
        // plus a dark-mode-safe CSS-text wordmark lockup (not the old raster
        // wordmark img), so assert on the lockup wordmark class + the mark.
        assert!(
            html.contains("aula-logo__wordmark") && html.contains("aulalite-mark.svg"),
            "default logo missing: {html}"
        );
        // No tenant token override leaked into the page.
        assert!(
            !html.contains("--color-primary:"),
            "unexpected token override: {html}"
        );
    }

    #[test]
    fn nav_link_ids_are_stable_slugs() {
        assert_eq!(nav_link_id("My Courses"), "nav-my-courses");
        assert_eq!(nav_link_id("Members"), "nav-members");
        assert_eq!(nav_link_id("Audit log"), "nav-audit-log");
    }

    #[test]
    fn nav_matching_prefers_meaningful_product_sections() {
        assert!(nav_link_is_active("/", "/"));
        assert!(nav_link_is_active("/courses/algebra/schedule", "/courses"));
        assert!(!nav_link_is_active("/courses?scope=all", "/courses"));
        assert!(nav_link_is_active(
            "/courses?scope=all",
            "/courses?scope=all"
        ));
        assert!(!nav_link_is_active("/admin/analytics", "/admin"));
        assert!(nav_link_is_active("/admin/analytics", "/admin/analytics"));
    }

    #[test]
    fn display_initials_are_compact_and_resilient() {
        assert_eq!(display_initials("Aisha Malik"), "AM");
        assert_eq!(display_initials("Aula"), "AU");
        assert_eq!(display_initials("  "), "A");
    }

    #[test]
    fn multi_workspace_switcher_is_labeled_and_role_aware() {
        fn app() -> Element {
            rsx! {
                WorkspaceSwitcher {
                    workspaces: vec![
                        WorkspaceDto {
                            tenant_id: "11111111-1111-1111-1111-111111111111".into(),
                            name: "Malik Tutoring".into(),
                            slug: "malik-tutoring".into(),
                            role: "org_admin".into(),
                            current: true,
                        },
                        WorkspaceDto {
                            tenant_id: "22222222-2222-2222-2222-222222222222".into(),
                            name: "Northview School".into(),
                            slug: "northview".into(),
                            role: "teacher".into(),
                            current: false,
                        },
                    ]
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("aria-label=\"Switch workspace\""), "{html}");
        assert!(
            html.contains("Malik Tutoring · Organization admin"),
            "{html}"
        );
        assert!(html.contains("Northview School · Teacher"), "{html}");
    }

    #[test]
    fn single_workspace_renders_quiet_context_instead_of_a_control() {
        fn app() -> Element {
            rsx! {
                WorkspaceSwitcher {
                    workspaces: vec![WorkspaceDto {
                        tenant_id: "11111111-1111-1111-1111-111111111111".into(),
                        name: "My academy".into(),
                        slug: "my-academy".into(),
                        role: "org_admin".into(),
                        current: true,
                    }]
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("workspace-context--single"), "{html}");
        assert!(!html.contains("<select"), "{html}");
    }

    #[test]
    fn tour_steps_include_members_only_for_admins() {
        let teacher = tour_steps(false);
        assert!(teacher.iter().all(|s| s.id != "members"));
        // Welcome + courses + go-live + cmdk.
        assert_eq!(teacher.len(), 4);
        let admin = tour_steps(true);
        assert!(admin.iter().any(|s| s.id == "members"));
        assert_eq!(admin.len(), 5);
        // Targets reference the stable nav ids the shell renders.
        assert!(admin.iter().any(|s| s.target_id == "nav-my-courses"));
        assert!(admin.iter().any(|s| s.target_id == "topbar-cmdk"));
    }

    #[test]
    fn nav_links_render_with_tour_target_ids() {
        fn app() -> Element {
            rsx! {
                AppShell {
                    user: ShellUser {
                        display_name: "T".into(),
                        email: "t@x".into(),
                        tenant_role: Some(core_types::TenantRole::Teacher),
                        is_platform_admin: false,
                    },
                    on_signout: |_| {},
                    div {}
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("id=\"nav-my-courses\""),
            "nav id missing: {html}"
        );
        assert!(
            html.contains("id=\"topbar-cmdk\""),
            "cmdk id missing: {html}"
        );
        // The tour itself must NOT render before the preference fetch opens it.
        assert!(
            !html.contains("ui-tour"),
            "tour should be closed by default: {html}"
        );
    }
}
