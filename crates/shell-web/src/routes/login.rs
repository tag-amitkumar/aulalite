// crates/shell-web/src/routes/login.rs
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api::{self, ApiContext};

use crate::contexts::{UserContext, UserContextSignal};
use crate::route_enum::Route;

#[derive(Clone, Copy, PartialEq)]
enum LoginAudience {
    Academy,
    Parent,
}

fn destination_for(user: &UserContext, admin_host: bool) -> Route {
    if admin_host {
        Route::Platform {}
    } else if user.tenant_role == Some(core_types::TenantRole::Parent) {
        Route::ParentHome {}
    } else if user.is_platform_admin && user.tenant_id.is_none() {
        Route::Platform {}
    } else {
        Route::Dashboard {}
    }
}

#[component]
pub fn Login() -> Element {
    rsx! { LoginExperience { audience: LoginAudience::Academy } }
}

#[component]
pub fn ParentLogin() -> Element {
    rsx! { LoginExperience { audience: LoginAudience::Parent } }
}

#[component]
fn LoginExperience(audience: LoginAudience) -> Element {
    let nav = use_navigator();
    let mut api_signal = use_context::<Signal<ApiContext>>();
    let mut user_signal = use_context::<UserContextSignal>();

    let on_success = move |token: String| {
        #[cfg(target_arch = "wasm32")]
        let base_url = api::web_api_base_url();
        #[cfg(not(target_arch = "wasm32"))]
        let base_url = api::native_api_base_url();
        api_signal.set(ApiContext {
            base_url,
            id_token: token,
        });
        let api_ctx = api_signal.read().clone();
        spawn(async move {
            if let Ok(dto) = api::get_me(&api_ctx).await {
                let user = UserContext::from_dto(dto);
                let destination = destination_for(&user, api::is_admin_host());
                user_signal.set(Some(user));
                nav.push(destination);
                return;
            }
            nav.push(Route::Dashboard {});
        });
    };

    let is_parent = audience == LoginAudience::Parent;
    let hero_kicker = if is_parent {
        "Family access"
    } else {
        "Elite live learning"
    };
    let hero_title = if is_parent {
        "Stay close to their learning."
    } else {
        "A workspace built for modern academies."
    };
    let hero_body = if is_parent {
        "See released grades, attendance, and upcoming classes for the children linked to your account."
    } else {
        "Run courses, live rooms, assignments, and schedules with the polish students expect."
    };

    rsx! {
        div { class: "auth-composite motion-page",
            section { class: "auth-hero-visual",
                design_system::AuthHero {}
                div { class: "auth-hero-copy",
                    div { class: "auth-hero-brand",
                        // mark.svg is a dark emblem with a cream "A" — reads cleanly on the warm canvas
                        img {
                            class: "auth-hero-mark",
                            src: "/assets/brand/aulalite-mark.svg",
                            alt: "AulaLite",
                        }
                        // serif wordmark drawn in CSS — dark green with a gold "Lite" on the cream stage
                        span { class: "auth-hero-wordmark", "Aula", span { class: "gold", "Lite" } }
                    }
                    p { class: "auth-hero-kicker", "{hero_kicker}" }
                    h2 { "{hero_title}" }
                    p { "{hero_body}" }
                    if is_parent {
                        div { class: "auth-hero-meta",
                            span { strong { "Private" } " linked-child access" }
                            span { strong { "Read-only" } " family view" }
                        }
                    } else {
                        div { class: "auth-hero-meta",
                            span { strong { "Live rooms" } " in every course" }
                            span { strong { "Multi-tenant" } " by design" }
                        }
                    }
                }
            }
            div { class: "auth-panel-stack",
                if is_parent {
                    features_auth::Login {
                        on_success: on_success,
                        kicker: "Family access".to_string(),
                        title: "Parent or guardian sign in".to_string(),
                        subtitle: "Use the email address invited by your child’s academy.".to_string(),
                        signup_label: "Create your invited account".to_string(),
                    }
                } else {
                    features_auth::Login { on_success: on_success }
                    a {
                        class: "auth-link auth-parent-entry",
                        href: "/parent/login",
                        "Parent or guardian? Use family sign in"
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(role: Option<core_types::TenantRole>, platform: bool, tenant: bool) -> UserContext {
        UserContext {
            user_id: "user-1".into(),
            display_name: "Test User".into(),
            email: "test@example.test".into(),
            tenant_id: tenant.then(|| "tenant-1".into()),
            tenant_role: role,
            is_platform_admin: platform,
        }
    }

    #[test]
    fn post_login_destination_is_role_specific() {
        assert!(
            destination_for(
                &user(Some(core_types::TenantRole::Parent), false, true),
                false,
            ) == Route::ParentHome {}
        );
        assert!(destination_for(&user(None, true, false), false) == Route::Platform {});
        assert!(
            destination_for(
                &user(Some(core_types::TenantRole::Teacher), false, true),
                false,
            ) == Route::Dashboard {}
        );
        assert!(
            destination_for(
                &user(Some(core_types::TenantRole::Teacher), false, true),
                true,
            ) == Route::Platform {}
        );
    }
}
