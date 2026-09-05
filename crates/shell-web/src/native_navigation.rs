//! Native WebView navigation adapter.
//!
//! The shared UI intentionally keeps semantic anchors for accessibility and
//! browser behavior. Dioxus native renderers do not route raw anchors, so this
//! layout intercepts only ordinary, same-app link activations and forwards
//! them to the type-aware router. External URLs, downloads, targets, fragments,
//! and modified/middle clicks retain their platform defaults.

use dioxus::prelude::*;
use dioxus_router::Outlet;

#[cfg(not(target_arch = "wasm32"))]
use crate::contexts::UserContext;
use crate::contexts::UserContextSignal;
use crate::route_enum::Route;

#[cfg(not(target_arch = "wasm32"))]
const NATIVE_LINK_BRIDGE_SCRIPT: &str = r#"
if (window.__aulaNativeLinkHandler) {
    document.removeEventListener("click", window.__aulaNativeLinkHandler, true);
}
window.__aulaNativeLinkHandler = (event) => {
    if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
        return;
    }
    const origin = event.target;
    const anchor = origin instanceof Element ? origin.closest("a[href]") : null;
    if (!anchor || anchor.hasAttribute("download")) {
        return;
    }
    const target = (anchor.getAttribute("target") || "").toLowerCase();
    if (target && target !== "_self") {
        return;
    }
    const href = (anchor.getAttribute("href") || "").trim();
    if (!href.startsWith("/") || href.startsWith("//") || /[\u0000-\u001f\u007f]/.test(href)) {
        return;
    }
    event.preventDefault();
    dioxus.send(href);
};
document.addEventListener("click", window.__aulaNativeLinkHandler, true);
await new Promise(() => {});
"#;

#[component]
pub fn NativeNavigationLayout() -> Element {
    let workspace_epoch = use_signal(|| 0_u64);
    let mut signout = crate::routes::use_signout_action();
    use_context_provider(|| {
        features_courses::app_shell::AppShellSignOut(EventHandler::new(move |_: ()| signout()))
    });

    let _router = dioxus_router::router();
    let _api_signal = try_consume_context::<Signal<features_courses::api::ApiContext>>();
    let _user_signal = try_consume_context::<UserContextSignal>();
    use_context_provider(|| {
        features_courses::app_shell::AppShellWorkspaceSwitch(EventHandler::new(
            move |tenant_id: String| {
                #[cfg(not(target_arch = "wasm32"))]
                let previous = features_courses::api::selected_workspace_id();
                features_courses::api::set_selected_workspace_id(Some(&tenant_id));

                #[cfg(target_arch = "wasm32")]
                {
                    if let Some(window) = web_sys::window() {
                        let _ = window.location().reload();
                    }
                }

                #[cfg(not(target_arch = "wasm32"))]
                if let (Some(api_signal), Some(mut user_signal)) = (_api_signal, _user_signal) {
                    let mut workspace_epoch = workspace_epoch;
                    spawn(async move {
                        let ctx = api_signal.read().clone();
                        match features_courses::api::get_me(&ctx).await {
                            Ok(dto) => user_signal.set(Some(UserContext::from_dto(dto))),
                            Err(_) => features_courses::api::set_selected_workspace_id(
                                previous.as_deref(),
                            ),
                        }
                        // The keyed outlet forces every workspace-scoped
                        // resource to remount after the refreshed permission
                        // context is installed (and also restores the picker
                        // after a failed transition). Moving through a stable
                        // route gives each role a deterministic landing page.
                        workspace_epoch.with_mut(|epoch| *epoch = epoch.wrapping_add(1));
                        let _ = _router.replace(Route::Dashboard {});
                    });
                }
            },
        ))
    });

    #[cfg(not(target_arch = "wasm32"))]
    {
        use_future(move || async move {
            let mut bridge = document::eval(NATIVE_LINK_BRIDGE_SCRIPT);
            while let Ok(href) = bridge.recv::<String>().await {
                if let Some(path) = platform_bridge::navigation::internal_route_path(&href) {
                    let _ = _router.push(path);
                }
            }
        });

        // Cold-start arguments and foreground Android/iOS URL callbacks land
        // in the bounded platform queue. Polling is intentionally lightweight
        // and avoids making the host callback hold a renderer-specific handle.
        use_future(move || async move {
            loop {
                if let Some(path) = platform_bridge::deep_links::pending_callback_route() {
                    let _ = _router.replace(path);
                } else if let Some(path) =
                    platform_bridge::deep_links::take_pending_notification_route()
                {
                    let _ = _router.replace(path);
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });
    }

    let epoch = *workspace_epoch.read();
    rsx! {
        div {
            key: "workspace-route-{epoch}",
            style: "display: contents",
            Outlet::<Route> {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_script_preserves_special_link_semantics() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            for guard in [
                "event.defaultPrevented",
                "event.button !== 0",
                "event.metaKey",
                "event.ctrlKey",
                "event.shiftKey",
                "event.altKey",
                "hasAttribute(\"download\")",
                "target !== \"_self\"",
                "href.startsWith(\"//\")",
            ] {
                assert!(NATIVE_LINK_BRIDGE_SCRIPT.contains(guard), "missing {guard}");
            }
        }
    }
}
