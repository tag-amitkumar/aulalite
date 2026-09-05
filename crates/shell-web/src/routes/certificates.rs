// crates/shell-web/src/routes/certificates.rs
//
// Certificate routes (learning-suite Cycle 5):
//   * `/certificates` — the signed-in user's earned certificates (AppShell).
//   * `/verify/:credential_id` — PUBLIC verification page. Deliberately does
//     NOT require auth and does not mount the AppShell: anyone holding a
//     credential link (e.g. an employer) can open it.
use design_system::PageHeader;
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::app_shell::{AppShell, ShellUser};
use features_courses::certificates_panel::{CertificateVerifyView, MyCertificates};

use crate::route_enum::Route;
use crate::routes::use_user_context;

#[component]
pub fn MyCertificatesPage() -> Element {
    let nav = use_navigator();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => {
            nav.push(Route::Login {});
            return rsx! { p { "Redirecting…" } };
        }
    };
    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                {
                    use platform_bridge::PlatformBridge;
                    spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                }
                nav.push(Route::Login {});
            },
            div { class: "certificates-page",
                PageHeader {
                    title: "My certificates".to_string(),
                    kicker: "Achievements".to_string(),
                    subtitle: "Issued certificates with public verification links; print any of them to PDF.".to_string(),
                }
                MyCertificates {}
            }
        }
    }
}

#[component]
pub fn CertificateVerifyPage(credential_id: String) -> Element {
    rsx! {
        main { class: "certificate-verify-standalone",
            header { class: "certificate-verify-header",
                h1 { "Certificate verification" }
                p { class: "muted", "AulaLite Academy credential check" }
            }
            CertificateVerifyView { credential_id }
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    #[test]
    fn verify_page_renders_without_auth_context() {
        // The public page must render with no user context and no AppShell.
        #[component]
        fn Harness() -> Element {
            use features_courses::api::ApiContext;
            let api_signal = use_signal(|| ApiContext {
                base_url: String::new(),
                id_token: String::new(),
            });
            use_context_provider::<Signal<ApiContext>>(|| api_signal);
            rsx! {
                CertificateVerifyPage { credential_id: "AULA-1A2B-3C4D".to_string() }
            }
        }
        let mut vdom = VirtualDom::new(Harness);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Certificate verification"), "got: {html}");
        assert!(
            !html.contains("app-shell-layout"),
            "verify page must not mount the app shell: {html}"
        );
    }
}
