use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Describes the trust boundary of an authenticated identity.
///
/// Global identities are verified by the platform authentication provider (or
/// the explicitly configured local-development provider). Enterprise
/// identities are asserted by an organization-controlled SSO/LTI provider and
/// are therefore valid only inside that one organization. Keeping this scope
/// in the server-signed session prevents a tenant administrator from turning
/// an asserted email address into access to another tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum IdentityScope {
    Global,
    Enterprise {
        tenant_id: Uuid,
        provider: EnterpriseProvider,
    },
}

impl IdentityScope {
    pub fn enterprise_tenant_id(self) -> Option<Uuid> {
        match self {
            Self::Global => None,
            Self::Enterprise { tenant_id, .. } => Some(tenant_id),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnterpriseProvider {
    Sso,
    Lti,
}

impl EnterpriseProvider {
    pub const fn as_db_str(self) -> &'static str {
        match self {
            Self::Sso => "sso",
            Self::Lti => "lti",
        }
    }
}
