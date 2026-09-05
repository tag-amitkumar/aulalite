// crates/core-types/src/lib.rs
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

pub mod live_room;
pub mod quiz;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantRole {
    OrgOwner,
    OrgAdmin,
    Teacher,
    Ta,
    Student,
    Parent,
}

/// Stable, product-level abilities used by every application surface.
///
/// Roles answer *who a member is*; capabilities answer *what that member may
/// do*. Keeping this mapping in the shared core crate prevents the backend and
/// clients from quietly inventing different role hierarchies. Resource scope
/// still matters: for example, `Grade` permits grading only after the caller
/// is confirmed as staff on the specific course.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    PlatformManage,
    OrganizationOwn,
    OrganizationManage,
    BillingManage,
    MembersManage,
    IntegrationsManage,
    Teach,
    Assist,
    Grade,
    Learn,
    ParentRead,
}

impl Capability {
    pub const ALL: [Self; 11] = [
        Self::PlatformManage,
        Self::OrganizationOwn,
        Self::OrganizationManage,
        Self::BillingManage,
        Self::MembersManage,
        Self::IntegrationsManage,
        Self::Teach,
        Self::Assist,
        Self::Grade,
        Self::Learn,
        Self::ParentRead,
    ];
}

const ORG_OWNER_CAPABILITIES: &[Capability] = &[
    Capability::OrganizationOwn,
    Capability::OrganizationManage,
    Capability::BillingManage,
    Capability::MembersManage,
    Capability::IntegrationsManage,
    Capability::Teach,
    Capability::Assist,
    Capability::Grade,
];
const ORG_ADMIN_CAPABILITIES: &[Capability] = &[
    Capability::OrganizationManage,
    Capability::MembersManage,
    Capability::IntegrationsManage,
    Capability::Teach,
    Capability::Assist,
    Capability::Grade,
];
const TEACHER_CAPABILITIES: &[Capability] =
    &[Capability::Teach, Capability::Assist, Capability::Grade];
const TA_CAPABILITIES: &[Capability] = &[Capability::Assist, Capability::Grade];
const STUDENT_CAPABILITIES: &[Capability] = &[Capability::Learn];
const PARENT_CAPABILITIES: &[Capability] = &[Capability::ParentRead];

impl TenantRole {
    pub const ALL: [Self; 6] = [
        Self::OrgOwner,
        Self::OrgAdmin,
        Self::Teacher,
        Self::Ta,
        Self::Student,
        Self::Parent,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TenantRole::OrgOwner => "org_owner",
            TenantRole::OrgAdmin => "org_admin",
            TenantRole::Teacher => "teacher",
            TenantRole::Ta => "ta",
            TenantRole::Student => "student",
            TenantRole::Parent => "parent",
        }
    }

    /// Exhaustive capability set for this tenant role.
    pub fn capabilities(self) -> &'static [Capability] {
        match self {
            Self::OrgOwner => ORG_OWNER_CAPABILITIES,
            Self::OrgAdmin => ORG_ADMIN_CAPABILITIES,
            Self::Teacher => TEACHER_CAPABILITIES,
            Self::Ta => TA_CAPABILITIES,
            Self::Student => STUDENT_CAPABILITIES,
            Self::Parent => PARENT_CAPABILITIES,
        }
    }

    pub fn has_capability(self, capability: Capability) -> bool {
        self.capabilities().contains(&capability)
    }
}

impl fmt::Display for TenantRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseTenantRoleError;

impl fmt::Display for ParseTenantRoleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unknown tenant role")
    }
}

impl std::error::Error for ParseTenantRoleError {}

impl FromStr for TenantRole {
    type Err = ParseTenantRoleError;

    fn from_str(role: &str) -> Result<Self, Self::Err> {
        match role {
            "org_owner" => Ok(Self::OrgOwner),
            "org_admin" => Ok(Self::OrgAdmin),
            "teacher" => Ok(Self::Teacher),
            "ta" => Ok(Self::Ta),
            "student" => Ok(Self::Student),
            "parent" => Ok(Self::Parent),
            _ => Err(ParseTenantRoleError),
        }
    }
}

#[cfg(test)]
mod role_tests {
    use super::{Capability, TenantRole};

    #[test]
    fn role_capability_matrix_is_explicit_and_least_privilege() {
        assert_eq!(
            TenantRole::OrgOwner.capabilities(),
            &[
                Capability::OrganizationOwn,
                Capability::OrganizationManage,
                Capability::BillingManage,
                Capability::MembersManage,
                Capability::IntegrationsManage,
                Capability::Teach,
                Capability::Assist,
                Capability::Grade,
            ]
        );
        assert_eq!(
            TenantRole::OrgAdmin.capabilities(),
            &[
                Capability::OrganizationManage,
                Capability::MembersManage,
                Capability::IntegrationsManage,
                Capability::Teach,
                Capability::Assist,
                Capability::Grade,
            ]
        );
        assert_eq!(
            TenantRole::Teacher.capabilities(),
            &[Capability::Teach, Capability::Assist, Capability::Grade]
        );
        assert_eq!(
            TenantRole::Ta.capabilities(),
            &[Capability::Assist, Capability::Grade]
        );
        assert_eq!(TenantRole::Student.capabilities(), &[Capability::Learn]);
        assert_eq!(TenantRole::Parent.capabilities(), &[Capability::ParentRead]);

        for role in TenantRole::ALL {
            assert!(!role.has_capability(Capability::PlatformManage));
        }
        assert!(!TenantRole::Ta.has_capability(Capability::Teach));
        assert!(!TenantRole::Teacher.has_capability(Capability::MembersManage));
        assert!(TenantRole::OrgOwner.has_capability(Capability::OrganizationOwn));
        assert!(TenantRole::OrgOwner.has_capability(Capability::BillingManage));
        assert!(!TenantRole::OrgAdmin.has_capability(Capability::OrganizationOwn));
        assert!(!TenantRole::OrgAdmin.has_capability(Capability::BillingManage));
    }

    #[test]
    fn role_strings_round_trip() {
        for role in TenantRole::ALL {
            assert_eq!(role.as_str().parse::<TenantRole>().unwrap(), role);
        }
        assert!("owner".parse::<TenantRole>().is_err());
    }
}
