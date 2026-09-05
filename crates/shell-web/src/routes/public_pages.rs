use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
struct PublicPageProps {
    eyebrow: String,
    title: String,
    summary: String,
    children: Element,
}

#[component]
fn PublicPage(props: PublicPageProps) -> Element {
    rsx! {
        div { class: "public-page",
            a { class: "marketing-skip", href: "#public-main", "Skip to content" }
            header { class: "public-page__nav",
                a { class: "marketing-brand", href: "/", aria_label: "AulaLite home",
                    img { src: "/assets/brand/aulalite-mark.svg", alt: "", aria_hidden: "true" }
                    span { "Aula", strong { "Lite" } }
                }
                nav { aria_label: "Public navigation",
                    a { href: "/#platform", "Platform" }
                    a { href: "/#pricing", "Pricing" }
                    a { href: "/login", "Sign in" }
                    a { class: "marketing-button marketing-button--small", href: "/signup", "Create academy" }
                }
            }
            main { class: "public-page__main", id: "public-main", tabindex: "-1",
                header { class: "public-page__hero",
                    p { class: "marketing-eyebrow", "{props.eyebrow}" }
                    h1 { "{props.title}" }
                    p { "{props.summary}" }
                }
                article { class: "public-page__content",
                    {props.children}
                }
            }
            footer { class: "public-page__footer",
                span { "© 2026 Elementors" }
                nav { aria_label: "Legal navigation",
                    a { href: "/privacy", "Privacy" }
                    a { href: "/terms", "Terms" }
                    a { href: "mailto:hello@elementors.guru", "Contact" }
                }
            }
        }
    }
}

#[component]
pub fn Privacy() -> Element {
    rsx! {
        PublicPage {
            eyebrow: "Trust center".to_string(),
            title: "Privacy at AulaLite".to_string(),
            summary: "How we handle account, learning, billing, and live-class information when you use AulaLite.".to_string(),
            section {
                p { class: "public-page__updated", "Last updated July 16, 2026" }
                h2 { "Information we process" }
                p { "We process the information needed to provide the service: account and workspace details, course content, enrollments, assignments, grades, attendance, messages, live-class participation, recordings when enabled, device and diagnostic data, support requests, and billing references." }

                h2 { "Why we use it" }
                p { "We use this information to authenticate users, deliver learning and live-class features, administer subscriptions, secure and improve the service, respond to support requests, meet legal obligations, and communicate important product or account changes." }

                h2 { "Workspace responsibility" }
                p { "An academy or organization controls the learning records and content in its workspace. It decides who may join, what is recorded, how long course material is retained, and which administrators can access workspace data. Learners should direct record-access or correction requests to their academy first." }

                h2 { "Service providers" }
                p { "We use carefully selected infrastructure, identity, payment, email, storage, and communications providers to operate AulaLite. They may process limited information on our instructions and under contractual safeguards. We do not sell personal information or use learning records for third-party advertising." }

                h2 { "Live classes and recordings" }
                p { "The classroom clearly indicates when a session is live or being recorded. Workspace administrators and instructors are responsible for obtaining any consent required for their learners and jurisdiction. Recordings are available only to authorized workspace users unless an academy deliberately shares them elsewhere." }

                h2 { "Retention and security" }
                p { "We retain information while an account or workspace is active and as needed for legitimate operational, security, backup, billing, dispute, and legal purposes. We use access controls, encryption in transit, tenant isolation, audit trails, and least-privilege operational practices to protect it. No online system can promise absolute security." }

                h2 { "Your choices" }
                p { "Depending on your relationship with a workspace and applicable law, you may ask to access, correct, export, restrict, or delete personal information. You can also manage notification, profile, and privacy preferences inside the product." }

                h2 { "Children and learners" }
                p { "AulaLite is provided to learners through an academy, instructor, employer, or parent-authorized program. Workspace owners must only invite learners when they have an appropriate legal basis and must follow local age, education, and parental-consent requirements." }

                h2 { "Contact" }
                p { "Questions or privacy requests can be sent to " a { href: "mailto:hello@elementors.guru", "hello@elementors.guru" } ". We may update this notice as the service and our obligations evolve; material changes will be communicated through the service or account email." }
            }
        }
    }
}

