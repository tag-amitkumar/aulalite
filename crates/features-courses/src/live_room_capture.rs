// crates/features-courses/src/live_room_capture.rs
//! Deciding WHAT to ask `getUserMedia` for, and what a failure means.
//!
//! Every capture site used to ask for `{audio: true, video: true}` and hope.
//! That request is all-or-nothing: on a machine with a microphone but no
//! camera the browser rejects the whole call with `NotFoundError`, so a
//! teacher who could have taught with audio got nothing at all, and the error
//! ("Requested device not found") named neither which device was missing nor
//! that the other one was fine.
//!
//! The logic here is deliberately pure -- it takes the enumerated device lists
//! and returns a decision -- so the interesting cases (no camera, no mic,
//! neither, a saved deviceId that no longer exists) are unit-testable without
//! a browser. The wasm call sites do the I/O around it.

use crate::live_room_devices::MediaDevice;

/// What to request from `getUserMedia`, after looking at what actually exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturePlan {
    pub want_video: bool,
    pub want_audio: bool,
    /// A camera deviceId that was confirmed present in this enumeration.
    /// `None` means "let the browser pick its default".
    pub camera_id: Option<String>,
    pub mic_id: Option<String>,
}

impl CapturePlan {
    /// True when the machine can do everything the room wants.
    pub fn is_complete(&self) -> bool {
        self.want_video && self.want_audio
    }

    /// A short, honest description of what is about to be captured. Shown in
    /// the UI so a degraded capture is never silent.
    pub fn summary(&self) -> &'static str {
        match (self.want_video, self.want_audio) {
            (true, true) => "camera and microphone",
            (true, false) => "camera only (no microphone found)",
            (false, true) => "microphone only (no camera found)",
            (false, false) => "nothing",
        }
    }
}

/// The outcome of looking at the device list before touching `getUserMedia`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureDecision {
    /// At least one input exists. Request exactly these.
    Capture(CapturePlan),
    /// No camera AND no microphone. There is nothing to publish, so the caller
    /// must NOT call `getUserMedia` -- doing so only produces a
    /// `NotFoundError` whose message is less useful than what we already know.
    NoInputDevices,
}

/// Decide what to capture from an enumeration plus whatever device ids were
/// previously selected.
///
/// A saved deviceId is only honoured if it is still in the list. Stale ids are
/// the other half of the `NotFoundError` family: a camera that was unplugged,
/// or an id captured on a different machine/profile, pins the request to a
/// device that cannot be opened. Dropping the id falls back to the browser
/// default, which is what the teacher wants anyway.
pub fn plan_capture(
    cameras: &[MediaDevice],
    mics: &[MediaDevice],
    saved_camera_id: &str,
    saved_mic_id: &str,
) -> CaptureDecision {
    if cameras.is_empty() && mics.is_empty() {
        return CaptureDecision::NoInputDevices;
    }
    CaptureDecision::Capture(CapturePlan {
        want_video: !cameras.is_empty(),
        want_audio: !mics.is_empty(),
        camera_id: valid_id(cameras, saved_camera_id),
        mic_id: valid_id(mics, saved_mic_id),
    })
}

/// `Some(id)` only when `id` is non-empty AND present in `devices`.
fn valid_id(devices: &[MediaDevice], id: &str) -> Option<String> {
    if id.is_empty() {
        return None;
    }
    devices
        .iter()
        .any(|d| d.device_id == id)
        .then(|| id.to_string())
}

/// The distinct ways `getUserMedia` fails. These need different handling and
/// different words -- lumping them together is why "Requested device not
/// found" was shown for a permission prompt the user had dismissed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GumFailure {
    /// The user (or policy) refused. Retrying identically will fail again.
    PermissionDenied,
    /// No device of the requested kind exists.
    DeviceNotFound,
    /// The device exists but the OS will not hand it over -- usually another
    /// application holds it.
    DeviceBusy,
    /// Constraints matched nothing. Distinct from `DeviceNotFound` because it
    /// is RECOVERABLE: dropping the deviceId and retrying with the browser
    /// default normally succeeds.
    Overconstrained,
    Unknown,
}

