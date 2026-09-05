// crates/features-courses/src/live_room_devices.rs
//! Media-device enumeration helpers.
//!
//! Wraps `navigator.mediaDevices.enumerateDevices()` into a plain
//! `Vec<MediaDevice>` so the prejoin / in-call device pickers can populate
//! `<select>` dropdowns without touching `web_sys` directly. Browser builds use
//! typed `web_sys`; native desktop/mobile builds ask the WebView media bridge
//! so device handles stay in the engine that owns the peer connection.
//!
//! Browsers withhold device *labels* until the page has been granted a
//! media permission at least once. The prejoin flow always calls
//! `getUserMedia` before enumerating, so by the time these helpers run the
//! labels are populated; `label_or_fallback` still provides a friendly
//! placeholder for the pre-permission case.

/// The three `MediaDeviceKind` values we care about, as a typed enum so call
/// sites can filter without matching on raw strings. Mirrors the WHATWG
/// `MediaDeviceKind` ("audioinput" | "audiooutput" | "videoinput").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    AudioInput,
    AudioOutput,
    VideoInput,
}

impl DeviceKind {
    /// Wire string, matching the DOM `MediaDeviceKind` values.
    pub fn as_wire(self) -> &'static str {
        match self {
            DeviceKind::AudioInput => "audioinput",
            DeviceKind::AudioOutput => "audiooutput",
            DeviceKind::VideoInput => "videoinput",
        }
    }
}

/// A single enumerated media device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaDevice {
    pub device_id: String,
    pub label: String,
    pub kind: DeviceKind,
}

impl MediaDevice {
    /// A user-presentable label. Browsers return an empty label before the
    /// page has been granted a media permission; in that case we synthesise a
    /// stable, numbered placeholder so the dropdown is never blank.
    pub fn label_or_fallback(&self, index: usize) -> String {
        if self.label.trim().is_empty() {
            let noun = match self.kind {
                DeviceKind::AudioInput => "Microphone",
                DeviceKind::AudioOutput => "Speaker",
                DeviceKind::VideoInput => "Camera",
            };
            format!("{noun} {}", index + 1)
        } else {
            self.label.clone()
        }
    }
}

/// Split a flat device list into (cameras, mics, speakers) preserving order.
/// Pure helper so it is unit-testable on host.
pub fn partition_by_kind(
    devices: &[MediaDevice],
) -> (Vec<MediaDevice>, Vec<MediaDevice>, Vec<MediaDevice>) {
    let mut cameras = Vec::new();
    let mut mics = Vec::new();
    let mut speakers = Vec::new();
    for d in devices {
        match d.kind {
            DeviceKind::VideoInput => cameras.push(d.clone()),
            DeviceKind::AudioInput => mics.push(d.clone()),
            DeviceKind::AudioOutput => speakers.push(d.clone()),
        }
    }
    (cameras, mics, speakers)
}

// ---------------------------------------------------------------------------
// Mobile camera selection (front / back via facingMode)
// ---------------------------------------------------------------------------

/// Which physical camera a mobile publisher wants. Maps to the WHATWG
/// `MediaTrackConstraints.facingMode` enum, the portable way to pick the
/// front/back camera on phones where `deviceId`s are opaque and unstable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Facing {
    /// Selfie camera — `facingMode: "user"`.
    Front,
    /// Rear / world-facing camera — `facingMode: "environment"`.
    Back,
}

impl Facing {
    /// The `facingMode` wire string this maps to.
    pub fn as_wire(self) -> &'static str {
        match self {
            Facing::Front => "user",
            Facing::Back => "environment",
        }
    }

    /// The opposite camera, for a one-tap "flip camera" control.
    pub fn flipped(self) -> Facing {
        match self {
            Facing::Front => Facing::Back,
            Facing::Back => Facing::Front,
        }
    }
}

/// Heuristic: does this look like a mobile/touch device where front/back
/// `facingMode` selection is meaningful (vs a desktop with named USB cams)?
///
/// Pure over a User-Agent string so it is unit-testable on host; the wasm
/// `is_mobile()` wrapper feeds it `navigator.userAgent`. Matches the common
/// mobile/tablet tokens. Deliberately conservative — a false negative just
/// falls back to the deviceId picker, which still works.
pub fn ua_is_mobile(user_agent: &str) -> bool {
    let ua = user_agent.to_ascii_lowercase();
    const TOKENS: &[&str] = &[
        "android",
        "iphone",
        "ipad",
        "ipod",
        "mobile",
        "windows phone",
        "blackberry",
        "webos",
    ];
    TOKENS.iter().any(|t| ua.contains(t))
}

