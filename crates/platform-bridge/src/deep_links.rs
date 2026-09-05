//! Strict parsing and bounded delivery for native SSO/LTI callbacks.

#![cfg(not(target_arch = "wasm32"))]

use crate::BridgeError;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use url::Url;

const MAX_LINK_BYTES: usize = 16 * 1024;
const MAX_TOKEN_BYTES: usize = 12 * 1024;
const DEFAULT_APP_LINK_HOST: &str = "aula.elementors.guru";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackKind {
    Sso,
    Lti,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthCallback {
    pub kind: CallbackKind,
    pub token: Option<String>,
    pub error: Option<String>,
    pub next_path: String,
}

fn queue() -> &'static Mutex<VecDeque<AuthCallback>> {
    static QUEUE: OnceLock<Mutex<VecDeque<AuthCallback>>> = OnceLock::new();
    QUEUE.get_or_init(|| Mutex::new(VecDeque::with_capacity(4)))
}

fn notification_queue() -> &'static Mutex<VecDeque<String>> {
    static QUEUE: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
    QUEUE.get_or_init(|| Mutex::new(VecDeque::with_capacity(8)))
}

fn configured_host() -> String {
    let runtime = cfg!(debug_assertions)
        .then(|| std::env::var("AULALITE_APP_LINK_HOST").ok())
        .flatten()
        .filter(|value| !value.trim().is_empty());
    runtime
        .or_else(|| option_env!("AULALITE_APP_LINK_HOST").map(str::to_string))
        .unwrap_or_else(|| DEFAULT_APP_LINK_HOST.to_string())
        .trim()
        .to_ascii_lowercase()
}

pub fn safe_next_path(path: &str) -> bool {
    path.len() <= 2 * 1024
        && path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
}

fn param(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .chain(
            url.fragment()
                .into_iter()
                .flat_map(|fragment| url::form_urlencoded::parse(fragment.as_bytes())),
        )
        .find_map(|(name, value)| (name == key).then(|| value.into_owned()))
}

fn fragment_param(url: &Url, key: &str) -> Option<String> {
    url.fragment()
        .into_iter()
        .flat_map(|fragment| url::form_urlencoded::parse(fragment.as_bytes()))
        .find_map(|(name, value)| (name == key).then(|| value.into_owned()))
}

pub fn parse_auth_callback(raw: &str) -> Result<AuthCallback, BridgeError> {
    if raw.len() > MAX_LINK_BYTES || raw.chars().any(char::is_control) {
        return Err(BridgeError::Authentication(
            "The sign-in callback is invalid.".into(),
        ));
    }
    let url = Url::parse(raw)
        .map_err(|_| BridgeError::Authentication("The sign-in callback is invalid.".into()))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(BridgeError::Authentication(
            "The sign-in callback is invalid.".into(),
        ));
    }

    let kind = match (url.scheme(), url.host_str(), url.path()) {
        ("aulalite", Some("auth"), "/sso") => CallbackKind::Sso,
        ("aulalite", Some("auth"), "/lti") => CallbackKind::Lti,
        ("https", Some(host), "/sso/finish") if host.eq_ignore_ascii_case(&configured_host()) => {
            CallbackKind::Sso
        }
        ("https", Some(host), "/lti/landing") if host.eq_ignore_ascii_case(&configured_host()) => {
            CallbackKind::Lti
        }
        _ => {
            return Err(BridgeError::Authentication(
                "The callback does not belong to AulaLite.".into(),
            ))
        }
    };

    // Bearer material is accepted only from the URL fragment so the token is
    // not sent to servers, proxies, or ordinary query-string telemetry.
    if url.query_pairs().any(|(name, _)| name == "token") {
        return Err(BridgeError::Authentication(
            "The sign-in callback is invalid.".into(),
        ));
    }
    let token = fragment_param(&url, "token").filter(|value| !value.trim().is_empty());
    if token
        .as_ref()
        .is_some_and(|value| value.len() > MAX_TOKEN_BYTES)
    {
        return Err(BridgeError::Authentication(
            "The sign-in callback is invalid.".into(),
        ));
    }
    let error = param(&url, "error")
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.chars().take(240).collect());
    if token.is_some() == error.is_some() {
        return Err(BridgeError::Authentication(
            "The callback must contain exactly one result.".into(),
        ));
    }
    let default_next = match kind {
        CallbackKind::Sso => "/".to_string(),
        CallbackKind::Lti => "/courses".to_string(),
    };
    let next_path = param(&url, "next")
        .filter(|path| safe_next_path(path))
        .unwrap_or(default_next);

    Ok(AuthCallback {
        kind,
        token,
        error,
        next_path,
    })
}

