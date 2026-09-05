// crates/backend/src/services/ice.rs
//! ICE server discovery for WebRTC live sessions.
//!
//! WebRTC clients need a list of STUN/TURN servers to gather connectivity
//! candidates. STUN lets a peer discover its public-facing address so two peers
//! behind ordinary NATs can connect directly; TURN is a relay of last resort
//! for symmetric NATs / restrictive firewalls where a direct path is
//! impossible.
//!
//! This module builds the server list the backend hands to clients in the
//! go-live and join responses. It is pure + side-effect-free apart from reading
//! process env, so it is trivially unit-testable.
//!
//! Policy:
//!   * A public Google STUN server is ALWAYS included so the common
//!     direct-connect case works with zero ops setup (this mirrors the STUN
//!     default the wasm clients already hard-code, so behavior is unchanged
//!     when no TURN relay is configured).
//!   * A TURN entry is appended ONLY when `AULALITE_TURN_URL` is set. Its
//!     `AULALITE_TURN_USERNAME` / `AULALITE_TURN_CREDENTIAL` are optional
//!     (long-term-credential TURN servers need them; some deployments use
//!     other auth and leave them unset).
//!   * `AULALITE_TURN_URL` may be a COMMA-SEPARATED list of relay URLs so a
//!     single deployment can advertise both a TLS relay (`turns:…:443`) and a
//!     plain-TCP relay (`turn:…:443?transport=tcp`) — the two forms that
//!     traverse a Cloudflare Tunnel / restrictive firewall. All URLs in the
//!     list share the one username/credential pair (the W3C `RTCIceServer`
//!     shape, where `urls` is itself an array). Each URL's scheme is validated
//!     against `stun:`/`stuns:`/`turn:`/`turns:`; unrecognized entries are
//!     dropped rather than shipped to the browser as a malformed URL.
//!
//! The serialized shape is the W3C `RTCIceServer` dictionary
//! (`{ urls, username?, credential? }`) so the client can feed the list
//! straight into `new RTCPeerConnection({ iceServers })` with no remapping.

use serde::{Deserialize, Serialize};

/// The default public STUN server. Kept in sync with the client-side fallback
/// in `features-courses`'s WHIP/WHEP modules.
pub const DEFAULT_STUN_URL: &str = "stun:stun.l.google.com:19302";

const ENV_TURN_URL: &str = "AULALITE_TURN_URL";
const ENV_TURN_USERNAME: &str = "AULALITE_TURN_USERNAME";
const ENV_TURN_CREDENTIAL: &str = "AULALITE_TURN_CREDENTIAL";

/// A single ICE server entry, serialized as the W3C `RTCIceServer` dictionary.
///
/// `urls` is always a list (the spec also allows a bare string, but a list is
/// universally accepted and keeps the type simple). `username` / `credential`
/// are omitted from the JSON when absent so a STUN-only entry stays clean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceServer {
    pub urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub credential: Option<String>,
}

impl IceServer {
    /// A STUN entry (no credentials).
    pub fn stun(url: impl Into<String>) -> Self {
        Self {
            urls: vec![url.into()],
            username: None,
            credential: None,
        }
    }

    /// A TURN entry with optional long-term credentials.
    pub fn turn(
        url: impl Into<String>,
        username: Option<String>,
        credential: Option<String>,
    ) -> Self {
        Self {
            urls: vec![url.into()],
            username,
            credential,
        }
    }

    /// A TURN entry holding one or more relay URLs that share one credential
    /// pair (e.g. a `turns:` + `turn:` pair). Serialized as a single
    /// `RTCIceServer` whose `urls` array carries every relay URL.
    pub fn turn_multi(
        urls: Vec<String>,
        username: Option<String>,
        credential: Option<String>,
    ) -> Self {
        Self {
            urls,
            username,
            credential,
        }
    }
}

