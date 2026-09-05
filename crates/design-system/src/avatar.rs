use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct AvatarProps {
    /// Display name; used for initials fallback + aria-label.
    pub name: String,
    #[props(default)]
    pub image_url: Option<String>,
    #[props(default)]
    pub size: AvatarSize,
}

#[derive(Clone, PartialEq, Default)]
pub enum AvatarSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
}

fn initials_from_name(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

#[component]
pub fn Avatar(props: AvatarProps) -> Element {
    let size_class = match props.size {
        AvatarSize::Xs => "ds-avatar ds-avatar--xs",
        AvatarSize::Sm => "ds-avatar ds-avatar--sm",
        AvatarSize::Md => "ds-avatar ds-avatar--md",
        AvatarSize::Lg => "ds-avatar ds-avatar--lg",
    };
    let initials = initials_from_name(&props.name);
    rsx! {
        span {
            class: size_class,
            "aria-label": "{props.name}",
            if let Some(url) = &props.image_url {
                img { class: "ds-avatar-img", src: "{url}", alt: "{props.name}" }
            } else {
                span { class: "ds-avatar-initials", "{initials}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_first_two_words() {
        assert_eq!(initials_from_name("Eve Lin"), "EL");
        assert_eq!(initials_from_name("Eve"), "E");
        assert_eq!(initials_from_name("eve  lin  smith"), "EL");
        assert_eq!(initials_from_name(""), "");
    }

    #[test]
    fn avatar_renders_initials_when_no_image() {
        fn app() -> Element {
            rsx! { Avatar { name: "Eve Lin".to_string() } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("EL"), "initials missing: {html}");
        assert!(
            html.contains("ds-avatar-initials"),
            "initials class missing: {html}"
        );
        assert!(
            html.contains("ds-avatar--md"),
            "default md class missing: {html}"
        );
    }

    #[test]
    fn avatar_renders_img_when_image_url_set() {
        fn app() -> Element {
            rsx! {
                Avatar {
                    name: "Eve".to_string(),
                    image_url: "/photo.jpg".to_string(),
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("<img"), "img tag missing: {html}");
        assert!(html.contains("src=\"/photo.jpg\""), "src missing: {html}");
    }
}
