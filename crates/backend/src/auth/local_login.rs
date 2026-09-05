use chrono::Utc;

use crate::auth::verify::FirebaseClaims;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalLoginProfile {
    pub name: String,
    pub token: String,
    pub password: String,
    pub email: String,
    pub display_name: String,
    pub firebase_uid: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalLoginPublicProfile {
    pub name: String,
    pub email: String,
    pub display_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalLoginConfig {
    pub app_env: String,
    pub enabled: bool,
    pub email: String,
    pub profiles: Vec<LocalLoginProfile>,
}

impl LocalLoginConfig {
    pub fn from_env() -> Self {
        let teacher_email = std::env::var("LOCAL_LOGIN_EMAIL")
            .unwrap_or_else(|_| "local.teacher@example.test".into());
        let student_email = std::env::var("LOCAL_LOGIN_STUDENT_EMAIL")
            .unwrap_or_else(|_| "local.student@example.test".into());
        let teacher = LocalLoginProfile {
            name: "teacher".into(),
            token: env_first_non_empty(&["LOCAL_LOGIN_TEACHER_TOKEN", "LOCAL_LOGIN_TOKEN"]),
            password: std::env::var("LOCAL_LOGIN_TEACHER_PASSWORD").unwrap_or_default(),
            email: teacher_email.clone(),
            display_name: std::env::var("LOCAL_LOGIN_DISPLAY_NAME")
                .unwrap_or_else(|_| "Local Teacher".into()),
            firebase_uid: std::env::var("LOCAL_LOGIN_FIREBASE_UID")
                .unwrap_or_else(|_| firebase_uid_for_email(&teacher_email)),
        };
        let student = LocalLoginProfile {
            name: "student".into(),
            token: std::env::var("LOCAL_LOGIN_STUDENT_TOKEN").unwrap_or_default(),
            password: std::env::var("LOCAL_LOGIN_STUDENT_PASSWORD").unwrap_or_default(),
            email: student_email.clone(),
            display_name: std::env::var("LOCAL_LOGIN_STUDENT_DISPLAY_NAME")
                .unwrap_or_else(|_| "Local Student".into()),
            firebase_uid: std::env::var("LOCAL_LOGIN_STUDENT_FIREBASE_UID")
                .unwrap_or_else(|_| firebase_uid_for_email(&student_email)),
        };
        Self::from_profiles(
            std::env::var("APP_ENV").unwrap_or_else(|_| "production".into()),
            env_bool("LOCAL_LOGIN_BYPASS_ENABLED"),
            merge_profiles(
                vec![teacher, student],
                parse_extra_profiles(
                    &std::env::var("LOCAL_LOGIN_EXTRA_PROFILES").unwrap_or_default(),
                ),
            ),
        )
    }

    pub fn from_profiles(
        app_env: impl Into<String>,
        enabled: bool,
        profiles: Vec<LocalLoginProfile>,
    ) -> Self {
        let email = profiles
            .iter()
            .find(|profile| is_complete_profile(profile))
            .or_else(|| profiles.first())
            .map(|profile| profile.email.clone())
            .unwrap_or_default();
        Self {
            app_env: app_env.into(),
            enabled,
            email,
            profiles,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled && !self.is_production() && self.profiles.iter().any(is_complete_profile)
    }

    pub fn public_profiles(&self) -> Vec<LocalLoginPublicProfile> {
        if !self.is_enabled() {
            return Vec::new();
        }
        self.profiles
            .iter()
            .filter(|profile| is_complete_profile(profile))
            .map(|profile| LocalLoginPublicProfile {
                name: profile.name.clone(),
                email: profile.email.clone(),
                display_name: profile.display_name.clone(),
            })
            .collect()
    }

    pub fn default_profile(&self) -> Option<&LocalLoginProfile> {
        if !self.is_enabled() {
            return None;
        }
        self.profiles
            .iter()
            .find(|profile| is_complete_profile(profile))
    }

    pub fn profile(&self, name: &str) -> Option<&LocalLoginProfile> {
        if !self.is_enabled() {
            return None;
        }
        self.profiles
            .iter()
            .find(|profile| profile.name == name && is_complete_profile(profile))
    }

    pub fn profile_for_token(&self, token: &str) -> Option<&LocalLoginProfile> {
        if !self.is_enabled() {
            return None;
        }
        self.profiles.iter().find(|profile| {
            is_complete_profile(profile)
                    // Constant-time like the password path: the bypass token is
                    // a bearer credential even though this route is dev-only.
                    && constant_time_eq(profile.token.as_bytes(), token.as_bytes())
        })
    }

    pub fn profile_for_email_password(
        &self,
        email: &str,
        password: &str,
    ) -> Option<&LocalLoginProfile> {
        if !self.is_enabled() {
            return None;
        }
        let email_lc = email.trim().to_ascii_lowercase();
        self.profiles.iter().find(|p| {
            is_complete_profile(p)
                && p.email.trim().to_ascii_lowercase() == email_lc
                && constant_time_eq(p.password.as_bytes(), password.as_bytes())
        })
    }

    pub fn is_token(&self, token: &str) -> bool {
        self.profile_for_token(token).is_some()
    }

    pub fn claims_for_token(&self, token: &str) -> Option<FirebaseClaims> {
        self.profile_for_token(token).map(claims_for_profile)
    }

    fn is_production(&self) -> bool {
        is_production(&self.app_env)
    }
}

/// Resolve whether `APP_ENV` denotes production using a FAIL-SAFE allowlist:
/// anything that is not an explicitly-known non-production value is treated as
/// production. This prevents a typo'd or unexpected `APP_ENV` (e.g. "staging",
/// "prod-eu", "prd", or an empty string) from silently enabling the local-login
/// bypass or the mock/ephemeral secret fallbacks. To run in a non-production
/// mode you must opt in explicitly with one of the known values.
pub fn is_production(app_env: &str) -> bool {
    !matches!(
        app_env.trim().to_ascii_lowercase().as_str(),
        "local" | "dev" | "development" | "test" | "ci"
    )
}

impl LocalLoginProfile {
    pub fn claims(&self) -> FirebaseClaims {
        claims_for_profile(self)
    }
}

fn claims_for_profile(profile: &LocalLoginProfile) -> FirebaseClaims {
    let now = Utc::now().timestamp();
    FirebaseClaims {
        sub: profile.firebase_uid.clone(),
        email: Some(profile.email.clone()),
        email_verified: Some(true),
        name: Some(profile.display_name.clone()),
        picture: None,
        aud: "local-login-bypass".into(),
        iss: "local-login-bypass".into(),
        exp: now + 24 * 60 * 60,
        iat: now,
        auth_time: Some(now),
    }
}

fn is_complete_profile(profile: &LocalLoginProfile) -> bool {
    !profile.name.trim().is_empty()
        && !profile.token.trim().is_empty()
        && !profile.password.trim().is_empty()
        && !profile.email.trim().is_empty()
        && !profile.firebase_uid.trim().is_empty()
}

fn env_bool(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn env_first_non_empty(names: &[&str]) -> String {
    names
        .iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_default()
}

/// Constant-time byte-slice comparison. Returns true iff the two slices have
/// identical length and identical contents. Branches on length only — the
/// inner loop reads every byte regardless of mismatch.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn firebase_uid_for_email(email: &str) -> String {
    let safe_email: String = email
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    format!("local-login-{safe_email}")
}


/// One entry of the `LOCAL_LOGIN_EXTRA_PROFILES` JSON array.
///
/// `name`, `email` and `password` are required; the rest are derived when
/// omitted so adding a test user is a three-field job:
///
/// ```json
/// [{"name":"alice","email":"alice@example.test","password":"s3cret"}]
/// ```
#[derive(serde::Deserialize)]
struct ExtraProfileSpec {
    name: String,
    email: String,
    password: String,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    firebase_uid: Option<String>,
}

/// Derive a stable bearer token from the profile's own credentials.
///
/// The token is a bearer credential, so it must be unguessable AND stable
/// across restarts (a rotating token would invalidate every persisted session
/// on redeploy). Hashing the password gives both: it cannot be reversed, and
/// it is no easier to guess than the password already is - the password is the
/// authentication factor either way. Domain-separated so this digest can never
/// collide with another hash in the system.
fn derived_token(email: &str, password: &str) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"aulalite-local-login-token:v1:");
    hasher.update(email.trim().to_ascii_lowercase().as_bytes());
    hasher.update(b":");
    hasher.update(password.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize())
}

/// Parse `LOCAL_LOGIN_EXTRA_PROFILES`. Invalid JSON is logged and ignored
/// rather than propagated: a typo in a dev-only convenience variable must not
/// take the whole API down at boot.
fn parse_extra_profiles(raw: &str) -> Vec<LocalLoginProfile> {
    if raw.trim().is_empty() {
        return Vec::new();
    }
    let specs: Vec<ExtraProfileSpec> = match serde_json::from_str(raw) {
        Ok(specs) => specs,
        Err(error) => {
            tracing::warn!(
                %error,
                "LOCAL_LOGIN_EXTRA_PROFILES is not a valid JSON array of profiles; ignoring it"
            );
            return Vec::new();
        }
    };
    specs
        .into_iter()
        .map(|spec| {
            let email = spec.email.trim().to_string();
            let token = spec
                .token
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| derived_token(&email, &spec.password));
            LocalLoginProfile {
                display_name: spec
                    .display_name
                    .filter(|d| !d.trim().is_empty())
                    .unwrap_or_else(|| spec.name.clone()),
                firebase_uid: spec
                    .firebase_uid
                    .filter(|u| !u.trim().is_empty())
                    .unwrap_or_else(|| firebase_uid_for_email(&email)),
                name: spec.name.trim().to_string(),
                token,
                password: spec.password,
                email,
            }
        })
        .collect()
}

