// crates/shell-web/src/routes/sso_finish.rs
//! Landing page for the enterprise-SSO (OIDC) login round-trip.
//!
//! The backend callback (`/v1/sso/callback`) 302s the browser to
//! `{app_origin}/sso/finish#token=<jwt>` on success, or
//! `{app_origin}/sso/finish?error=<reason>` on failure. This page:
//!   * (wasm) reads the `#token=` URL fragment, stores it as the bearer
//!     credential exactly where the dev/local-login path stores its token
//!     (`platform_bridge::web::persist_local_token`, key `aulalite.local_token`),
//!     pushes it into the live `ApiContext` signal, fetches `/v1/me`, then routes
//!     to `/`.
//!   * On `?error=`, shows a brief message + a link back to `/login`.
//!
//! The token rides in the URL FRAGMENT (never sent to the server / no Referer
//! leak); we clear it from the address bar after reading it.

use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::api::ApiContext;

// `UserContext` + `Route` are referenced only inside the wasm-only branch, so on
// the SSR/native target they would otherwise read as unused.
#[allow(unused_imports)]
use crate::contexts::UserContext;
use crate::contexts::UserContextSignal;
#[allow(unused_imports)]
use crate::route_enum::Route;

#[component]
pub fn SsoFinish() -> Element {
    // Read contexts via try_consume_context so SSR/test renders (which provide
    // no context) don't panic.
    let api_signal = try_consume_context::<Signal<ApiContext>>();
    let user_signal = try_consume_context::<UserContextSignal>();
    let nav = use_navigator();

    // The error reason parsed from `?error=` (if any). When set, we render the
    // failure UI instead of the "signing you in" splash.
    let mut error = use_signal(|| Option::<String>::None);
    let mut started = use_signal(|| false);

    use_future(move || async move {
        if *started.read() {
            return;
        }
        started.set(true);

        #[cfg(target_arch = "wasm32")]
        {
            // Surface a server-reported error first.
            if let Some(reason) = query_param_for("error") {
                error.set(Some(humanize_error(&reason)));
                return;
            }

            let token = match fragment_param_for("token") {
                Some(t) if !t.is_empty() => t,
                _ => {
                    error.set(Some("No sign-in token was returned.".into()));
                    return;
                }
            };

            // Clear the fragment from the address bar so the token isn't left in
            // the URL/history. Best-effort.
            clear_url_fragment();

            // Persist where the bootstrap path reads it (survives reloads), then
            // populate the live ApiContext so the very next request is authed.
            platform_bridge::web::persist_local_token(&token);

            let ctx = ApiContext {
                base_url: api::web_api_base_url(),
                id_token: token,
            };
            if let Some(mut api_signal) = api_signal {
                api_signal.set(ctx.clone());
            }
            match api::get_me(&ctx).await {
                Ok(dto) => {
                    if let Some(mut user_signal) = user_signal {
                        user_signal.set(Some(UserContext::from_dto(dto)));
                    }
                    nav.push(Route::Dashboard {});
                }
                Err(_) => {
                    platform_bridge::web::clear_local_token();
                    error.set(Some(
                        "We couldn't complete sign-in. Please try again.".into(),
                    ));
                }
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            use platform_bridge::PlatformBridge;

            let callback = match platform_bridge::deep_links::take_pending_auth_callback() {
                Some(callback)
                    if callback.kind == platform_bridge::deep_links::CallbackKind::Sso =>
                {
                    callback
                }
                _ => {
                    error.set(Some("No sign-in callback was returned.".into()));
                    return;
                }
            };
            if let Some(reason) = callback.error {
                error.set(Some(humanize_error(&reason)));
                return;
            }
            let Some(token) = callback.token else {
                error.set(Some("No sign-in token was returned.".into()));
                return;
            };
            if platform_bridge::native::NativeBridge::persist_backend_callback_token(&token)
                .is_err()
            {
                error.set(Some(
                    "The sign-in response was invalid. Please try again.".into(),
                ));
                return;
            }
            let ctx = ApiContext {
                base_url: api::native_api_base_url(),
                id_token: token,
            };
            if let Some(mut api_signal) = api_signal {
                api_signal.set(ctx.clone());
            }
            match api::get_me(&ctx).await {
                Ok(dto) => {
                    if let Some(mut user_signal) = user_signal {
                        user_signal.set(Some(UserContext::from_dto(dto)));
                    }
                    nav.replace(callback.next_path);
                }
                Err(_) => {
                    let _ = platform_bridge::native::NativeBridge.sign_out().await;
                    error.set(Some(
                        "We couldn't complete sign-in. Please try again.".into(),
                    ));
                }
            }
        }
    });

    rsx! {
        div { class: "auth-composite motion-page",
            div { class: "auth-panel-stack",
                if let Some(message) = error.read().clone() {
                    div { class: "auth-panel", role: "alert",
                        h2 { "Sign-in failed" }
                        p { "{message}" }
                        a { href: "/login", class: "btn btn-primary", "Back to sign in" }
                    }
                } else {
                    div { class: "app-auth-splash", "aria-busy": "true",
                        span { class: "app-auth-splash-mark", "Aula", span { class: "gold", "Lite" } }
                        p { "Signing you in…" }
                    }
                }
            }
        }
    }
}

/// Read a query-string parameter (`?key=value`) from the current location.
/// Shared with `lti_landing`.
#[cfg(target_arch = "wasm32")]
pub(crate) fn query_param_for(key: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    parse_param(search.trim_start_matches('?'), key)
}

/// Read a URL-fragment parameter (`#key=value`) from the current location.
/// Shared with `lti_landing`.
#[cfg(target_arch = "wasm32")]
pub(crate) fn fragment_param_for(key: &str) -> Option<String> {
    let hash = web_sys::window()?.location().hash().ok()?;
    parse_param(hash.trim_start_matches('#'), key)
}

/// Remove the sensitive fragment from the current history entry. `set_hash`
/// would push a second entry and leave the bearer reachable via Back.
#[cfg(target_arch = "wasm32")]
pub(crate) fn clear_url_fragment() {
    if let Some(win) = web_sys::window() {
        let location = win.location();
        let clean_url = format!(
            "{}{}",
            location.pathname().unwrap_or_else(|_| "/".into()),
            location.search().unwrap_or_default()
        );
        if let Ok(history) = win.history() {
            let _ =
                history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&clean_url));
        }
    }
}