impl GumFailure {
    /// Whether retrying without the deviceId constraints is worth attempting.
    pub fn is_retryable_without_device_id(self) -> bool {
        matches!(self, GumFailure::Overconstrained)
    }
}

/// Map a DOMException `name` onto a [`GumFailure`].
pub fn classify_gum_failure(name: &str) -> GumFailure {
    match name {
        "NotAllowedError" | "SecurityError" | "PermissionDeniedError" => {
            GumFailure::PermissionDenied
        }
        "NotFoundError" | "DevicesNotFoundError" => GumFailure::DeviceNotFound,
        "NotReadableError" | "TrackStartError" => GumFailure::DeviceBusy,
        "OverconstrainedError" | "ConstraintNotSatisfiedError" => GumFailure::Overconstrained,
        _ => GumFailure::Unknown,
    }
}

/// A message that says what went wrong AND what to do about it.
pub fn gum_failure_message(kind: GumFailure) -> String {
    match kind {
        GumFailure::PermissionDenied =>
            "Camera and microphone access was blocked. Allow access in your browser's site settings, then reload this page."
                .to_string(),
        GumFailure::DeviceNotFound =>
            "No camera or microphone is connected to this computer. Connect one and reload -- the class cannot be broadcast without an input device."
                .to_string(),
        GumFailure::DeviceBusy =>
            "Your camera or microphone is already in use by another application. Close it (video call, recorder) and try again."
                .to_string(),
        GumFailure::Overconstrained =>
            "The selected camera or microphone is no longer available. Pick a different device and try again."
                .to_string(),
        GumFailure::Unknown =>
            "Could not start your camera and microphone. Check your browser's device permissions and try again."
                .to_string(),
    }
}

/// The message for the pre-flight case: we enumerated, found nothing, and are
/// deliberately not calling `getUserMedia` at all.
pub fn no_input_devices_message() -> String {
    gum_failure_message(GumFailure::DeviceNotFound)
}

