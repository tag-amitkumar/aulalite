// crates/features-courses/src/command_palette.rs
//! Global command palette (Cmd-K / Ctrl-K).
//!
//! Mounted once by the `AppShell`, so the keyboard shortcut and the palette
//! are available on every authed screen. It owns an `open: Signal<bool>` and
//! registers a GLOBAL `document` `keydown` listener (wasm only):
//!
//! * `(metaKey || ctrlKey) + 'k'` toggles the palette (and `prevent_default`s
//!   so the browser's own Cmd-K / Ctrl-K — focus address bar — does not fire);
//! * `Escape` closes it.
//!
//! The `Closure` backing the listener is owned for the component's lifetime
//! (stored in a `use_hook` cell) and detached in `use_drop`, mirroring the
//! closure-ownership / teardown contract used by `live_room_socket` and
//! `notification_bell` so the listener is not dropped early and is removed on
//! unmount. Off-wasm everything is a no-op except the pure command model and
//! the rendered overlay (so SSR tests can force it open).
//!
//! Rendering uses the kinetics `CommandMenu` (re-exported as
//! `design_system::kinetics_ui::CommandMenu`). That component is fully
//! *controlled*: it does not filter internally, so this module owns the
//! query, the visible-selection, and a case-insensitive contains filter over
//! the role-aware command list. Selecting a command navigates by setting
//! `window.location.href` (the same same-origin convention the AppShell nav
//! `<a href>` links use) and closes the palette.

use core_types::TenantRole;
use design_system::kinetics_ui::{CommandGroup, CommandItem, CommandMenu};
use dioxus::prelude::*;

/// A single navigable destination in the palette.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Destination {
    /// Stable id; doubles as the navigation target (`window.location.href`).
    href: &'static str,
    label: &'static str,
    description: &'static str,
}