/// Append `extra` to `base`, dropping any entry whose name or email (case
/// insensitively) already exists. Lookups resolve by first match, so allowing
/// a duplicate would silently shadow the earlier profile and make which
/// password works depend on ordering.
fn merge_profiles(
    base: Vec<LocalLoginProfile>,
    extra: Vec<LocalLoginProfile>,
) -> Vec<LocalLoginProfile> {
    let mut out = base;
    for candidate in extra {
        let name_lc = candidate.name.trim().to_ascii_lowercase();
        let email_lc = candidate.email.trim().to_ascii_lowercase();
        let clash = out.iter().any(|p| {
            p.name.trim().to_ascii_lowercase() == name_lc
                || p.email.trim().to_ascii_lowercase() == email_lc
        });
        if clash {
            tracing::warn!(
                profile = %candidate.name,
                "LOCAL_LOGIN_EXTRA_PROFILES entry duplicates an existing name or email; skipping"
            );
            continue;
        }
        out.push(candidate);
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str, email: &str, password: &str, token: &str) -> LocalLoginProfile {
        LocalLoginProfile {
            name: name.to_string(),
            token: token.to_string(),
            password: password.to_string(),
            email: email.to_string(),
            display_name: format!("Display {name}"),
            firebase_uid: format!("uid-{name}"),
        }
    }

    fn config_with(profiles: Vec<LocalLoginProfile>) -> LocalLoginConfig {
        LocalLoginConfig::from_profiles("local", true, profiles)
    }

    #[test]
    fn profile_for_email_password_matches_exact_email_and_password() {
        let cfg = config_with(vec![profile(
            "teacher",
            "t@example.test",
            "teacher-pass",
            "t-token",
        )]);
        let hit = cfg.profile_for_email_password("t@example.test", "teacher-pass");
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().name, "teacher");
    }

    #[test]
    fn profile_for_email_password_is_email_case_insensitive() {
        let cfg = config_with(vec![profile(
            "teacher",
            "t@example.test",
            "teacher-pass",
            "t-token",
        )]);
        assert!(cfg
            .profile_for_email_password("T@Example.TEST", "teacher-pass")
            .is_some());
    }

    #[test]
    fn profile_for_email_password_rejects_wrong_password() {
        let cfg = config_with(vec![profile(
            "teacher",
            "t@example.test",
            "teacher-pass",
            "t-token",
        )]);
        assert!(cfg
            .profile_for_email_password("t@example.test", "wrong")
            .is_none());
    }

    #[test]
    fn profile_for_email_password_returns_none_when_disabled() {
        let cfg = LocalLoginConfig::from_profiles(
            "production",
            true,
            vec![profile(
                "teacher",
                "t@example.test",
                "teacher-pass",
                "t-token",
            )],
        );
        assert!(cfg
            .profile_for_email_password("t@example.test", "teacher-pass")
            .is_none());
    }

    #[test]
    fn profile_without_password_is_incomplete() {
        let cfg = config_with(vec![profile("teacher", "t@example.test", "", "t-token")]);
        assert!(cfg
            .profile_for_email_password("t@example.test", "")
            .is_none());
    }
}