/// Whether a captured stream is fit to hand to the WHIP publisher.
///
/// Counts are of tracks whose `readyState` is `"live"`. A track that exists but
/// has already ended publishes a black or silent path to MediaMTX, which looks
/// exactly like a broken encoder from the student side, so it is rejected here
/// where the message can still be useful.
pub fn publishable_verdict(live_audio: usize, live_video: usize) -> Result<(), String> {
    if live_audio == 0 && live_video == 0 {
        return Err(
            "Capture produced no live audio or video tracks, so there is nothing to broadcast."
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live_room_devices::DeviceKind;

    fn cam(id: &str) -> MediaDevice {
        MediaDevice {
            device_id: id.into(),
            label: "Camera".into(),
            kind: DeviceKind::VideoInput,
        }
    }
    fn mic(id: &str) -> MediaDevice {
        MediaDevice {
            device_id: id.into(),
            label: "Mic".into(),
            kind: DeviceKind::AudioInput,
        }
    }

    #[test]
    fn no_inputs_at_all_is_decided_before_touching_getusermedia() {
        // The exact shape of the reported failure: a desktop with speakers but
        // no capture hardware. Asking the browser adds nothing -- we already
        // know the answer, and we can say it better than "NotFoundError".
        assert_eq!(plan_capture(&[], &[], "", ""), CaptureDecision::NoInputDevices);
        assert!(no_input_devices_message().contains("No camera or microphone"));
    }

    #[test]
    fn a_missing_camera_still_publishes_the_microphone() {
        // The regression that mattered most: one combined {audio,video}
        // request meant a missing camera cost the teacher their audio too.
        let d = plan_capture(&[], &[mic("m1")], "", "");
        let CaptureDecision::Capture(plan) = d else {
            panic!("a machine with a microphone must still capture");
        };
        assert!(plan.want_audio);
        assert!(!plan.want_video);
        assert!(!plan.is_complete());
        assert_eq!(plan.summary(), "microphone only (no camera found)");
    }

    #[test]
    fn a_missing_microphone_still_publishes_the_camera() {
        let d = plan_capture(&[cam("c1")], &[], "", "");
        let CaptureDecision::Capture(plan) = d else {
            panic!("a machine with a camera must still capture");
        };
        assert!(plan.want_video);
        assert!(!plan.want_audio);
        assert_eq!(plan.summary(), "camera only (no microphone found)");
    }

    #[test]
    fn a_stale_device_id_is_dropped_rather_than_requested() {
        // An unplugged camera (or an id from another machine) would otherwise
        // pin the request to a device that cannot be opened.
        let d = plan_capture(&[cam("c1")], &[mic("m1")], "GONE", "ALSO-GONE");
        let CaptureDecision::Capture(plan) = d else { panic!("should capture") };
        assert_eq!(plan.camera_id, None, "stale camera id must not be reused");
        assert_eq!(plan.mic_id, None, "stale mic id must not be reused");
        assert!(plan.want_video && plan.want_audio);
    }

    #[test]
    fn a_live_device_id_is_honoured() {
        let d = plan_capture(&[cam("c1"), cam("c2")], &[mic("m1")], "c2", "m1");
        let CaptureDecision::Capture(plan) = d else { panic!("should capture") };
        assert_eq!(plan.camera_id.as_deref(), Some("c2"));
        assert_eq!(plan.mic_id.as_deref(), Some("m1"));
        assert!(plan.is_complete());
        assert_eq!(plan.summary(), "camera and microphone");
    }

    #[test]
    fn empty_saved_id_means_browser_default_not_a_constraint() {
        let d = plan_capture(&[cam("c1")], &[mic("m1")], "", "");
        let CaptureDecision::Capture(plan) = d else { panic!("should capture") };
        assert_eq!(plan.camera_id, None);
        assert_eq!(plan.mic_id, None);
    }

    #[test]
    fn failures_are_classified_separately() {
        // Permission and absence are different problems with different fixes,
        // and were previously shown with the same words.
        assert_eq!(classify_gum_failure("NotAllowedError"), GumFailure::PermissionDenied);
        assert_eq!(classify_gum_failure("NotFoundError"), GumFailure::DeviceNotFound);
        assert_eq!(classify_gum_failure("NotReadableError"), GumFailure::DeviceBusy);
        assert_eq!(classify_gum_failure("OverconstrainedError"), GumFailure::Overconstrained);
        assert_eq!(classify_gum_failure("WeirdNewError"), GumFailure::Unknown);

        let denied = gum_failure_message(GumFailure::PermissionDenied);
        let missing = gum_failure_message(GumFailure::DeviceNotFound);
        assert_ne!(denied, missing);
        assert!(denied.to_lowercase().contains("blocked"));
        assert!(missing.to_lowercase().contains("connect"));
    }

    #[test]
    fn only_overconstrained_is_worth_retrying_without_the_device_id() {
        // Retrying a denied permission or absent hardware just fails again,
        // slower.
        assert!(GumFailure::Overconstrained.is_retryable_without_device_id());
        for k in [
            GumFailure::PermissionDenied,
            GumFailure::DeviceNotFound,
            GumFailure::DeviceBusy,
            GumFailure::Unknown,
        ] {
            assert!(!k.is_retryable_without_device_id(), "{k:?} must not loop");
        }
    }

    #[test]
    fn a_stream_with_no_live_tracks_is_refused() {
        assert!(publishable_verdict(0, 0).is_err());
        // One live track is enough -- audio-only is a legitimate broadcast.
        assert!(publishable_verdict(1, 0).is_ok());
        assert!(publishable_verdict(0, 1).is_ok());
        assert!(publishable_verdict(1, 1).is_ok());
    }
}
