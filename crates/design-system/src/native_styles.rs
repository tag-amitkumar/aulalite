//! Native WebViews do not load `shell-web/index.html`, so its stylesheet links
//! are absent. Embed the synchronized design-system CSS once at the app root on
//! non-WASM targets; web keeps using cacheable external stylesheets.

use dioxus::prelude::*;

#[cfg(not(target_arch = "wasm32"))]
const TOKENS_CSS: &str = include_str!("../assets/tokens.css");
#[cfg(not(target_arch = "wasm32"))]
const COMPONENTS_CSS: &str = include_str!("../assets/components.css");

#[component]
pub fn NativeBaseStyles() -> Element {
    #[cfg(not(target_arch = "wasm32"))]
    {
        return rsx! {
            style { id: "aulalite-native-tokens", dangerous_inner_html: TOKENS_CSS }
            style { id: "aulalite-native-components", dangerous_inner_html: COMPONENTS_CSS }
        };
    }
    #[cfg(target_arch = "wasm32")]
    {
        VNode::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_styles_embed_tokens_and_component_rules() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            assert!(TOKENS_CSS.contains("--color-primary"));
            assert!(COMPONENTS_CSS.contains(".app-shell"));
            let mut dom = VirtualDom::new(NativeBaseStyles);
            dom.rebuild_in_place();
            let html = dioxus_ssr::render(&dom);
            assert!(html.contains("aulalite-native-tokens"), "{html}");
            assert!(html.contains("aulalite-native-components"), "{html}");
        }
    }
}
