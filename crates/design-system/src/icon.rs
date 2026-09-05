use dioxus::prelude::*;
use dioxus_free_icons::icons::ld_icons::{
    LdBookOpen, LdCalendar, LdClipboardList, LdFileText, LdGraduationCap, LdHome, LdLogOut,
    LdMessageSquareText, LdPlus, LdRadio, LdSettings, LdUser, LdUsers, LdVideo,
};
use dioxus_free_icons::Icon;

#[derive(Clone, PartialEq)]
pub enum UiIcon {
    Dashboard,
    Courses,
    Schedule,
    Redeem,
    Assignment,
    File,
    Live,
    Video,
    Chat,
    People,
    User,
    SignOut,
    Add,
    Settings,
    Academy,
}

#[derive(Props, Clone, PartialEq)]
pub struct UiIconProps {
    pub icon: UiIcon,
    #[props(default = 18)]
    pub size: u32,
    #[props(default = "ui-icon".to_string())]
    pub class: String,
    #[props(default)]
    pub title: Option<String>,
}

#[component]
pub fn UiIconView(props: UiIconProps) -> Element {
    let size = props.size;
    let class = props.class.clone();
    let title = props.title.clone().unwrap_or_default();
    match props.icon {
        UiIcon::Dashboard => {
            rsx! { Icon { icon: LdHome, width: size, height: size, class, title } }
        }
        UiIcon::Courses => {
            rsx! { Icon { icon: LdBookOpen, width: size, height: size, class, title } }
        }
        UiIcon::Schedule => {
            rsx! { Icon { icon: LdCalendar, width: size, height: size, class, title } }
        }
        UiIcon::Redeem => {
            rsx! { Icon { icon: LdGraduationCap, width: size, height: size, class, title } }
        }
        UiIcon::Assignment => {
            rsx! { Icon { icon: LdClipboardList, width: size, height: size, class, title } }
        }
        UiIcon::File => rsx! { Icon { icon: LdFileText, width: size, height: size, class, title } },
        UiIcon::Live => rsx! { Icon { icon: LdRadio, width: size, height: size, class, title } },
        UiIcon::Video => rsx! { Icon { icon: LdVideo, width: size, height: size, class, title } },
        UiIcon::Chat => {
            rsx! { Icon { icon: LdMessageSquareText, width: size, height: size, class, title } }
        }
        UiIcon::People => rsx! { Icon { icon: LdUsers, width: size, height: size, class, title } },
        UiIcon::User => rsx! { Icon { icon: LdUser, width: size, height: size, class, title } },
        UiIcon::SignOut => {
            rsx! { Icon { icon: LdLogOut, width: size, height: size, class, title } }
        }
        UiIcon::Add => rsx! { Icon { icon: LdPlus, width: size, height: size, class, title } },
        UiIcon::Settings => {
            rsx! { Icon { icon: LdSettings, width: size, height: size, class, title } }
        }
        UiIcon::Academy => {
            rsx! { Icon { icon: LdGraduationCap, width: size, height: size, class, title } }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_renders_svg() {
        fn app() -> Element {
            rsx! { UiIconView { icon: UiIcon::Dashboard, title: Some("Dashboard".to_string()) } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("<svg"), "got: {html}");
        assert!(html.contains("Dashboard"), "got: {html}");
    }
}
