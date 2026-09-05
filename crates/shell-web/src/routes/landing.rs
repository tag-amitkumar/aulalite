use dioxus::prelude::*;

#[component]
pub fn Landing() -> Element {
    let mut active_panel = use_signal(|| 0usize);
    let mut active_story = use_signal(|| 0usize);

    rsx! {
        div { class: "marketing-page",
            a { class: "marketing-skip", href: "#marketing-main", "Skip to content" }

            header { class: "marketing-nav-wrap",
                div { class: "marketing-nav",
                    a { class: "marketing-brand", href: "/", aria_label: "AulaLite home",
                        img {
                            src: "/assets/brand/aulalite-mark.svg",
                            alt: "",
                            aria_hidden: "true",
                        }
                        span { "Aula", strong { "Lite" } }
                    }
                    nav { class: "marketing-nav__links", aria_label: "Public navigation",
                        a { href: "#platform", "Platform" }
                        a { href: "#live-class", "Live class" }
                        a { href: "#pricing", "Pricing" }
                    }
                    div { class: "marketing-nav__actions",
                        a { class: "marketing-link", href: "/login", "Sign in" }
                        a { class: "marketing-button marketing-button--small", href: "/signup", "Start your academy" }
                    }
                }
            }

            main { id: "marketing-main", class: "marketing-main", tabindex: "-1",
                section { class: "marketing-hero", aria_label: "AulaLite introduction",
                    div { class: "marketing-hero__glow marketing-hero__glow--one", aria_hidden: "true" }
                    div { class: "marketing-hero__glow marketing-hero__glow--two", aria_hidden: "true" }

                    div { class: "marketing-hero__copy",
                        h1 { class: "marketing-hero__headline max-w-6xl",
                            "Live learning, "
                            span { class: "marketing-hero__inline-image", aria_hidden: "true",
                                img {
                                    src: "https://picsum.photos/seed/aulalite-classroom/360/180",
                                    alt: "",
                                    width: "360",
                                    height: "180",
                                    loading: "eager",
                                }
                            }
                            " all in one rhythm."
                        }
                        p { class: "marketing-hero__lede",
                            "Run your academy from one composed workspace for live classes, courses, assignments, attendance, and the people behind every lesson."
                        }
                        div { class: "marketing-hero__actions",
                            a { class: "marketing-button marketing-button--accent", href: "/signup",
                                span { "Create your academy" }
                                span { class: "marketing-button__line", aria_hidden: "true" }
                            }
                            a { class: "marketing-button marketing-button--ghost", href: "#platform", "Explore the platform" }
                        }
                    }

                    figure { class: "marketing-hero__visual", "data-motion-image": "true",
                        div { class: "marketing-hero__image-frame",
                            img {
                                src: "https://picsum.photos/seed/aulalite-live-teaching/1600/1200",
                                alt: "",
                                width: "1600",
                                height: "1200",
                                loading: "eager",
                                "fetchpriority": "high",
                            }
                        }
                        figcaption { class: "marketing-hero__caption",
                            span { class: "marketing-live-dot", aria_hidden: "true" }
                            div {
                                strong { "Algebra foundations" }
                                span { "Live room is ready" }
                            }
                            span { class: "marketing-hero__caption-time", "09:30" }
                        }
                        div { class: "marketing-hero__note",
                            span { "Room health" }
                            strong { "Clear, steady, recording" }
                        }
                    }
                }

                section { class: "marketing-trust", aria_label: "Teams that AulaLite supports",
                    p { "Built around the human scale of teaching" }
                    div {
                        span { "Independent tutors" }
                        span { "Coaching academies" }
                        span { "Language schools" }
                        span { "Training teams" }
                    }
                }

                section { class: "marketing-section marketing-platform", id: "platform",
                    div { class: "marketing-section__intro marketing-section__intro--wide",
                        h2 { "One academy. No hand-offs." }
                        p { "The class, the course, and the follow-through share the same context. Your team spends less time rebuilding it and more time moving learners forward." }
                    }

                    div { class: "marketing-feature-grid",
                        a { class: "marketing-feature marketing-feature--primary", href: "#live-class",
                            div { class: "marketing-feature__copy",
                                p { class: "marketing-feature__signal", "Live class, built in" }
                                h3 { "The room is the operating system." }
                                p { "Schedule, teach, moderate, record, and reconcile attendance without sending learners through another tab." }
                                span { class: "marketing-text-link", "See the teaching loop" }
                            }
                            div { class: "marketing-room-mini", aria_hidden: "true",
                                div { class: "marketing-room-mini__top",
                                    span { "AulaLite" }
                                    span { "Live now" }
                                }
                                div { class: "marketing-room-mini__stage",
                                    p { "See the pattern before the formula." }
                                    strong { "x² + 5x + 6" }
                                }
                                div { class: "marketing-room-mini__people",
                                    span { "AM" }
                                    span { "JL" }
                                    span { "SK" }
                                    small { "24 present" }
                                }
                            }
                        }

                        article { class: "marketing-feature marketing-feature--image", "data-motion-image": "true",
                            img {
                                src: "https://picsum.photos/seed/aulalite-course-context/1200/800",
                                alt: "",
                                width: "1200",
                                height: "800",
                                loading: "lazy",
                            }
                            div { class: "marketing-feature__overlay" }
                            div { class: "marketing-feature__copy",
                                p { class: "marketing-feature__signal", "Courses keep their context" }
                                h3 { "Every lesson knows what comes next." }
                            }
                        }

                        article { class: "marketing-feature marketing-feature--accent",
                            div { class: "marketing-feature__copy",
                                p { class: "marketing-feature__signal", "Progress you can act on" }
                                h3 { "Notice the learner before the number." }
                                p { "Attendance, submissions, feedback, and progress become one readable story." }
                            }
                            div { class: "marketing-progress-orbit", aria_hidden: "true",
                                span { class: "marketing-progress-orbit__ring" }
                                span { class: "marketing-progress-orbit__value", "82%" }
                                span { class: "marketing-progress-orbit__note", "On track" }
                            }
                        }
                    }
                }

                section { class: "marketing-section marketing-journey", aria_label: "AulaLite workflow",
                    div { class: "marketing-section__intro",
                        h2 { "From first invite to Friday follow-through." }
                        p { "A connected week has fewer loose ends. Move through the full teaching rhythm without losing the thread." }
                    }

                    div { class: "marketing-accordion", "data-accordion": "true",
                        article {
                            class: if *active_panel.read() == 0 { "marketing-accordion__panel is-active" } else { "marketing-accordion__panel" },
                            "data-accordion-panel": "true",
                            img {
                                src: "https://picsum.photos/seed/aulalite-prepare/1000/1200",
                                alt: "",
                                width: "1000",
                                height: "1200",
                                loading: "lazy",
                            }
                            div { class: "marketing-accordion__wash" }
                            button {
                                class: "marketing-accordion__trigger",
                                r#type: "button",
                                aria_expanded: if *active_panel.read() == 0 { "true" } else { "false" },
                                aria_controls: "marketing-accordion-prepare",
                                onclick: move |_| active_panel.set(0),
                                "Prepare"
                            }
                            div {
                                id: "marketing-accordion-prepare",
                                class: "marketing-accordion__content",
                                role: "region",
                                aria_hidden: if *active_panel.read() == 0 { "false" } else { "true" },
                                h3 { "Start with a room that already knows the lesson." }
                                p { "Schedule the cohort, attach the material, and let learners see what is next." }
                            }
                        }
                        article {
                            class: if *active_panel.read() == 1 { "marketing-accordion__panel is-active" } else { "marketing-accordion__panel" },
                            "data-accordion-panel": "true",
                            img {
                                src: "https://picsum.photos/seed/aulalite-teach/1000/1200",
                                alt: "",
                                width: "1000",
                                height: "1200",
                                loading: "lazy",
                            }
                            div { class: "marketing-accordion__wash" }
                            button {
                                class: "marketing-accordion__trigger",
                                r#type: "button",
                                aria_expanded: if *active_panel.read() == 1 { "true" } else { "false" },
                                aria_controls: "marketing-accordion-teach",
                                onclick: move |_| active_panel.set(1),
                                "Teach"
                            }
                            div {
                                id: "marketing-accordion-teach",
                                class: "marketing-accordion__content",
                                role: "region",
                                aria_hidden: if *active_panel.read() == 1 { "false" } else { "true" },
                                h3 { "Stay present while the platform handles the pulse." }
                                p { "Monitor room health, participation, chat, polls, and recording in one composed view." }
                            }
                        }
                        article {
                            class: if *active_panel.read() == 2 { "marketing-accordion__panel is-active" } else { "marketing-accordion__panel" },
                            "data-accordion-panel": "true",
                            img {
                                src: "https://picsum.photos/seed/aulalite-respond/1000/1200",
                                alt: "",
                                width: "1000",
                                height: "1200",
                                loading: "lazy",
                            }
                            div { class: "marketing-accordion__wash" }
                            button {
                                class: "marketing-accordion__trigger",
                                r#type: "button",
                                aria_expanded: if *active_panel.read() == 2 { "true" } else { "false" },
                                aria_controls: "marketing-accordion-respond",
                                onclick: move |_| active_panel.set(2),
                                "Respond"
                            }
                            div {
                                id: "marketing-accordion-respond",
                                class: "marketing-accordion__content",
                                role: "region",
                                aria_hidden: if *active_panel.read() == 2 { "false" } else { "true" },
                                h3 { "Turn the class into the next useful action." }
                                p { "Publish the recording, assign practice, grade with context, and respond while it is fresh." }
                            }
                        }
                        article {
                            class: if *active_panel.read() == 3 { "marketing-accordion__panel is-active" } else { "marketing-accordion__panel" },
                            "data-accordion-panel": "true",
                            img {
                                src: "https://picsum.photos/seed/aulalite-grow/1000/1200",
                                alt: "",
                                width: "1000",
                                height: "1200",
                                loading: "lazy",
                            }
                            div { class: "marketing-accordion__wash" }
                            button {
                                class: "marketing-accordion__trigger",
                                r#type: "button",
                                aria_expanded: if *active_panel.read() == 3 { "true" } else { "false" },
                                aria_controls: "marketing-accordion-grow",
                                onclick: move |_| active_panel.set(3),
                                "Grow"
                            }
                            div {
                                id: "marketing-accordion-grow",
                                class: "marketing-accordion__content",
                                role: "region",
                                aria_hidden: if *active_panel.read() == 3 { "false" } else { "true" },
                                h3 { "See where care creates momentum." }
                                p { "Read academy trends, strengthen cohorts, and expand without changing systems." }
                            }
                        }
                    }
                }

                section { class: "marketing-live-section", id: "live-class",
                    div { class: "marketing-live-section__intro",
                        h2 { "A better class leaves a useful trail." }
                        p { "AulaLite treats live teaching as a complete loop. Each moment arrives with context and leaves the next person ready." }
                    }

                    div { class: "marketing-stack", "data-card-stack": "true",
                        article { class: "marketing-stack-card marketing-stack-card--prepare",
                            div { class: "marketing-stack-card__copy",
                                p { "Before the room opens" }
                                h3 { "The plan is already in the room." }
                                span { "Series scheduling, lesson context, invitations, and device checks arrive together." }
                            }
                            figure { class: "marketing-stack-card__visual", "data-motion-image": "true",
                                img {
                                    src: "https://picsum.photos/seed/aulalite-before-class/1400/1000",
                                    alt: "",
                                    width: "1400",
                                    height: "1000",
                                    loading: "lazy",
                                }
                                figcaption { "Next class · Wednesday, 6:00 PM" }
                            }
                        }
                        article { class: "marketing-stack-card marketing-stack-card--live",
                            div { class: "marketing-stack-card__copy",
                                p { "While everyone is together" }
                                h3 { "Control without leaving the teaching moment." }
                                span { "Presence, hand raises, polls, chat, stream health, and recording stay within reach." }
                            }
                            div { class: "marketing-stack-card__console", role: "group", aria_label: "Live room health preview",
                                div { class: "marketing-console__head",
                                    span { class: "marketing-live-dot", aria_hidden: "true" }
                                    strong { "Room is healthy" }
                                    small { "24 present" }
                                }
                                div { class: "marketing-console__meter",
                                    span { style: "--meter-width: 94%" }
                                }
                                div { class: "marketing-console__rows",
                                    div { span { "Stream" } strong { "Excellent" } }
                                    div { span { "Recording" } strong { "Active" } }
                                    div { span { "Participation" } strong { "High" } }
                                }
                            }
                        }
                        article { class: "marketing-stack-card marketing-stack-card--follow",
                            div { class: "marketing-stack-card__copy",
                                p { "After the room closes" }
                                h3 { "Nothing valuable disappears with the call." }
                                span { "Recording, attendance, assignments, notes, and progress continue the same learning story." }
                            }
                            figure { class: "marketing-stack-card__visual marketing-stack-card__visual--warm", "data-motion-image": "true",
                                img {
                                    src: "https://picsum.photos/seed/aulalite-after-class/1400/1000",
                                    alt: "",
                                    width: "1400",
                                    height: "1000",
                                    loading: "lazy",
                                }
                                figcaption { "Recording published · Practice assigned" }
                            }
                        }
                    }
                }

                section { class: "marketing-section marketing-testimonials", aria_label: "Customer stories",
                    div { class: "marketing-testimonials__shell", "data-carousel": "true",
                        div { class: "marketing-testimonials__portraits", aria_hidden: "true",
                            img { src: "https://picsum.photos/seed/aulalite-tutor/240/240", alt: "", width: "240", height: "240", loading: "lazy" }
                            img { src: "https://picsum.photos/seed/aulalite-director/240/240", alt: "", width: "240", height: "240", loading: "lazy" }
                            img { src: "https://picsum.photos/seed/aulalite-coach/240/240", alt: "", width: "240", height: "240", loading: "lazy" }
                        }
                        div { class: "marketing-testimonials__content",
                            div { class: "marketing-quote-stage",
                                match *active_story.read() {
                                    0 => rsx! {
                                        blockquote { class: "marketing-quote is-active", "data-carousel-slide": "true",
                                            p { "The class and everything around it finally feel like one product. Our teachers are calmer because the next step is always visible." }
                                            footer { strong { "Maya Laurent" } span { "Director, Northline Languages" } }
                                        }
                                    },
                                    1 => rsx! {
                                        blockquote { class: "marketing-quote is-active", "data-carousel-slide": "true",
                                            p { "We stopped stitching together attendance, recordings, and assignments. That alone gave our small team an afternoon back every week." }
                                            footer { strong { "Daniel Okafor" } span { "Founder, Fieldwork Academy" } }
                                        }
                                    },
                                    _ => rsx! {
                                        blockquote { class: "marketing-quote is-active", "data-carousel-slide": "true",
                                            p { "AulaLite feels designed around teaching, not around software administration. Learners notice the difference immediately." }
                                            footer { strong { "Sara Kim" } span { "Learning Lead, Common Ground" } }
                                        }
                                    },
                                }
                            }
                            div { class: "marketing-carousel__controls",
                                button {
                                    r#type: "button",
                                    aria_label: "Previous story",
                                    onclick: move |_| {
                                        let current = *active_story.read();
                                        active_story.set((current + 2) % 3);
                                    },
                                    "Prev"
                                }
                                span { aria_live: "polite", "{*active_story.read() + 1} / 3" }
                                button {
                                    r#type: "button",
                                    aria_label: "Next story",
                                    onclick: move |_| {
                                        let current = *active_story.read();
                                        active_story.set((current + 1) % 3);
                                    },
                                    "Next"
                                }
                            }
                        }
                    }
                }

                section { class: "marketing-section marketing-pricing", id: "pricing",
                    div { class: "marketing-section__intro marketing-section__intro--wide",
                        h2 { "Choose the pace. Keep the whole academy." }
                        p { "One subscription covers your institution. Teachers, learners, and parents work from the same calm, branded home." }
                    }
                    div { class: "marketing-pricing-grid",
                        article { class: "marketing-price-card",
                            div { class: "marketing-price-card__head",
                                p { class: "marketing-price-card__name", "Starter" }
                                p { class: "marketing-price", span { "$49" } " / month" }
                            }
                            p { "For tutors and focused academies building a professional learning operation." }
                            ul {
                                li { "Up to 50 seats" }
                                li { "10,000 live-class minutes" }
                                li { "50 GB recording storage" }
                                li { "Courses, assignments, analytics, and branding" }
                            }
                            a { class: "marketing-button marketing-button--wide marketing-button--ghost-dark", href: "/signup", "Start with Starter" }
                        }
                        article { class: "marketing-price-card marketing-price-card--featured",
                            div { class: "marketing-price-card__head",
                                p { class: "marketing-price-card__name", "Pro" }
                                p { class: "marketing-price", span { "$149" } " / month" }
                            }
                            p { "For multi-teacher teams ready for more cohorts, live time, and recording capacity." }
                            ul {
                                li { "Up to 250 seats" }
                                li { "60,000 live-class minutes" }
                                li { "250 GB recording storage" }
                                li { "Full admin, integrations, SSO, and audit surfaces" }
                            }
                            a { class: "marketing-button marketing-button--wide marketing-button--accent", href: "/signup", "Choose Pro" }
                        }
                    }
                    p { class: "marketing-pricing__note", "Prices shown in USD. Usage safeguards help prevent accidental overages." }
                }

                section { class: "marketing-cta",
                    div { class: "marketing-cta__glow", aria_hidden: "true" }
                    h2 { "Make the next class feel like one clear thought." }
                    p { "Create your workspace, invite your team, and shape AulaLite around the way you teach." }
                    div { class: "marketing-cta__actions",
                        a { class: "marketing-button marketing-button--accent", href: "/signup", "Create your academy" }
                        a { class: "marketing-link marketing-link--light", href: "/login", "Already use AulaLite? Sign in" }
                    }
                }
            }

            footer { class: "marketing-footer",
                div { class: "marketing-footer__brand",
                    div { class: "marketing-brand marketing-brand--footer",
                        img { src: "/assets/brand/aulalite-mark.svg", alt: "" }
                        span { "Aula", strong { "Lite" } }
                    }
                    p { "Live learning infrastructure for modern academies, by Elementors." }
                }
                nav { aria_label: "Footer navigation",
                    a { href: "/login", "Sign in" }
                    a { href: "/signup", "Create account" }
                    a { href: "/privacy", "Privacy" }
                    a { href: "/terms", "Terms" }
                    a { href: "mailto:hello@elementors.guru", "Contact" }
                }
                small { "© 2026 Elementors. All rights reserved." }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landing_renders_acquisition_and_pricing_paths() {
        let mut vdom = VirtualDom::new(Landing);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Live learning"), "hero missing: {html}");
        assert!(html.contains("Create your academy"), "CTA missing: {html}");
        assert!(html.contains("$49"), "starter price missing: {html}");
        assert!(html.contains("$149"), "pro price missing: {html}");
        assert!(
            html.contains("href=\"/login\""),
            "login path missing: {html}"
        );
        assert!(
            html.contains("href=\"/signup\""),
            "signup path missing: {html}"
        );
    }
}