/// Called by desktop process-argument bootstrap and by the Android/iOS host
/// callback. The queue is deliberately tiny; the newest callback wins if an OS
/// repeatedly redelivers an intent.
pub fn submit_deep_link_from_host(raw: &str) -> Result<(), BridgeError> {
    let callback = parse_auth_callback(raw)?;
    let mut pending = queue()
        .lock()
        .map_err(|_| BridgeError::Io("deep-link queue unavailable".into()))?;
    if pending.len() == 4 {
        pending.pop_front();
    }
    pending.push_back(callback);
    Ok(())
}

pub fn initialize_from_process_args(args: impl IntoIterator<Item = String>) {
    for arg in args.into_iter().skip(1) {
        if arg.starts_with("aulalite://") || arg.starts_with("https://") {
            let _ = submit_deep_link_from_host(&arg);
        }
    }
}

pub fn take_pending_auth_callback() -> Option<AuthCallback> {
    queue().lock().ok()?.pop_front()
}

pub fn pending_callback_route() -> Option<&'static str> {
    let pending = queue().lock().ok()?;
    match pending.front()?.kind {
        CallbackKind::Sso => Some("/sso/finish"),
        CallbackKind::Lti => Some("/lti/landing"),
    }
}

/// Queue a route supplied by a trusted native notification host. The native
/// SDK is still treated as an input boundary: only bounded, same-application
/// paths can reach the router and the newest eight taps are retained.
pub fn submit_notification_route_from_host(raw: &str) -> Result<(), BridgeError> {
    if raw.len() > 2 * 1024 || raw.contains('\\') {
        return Err(BridgeError::Configuration(
            "The notification destination is invalid.".into(),
        ));
    }
    let route = crate::navigation::internal_route_path(raw).ok_or_else(|| {
        BridgeError::Configuration("The notification destination is invalid.".into())
    })?;
    let mut pending = notification_queue()
        .lock()
        .map_err(|_| BridgeError::Io("notification route queue unavailable".into()))?;
    if pending.len() == 8 {
        pending.pop_front();
    }
    pending.push_back(route);
    Ok(())
}

pub fn take_pending_notification_route() -> Option<String> {
    notification_queue().lock().ok()?.pop_front()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_custom_scheme_sso_and_lti_callbacks() {
        let sso = parse_auth_callback("aulalite://auth/sso#token=a.b.c").unwrap();
        assert_eq!(sso.kind, CallbackKind::Sso);
        assert_eq!(sso.token.as_deref(), Some("a.b.c"));
        assert_eq!(sso.next_path, "/");

        let lti =
            parse_auth_callback("aulalite://auth/lti?next=%2Fcourses%2Falgebra#token=launch-token")
                .unwrap();
        assert_eq!(lti.kind, CallbackKind::Lti);
        assert_eq!(lti.next_path, "/courses/algebra");
    }

    #[test]
    fn rejects_untrusted_hosts_open_redirects_and_ambiguous_results() {
        assert!(parse_auth_callback("https://evil.example/sso/finish#token=x").is_err());
        assert!(parse_auth_callback("aulalite://auth/sso#token=x&error=nope").is_err());
        assert!(parse_auth_callback("aulalite://auth/sso?token=query-token").is_err());
        let callback =
            parse_auth_callback("aulalite://auth/lti?next=https%3A%2F%2Fevil.example#token=x")
                .unwrap();
        assert_eq!(callback.next_path, "/courses");
    }

    #[test]
    fn safe_next_path_is_same_origin_only() {
        assert!(safe_next_path("/courses/math?tab=lessons"));
        assert!(!safe_next_path("//evil.example"));
        assert!(!safe_next_path("/\\evil.example"));
        assert!(!safe_next_path("https://evil.example"));
    }

    #[test]
    fn notification_routes_are_bounded_and_same_origin() {
        submit_notification_route_from_host(" /courses/algebra?tab=lessons ").unwrap();
        assert_eq!(
            take_pending_notification_route().as_deref(),
            Some("/courses/algebra?tab=lessons")
        );
        assert!(submit_notification_route_from_host("https://evil.example").is_err());
        assert!(submit_notification_route_from_host("//evil.example/path").is_err());
        assert!(submit_notification_route_from_host("/courses\\evil").is_err());
        assert!(submit_notification_route_from_host(&format!("/{}", "x".repeat(2049))).is_err());
    }
}