/// True when `url` starts with a scheme WebRTC recognizes for an ICE server.
/// Anything else (a bare host, an `https://` typo, an empty fragment left by a
/// trailing comma) is rejected so it never reaches the browser as a malformed
/// `RTCIceServer` URL that aborts ICE gathering.
fn is_valid_ice_scheme(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ["stun:", "stuns:", "turn:", "turns:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}

/// Build the ICE server list from the process environment.
///
/// Always returns at least the public STUN default; appends a TURN entry when
/// `AULALITE_TURN_URL` is set (with optional username/credential).
pub fn ice_servers() -> Vec<IceServer> {
    from_env(|k| std::env::var(k).ok())
}

/// Testable core: `lookup` resolves an env var name to its value. Pulled out so
/// unit tests can drive it without touching the real process environment (which
/// is global and racy across parallel tests).
fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Vec<IceServer> {
    let mut servers = vec![IceServer::stun(DEFAULT_STUN_URL)];

    // A TURN URL with only whitespace is treated as unset. The value may be a
    // comma-separated list; each entry is trimmed, blanks are dropped, and any
    // entry without a recognized ICE scheme is discarded (rather than shipped
    // to the browser as a malformed URL that would abort ICE gathering).
    if let Some(raw) = lookup(ENV_TURN_URL).filter(|s| !s.trim().is_empty()) {
        let urls: Vec<String> = raw
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .filter(|s| is_valid_ice_scheme(s))
            .map(|s| s.to_string())
            .collect();

        if !urls.is_empty() {
            let username = lookup(ENV_TURN_USERNAME).filter(|s| !s.trim().is_empty());
            let credential = lookup(ENV_TURN_CREDENTIAL).filter(|s| !s.trim().is_empty());
            servers.push(IceServer::turn_multi(urls, username, credential));
        }
    }

    servers
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup_from<'a>(map: &'a HashMap<&str, &str>) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| map.get(k).map(|v| v.to_string())
    }

    #[test]
    fn stun_only_when_no_turn_configured() {
        let env: HashMap<&str, &str> = HashMap::new();
        let servers = from_env(lookup_from(&env));
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].urls, vec![DEFAULT_STUN_URL.to_string()]);
        assert!(servers[0].username.is_none());
        assert!(servers[0].credential.is_none());
    }

    #[test]
    fn appends_turn_with_credentials() {
        let env = HashMap::from([
            (ENV_TURN_URL, "turn:turn.example.com:3478"),
            (ENV_TURN_USERNAME, "alice"),
            (ENV_TURN_CREDENTIAL, "s3cret"),
        ]);
        let servers = from_env(lookup_from(&env));
        assert_eq!(servers.len(), 2);
        // STUN default still first.
        assert_eq!(servers[0].urls, vec![DEFAULT_STUN_URL.to_string()]);
        // TURN appended with creds.
        assert_eq!(
            servers[1].urls,
            vec!["turn:turn.example.com:3478".to_string()]
        );
        assert_eq!(servers[1].username.as_deref(), Some("alice"));
        assert_eq!(servers[1].credential.as_deref(), Some("s3cret"));
    }

    #[test]
    fn turn_without_credentials_is_allowed() {
        let env = HashMap::from([(ENV_TURN_URL, "turn:turn.example.com:3478")]);
        let servers = from_env(lookup_from(&env));
        assert_eq!(servers.len(), 2);
        assert!(servers[1].username.is_none());
        assert!(servers[1].credential.is_none());
    }

    #[test]
    fn blank_turn_url_is_treated_as_unset() {
        let env = HashMap::from([(ENV_TURN_URL, "   ")]);
        let servers = from_env(lookup_from(&env));
        assert_eq!(servers.len(), 1, "blank TURN url must not add an entry");
    }

    #[test]
    fn turn_url_is_trimmed() {
        let env = HashMap::from([(ENV_TURN_URL, "  turn:turn.example.com:3478  ")]);
        let servers = from_env(lookup_from(&env));
        assert_eq!(
            servers[1].urls,
            vec!["turn:turn.example.com:3478".to_string()]
        );
    }

    #[test]
    fn comma_separated_turn_urls_share_one_credential() {
        // Production shape: a TLS relay AND a plain-TCP relay on 443, both of
        // which can traverse a Cloudflare Tunnel / restrictive firewall.
        let env = HashMap::from([
            (
                ENV_TURN_URL,
                "turns:relay.example.com:443?transport=tcp,turn:relay.example.com:443?transport=tcp",
            ),
            (ENV_TURN_USERNAME, "u"),
            (ENV_TURN_CREDENTIAL, "p"),
        ]);
        let servers = from_env(lookup_from(&env));
        assert_eq!(servers.len(), 2, "STUN default + one multi-URL TURN entry");
        assert_eq!(
            servers[1].urls,
            vec![
                "turns:relay.example.com:443?transport=tcp".to_string(),
                "turn:relay.example.com:443?transport=tcp".to_string(),
            ]
        );
        assert_eq!(servers[1].username.as_deref(), Some("u"));
        assert_eq!(servers[1].credential.as_deref(), Some("p"));
    }

    #[test]
    fn turns_tls_scheme_is_accepted() {
        let env = HashMap::from([(ENV_TURN_URL, "turns:relay.example.com:5349")]);
        let servers = from_env(lookup_from(&env));
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[1].urls, vec!["turns:relay.example.com:5349"]);
    }

    #[test]
    fn invalid_schemes_and_blanks_are_dropped() {
        // Trailing comma (empty entry), an https:// typo, and a bare host must
        // all be discarded; only the one valid turn: URL survives.
        let env = HashMap::from([(
            ENV_TURN_URL,
            "turn:good.example.com:3478, ,https://bad.example.com,relay.example.com",
        )]);
        let servers = from_env(lookup_from(&env));
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[1].urls, vec!["turn:good.example.com:3478"]);
    }

    #[test]
    fn all_invalid_turn_urls_yield_stun_only() {
        let env = HashMap::from([(ENV_TURN_URL, "https://not-a-turn-url.example.com, ,")]);
        let servers = from_env(lookup_from(&env));
        assert_eq!(
            servers.len(),
            1,
            "no valid TURN URL means STUN-only, not a malformed entry"
        );
    }

    #[test]
    fn serializes_to_rtc_ice_server_shape() {
        // STUN-only entry: username/credential omitted from JSON.
        let stun = IceServer::stun(DEFAULT_STUN_URL);
        let v = serde_json::to_value(&stun).unwrap();
        assert_eq!(v["urls"], serde_json::json!([DEFAULT_STUN_URL]));
        assert!(v.get("username").is_none(), "username must be omitted: {v}");
        assert!(
            v.get("credential").is_none(),
            "credential must be omitted: {v}"
        );

        // TURN entry with creds: all three fields present.
        let turn = IceServer::turn(
            "turn:turn.example.com:3478",
            Some("alice".into()),
            Some("s3cret".into()),
        );
        let v = serde_json::to_value(&turn).unwrap();
        assert_eq!(v["urls"], serde_json::json!(["turn:turn.example.com:3478"]));
        assert_eq!(v["username"], "alice");
        assert_eq!(v["credential"], "s3cret");
    }
}