#[cfg(test)]
mod extra_profile_tests {
    use super::*;

    #[test]
    fn empty_or_blank_env_yields_no_profiles() {
        assert!(parse_extra_profiles("").is_empty());
        assert!(parse_extra_profiles("   \n ").is_empty());
    }

    #[test]
    fn invalid_json_is_ignored_rather_than_panicking() {
        assert!(parse_extra_profiles("{not json").is_empty());
        assert!(parse_extra_profiles(r#"{"name":"x"}"#).is_empty());
    }

    #[test]
    fn minimal_spec_derives_token_display_name_and_uid() {
        let parsed = parse_extra_profiles(
            r#"[{"name":"alice","email":"alice@example.test","password":"s3cret"}]"#,
        );
        assert_eq!(parsed.len(), 1);
        let p = &parsed[0];
        assert_eq!(p.name, "alice");
        assert_eq!(p.display_name, "alice");
        assert_eq!(p.firebase_uid, firebase_uid_for_email("alice@example.test"));
        assert!(!p.token.is_empty());
        // A derived profile must satisfy the completeness gate, or it would be
        // silently dropped from every lookup.
        assert!(is_complete_profile(p));
    }

    #[test]
    fn derived_token_is_stable_and_password_dependent() {
        let a = derived_token("alice@example.test", "s3cret");
        let b = derived_token("alice@example.test", "s3cret");
        let c = derived_token("alice@example.test", "other");
        assert_eq!(a, b, "same inputs must give the same token across restarts");
        assert_ne!(a, c, "a different password must give a different token");
        assert_eq!(
            a,
            derived_token("ALICE@Example.TEST", "s3cret"),
            "email casing must not change the token"
        );
    }

    #[test]
    fn explicit_fields_override_derived_ones() {
        let parsed = parse_extra_profiles(
            r#"[{"name":"bob","email":"b@example.test","password":"pw",
                 "token":"tok","display_name":"Bob B","firebase_uid":"uid-bob"}]"#,
        );
        let p = &parsed[0];
        assert_eq!(p.token, "tok");
        assert_eq!(p.display_name, "Bob B");
        assert_eq!(p.firebase_uid, "uid-bob");
    }

