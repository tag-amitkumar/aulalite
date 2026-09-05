use dioxus::prelude::*;

#[test]
fn forgot_password_renders_email_and_reset_control() {
    fn app() -> Element {
        rsx! {
            features_auth::ForgotPassword {}
        }
    }

    let mut vdom = VirtualDom::new(app);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);

    assert!(html.contains("Reset your password"));
    assert!(html.contains("placeholder=\"you@example.com\""));
    assert!(html.contains("Send reset link"));
    assert!(html.contains("auth-screen"));
    // PageHeader (Hero) replaces the ad-hoc heading block.
    assert!(html.contains("ds-page-header"));
    assert!(html.contains("AulaLite"));
}
