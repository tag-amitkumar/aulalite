// crates/features-courses/src/live_room_video_fx.rs
//! Background blur / virtual-background bridge binding.
//!
//! Marshals calls to the `window.aula.blur` JS bridge
//! (`shell-web/public/assets/blur-bridge.js`) via `js_sys::Reflect`, mirroring
//! how `live_room_view::render_hls` drives `window.Hls`. The bridge runs
//! MediaPipe selfie segmentation in JS and returns a processed `MediaStream`
//! (canvas video + the original audio). Every failure path degrades to "no FX"
//! — the caller then publishes the raw camera — so background effects can never
//! break the broadcast.
//!
//! Mode strings are the wire contract with the bridge: `"off" | "blur" | "image"`.

/// Background-FX mode the teacher has selected.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FxMode {
    Off,
    Blur,
    Image,
}

impl FxMode {
    pub fn as_wire(self) -> &'static str {
        match self {
            FxMode::Off => "off",
            FxMode::Blur => "blur",
            FxMode::Image => "image",
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::MediaStream;

    /// The `window.aula.blur` object, or `None` if the bridge script didn't load.
    fn blur_obj() -> Option<js_sys::Object> {
        let win = web_sys::window()?;
        let aula = js_sys::Reflect::get(&win, &JsValue::from_str("aula")).ok()?;
        if aula.is_undefined() || aula.is_null() {
            return None;
        }
        let blur = js_sys::Reflect::get(&aula, &JsValue::from_str("blur")).ok()?;
        if blur.is_undefined() || blur.is_null() {
            return None;
        }
        blur.dyn_into::<js_sys::Object>().ok()
    }

    fn call(obj: &js_sys::Object, name: &str, args: &js_sys::Array) -> Option<JsValue> {
        let func = js_sys::Reflect::get(obj, &JsValue::from_str(name)).ok()?;
        let func = func.dyn_into::<js_sys::Function>().ok()?;
        js_sys::Reflect::apply(&func, obj, args).ok()
    }

    /// True when the bridge loaded and the browser can run the pipeline.
    pub fn is_supported() -> bool {
        match blur_obj() {
            Some(obj) => call(&obj, "isSupported", &js_sys::Array::new())
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            None => false,
        }
    }

    /// Start FX on `stream` in `mode`. Returns the processed stream, or the
    /// caller's own stream when FX is unavailable, so the caller always has a
    /// stream to publish.
    ///
    /// Every fallback returns `same_stream(stream)`, NOT `stream.clone()`: the
    /// latter is the DOM's `MediaStream.clone()`, which returns a detached
    /// stream of cloned tracks. The caller publishes what we return while also
    /// holding `stream` for teardown, so handing back copies meant stopping one
    /// never stopped the other and the camera stayed open.
    pub async fn start(stream: &MediaStream, mode: &str, bg_url: &str) -> MediaStream {
        let Some(obj) = blur_obj() else {
            return crate::live_room_ice::same_stream(stream);
        };
        let args = js_sys::Array::new();
        args.push(stream);
        args.push(&JsValue::from_str(mode));
        args.push(&JsValue::from_str(bg_url));
        let Some(ret) = call(&obj, "start", &args) else {
            return crate::live_room_ice::same_stream(stream);
        };
        // `start()` resolves to a Promise<MediaStream>.
        let resolved = match ret.dyn_into::<js_sys::Promise>() {
            Ok(p) => match JsFuture::from(p).await {
                Ok(v) => v,
                Err(_) => return crate::live_room_ice::same_stream(stream),
            },
            Err(v) => v,
        };
        resolved
            .dyn_into::<MediaStream>()
            .unwrap_or_else(|_| crate::live_room_ice::same_stream(stream))
    }

    /// True when a pipeline is currently running (the bridge's `start()` has
    /// been called and `stop()` has not). The broadcast uses this to decide
    /// between the FX-aware `set_source` path and the raw `replace_video_track`
    /// path on an in-call camera switch.
    pub fn is_active() -> bool {
        match blur_obj() {
            Some(obj) => call(&obj, "isActive", &js_sys::Array::new())
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            None => false,
        }
    }

    /// Swap the INPUT camera stream feeding the segmentation / passthrough
    /// pipeline without changing the published canvas track (so no WebRTC
    /// renegotiation). The bridge keeps the new stream's audio handling
    /// consistent with `start()`. No-op when no pipeline is running.
    pub fn set_source(stream: &MediaStream) {
        if let Some(obj) = blur_obj() {
            let args = js_sys::Array::new();
            args.push(stream);
            let _ = call(&obj, "setSource", &args);
        }
    }

    pub fn set_mode(mode: &str) {
        if let Some(obj) = blur_obj() {
            let args = js_sys::Array::new();
            args.push(&JsValue::from_str(mode));
            let _ = call(&obj, "setMode", &args);
        }
    }

    pub fn set_background(url: &str) {
        if let Some(obj) = blur_obj() {
            let args = js_sys::Array::new();
            args.push(&JsValue::from_str(url));
            let _ = call(&obj, "setBackground", &args);
        }
    }

    pub fn stop() {
        if let Some(obj) = blur_obj() {
            let _ = call(&obj, "stop", &js_sys::Array::new());
        }
    }
}

// Native builds capability-hide browser-only background blur/virtual backdrop
// controls. Those pipelines depend on the web shell's canvas/MediaPipe asset
// bridge and have not been exposed as an installed-app host capability.
#[cfg(not(target_arch = "wasm32"))]
mod imp {
    pub fn is_supported() -> bool {
        false
    }

    /// Always inactive when the native capability is hidden.
    pub fn is_active() -> bool {
        false
    }
}

pub use imp::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fx_mode_wire_strings_match_bridge_contract() {
        assert_eq!(FxMode::Off.as_wire(), "off");
        assert_eq!(FxMode::Blur.as_wire(), "blur");
        assert_eq!(FxMode::Image.as_wire(), "image");
    }

    /// On host (no browser / no bridge), both capability probes report false so
    /// the broadcast hides the FX control and never routes through the
    /// FX-aware camera switch.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn host_probes_report_inactive() {
        assert!(!is_supported());
        assert!(!is_active());
    }
}