/// Parse `key=value` out of an `&`-separated, URL-encoded parameter string.
/// Shared by the query + fragment readers; pure + unit-testable.
fn parse_param(params: &str, key: &str) -> Option<String> {
    params.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        if k != key {
            return None;
        }
        Some(percent_decode(v))
    })
}

/// Minimal percent-decode (also turns '+' into space, per form-encoding).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_val(bytes[i + 1]);
                let lo = hex_val(bytes[i + 2]);
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push((h << 4) | l);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Turn a raw IdP/SSO error code into a friendlier sentence (best-effort; falls
/// back to the raw reason).
fn humanize_error(reason: &str) -> String {
    match reason {
        "access_denied" => "Access was denied at your identity provider.".into(),
        "" => "Sign-in did not complete.".into(),
        other => format!("Sign-in failed: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_param_from_fragment_or_query() {
        assert_eq!(
            parse_param("token=abc.def", "token").as_deref(),
            Some("abc.def")
        );
        assert_eq!(
            parse_param("error=access_denied", "error").as_deref(),
            Some("access_denied")
        );
        assert_eq!(
            parse_param("a=1&token=xyz&b=2", "token").as_deref(),
            Some("xyz")
        );
        assert_eq!(parse_param("a=1", "token"), None);
    }

    #[test]
    fn percent_decodes_values() {
        assert_eq!(percent_decode("access%20denied"), "access denied");
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("plain"), "plain");
        // Malformed escapes are passed through verbatim.
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn humanize_maps_known_codes() {
        assert!(humanize_error("access_denied").contains("denied"));
        assert!(humanize_error("weird").contains("weird"));
    }
}