/// Builds the full, role-aware command model as `(group-label, destinations)`
/// pairs. Pure so it is unit-testable on the host target.
///
/// * `Navigate` — always shown.
/// * `Admin` — only for workspace owners/admins or a contextual platform
///   operator. Billing appears only for an owner/platform operator.
/// * `Account` — always shown.
fn build_destinations(
    tenant_role: Option<&TenantRole>,
    is_platform_admin: bool,
) -> Vec<(&'static str, Vec<Destination>)> {
    if tenant_role == Some(&TenantRole::Parent) && !is_platform_admin {
        return vec![
            (
                "Family",
                vec![Destination {
                    href: "/parent",
                    label: "Family overview",
                    description: "View linked children, grades, attendance, and schedule",
                }],
            ),
            (
                "Account",
                vec![Destination {
                    href: "/settings/notifications",
                    label: "Notification settings",
                    description: "Manage family update preferences",
                }],
            ),
        ];
    }

    let mut groups: Vec<(&'static str, Vec<Destination>)> = Vec::new();

    groups.push((
        "Navigate",
        vec![
            Destination {
                href: "/",
                label: "Dashboard",
                description: "Go to your dashboard",
            },
            Destination {
                href: "/courses",
                label: "My Courses",
                description: "Browse your courses",
            },
            Destination {
                href: "/schedule",
                label: "My Schedule",
                description: "View your upcoming sessions",
            },
            Destination {
                href: "/redeem",
                label: "Redeem code",
                description: "Enroll with an access code",
            },
        ],
    ));

    let is_owner = is_platform_admin || tenant_role == Some(&TenantRole::OrgOwner);
    let is_admin = is_owner || tenant_role == Some(&TenantRole::OrgAdmin);
    if is_admin {
        let mut destinations = vec![
            Destination {
                href: "/admin",
                label: "Admin home",
                description: "Administration overview",
            },
            Destination {
                href: "/admin/tenant",
                label: "Members",
                description: "Manage workspace members",
            },
            Destination {
                href: "/admin/analytics",
                label: "Analytics",
                description: "Workspace analytics",
            },
            Destination {
                href: "/admin/integrations",
                label: "Integrations",
                description: "API keys, webhooks, SSO, and LTI",
            },
            Destination {
                href: "/admin/audit",
                label: "Audit log",
                description: "Review audit events",
            },
            Destination {
                href: "/admin/files",
                label: "Files",
                description: "Workspace file library",
            },
        ];
        if is_owner {
            destinations.insert(
                3,
                Destination {
                    href: "/admin/billing",
                    label: "Billing",
                    description: "Plan and invoices",
                },
            );
        }
        groups.push(("Admin", destinations));
    }

    groups.push((
        "Account",
        vec![Destination {
            href: "/settings/notifications",
            label: "Notification settings",
            description: "Manage notification preferences",
        }],
    ));

    groups
}

/// Case-insensitive contains match over a destination's label + description.
/// An empty (or whitespace-only) query matches everything.
fn matches_query(dest: &Destination, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    dest.label.to_lowercase().contains(&q)
        || dest.description.to_lowercase().contains(&q)
        || dest.href.to_lowercase().contains(&q)
}

/// Converts the role-aware model into the kinetics `CommandGroup`/`CommandItem`
/// shapes, dropping any group whose items are all filtered out by `query`.
fn filtered_groups(model: &[(&'static str, Vec<Destination>)], query: &str) -> Vec<CommandGroup> {
    model
        .iter()
        .filter_map(|(label, dests)| {
            let items: Vec<CommandItem> = dests
                .iter()
                .filter(|d| matches_query(d, query))
                .map(|d| {
                    // The kinetics CommandMenu keys selection/navigation off
                    // the item `id`; we use the href so `on_select` receives
                    // the destination directly.
                    CommandItem::new(d.href, d.label, d.description)
                })
                .collect();
            if items.is_empty() {
                None
            } else {
                Some(CommandGroup::new(*label, items))
            }
        })
        .collect()
}

/// First item id across the filtered groups, or empty when nothing matches.
/// Used to keep a sensible default selection as the query narrows the list.
fn first_id(groups: &[CommandGroup]) -> String {
    groups
        .first()
        .and_then(|g| g.items.first())
        .map(|i| i.id.clone())
        .unwrap_or_default()
}

#[derive(Props, Clone, PartialEq)]
pub struct CommandPaletteProps {
    /// The viewer's tenant role (drives whether the Admin group is shown).
    #[props(default)]
    pub tenant_role: Option<TenantRole>,
    /// Platform admins always see the Admin group regardless of tenant role.
    #[props(default)]
    pub is_platform_admin: bool,
    /// SSR/test seam: force the palette open on first render. Defaults to
    /// closed (the real shortcut toggles `open` at runtime).
    #[props(default)]
    pub default_open: bool,
}

#[component]
pub fn CommandPalette(props: CommandPaletteProps) -> Element {
    let router = dioxus_router::try_router();
    let mut open = use_signal(|| props.default_open);
    let mut query = use_signal(String::new);
    let mut selected_id = use_signal(String::new);

    let tenant_role = props.tenant_role;
    let is_platform_admin = props.is_platform_admin;

    // Build (cheap, pure) the full role-aware model and the query-filtered view.
    let model = build_destinations(tenant_role.as_ref(), is_platform_admin);
    let q = query.read().clone();
    let groups = filtered_groups(&model, &q);

    // Keep the highlighted row valid: if it points at a now-filtered-out item
    // (or is empty), snap it to the first visible item.
    {
        let current = selected_id.read().clone();
        let still_visible = groups
            .iter()
            .any(|g| g.items.iter().any(|i| i.id == current));
        if !still_visible {
            let fallback = first_id(&groups);
            if *selected_id.read() != fallback {
                selected_id.set(fallback);
            }
        }
    }

    // ── Global document keydown listener (wasm only) ───────────────────────
    // Toggle on (meta||ctrl)+k; close on Escape. The Closure is owned for the
    // component lifetime in a hook cell and removed from the document in
    // `use_drop`, mirroring the closure-ownership pattern in
    // `live_room_socket` / `notification_bell` so it is not dropped early.
    #[cfg(target_arch = "wasm32")]
    {
        use std::cell::RefCell;
        use std::rc::Rc;
        use wasm_bindgen::closure::Closure;
        use wasm_bindgen::JsCast;

        type KeyClosure = Closure<dyn FnMut(web_sys::KeyboardEvent)>;
        // Stored as a cell so `use_drop` can take ownership back out and
        // detach it from the document during teardown.
        let listener: Rc<RefCell<Option<KeyClosure>>> = use_hook(|| Rc::new(RefCell::new(None)));

        let listener_setup = listener.clone();
        use_effect(move || {
            // Install exactly once per mount.
            if listener_setup.borrow().is_some() {
                return;
            }
            let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
                return;
            };
            let closure = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
                move |evt: web_sys::KeyboardEvent| {
                    let key = evt.key();
                    if (evt.meta_key() || evt.ctrl_key()) && key.eq_ignore_ascii_case("k") {
                        // Suppress the browser's native Cmd-K / Ctrl-K.
                        evt.prevent_default();
                        let next = !*open.read();
                        open.set(next);
                        if next {
                            // Fresh query each open so the list starts unfiltered.
                            query.set(String::new());
                        }
                    } else if key == "Escape" && *open.read() {
                        open.set(false);
                    }
                },
            );
            if doc
                .add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref())
                .is_ok()
            {
                *listener_setup.borrow_mut() = Some(closure);
            }
        });

        let listener_drop = listener.clone();
        use_drop(move || {
            if let Some(closure) = listener_drop.borrow_mut().take() {
                if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                    let _ = doc.remove_event_listener_with_callback(
                        "keydown",
                        closure.as_ref().unchecked_ref(),
                    );
                }
                // `closure` drops here, freeing the JS-side function.
            }
        });
    }

    let on_query = move |value: String| {
        query.set(value);
    };
    let on_selection_change = move |id: String| {
        selected_id.set(id);
    };
    let on_select = move |href: String| {
        navigate_to(router, &href);
        open.set(false);
    };
    let on_dismiss = move |_| {
        open.set(false);
    };

    rsx! {
        CommandMenu {
            id: "aula-command-palette".to_string(),
            open: *open.read(),
            query: q,
            selected_id: selected_id.read().clone(),
            empty_text: "No matching commands".to_string(),
            groups,
            on_query,
            on_select,
            on_selection_change,
            on_dismiss,
        }
    }
}