/// Count distinct cameras whose label hints at a front/back facing direction.
/// Mobile browsers often expose multiple `videoinput`s with labels like
/// "front camera" / "back camera"; when at least two are present a flip control
/// is worth showing even if the UA sniff is inconclusive. Pure + testable.
pub fn has_multiple_cameras(cameras: &[MediaDevice]) -> bool {
    cameras.len() > 1
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::{ua_is_mobile, DeviceKind, Facing, MediaDevice};
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;

    /// Whether the running browser looks like a mobile/touch device, so the
    /// prejoin/broadcast UI can show a front/back "flip camera" control instead
    /// of (or alongside) the deviceId dropdown. Reads `navigator.userAgent` and
    /// defers the actual match to the pure `ua_is_mobile`.
    pub fn is_mobile() -> bool {
        web_sys::window()
            .map(|w| w.navigator().user_agent().unwrap_or_default())
            .map(|ua| ua_is_mobile(&ua))
            .unwrap_or(false)
    }

    /// True when the viewport is in portrait orientation (taller than wide).
    /// Used to swap the requested capture width/height so the camera frame is
    /// not letter-boxed / rotated on a phone held upright.
    fn is_portrait() -> bool {
        if let Some(win) = web_sys::window() {
            let w = win
                .inner_width()
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let h = win
                .inner_height()
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            return h > w;
        }
        false
    }

    /// Build a video `MediaTrackConstraints` for the requested facing camera,
    /// with an orientation-aware ideal resolution. `exact` requests the camera
    /// hard (getUserMedia rejects if unavailable) — we use `ideal` instead so a
    /// phone with only one camera still opens rather than throwing.
    fn facing_video_constraints(facing: Facing) -> web_sys::MediaTrackConstraints {
        let vc = web_sys::MediaTrackConstraints::new();

        // facingMode as a ConstrainDOMString { ideal: "user" | "environment" }
        // so it is a soft preference, not a hard requirement.
        let fm = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &fm,
            &JsValue::from_str("ideal"),
            &JsValue::from_str(facing.as_wire()),
        );
        vc.set_facing_mode(&fm);

        // Orientation-aware ideal resolution. Portrait phones want the long
        // edge vertical; landscape wants it horizontal. `ideal` keeps these as
        // hints so the camera picks the nearest supported mode.
        let (ideal_w, ideal_h) = if is_portrait() {
            (720, 1280)
        } else {
            (1280, 720)
        };
        let w = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &w,
            &JsValue::from_str("ideal"),
            &JsValue::from_f64(ideal_w as f64),
        );
        vc.set_width(&w);
        let h = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &h,
            &JsValue::from_str("ideal"),
            &JsValue::from_f64(ideal_h as f64),
        );
        vc.set_height(&h);

        vc
    }

    /// Open a camera by front/back facing direction (+ optional mic) via
    /// `getUserMedia`, returning the live `MediaStream`. This is the mobile
    /// publish entry point: the prejoin/broadcast UI calls it to pick the
    /// selfie vs world camera without knowing opaque mobile deviceIds.
    ///
    /// `with_audio` controls whether a mic track is requested too; pass `false`
    /// when only swapping the camera mid-call (audio is handled elsewhere).
    /// Returns the same humanized error shape as the deviceId path on failure.
    pub async fn open_camera_facing(
        facing: Facing,
        with_audio: bool,
    ) -> Result<web_sys::MediaStream, String> {
        let win = web_sys::window().ok_or_else(|| "no window".to_string())?;
        let media = win
            .navigator()
            .media_devices()
            .map_err(|e| format!("media_devices: {e:?}"))?;

        let constraints = web_sys::MediaStreamConstraints::new();
        let vc = facing_video_constraints(facing);
        constraints.set_video(vc.as_ref());
        if with_audio {
            constraints.set_audio(&JsValue::TRUE);
        } else {
            constraints.set_audio(&JsValue::FALSE);
        }

        let promise = media
            .get_user_media_with_constraints(&constraints)
            .map_err(|e| format!("getUserMedia: {e:?}"))?;
        let value = JsFuture::from(promise)
            .await
            .map_err(|e| format!("getUserMedia await: {e:?}"))?;
        value
            .dyn_into::<web_sys::MediaStream>()
            .map_err(|_| "stream cast".to_string())
    }

    /// Request Picture-in-Picture for the `<video>` element with the given id.
    ///
    /// Calls `HTMLVideoElement.requestPictureInPicture()` dynamically via
    /// `Reflect` rather than the typed web-sys binding, because that binding is
    /// gated behind `--cfg=web_sys_unstable_apis`. Reflect keeps us off the
    /// unstable flag while still using the real DOM API. Returns `Ok(())` once
    /// the request resolves; `Err` when the element is missing, the API is
    /// unsupported, or the browser rejects the request (e.g. user gesture /
    /// `disablePictureInPicture`).
    pub async fn request_picture_in_picture(element_id: &str) -> Result<(), String> {
        let win = web_sys::window().ok_or_else(|| "no window".to_string())?;
        let doc = win.document().ok_or_else(|| "no document".to_string())?;
        let el = doc
            .get_element_by_id(element_id)
            .ok_or_else(|| format!("no element #{element_id}"))?;
        let func = js_sys::Reflect::get(&el, &JsValue::from_str("requestPictureInPicture"))
            .map_err(|_| "no requestPictureInPicture".to_string())?;
        let func = func
            .dyn_into::<js_sys::Function>()
            .map_err(|_| "Picture-in-Picture is not supported in this browser".to_string())?;
        let ret = js_sys::Reflect::apply(&func, &el, &js_sys::Array::new())
            .map_err(|e| format!("requestPictureInPicture: {e:?}"))?;
        if let Ok(promise) = ret.dyn_into::<js_sys::Promise>() {
            JsFuture::from(promise)
                .await
                .map_err(|e| format!("Picture-in-Picture was rejected: {e:?}"))?;
        }
        Ok(())
    }

    fn map_kind(kind: web_sys::MediaDeviceKind) -> DeviceKind {
        match kind {
            web_sys::MediaDeviceKind::Audioinput => DeviceKind::AudioInput,
            web_sys::MediaDeviceKind::Audiooutput => DeviceKind::AudioOutput,
            web_sys::MediaDeviceKind::Videoinput => DeviceKind::VideoInput,
            // `MediaDeviceKind` is `#[non_exhaustive]`-style (string enum);
            // any unknown future value is treated as a mic so it still shows
            // up somewhere rather than vanishing.
            _ => DeviceKind::AudioInput,
        }
    }

    /// Enumerate the browser's media devices. Returns an empty list (never an
    /// error) when the API is unavailable, so the picker degrades gracefully
    /// instead of blocking the join flow.
    pub async fn enumerate() -> Vec<MediaDevice> {
        let Some(win) = web_sys::window() else {
            return Vec::new();
        };
        let nav = win.navigator();
        let Ok(media) = nav.media_devices() else {
            return Vec::new();
        };
        let Ok(promise) = media.enumerate_devices() else {
            return Vec::new();
        };
        let Ok(value) = JsFuture::from(promise).await else {
            return Vec::new();
        };
        let array: js_sys::Array = value.unchecked_into();
        let mut out = Vec::with_capacity(array.length() as usize);
        for i in 0..array.length() {
            let item = array.get(i);
            if let Ok(info) = item.dyn_into::<web_sys::MediaDeviceInfo>() {
                out.push(MediaDevice {
                    device_id: info.device_id(),
                    label: info.label(),
                    kind: map_kind(info.kind()),
                });
            }
        }
        out
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::{DeviceKind, Facing, MediaDevice};

    /// Enumerate devices through the native renderer's WebView. Dioxus desktop
    /// and mobile both host the shared UI in a browser engine; keeping the
    /// device handles in that engine lets WebRTC consume them without moving
    /// platform media objects through Rust.
    pub async fn enumerate() -> Vec<MediaDevice> {
        let Ok(result) = crate::live_room_native::enumerate().await else {
            return Vec::new();
        };
        result
            .devices
            .into_iter()
            .filter_map(|device| {
                let kind = match device.kind.as_str() {
                    "audioinput" => DeviceKind::AudioInput,
                    "audiooutput" => DeviceKind::AudioOutput,
                    "videoinput" => DeviceKind::VideoInput,
                    _ => return None,
                };
                Some(MediaDevice {
                    device_id: device.device_id,
                    label: device.label,
                    kind,
                })
            })
            .collect()
    }

    pub async fn request_picture_in_picture(element_id: &str) -> Result<(), String> {
        crate::live_room_native::request_picture_in_picture(element_id).await
    }

    /// Native builds know their target without user-agent sniffing.
    pub fn is_mobile() -> bool {
        cfg!(any(target_os = "android", target_os = "ios"))
    }

    /// Ask the WebView-owned publisher to replace its camera by facing mode.
    /// The unit return type reflects that native media objects remain inside
    /// the WebView rather than crossing into Rust.
    pub async fn open_camera_facing(facing: Facing, _with_audio: bool) -> Result<(), String> {
        crate::live_room_native::switch_camera("", Some(facing.as_wire()), "broadcast-self-video")
            .await
    }
}

