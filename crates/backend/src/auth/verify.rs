#[cfg(test)]
use jsonwebtoken::DecodingKey;
use jsonwebtoken::{decode, Algorithm, Validation};
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VerifyError {
    #[error("invalid token: {0}")]
    Invalid(String),
    #[error("jwks: {0}")]
    Jwks(#[from] super::jwks::JwksError),
    #[error("missing kid header")]
    MissingKid,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FirebaseClaims {
    pub sub: String,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub aud: String,
    pub iss: String,
    pub exp: i64,
    pub iat: i64,
    pub auth_time: Option<i64>,
}

pub struct Verifier {
    jwks: super::jwks::JwksCache,
    project_id: String,
    issuer: String,
}

impl Verifier {
    pub fn new(
        jwks: super::jwks::JwksCache,
        project_id: impl Into<String>,
        issuer: impl Into<String>,
    ) -> Self {
        Self {
            jwks,
            project_id: project_id.into(),
            issuer: issuer.into(),
        }
    }

    pub async fn verify(&self, token: &str) -> Result<FirebaseClaims, VerifyError> {
        let header = jsonwebtoken::decode_header(token)
            .map_err(|err| VerifyError::Invalid(err.to_string()))?;
        let kid = header.kid.ok_or(VerifyError::MissingKid)?;
        let key = self.jwks.key_for_kid(&kid).await?;

        decode_with_validation(token, &key, &self.project_id, &self.issuer)
    }

    #[cfg(test)]
    pub fn verify_with_key(
        token: &str,
        key: &DecodingKey,
        project_id: &str,
        issuer: &str,
    ) -> Result<FirebaseClaims, VerifyError> {
        decode_with_validation(token, key, project_id, issuer)
    }
}

fn decode_with_validation(
    token: &str,
    key: &jsonwebtoken::DecodingKey,
    project_id: &str,
    issuer: &str,
) -> Result<FirebaseClaims, VerifyError> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.leeway = 0;
    validation.set_audience(&[project_id]);
    validation.set_issuer(&[issuer]);

    let data = decode::<FirebaseClaims>(token, key, &validation)
        .map_err(|err| VerifyError::Invalid(err.to_string()))?;
    Ok(data.claims)
}

#[cfg(test)]
mod tests {
    use super::Verifier;
    use jsonwebtoken::{encode, Algorithm, DecodingKey, EncodingKey, Header};

    fn pem_keypair() -> (EncodingKey, DecodingKey) {
        let priv_pem =
            std::fs::read("tests/fixtures/test_priv.pem").expect("test private key missing");
        let pub_pem =
            std::fs::read("tests/fixtures/test_pub.pem").expect("test public key missing");

        (
            EncodingKey::from_rsa_pem(&priv_pem).unwrap(),
            DecodingKey::from_rsa_pem(&pub_pem).unwrap(),
        )
    }

    #[test]
    fn verify_with_key_accepts_well_formed_token() {
        let (encoding_key, decoding_key) = pem_keypair();
        let claims = serde_json::json!({
            "sub": "fbuid_test",
            "email": "test@example.com",
            "aud": "aulalite-dev",
            "iss": "https://securetoken.google.com/aulalite-dev",
            "exp": chrono::Utc::now().timestamp() + 600,
            "iat": chrono::Utc::now().timestamp(),
        });
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-kid".into());
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let result = Verifier::verify_with_key(
            &token,
            &decoding_key,
            "aulalite-dev",
            "https://securetoken.google.com/aulalite-dev",
        )
        .expect("should verify");

        assert_eq!(result.sub, "fbuid_test");
        assert_eq!(result.email.as_deref(), Some("test@example.com"));
    }

    #[test]
    fn verify_with_key_rejects_wrong_audience() {
        let (encoding_key, decoding_key) = pem_keypair();
        let claims = serde_json::json!({
            "sub": "fbuid_test",
            "aud": "wrong-project",
            "iss": "https://securetoken.google.com/aulalite-dev",
            "exp": chrono::Utc::now().timestamp() + 600,
            "iat": chrono::Utc::now().timestamp(),
        });
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-kid".into());
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let result = Verifier::verify_with_key(
            &token,
            &decoding_key,
            "aulalite-dev",
            "https://securetoken.google.com/aulalite-dev",
        );

        assert!(result.is_err());
    }

    #[test]
    fn verify_with_key_rejects_expired_token() {
        let (encoding_key, decoding_key) = pem_keypair();
        let claims = serde_json::json!({
            "sub": "fbuid_test",
            "aud": "aulalite-dev",
            "iss": "https://securetoken.google.com/aulalite-dev",
            "exp": chrono::Utc::now().timestamp() - 60,
            "iat": chrono::Utc::now().timestamp() - 600,
        });
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-kid".into());
        let token = encode(&header, &claims, &encoding_key).unwrap();

        let result = Verifier::verify_with_key(
            &token,
            &decoding_key,
            "aulalite-dev",
            "https://securetoken.google.com/aulalite-dev",
        );

        assert!(result.is_err());
    }
}
