// crates/shell-web/src/routes/forgot.rs
use dioxus::prelude::*;

#[component]
pub fn Forgot() -> Element {
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
                    p { class: "auth-hero-kicker", "Account recovery" }
                    h2 { "Back into your academy in a moment." }
                    p { "Enter your email and we'll send a secure link to reset your password." }
                    div { class: "auth-hero-meta",
                        span { strong { "Secure" } " reset link" }
                        span { strong { "No password" } " ever stored" }
                    }
                }
            }
            div { class: "auth-panel-stack",
                features_auth::ForgotPassword {}
            }
        }
    }
}