pub use imp::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: &str, label: &str, kind: DeviceKind) -> MediaDevice {
        MediaDevice {
            device_id: id.into(),
            label: label.into(),
            kind,
        }
    }

    #[test]
    fn partition_groups_by_kind_and_preserves_order() {
        let devices = vec![
            dev("c1", "FaceTime", DeviceKind::VideoInput),
            dev("m1", "Built-in Mic", DeviceKind::AudioInput),
            dev("s1", "Built-in Output", DeviceKind::AudioOutput),
            dev("c2", "USB Cam", DeviceKind::VideoInput),
        ];
        let (cams, mics, spks) = partition_by_kind(&devices);
        assert_eq!(cams.len(), 2);
        assert_eq!(cams[0].device_id, "c1");
        assert_eq!(cams[1].device_id, "c2");
        assert_eq!(mics.len(), 1);
        assert_eq!(spks.len(), 1);
    }

    #[test]
    fn empty_label_falls_back_to_numbered_noun() {
        let cam = dev("c1", "", DeviceKind::VideoInput);
        assert_eq!(cam.label_or_fallback(0), "Camera 1");
        let mic = dev("m1", "   ", DeviceKind::AudioInput);
        assert_eq!(mic.label_or_fallback(2), "Microphone 3");
    }

    #[test]
    fn present_label_is_used_verbatim() {
        let cam = dev("c1", "Logitech BRIO", DeviceKind::VideoInput);
        assert_eq!(cam.label_or_fallback(5), "Logitech BRIO");
    }

    #[test]
    fn kind_wire_strings_match_dom_contract() {
        assert_eq!(DeviceKind::AudioInput.as_wire(), "audioinput");
        assert_eq!(DeviceKind::AudioOutput.as_wire(), "audiooutput");
        assert_eq!(DeviceKind::VideoInput.as_wire(), "videoinput");
    }

    #[test]
    fn facing_wire_strings_match_dom_contract() {
        assert_eq!(Facing::Front.as_wire(), "user");
        assert_eq!(Facing::Back.as_wire(), "environment");
    }

    #[test]
    fn facing_flip_toggles_direction() {
        assert_eq!(Facing::Front.flipped(), Facing::Back);
        assert_eq!(Facing::Back.flipped(), Facing::Front);
        // Flipping twice is the identity.
        assert_eq!(Facing::Front.flipped().flipped(), Facing::Front);
    }

    #[test]
    fn ua_sniff_detects_common_mobile_devices() {
        let iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) Safari/605";
        let android = "Mozilla/5.0 (Linux; Android 14; Pixel 8) Chrome/120 Mobile Safari";
        let ipad = "Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) Safari/605";
        assert!(ua_is_mobile(iphone));
        assert!(ua_is_mobile(android));
        assert!(ua_is_mobile(ipad));
    }

    #[test]
    fn ua_sniff_rejects_desktop() {
        let mac = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Chrome/120 Safari/537";
        let win = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120 Safari/537";
        assert!(!ua_is_mobile(mac));
        assert!(!ua_is_mobile(win));
    }

    #[test]
    fn has_multiple_cameras_needs_two() {
        let one = vec![dev("c1", "Cam", DeviceKind::VideoInput)];
        let two = vec![
            dev("c1", "Front", DeviceKind::VideoInput),
            dev("c2", "Back", DeviceKind::VideoInput),
        ];
        assert!(!has_multiple_cameras(&one));
        assert!(has_multiple_cameras(&two));
    }
}
