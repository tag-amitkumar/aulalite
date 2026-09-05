//! Small browser-runtime probes shared by background UI work.

/// Whether the document is currently hidden (background tab, minimized PWA).
/// Host/SSR builds are always treated as visible.
pub fn page_is_hidden() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        return web_sys::window()
            .and_then(|window| window.document())
            .map(|document| document.hidden())
            .unwrap_or(false);
    }

    #[cfg(not(target_arch = "wasm32"))]
    false
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn host_runtime_is_visible() {
        assert!(!super::page_is_hidden());
    }
}
