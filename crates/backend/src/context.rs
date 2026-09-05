use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct RequestContext {
    pub user_id: Uuid,
    pub firebase_uid: String,
    pub email: String,
    pub display_name: Option<String>,
    pub tenant_id: Option<Uuid>,
    pub tenant_role: Option<core_types::TenantRole>,
    pub is_platform_admin: bool,
    /// The authentication trust boundary carried by the verified credential.
    /// Enterprise identities may only select their signed tenant.
    pub identity_scope: crate::auth::identity::IdentityScope,
}

impl RequestContext {
    /// Central authorization predicate. Platform owners are the only callers
    /// with `PlatformManage`; when they deliberately enter a tenant context,
    /// they retain the other capabilities as a support/recovery override.
    pub fn has_capability(&self, capability: core_types::Capability) -> bool {
        if self.can_manage_platform() {
            return capability == core_types::Capability::PlatformManage
                || (self.tenant_id.is_some()
                    && capability != core_types::Capability::OrganizationOwn);
        }
        self.tenant_role
            .is_some_and(|role| role.has_capability(capability))
    }

    pub fn can_manage_platform(&self) -> bool {
        self.is_platform_admin
            && matches!(
                self.identity_scope,
                crate::auth::identity::IdentityScope::Global
            )
    }

    pub fn can_manage_organization(&self) -> bool {
        self.has_capability(core_types::Capability::OrganizationManage)
    }

    pub fn owns_organization(&self) -> bool {
        matches!(self.tenant_role, Some(core_types::TenantRole::OrgOwner))
    }

    pub fn can_manage_billing(&self) -> bool {
        self.has_capability(core_types::Capability::BillingManage)
    }

    pub fn can_manage_members(&self) -> bool {
        self.has_capability(core_types::Capability::MembersManage)
    }

    pub fn can_teach(&self) -> bool {
        self.has_capability(core_types::Capability::Teach)
    }

    pub fn can_assist(&self) -> bool {
        self.has_capability(core_types::Capability::Assist)
    }

    pub fn can_grade(&self) -> bool {
        self.has_capability(core_types::Capability::Grade)
    }

    /// Whether this credential may create or mutate state in `target_tenant`.
    /// Global identities may deliberately join multiple workspaces; an
    /// organization-controlled SSO/LTI assertion is cryptographically bound to
    /// its one tenant and may never redeem another tenant's secret/code.
    pub fn can_target_tenant(&self, target_tenant: Uuid) -> bool {
        self.identity_scope
            .enterprise_tenant_id()
            .is_none_or(|identity_tenant| identity_tenant == target_tenant)
    }
}

#[cfg(test)]
mod tests {
    use super::RequestContext;
    use core_types::{Capability, TenantRole};
    use uuid::Uuid;

    fn context(role: Option<TenantRole>, platform: bool) -> RequestContext {
        RequestContext {
            user_id: Uuid::nil(),
            firebase_uid: "test".into(),
            email: "test@example.test".into(),
            display_name: None,
            tenant_id: role.map(|_| Uuid::nil()),
            tenant_role: role,
            is_platform_admin: platform,
            identity_scope: crate::auth::identity::IdentityScope::Global,
        }
    }

    #[test]
    fn request_context_applies_role_capabilities() {
        let teacher = context(Some(TenantRole::Teacher), false);
        assert!(teacher.can_teach());
        assert!(teacher.can_grade());
        assert!(!teacher.can_manage_members());

        let ta = context(Some(TenantRole::Ta), false);
        assert!(ta.can_assist());
        assert!(ta.can_grade());
        assert!(!ta.can_teach());

        let student = context(Some(TenantRole::Student), false);
        assert!(student.has_capability(Capability::Learn));
        assert!(!student.can_assist());

        let organization_owner = context(Some(TenantRole::OrgOwner), false);
        assert!(organization_owner.owns_organization());
        assert!(organization_owner.can_manage_organization());
        assert!(organization_owner.can_manage_members());
        assert!(organization_owner.can_manage_billing());
        assert!(!context(Some(TenantRole::OrgAdmin), false).can_manage_billing());
    }

    #[test]
    fn platform_admin_is_the_platform_management_authority() {
        let owner = context(None, true);
        assert!(owner.can_manage_platform());
        assert!(!owner.has_capability(Capability::OrganizationManage));

        let support_context = RequestContext {
            tenant_id: Some(Uuid::nil()),
            ..owner
        };
        for capability in Capability::ALL {
            assert_eq!(
                support_context.has_capability(capability),
                capability != Capability::OrganizationOwn
            );
        }
        assert!(!support_context.owns_organization());
        assert!(!context(Some(TenantRole::OrgAdmin), false).can_manage_platform());
    }

    #[test]
    fn enterprise_identity_cannot_target_another_tenant() {
        let tenant = Uuid::new_v4();
        let other = Uuid::new_v4();
        let mut enterprise = context(Some(TenantRole::Student), false);
        enterprise.tenant_id = Some(tenant);
        enterprise.identity_scope = crate::auth::identity::IdentityScope::Enterprise {
            tenant_id: tenant,
            provider: crate::auth::identity::EnterpriseProvider::Sso,
        };
        assert!(enterprise.can_target_tenant(tenant));
        assert!(!enterprise.can_target_tenant(other));
        assert!(context(Some(TenantRole::Student), false).can_target_tenant(other));
    }

    #[test]
    fn enterprise_identity_never_inherits_platform_authority_from_a_bad_flag() {
        let tenant = Uuid::new_v4();
        let mut enterprise = context(Some(TenantRole::OrgAdmin), true);
        enterprise.identity_scope = crate::auth::identity::IdentityScope::Enterprise {
            tenant_id: tenant,
            provider: crate::auth::identity::EnterpriseProvider::Sso,
        };

        assert!(!enterprise.can_manage_platform());
        assert!(!enterprise.has_capability(Capability::PlatformManage));
        assert!(enterprise.can_manage_organization());
    }
}
