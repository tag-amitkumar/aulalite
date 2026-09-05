//! Safe native hand-off to the operating system browser.

#![cfg(not(target_arch = "wasm32"))]

use crate::BridgeError;
use url::Url;

/// Why an external browser is being opened. Keeping this explicit makes it
/// harder for future call sites to turn an arbitrary backend string into an
/// open redirect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalPurpose {
    Billing,
    FederatedSignIn,
    LearningContent,
    Support,
}

fn configured_hosts(name: &str, defaults: &[&str]) -> Vec<String> {
    let mut hosts: Vec<String> = defaults.iter().map(|host| (*host).to_string()).collect();
    if let Ok(configured) = std::env::var(name) {
        hosts.extend(
            configured
                .split(',')
                .map(str::trim)
                .filter(|host| !host.is_empty())
                .map(str::to_ascii_lowercase),
        );
    }
    hosts
}

fn host_matches(host: &str, allowed: &str) -> bool {
    allowed
        .strip_prefix("*.")
        .is_some_and(|suffix| host.ends_with(&format!(".{suffix}")))
        || host == allowed
}

fn purpose_allows_host(purpose: ExternalPurpose, host: &str) -> bool {
    let mut allowed = match purpose {
        ExternalPurpose::Billing => configured_hosts(
            "AULALITE_BILLING_HOSTS",
            &["checkout.stripe.com", "billing.stripe.com"],
        ),
        ExternalPurpose::FederatedSignIn => {
            configured_hosts("AULALITE_SSO_HOSTS", &["aula.elementors.guru"])
        }
        ExternalPurpose::LearningContent => configured_hosts(
            "AULALITE_CONTENT_HOSTS",
            &["aula.elementors.guru", "storage.elementors.guru"],
        ),
        ExternalPurpose::Support => configured_hosts(
            "AULALITE_SUPPORT_HOSTS",
            &["elementors.guru", "*.elementors.guru"],
        ),
    };
    if purpose == ExternalPurpose::LearningContent {
        if let Ok(origin) = Url::parse(&crate::native::api_base_url()) {
            if let Some(api_host) = origin.host_str() {
                allowed.push(api_host.to_ascii_lowercase());
            }
        }
    }
    allowed.iter().any(|allowed| host_matches(host, allowed))
}

pub fn validated_external_url(raw: &str, purpose: ExternalPurpose) -> Result<Url, BridgeError> {
    if raw.len() > 8 * 1024 || raw.chars().any(char::is_control) {
        return Err(BridgeError::Configuration(
            "The external destination is invalid.".into(),
        ));
    }
    let parsed = Url::parse(raw)
        .map_err(|_| BridgeError::Configuration("The external destination is invalid.".into()))?;
    let local_debug_http = cfg!(debug_assertions)
        && parsed.scheme() == "http"
        && matches!(
            parsed.host_str(),
            Some("localhost" | "127.0.0.1" | "10.0.2.2")
        );
    if (parsed.scheme() != "https" && !local_debug_http)
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(BridgeError::Configuration(
            "External destinations must use HTTPS and cannot contain credentials.".into(),
        ));
    }
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if !local_debug_http && !purpose_allows_host(purpose, &host) {
        return Err(BridgeError::Configuration(
            "This external destination is not approved for that action.".into(),
        ));
    }
    Ok(parsed)
}

pub fn open_external_url(raw: &str, purpose: ExternalPurpose) -> Result<(), BridgeError> {
    let url = validated_external_url(raw, purpose)?;
    webbrowser::open(url.as_str())
        .map(|_| ())
        .map_err(|_| BridgeError::Io("The system browser could not be opened.".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_https_without_credentials() {
        let url = validated_external_url(
            "https://checkout.stripe.com/c/pay/cs_test_123#safe",
            ExternalPurpose::Billing,
        )
        .unwrap();
        assert_eq!(url.host_str(), Some("checkout.stripe.com"));
    }

    #[test]
    fn rejects_unsafe_external_destinations() {
        for raw in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://user:secret@example.test/path",
            "https://example.test/\nheader",
        ] {
            assert!(
                validated_external_url(raw, ExternalPurpose::Support).is_err(),
                "{raw}"
            );
        }
        assert!(validated_external_url(
            "https://checkout.evil.example/session",
            ExternalPurpose::Billing,
        )
        .is_err());
        assert!(validated_external_url(
            "https://storage.elementors.guru/course/file.pdf",
            ExternalPurpose::LearningContent,
        )
        .is_ok());
    }
}
