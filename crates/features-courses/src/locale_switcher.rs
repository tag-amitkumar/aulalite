//! Language switcher for the app chrome. A compact `<select>` listing every
//! supported locale by its endonym; choosing one updates the active-locale
//! context signal. The [`design_system::LocaleProvider`] mounted at the app
//! root reacts to that change by setting `document.dir` / `lang` and persisting
//! the choice to `localStorage` (wasm) — so this component stays presentational
//! and dependency-free (no direct DOM / storage access here).
//!
//! Reuses the design-system `Select` primitive so it inherits the form-control
//! styling and dark-mode treatment. Mount it in the shell topbar (next to the
//! theme toggle) or the settings page.

use design_system::{use_locale, Locale, Select, SelectOption};
use dioxus::prelude::*;

#[component]
pub fn LocaleSwitcher() -> Element {
    // The active-locale signal lives in LocaleProvider's context. Writing it
    // triggers the provider's effect, which applies dir/lang + persistence.
    let mut locale = use_locale();
    let current = *locale.read();

    let options: Vec<SelectOption> = Locale::ALL
        .iter()
        .map(|l| SelectOption {
            value: l.code().to_string(),
            label: l.native_label().to_string(),
        })
        .collect();

    rsx! {
        label { class: "locale-switcher", "aria-label": "Language",
            // A globe-ish glyph keeps the control recognizable without pulling
            // in an icon dependency; decorative, so hidden from a11y tree.
            span { class: "locale-switcher-glyph", "aria-hidden": "true", "🌐" }
            Select {
                value: current.code().to_string(),
                options,
                on_change: move |code: String| {
                    locale.set(Locale::parse(&code));
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use design_system::LocaleProvider;

    #[test]
    fn renders_all_locales_with_current_selected() {
        fn app() -> Element {
            rsx! {
                LocaleProvider { initial: Locale::Es, LocaleSwitcher {} }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        // Every locale's endonym is listed as an option.
        assert!(html.contains("English"), "missing English option: {html}");
        assert!(html.contains("Español"), "missing Spanish option: {html}");
        assert!(html.contains("العربية"), "missing Arabic option: {html}");
        // The provider's initial locale is the selected value.
        assert!(
            html.contains("value=\"es\""),
            "Spanish should be selected: {html}"
        );
        assert!(html.contains("locale-switcher"));
    }
}