    #[test]
    fn merge_skips_duplicate_name_or_email_case_insensitively() {
        let base = vec![LocalLoginProfile {
            name: "teacher".into(),
            token: "t".into(),
            password: "p".into(),
            email: "t@example.test".into(),
            display_name: "T".into(),
            firebase_uid: "uid-t".into(),
        }];
        let extra = parse_extra_profiles(
            r#"[{"name":"TEACHER","email":"new@example.test","password":"p"},
                {"name":"other","email":"T@Example.TEST","password":"p"},
                {"name":"carol","email":"c@example.test","password":"p"}]"#,
        );
        let merged = merge_profiles(base, extra);
        let names: Vec<&str> = merged.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["teacher", "carol"]);
    }

    #[test]
    fn extra_profiles_are_authenticatable_end_to_end() {
        let profiles = merge_profiles(
            Vec::new(),
            parse_extra_profiles(
                r#"[{"name":"dana","email":"dana@example.test","password":"pw-dana"}]"#,
            ),
        );
        let cfg = LocalLoginConfig::from_profiles("local", true, profiles);
        assert!(cfg.is_enabled());
        let hit = cfg.profile_for_email_password("DANA@example.test", "pw-dana");
        assert!(hit.is_some(), "extra profile must authenticate by email+password");
        let token = hit.unwrap().token.clone();
        assert!(
            cfg.profile_for_token(&token).is_some(),
            "the derived token must round-trip through token lookup"
        );
        assert!(cfg.profile_for_email_password("dana@example.test", "wrong").is_none());
    }

    #[test]
    fn extra_profiles_stay_disabled_in_production() {
        let profiles = parse_extra_profiles(
            r#"[{"name":"dana","email":"dana@example.test","password":"pw"}]"#,
        );
        let cfg = LocalLoginConfig::from_profiles("production", true, profiles);
        assert!(!cfg.is_enabled(), "production must never enable the bypass");
        assert!(cfg.profile_for_email_password("dana@example.test", "pw").is_none());
    }
}