/// Navigate to a same-origin path with native-router parity.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_group_hidden_for_student() {
        let groups = build_destinations(Some(&TenantRole::Student), false);
        let labels: Vec<_> = groups.iter().map(|(l, _)| *l).collect();
        assert!(labels.contains(&"Navigate"));
        assert!(labels.contains(&"Account"));
        assert!(!labels.contains(&"Admin"));
    }

    #[test]
    fn admin_group_shown_for_org_admin() {
        let groups = build_destinations(Some(&TenantRole::OrgAdmin), false);
        assert!(groups.iter().any(|(l, _)| *l == "Admin"));
        assert!(!groups
            .iter()
            .flat_map(|(_, destinations)| destinations)
            .any(|destination| destination.href == "/admin/billing"));
    }

    #[test]
    fn owner_admin_group_includes_billing() {
        let groups = build_destinations(Some(&TenantRole::OrgOwner), false);
        assert!(groups
            .iter()
            .flat_map(|(_, destinations)| destinations)
            .any(|destination| destination.href == "/admin/billing"));
    }

    #[test]
    fn admin_group_shown_for_platform_admin_regardless_of_role() {
        // A teacher who is a platform admin still gets Admin.
        let groups = build_destinations(Some(&TenantRole::Teacher), true);
        assert!(groups.iter().any(|(l, _)| *l == "Admin"));
    }

    #[test]
    fn navigate_group_destinations_present() {
        let groups = build_destinations(None, false);
        let nav = &groups.iter().find(|(l, _)| *l == "Navigate").unwrap().1;
        let hrefs: Vec<_> = nav.iter().map(|d| d.href).collect();
        assert_eq!(hrefs, vec!["/", "/courses", "/schedule", "/redeem"]);
    }

    #[test]
    fn query_filter_is_case_insensitive_contains() {
        let d = Destination {
            href: "/admin/audit",
            label: "Audit log",
            description: "Review audit events",
        };
        assert!(matches_query(&d, ""));
        assert!(matches_query(&d, "   "));
        assert!(matches_query(&d, "AUDIT"));
        assert!(matches_query(&d, "log"));
        // Matches against the href too.
        assert!(matches_query(&d, "/admin"));
        assert!(!matches_query(&d, "billing"));
    }

    #[test]
    fn filtered_groups_drops_empty_groups() {
        let model = build_destinations(Some(&TenantRole::OrgAdmin), false);
        // A query that only matches a Navigate item should drop Admin/Account.
        let groups = filtered_groups(&model, "redeem");
        let labels: Vec<_> = groups.iter().map(|g| g.label.clone()).collect();
        assert_eq!(labels, vec!["Navigate".to_string()]);
        // And only the matching item survives within Navigate.
        assert_eq!(groups[0].items.len(), 1);
        assert_eq!(groups[0].items[0].id, "/redeem");
    }

    #[test]
    fn first_id_picks_first_visible_item() {
        let model = build_destinations(None, false);
        let groups = filtered_groups(&model, "");
        assert_eq!(first_id(&groups), "/");
        // No matches → empty.
        let none = filtered_groups(&model, "zzzz-nope");
        assert!(none.is_empty());
        assert_eq!(first_id(&none), "");
    }

    /// SSR: forced open, the palette renders its command labels and the
    /// kinetics dialog scaffolding (role/aria from the component).
    #[test]
    fn palette_renders_labels_when_forced_open() {
        fn app() -> Element {
            rsx! {
                CommandPalette {
                    tenant_role: Some(TenantRole::OrgAdmin),
                    is_platform_admin: false,
                    default_open: true,
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        // Navigate group + an admin-only label both present for an org admin.
        assert!(html.contains("Dashboard"), "missing Dashboard: {html}");
        assert!(html.contains("My Courses"), "missing My Courses: {html}");
        assert!(html.contains("Audit log"), "missing Audit log: {html}");
        assert!(
            html.contains("Notification settings"),
            "missing Notification settings: {html}"
        );
        // The kinetics CommandMenu renders an accessible dialog.
        assert!(
            html.contains("ui-command-menu"),
            "missing palette class: {html}"
        );
        assert!(
            html.contains("role=\"dialog\""),
            "missing dialog role: {html}"
        );
    }

    /// SSR: closed by default, the palette renders nothing (CommandMenu early
    /// returns when `open` is false).
    #[test]
    fn palette_renders_nothing_when_closed() {
        fn app() -> Element {
            rsx! {
                CommandPalette {
                    tenant_role: Some(TenantRole::Student),
                    is_platform_admin: false,
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            !html.contains("ui-command-menu"),
            "should be closed: {html}"
        );
    }

    /// SSR: a student forced open must NOT see the Admin group's labels.
    #[test]
    fn palette_hides_admin_for_student_when_open() {
        fn app() -> Element {
            rsx! {
                CommandPalette {
                    tenant_role: Some(TenantRole::Student),
                    is_platform_admin: false,
                    default_open: true,
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Dashboard"));
        assert!(
            !html.contains("Audit log"),
            "student saw admin item: {html}"
        );
    }
}
