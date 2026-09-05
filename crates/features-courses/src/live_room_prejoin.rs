// crates/features-courses/src/live_room_prejoin.rs
//! Pre-join device test + picker.
//!
//! Renders a self-view camera preview, a live mic-level meter, and
//! camera / mic / speaker `<select>` dropdowns, then fires `on_join` with the
//! chosen `{ camera_id, mic_id }` when the operator clicks "Go live".
//!
//! Browser builds use `web_sys`; native Dioxus renderers run the equivalent
//! capture/analyser flow inside their WebView through `live_room_native`.
//!
//! Lifecycle: the active preview `MediaStream` and `AudioContext` are held in
//! a `use_hook`-allocated `Rc<RefCell<…>>` so a device change can tear the old
//! capture down before opening the next, and `use_drop` releases everything on
//! unmount — without this the camera light stays on after leaving prejoin.

use crate::live_room_devices::MediaDevice;
use design_system::{Button, ButtonVariant, Select, SelectOption};
use dioxus::prelude::*;

/// What the operator picked in prejoin. Empty strings mean "browser default".
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrejoinChoice {
    pub camera_id: String,
    pub mic_id: String,
    pub speaker_id: String,
}

#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomPrejoinProps {
    /// Fired when the operator commits. Carries the selected device ids.
    pub on_join: EventHandler<PrejoinChoice>,
    /// CTA label. Teachers see "Go live"; students joining a call see "Join".
    #[props(default = "Go live".to_string())]
    pub join_label: String,
    /// Heading shown above the preview.
    #[props(default = "Check your camera and mic".to_string())]
    pub heading: String,
}

/// The deviceId of the camera AFTER `current` in `cameras`, wrapping around to
/// the first. Used by the prejoin flip control to cycle through cameras. When
/// `current` is unknown (or the list is short) it returns the first camera's id
/// (or empty when there are none). Pure so it is unit-testable on host.
pub fn next_camera_id(cameras: &[MediaDevice], current: &str) -> String {
    if cameras.is_empty() {
        return String::new();
    }
    let idx = cameras
        .iter()
        .position(|c| c.device_id == current)
        .unwrap_or(0);
    let next = (idx + 1) % cameras.len();
    cameras[next].device_id.clone()
}

/// Build `SelectOption`s from a device list, applying the numbered-fallback
/// labels. Pure so it is unit-testable on host.
pub fn device_options(devices: &[MediaDevice]) -> Vec<SelectOption> {
    devices
        .iter()
        .enumerate()
        .map(|(i, d)| SelectOption {
            value: d.device_id.clone(),
            label: d.label_or_fallback(i),
        })
        .collect()
}

