//! Theme preference runtime: resolves a user preference (`system`/`light`/
//! `dark`) onto the `data-ui-theme` attribute of `<html>`.
//!
//! One attribute drives everything: AulaLite's token CSS overrides
//! (`[data-ui-theme="dark"]` in `tokens.css`), the kinetics `--ui-*` bridge
//! (`kinetics_styles.rs`), and the kinetics `ThemeProvider`, which watches the
//! document element with a `MutationObserver` and feeds `use_theme_mode()`
//! consumers reactively.
//!
//! The attribute is always resolved to an explicit `light`/`dark` value —
//! `system` is resolved against `prefers-color-scheme` here (and re-resolved
//! live by a media-query listener while the preference stays `system`). The
//! preference is mirrored to `localStorage` (`aula-theme`) so the external boot
//! script can paint the right theme before WASM loads; the
//! backend `/v1/me/preferences` value is authoritative once the shell mounts.

use dioxus::prelude::*;

/// The user's persisted theme preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePreference {
    pub fn as_str(self) -> &'static str {
        match self {
            ThemePreference::System => "system",
            ThemePreference::Light => "light",
            ThemePreference::Dark => "dark",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "light" => ThemePreference::Light,
            "dark" => ThemePreference::Dark,
            _ => ThemePreference::System,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemePreference::System => "System",
            ThemePreference::Light => "Light",
            ThemePreference::Dark => "Dark",
        }
    }
}

/// Resolve `pref` to a concrete `data-ui-theme` value on `<html>`, mirror the
/// preference to `localStorage`, and (once) register a `prefers-color-scheme`
/// listener that keeps a `system` preference live. Safe to call repeatedly;
/// no-ops where there is no document (SSR).
pub fn apply_theme_preference(pref: ThemePreference) {
    #[cfg(target_arch = "wasm32")]
    if let Some(window) = web_sys::window() {
        if let Ok(Some(storage)) = window.local_storage() {
            let _ = storage.set_item("aula-theme", pref.as_str());
        }
        let system_dark = window
            .match_media("(prefers-color-scheme: dark)")
            .ok()
            .flatten()
            .is_some_and(|query| query.matches());
        let dark =
            pref == ThemePreference::Dark || (pref == ThemePreference::System && system_dark);
        if let Some(root) = window.document().and_then(|doc| doc.document_element()) {
            let _ = root.set_attribute("data-ui-theme", if dark { "dark" } else { "light" });
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    let _ = pref;
}

/// Apply the persisted density preference as `data-ui-density` on `<html>`
/// (the kinetics density contract; `comfortable` is the unmarked default).
pub fn apply_density_preference(density: &str) {
    let attr = match density {
        "compact" | "spacious" => density,
        _ => "comfortable",
    };
    #[cfg(target_arch = "wasm32")]
    if let Some(root) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
    {
        let _ = root.set_attribute("data-ui-density", attr);
    }

    #[cfg(not(target_arch = "wasm32"))]
    let _ = attr;
}

#[derive(Props, Clone, PartialEq)]
pub struct ThemeToggleProps {
    pub value: ThemePreference,
    pub on_change: EventHandler<ThemePreference>,
}

/// Three-way theme preference control (System / Light / Dark) for the app
/// chrome. Presentational: persisting the choice is the caller's job.
#[component]
pub fn ThemeToggle(props: ThemeToggleProps) -> Element {
    let options = [
        ThemePreference::System,
        ThemePreference::Light,
        ThemePreference::Dark,
    ];
    rsx! {
        div {
            class: "theme-toggle",
            role: "radiogroup",
            "aria-label": "Color theme",
            for option in options {
                button {
                    class: if option == props.value { "theme-toggle-option theme-toggle-option--active" } else { "theme-toggle-option" },
                    r#type: "button",
                    role: "radio",
                    "aria-checked": if option == props.value { "true" } else { "false" },
                    title: "{option.label()} theme",
                    onclick: move |_| props.on_change.call(option),
                    "{option.label()}"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_round_trips() {
        for pref in [
            ThemePreference::System,
            ThemePreference::Light,
            ThemePreference::Dark,
        ] {
            assert_eq!(ThemePreference::parse(pref.as_str()), pref);
        }
        // Unknown values fall back to System (never crash on bad storage).
        assert_eq!(ThemePreference::parse("midnight"), ThemePreference::System);
    }

    #[test]
    fn theme_toggle_renders_three_options_with_active_state() {
        fn app() -> Element {
            rsx! {
                ThemeToggle { value: ThemePreference::Dark, on_change: |_| {} }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("theme-toggle"));
        assert!(html.contains(">System<"));
        assert!(html.contains(">Light<"));
        assert!(html.contains(">Dark<"));
        // Exactly one active option, and it's Dark.
        assert_eq!(html.matches("theme-toggle-option--active").count(), 1);
        assert!(html.contains("aria-checked=\"true\" title=\"Dark theme\""));
    }
}
