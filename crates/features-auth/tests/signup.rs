use dioxus::prelude::*;

#[test]
fn signup_renders_account_fields_and_submit_control() {
    fn app() -> Element {
        rsx! {
            features_auth::Signup {
                on_success: |_| {},
            }
        }
    }

    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);

    assert!(html.contains("Create your AulaLite account"));
    assert!(html.contains("placeholder=\"you@example.com\""));
    assert!(html.contains("placeholder=\"At least 8 characters\""));
    assert!(html.contains("placeholder=\"Repeat your password\""));
    assert!(html.contains("Create account"));
    assert!(html.contains("auth-screen"));
    // PageHeader (Hero) replaces the ad-hoc heading block.
    assert!(html.contains("ds-page-header"));
    assert!(html.contains("AulaLite"));
}
