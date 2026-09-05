// crates/shell-web/src/contexts.rs
use core_types::{Capability, TenantRole};
use dioxus::prelude::*;
use features_courses::api::UserDto;

#[derive(Clone, Debug, PartialEq)]
pub struct UserContext {
    pub user_id: String,
    pub display_name: String,
    pub email: String,
    pub tenant_id: Option<String>,
    pub tenant_role: Option<TenantRole>,
    pub is_platform_admin: bool,
}

impl UserContext {
    /// Keep client-side navigation and affordances aligned with the backend's
    /// capability model. The server remains authoritative for every action.
    pub fn has_capability(&self, capability: Capability) -> bool {
        if self.is_platform_admin {
            if capability == Capability::PlatformManage {
                return true;
            }
            // Ownership transfer models the organization's legal authority,
            // not a support override. Platform operators may administer a
            // selected workspace, but cannot impersonate its owner here.
            if capability == Capability::OrganizationOwn {
                return self.tenant_role == Some(TenantRole::OrgOwner);
            }
            return self.tenant_id.is_some();
        }
        self.tenant_role
            .is_some_and(|role| role.has_capability(capability))
    }

    pub fn can_manage_platform(&self) -> bool {
        self.has_capability(Capability::PlatformManage)
    }

    /// True only for the workspace owner. Ownership is deliberately narrower
    /// than ordinary administration: billing, ownership transfer, and future
    /// destructive workspace controls must use this gate.
    pub fn can_own_organization(&self) -> bool {
        self.has_capability(Capability::OrganizationOwn)
    }

    pub fn can_manage_organization(&self) -> bool {
        self.has_capability(Capability::OrganizationManage)
    }

    pub fn can_manage_billing(&self) -> bool {
        self.has_capability(Capability::BillingManage)
    }

    pub fn can_manage_members(&self) -> bool {
        self.has_capability(Capability::MembersManage)
    }

    pub fn can_manage_integrations(&self) -> bool {
        self.has_capability(Capability::IntegrationsManage)
    }

    pub fn can_teach(&self) -> bool {
        self.has_capability(Capability::Teach)
    }

    pub fn can_assist(&self) -> bool {
        self.has_capability(Capability::Assist)
    }

    pub fn can_grade(&self) -> bool {
        self.has_capability(Capability::Grade)
    }

    pub fn can_learn(&self) -> bool {
        self.has_capability(Capability::Learn)
    }

    pub fn can_read_parent_dashboard(&self) -> bool {
        self.has_capability(Capability::ParentRead)
    }

    /// Course staff includes teachers, teaching assistants, and workspace
    /// admins. Use the narrower capability methods above for action buttons;
    /// this predicate is only for staff views such as moderation and drafts.
    pub fn is_course_staff(&self) -> bool {
        self.can_teach() || self.can_assist() || self.can_grade()
    }

    pub fn from_dto(dto: UserDto) -> Self {
        let role = dto.tenant_role.as_deref().and_then(parse_role);
        Self {
            user_id: dto.user_id,
            display_name: dto.display_name.unwrap_or_else(|| dto.email.clone()),
            email: dto.email,
            tenant_id: dto.tenant_id,
            tenant_role: role,
            is_platform_admin: dto.is_platform_admin,
        }
    }
}

fn parse_role(s: &str) -> Option<TenantRole> {
    match s {
        "org_owner" => Some(TenantRole::OrgOwner),
        "org_admin" => Some(TenantRole::OrgAdmin),
        "teacher" => Some(TenantRole::Teacher),
        "ta" => Some(TenantRole::Ta),
        "student" => Some(TenantRole::Student),
        "parent" => Some(TenantRole::Parent),
        _ => None,
    }
}

/// Provided at the App root by the App scaffold (Task 5). Routes read via
/// `use_context::<UserContextSignal>()`.
pub type UserContextSignal = Signal<Option<UserContext>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn user(role: Option<TenantRole>, platform: bool, in_workspace: bool) -> UserContext {
        UserContext {
            user_id: "user-1".into(),
            display_name: "User".into(),
            email: "user@example.test".into(),
            tenant_id: in_workspace.then(|| "tenant-1".into()),
            tenant_role: role,
            is_platform_admin: platform,
        }
    }

    #[test]
    fn capabilities_match_backend_role_boundaries() {
        let owner = user(Some(TenantRole::OrgOwner), false, true);
        assert!(owner.can_own_organization());
        assert!(owner.can_manage_billing());
        assert!(owner.can_manage_members());

        assert!(user(Some(TenantRole::OrgAdmin), false, true).can_manage_organization());
        assert!(user(Some(TenantRole::OrgAdmin), false, true).can_manage_members());
        assert!(user(Some(TenantRole::OrgAdmin), false, true).can_manage_integrations());
        assert!(!user(Some(TenantRole::OrgAdmin), false, true).can_own_organization());
        assert!(!user(Some(TenantRole::OrgAdmin), false, true).can_manage_billing());
        assert!(!user(Some(TenantRole::Teacher), false, true).can_manage_organization());
        let teacher = user(Some(TenantRole::Teacher), false, true);
        assert!(teacher.can_teach());
        assert!(teacher.can_grade());

        let ta = user(Some(TenantRole::Ta), false, true);
        assert!(!ta.can_teach());
        assert!(ta.can_assist());
        assert!(ta.can_grade());
        assert!(ta.is_course_staff());

        let student = user(Some(TenantRole::Student), false, true);
        assert!(student.can_learn());
        assert!(!student.is_course_staff());

        let parent = user(Some(TenantRole::Parent), false, true);
        assert!(parent.can_read_parent_dashboard());
        assert!(!parent.can_learn());
    }

    #[test]
    fn platform_owner_needs_workspace_context_for_workspace_capabilities() {
        let global = user(None, true, false);
        assert!(global.can_manage_platform());
        assert!(!global.can_manage_organization());
        assert!(!global.can_own_organization());

        let contextual = user(Some(TenantRole::Student), true, true);
        assert!(contextual.can_manage_platform());
        assert!(contextual.can_manage_organization());
        assert!(!contextual.can_own_organization());
    }
}