#[component]
pub fn Terms() -> Element {
    rsx! {
        PublicPage {
            eyebrow: "Service terms".to_string(),
            title: "Clear rules for a learning workspace".to_string(),
            summary: "These terms govern access to AulaLite and help academy owners, instructors, and learners understand their responsibilities.".to_string(),
            section {
                p { class: "public-page__updated", "Last updated July 16, 2026" }
                h2 { "Using AulaLite" }
                p { "You must provide accurate account information, keep credentials secure, and use the service only when you are legally able and authorized to do so. If you create a workspace for an organization, you confirm that you can accept these terms for it." }

                h2 { "Workspace administration" }
                p { "Workspace owners control membership, roles, course access, recordings, and integrations. They are responsible for their instructors and learners, the lawfulness of uploaded content, required notices and consents, and responding to requests about workspace-controlled education records." }

                h2 { "Subscriptions and payment" }
                p { "Paid plans renew for the billing period shown at checkout until cancelled. Fees, included usage, and applicable taxes are presented before purchase. Plan changes, usage safeguards, cancellation timing, and any refund eligibility are governed by the checkout terms and billing page available to the workspace administrator." }

                h2 { "Your content" }
                p { "You retain ownership of content you or your workspace upload. You grant us the limited permission needed to host, process, transmit, back up, and display it solely to provide, secure, and improve AulaLite. You must have the rights needed to upload and share that content." }

                h2 { "Acceptable use" }
                p { "Do not use AulaLite to break the law; infringe rights; harass or exploit others; distribute malware; probe or bypass security; disrupt the service; scrape data without permission; impersonate another person; or expose recordings, learner data, or credentials to unauthorized people." }

                h2 { "Service changes and availability" }
                p { "We may improve, replace, or discontinue features and will provide reasonable notice when a material change affects paid use. Maintenance, internet conditions, third-party dependencies, emergencies, and security work can occasionally affect availability." }

                h2 { "Suspension and termination" }
                p { "We may limit or suspend access when reasonably necessary to protect users, the service, or third parties; address non-payment; investigate abuse; or comply with law. You may stop using the service at any time and workspace administrators can cancel through billing controls." }

                h2 { "Disclaimers and responsibility" }
                p { "AulaLite is a software platform, not a school, accreditation body, legal adviser, or guarantor of learning outcomes. To the extent permitted by law, the service is provided without implied warranties beyond any express commitments in an order or written agreement, and neither party is responsible for indirect or unforeseeable losses." }

                h2 { "Contact and changes" }
                p { "Questions about these terms can be sent to " a { href: "mailto:hello@elementors.guru", "hello@elementors.guru" } ". If these terms materially change, we will give reasonable notice before the new terms take effect." }
            }
        }
    }
}

#[component]
pub fn NotFound(segments: Vec<String>) -> Element {
    let attempted = if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    };
    rsx! {
        div { class: "not-found-page",
            div { class: "not-found-page__orb", aria_hidden: "true", "404" }
            a { class: "marketing-brand", href: "/", aria_label: "AulaLite home",
                img { src: "/assets/brand/aulalite-mark.svg", alt: "", aria_hidden: "true" }
                span { "Aula", strong { "Lite" } }
            }
            main {
                p { class: "marketing-eyebrow", "A quiet corner of the academy" }
                h1 { "This page isn’t on the lesson plan." }
                p { "We couldn’t find " code { "{attempted}" } ". It may have moved, or you may not have access to it." }
                div { class: "not-found-page__actions",
                    a { class: "marketing-button", href: "/", "Return home" }
                    a { class: "marketing-button marketing-button--quiet", href: "/login", "Sign in" }
                }
            }
            p { class: "not-found-page__help", "Still stuck? " a { href: "mailto:hello@elementors.guru", "Contact support" } }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legal_pages_have_dates_and_contact_paths() {
        let mut privacy = VirtualDom::new(Privacy);
        privacy.rebuild_in_place();
        let html = dioxus_ssr::render(&privacy);
        assert!(html.contains("Last updated July 16, 2026"));
        assert!(html.contains("hello@elementors.guru"));
        assert!(html.contains("We do not sell personal information"));
    }

    #[test]
    fn not_found_keeps_a_recovery_path() {
        fn app() -> Element {
            rsx! { NotFound { segments: vec!["missing".into(), "page".into()] } }
        }
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert!(html.contains("/missing/page"));
        assert!(html.contains("Return home"));
    }
}