#[component]
pub fn LiveRoomPrejoin(props: LiveRoomPrejoinProps) -> Element {
    // Device lists + current selections.
    let cameras = use_signal(Vec::<MediaDevice>::new);
    let mics = use_signal(Vec::<MediaDevice>::new);
    let speakers = use_signal(Vec::<MediaDevice>::new);
    let camera_id = use_signal(String::new);
    let mic_id = use_signal(String::new);
    let speaker_id = use_signal(String::new);
    // Mic level 0..=100, driven by the analyser RAF/interval loop.
    let mic_level = use_signal(|| 0u32);
    // Surfaced when getUserMedia is blocked/denied.
    let error = use_signal(|| Option::<String>::None);

    #[cfg(target_arch = "wasm32")]
    {
        imp::use_prejoin_media(cameras, mics, speakers, camera_id, mic_id, mic_level, error);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        native_imp::use_prejoin_media(cameras, mics, speakers, camera_id, mic_id, error);
    }

    let snapshot_cameras = cameras.read().clone();
    let snapshot_mics = mics.read().clone();
    let snapshot_speakers = speakers.read().clone();
    // Show a one-tap flip control on mobile (or whenever 2+ cameras exist). It
    // cycles `camera_id` to the next enumerated camera, which re-fires the
    // capture effect against the new deviceId. Best-effort: on a phone the
    // deviceIds are opaque but enumerable, so cycling still toggles front/back.
    let show_flip = crate::live_room_devices::is_mobile()
        || crate::live_room_devices::has_multiple_cameras(&snapshot_cameras);
    let camera_opts = device_options(&snapshot_cameras);
    let mic_opts = device_options(&snapshot_mics);
    let speaker_opts = device_options(&snapshot_speakers);
    let has_speaker_picker = !speaker_opts.is_empty();

    let level = (*mic_level.read()).clamp(0, 100);
    let level_pct = format!("{level}%");

    let error_body: Element = match error.read().as_ref() {
        Some(msg) => rsx! {
            div { class: "prejoin-error system-state system-state--error", "{msg}" }
        },
        None => rsx! {},
    };

    let on_join = props.on_join;
    let join_click = move |_| {
        on_join.call(PrejoinChoice {
            camera_id: camera_id.read().clone(),
            mic_id: mic_id.read().clone(),
            speaker_id: speaker_id.read().clone(),
        });
    };

    rsx! {
        div { class: "live-room-prejoin",
            h2 { class: "prejoin-heading", "{props.heading}" }
            {error_body}
            div { class: "prejoin-stage",
                // Self-view preview. The effect wires the camera stream onto
                // this element by id.
                video {
                    id: "prejoin-preview-video",
                    class: "prejoin-preview",
                    autoplay: true,
                    muted: true,
                    playsinline: true,
                }
                // Mic-level meter overlaid at the bottom of the preview.
                div { class: "prejoin-mic-meter", title: "Microphone level",
                    span {
                        class: "prejoin-mic-meter-fill",
                        style: "width: {level_pct}",
                        "aria-hidden": "true",
                    }
                }
            }

            div { class: "prejoin-controls",
                label { class: "prejoin-field",
                    span { class: "prejoin-field-label", "Camera" }
                    Select {
                        value: camera_id.read().clone(),
                        options: camera_opts,
                        on_change: move |v: String| {
                            let mut camera_id = camera_id;
                            camera_id.set(v);
                        },
                    }
                    // Best-effort flip: cycle to the next enumerated camera.
                    // Re-fires the capture effect (it subscribes to camera_id).
                    if show_flip {
                        button {
                            r#type: "button",
                            class: "prejoin-camera-flip",
                            title: "Switch between cameras",
                            onclick: move |_| {
                                let mut camera_id = camera_id;
                                let next = next_camera_id(&cameras.read(), &camera_id.read());
                                if !next.is_empty() {
                                    camera_id.set(next);
                                }
                            },
                            "Flip camera"
                        }
                    }
                }
                label { class: "prejoin-field",
                    span { class: "prejoin-field-label", "Microphone" }
                    Select {
                        value: mic_id.read().clone(),
                        options: mic_opts,
                        on_change: move |v: String| {
                            let mut mic_id = mic_id;
                            mic_id.set(v);
                        },
                    }
                }
                if has_speaker_picker {
                    label { class: "prejoin-field",
                        span { class: "prejoin-field-label", "Speaker" }
                        Select {
                            value: speaker_id.read().clone(),
                            options: speaker_opts,
                            on_change: move |v: String| {
                                let mut speaker_id = speaker_id;
                                speaker_id.set(v);
                            },
                        }
                    }
                }
            }

            div { class: "prejoin-actions",
                Button {
                    label: props.join_label.clone(),
                    variant: ButtonVariant::Primary,
                    on_click: join_click,
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native_imp {
    use crate::live_room_devices::{partition_by_kind, DeviceKind, MediaDevice};
    use dioxus::prelude::*;
    use std::cell::Cell;
    use std::rc::Rc;

    fn map_devices(devices: Vec<crate::live_room_native::NativeMediaDevice>) -> Vec<MediaDevice> {
        devices
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

    #[allow(clippy::too_many_arguments)]
    pub fn use_prejoin_media(
        mut cameras: Signal<Vec<MediaDevice>>,
        mut mics: Signal<Vec<MediaDevice>>,
        mut speakers: Signal<Vec<MediaDevice>>,
        mut camera_id: Signal<String>,
        mut mic_id: Signal<String>,
        mut error: Signal<Option<String>>,
    ) {
        let stopped: Rc<Cell<bool>> = use_hook(|| Rc::new(Cell::new(false)));
        {
            let stopped = stopped.clone();
            use_drop(move || {
                stopped.set(true);
                spawn(async {
                    let _ = crate::live_room_native::stop_prejoin().await;
                });
            });
        }

        let stopped_for_effect = stopped.clone();
        use_effect(move || {
            let selected_camera = camera_id.read().clone();
            let selected_mic = mic_id.read().clone();
            let stopped = stopped_for_effect.clone();
            spawn(async move {
                match crate::live_room_native::start_prejoin(&selected_camera, &selected_mic).await
                {
                    Ok(result) if !stopped.get() => {
                        let (cams, microphones, outputs) =
                            partition_by_kind(&map_devices(result.devices));
                        if camera_id.read().is_empty() {
                            if let Some(first) = cams.first() {
                                camera_id.set(first.device_id.clone());
                            }
                        }
                        if mic_id.read().is_empty() {
                            if let Some(first) = microphones.first() {
                                mic_id.set(first.device_id.clone());
                            }
                        }
                        cameras.set(cams);
                        mics.set(microphones);
                        speakers.set(outputs);
                        error.set(None);
                    }
                    Err(message) if !stopped.get() => error.set(Some(message)),
                    _ => {}
                }
            });
        });
    }
}

// ---------------------------------------------------------------------------
// wasm media wiring
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
mod imp {
    use crate::live_room_devices::{partition_by_kind, MediaDevice};
    use dioxus::prelude::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;

    /// Per-mount media handles we must release on unmount or device-switch:
    /// the live preview stream, the audio analyser graph, and the interval id
    /// driving the meter.
    #[derive(Default)]
    struct PreviewHandles {
        stream: Option<web_sys::MediaStream>,
        audio_ctx: Option<web_sys::AudioContext>,
        // Interval handle id from setInterval, cleared on teardown.
        meter_interval: Option<i32>,
        // Keep the interval closure alive for as long as the interval runs.
        _meter_closure: Option<Closure<dyn FnMut()>>,
    }

    impl PreviewHandles {
        fn teardown(&mut self) {
            if let (Some(id), Some(win)) = (self.meter_interval.take(), web_sys::window()) {
                win.clear_interval_with_handle(id);
            }
            self._meter_closure = None;
            if let Some(ctx) = self.audio_ctx.take() {
                let _ = ctx.close();
            }
            if let Some(stream) = self.stream.take() {
                let tracks = stream.get_tracks();
                for i in 0..tracks.length() {
                    if let Ok(t) = tracks.get(i).dyn_into::<web_sys::MediaStreamTrack>() {
                        t.stop();
                    }
                }
            }
        }
    }

    /// Hook orchestrating prejoin media: getUserMedia → preview + analyser,
    /// then enumerate devices into the dropdown signals. Re-opens capture when
    /// `camera_id` / `mic_id` change. Releases everything on unmount.
    #[allow(clippy::too_many_arguments)]
    pub fn use_prejoin_media(
        mut cameras: Signal<Vec<MediaDevice>>,
        mut mics: Signal<Vec<MediaDevice>>,
        mut speakers: Signal<Vec<MediaDevice>>,
        mut camera_id: Signal<String>,
        mut mic_id: Signal<String>,
        mic_level: Signal<u32>,
        mut error: Signal<Option<String>>,
    ) {
        let handles: Rc<RefCell<PreviewHandles>> =
            use_hook(|| Rc::new(RefCell::new(PreviewHandles::default())));

        // Release on unmount.
        {
            let handles_drop = handles.clone();
            use_drop(move || {
                handles_drop.borrow_mut().teardown();
            });
        }

        // (Re)open capture whenever the selected camera / mic changes. Reading
        // the signals inside the effect subscribes us to those changes.
        let handles_eff = handles.clone();
        use_effect(move || {
            let want_camera = camera_id.read().clone();
            let want_mic = mic_id.read().clone();
            let handles_eff = handles_eff.clone();
            wasm_bindgen_futures::spawn_local(async move {
                // Tear down any prior capture before opening the next.
                handles_eff.borrow_mut().teardown();

                let Some(win) = web_sys::window() else {
                    return;
                };
                let media = match win.navigator().media_devices() {
                    Ok(m) => m,
                    Err(e) => {
                        error.set(Some(format!("Media devices unavailable: {e:?}")));
                        return;
                    }
                };

                // Build constraints honoring any chosen device ids.
                let constraints = build_constraints(&want_camera, &want_mic);
                let stream = match media.get_user_media_with_constraints(&constraints) {
                    Ok(promise) => match JsFuture::from(promise).await {
                        Ok(v) => v.unchecked_into::<web_sys::MediaStream>(),
                        Err(e) => {
                            error.set(Some(humanize_gum_error(&e)));
                            return;
                        }
                    },
                    Err(e) => {
                        error.set(Some(humanize_gum_error(&e)));
                        return;
                    }
                };
                error.set(None);

                // Wire the stream onto the preview <video>.
                if let Some(doc) = win.document() {
                    if let Some(el) = doc.get_element_by_id("prejoin-preview-video") {
                        if let Ok(media_el) = el.dyn_into::<web_sys::HtmlMediaElement>() {
                            media_el.set_src_object(Some(&stream));
                        }
                    }
                }

                // Spin up the mic-level analyser.
                start_mic_meter(&stream, &handles_eff, mic_level);

                handles_eff.borrow_mut().stream = Some(stream);

                // Enumerate devices now that permission is granted (labels are
                // populated). Seed the selected ids from the active tracks so
                // the dropdowns reflect what is actually live.
                let devices = crate::live_room_devices::enumerate().await;
                let (cams, ms, sps) = partition_by_kind(&devices);

                // Default-select the first of each list if nothing chosen yet.
                if camera_id.read().is_empty() {
                    if let Some(first) = cams.first() {
                        camera_id.set(first.device_id.clone());
                    }
                }
                if mic_id.read().is_empty() {
                    if let Some(first) = ms.first() {
                        mic_id.set(first.device_id.clone());
                    }
                }

                cameras.set(cams);
                mics.set(ms);
                speakers.set(sps);
            });
        });
    }

    /// getUserMedia constraints honoring optional exact device ids.
    fn build_constraints(camera_id: &str, mic_id: &str) -> web_sys::MediaStreamConstraints {
        let constraints = web_sys::MediaStreamConstraints::new();

        if camera_id.is_empty() {
            constraints.set_video(&JsValue::TRUE);
        } else {
            let vc = web_sys::MediaTrackConstraints::new();
            vc.set_device_id(&JsValue::from_str(camera_id));
            constraints.set_video(vc.as_ref());
        }

        if mic_id.is_empty() {
            constraints.set_audio(&JsValue::TRUE);
        } else {
            let ac = web_sys::MediaTrackConstraints::new();
            ac.set_device_id(&JsValue::from_str(mic_id));
            constraints.set_audio(ac.as_ref());
        }
        constraints
    }

    /// Build an AudioContext → analyser graph off the stream's audio track and
    /// start a setInterval loop that maps the RMS of the time-domain samples
    /// to a 0..=100 level, pushing it into `mic_level`.
    fn start_mic_meter(
        stream: &web_sys::MediaStream,
        handles: &Rc<RefCell<PreviewHandles>>,
        mut mic_level: Signal<u32>,
    ) {
        let Ok(ctx) = web_sys::AudioContext::new() else {
            return;
        };
        let source = match ctx.create_media_stream_source(stream) {
            Ok(s) => s,
            Err(_) => {
                let _ = ctx.close();
                return;
            }
        };
        let analyser = match ctx.create_analyser() {
            Ok(a) => a,
            Err(_) => {
                let _ = ctx.close();
                return;
            }
        };
        analyser.set_fft_size(512);
        // source.connect(analyser); `source` is a MediaStreamAudioSourceNode
        // which derefs to AudioNode.
        if source.connect_with_audio_node(&analyser).is_err() {
            let _ = ctx.close();
            return;
        }

        let bin_count = analyser.frequency_bin_count() as usize;
        let analyser_for_tick = analyser.clone();
        let tick = Closure::<dyn FnMut()>::new(move || {
            let mut buf = vec![0u8; bin_count];
            analyser_for_tick.get_byte_time_domain_data(&mut buf);
            // RMS deviation from the 128 midpoint → 0..=100.
            let mut sum_sq = 0.0_f64;
            for &b in buf.iter() {
                let centered = b as f64 - 128.0;
                sum_sq += centered * centered;
            }
            let rms = (sum_sq / buf.len().max(1) as f64).sqrt();
            // 128 is full-scale deviation; scale and clamp. Multiply for a
            // livelier meter at normal speech levels.
            let level = ((rms / 128.0) * 220.0).round().clamp(0.0, 100.0) as u32;
            mic_level.set(level);
        });

        if let Some(win) = web_sys::window() {
            if let Ok(id) = win.set_interval_with_callback_and_timeout_and_arguments_0(
                tick.as_ref().unchecked_ref(),
                100,
            ) {
                let mut h = handles.borrow_mut();
                h.audio_ctx = Some(ctx);
                h.meter_interval = Some(id);
                h._meter_closure = Some(tick);
                return;
            }
        }
        // Couldn't schedule — drop the graph.
        let _ = ctx.close();
        drop(tick);
    }

    /// Turn a getUserMedia rejection into a user-facing message. The error is a
    /// DOMException; its `name` tells us why.
    fn humanize_gum_error(e: &JsValue) -> String {
        let name = js_sys::Reflect::get(e, &JsValue::from_str("name"))
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default();
        match name.as_str() {
            "NotAllowedError" | "SecurityError" => {
                "Camera/microphone access was blocked. Allow access in your browser, then reload."
                    .to_string()
            }
            "NotFoundError" | "OverconstrainedError" => {
                "No camera or microphone was found.".to_string()
            }
            "NotReadableError" => {
                "Your camera or microphone is already in use by another app.".to_string()
            }
            _ => "Couldn't start your camera and microphone.".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live_room_devices::DeviceKind;

    fn dev(id: &str, label: &str, kind: DeviceKind) -> MediaDevice {
        MediaDevice {
            device_id: id.into(),
            label: label.into(),
            kind,
        }
    }

    #[test]
    fn device_options_maps_id_and_label() {
        let devices = vec![
            dev("c1", "BRIO", DeviceKind::VideoInput),
            dev("c2", "", DeviceKind::VideoInput),
        ];
        let opts = device_options(&devices);
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].value, "c1");
        assert_eq!(opts[0].label, "BRIO");
        // Empty label falls back to a numbered placeholder.
        assert_eq!(opts[1].value, "c2");
        assert_eq!(opts[1].label, "Camera 2");
    }

    #[test]
    fn prejoin_renders_chrome_on_host() {
        fn app() -> Element {
            rsx! {
                LiveRoomPrejoin {
                    on_join: |_choice: PrejoinChoice| {},
                    join_label: "Join".to_string(),
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("live-room-prejoin"),
            "root class missing: {html}"
        );
        assert!(
            html.contains("prejoin-preview-video"),
            "preview video missing: {html}"
        );
        assert!(
            html.contains("prejoin-mic-meter"),
            "mic meter missing: {html}"
        );
        assert!(html.contains("Join"), "CTA label missing: {html}");
    }

    #[test]
    fn next_camera_id_cycles_and_wraps() {
        let cams = vec![
            dev("c1", "Front", DeviceKind::VideoInput),
            dev("c2", "Back", DeviceKind::VideoInput),
        ];
        // From c1 → c2, from c2 → wraps to c1.
        assert_eq!(next_camera_id(&cams, "c1"), "c2");
        assert_eq!(next_camera_id(&cams, "c2"), "c1");
        // Unknown current → first camera.
        assert_eq!(next_camera_id(&cams, "unknown"), "c2");
        // Empty list → empty id.
        assert_eq!(next_camera_id(&[], "c1"), "");
    }

    #[test]
    fn prejoin_choice_default_is_empty() {
        let c = PrejoinChoice::default();
        assert!(c.camera_id.is_empty());
        assert!(c.mic_id.is_empty());
        assert!(c.speaker_id.is_empty());
    }
}
