// crates/features-courses/src/live_room_audio_publisher.rs
//! Audio-only WHIP publisher used when a student is promoted via hand-raise.

#[cfg(target_arch = "wasm32")]
pub async fn publish_audio(
    publish_url: &str,
    password: &str,
) -> Result<crate::live_room_whip::WhipPublisher, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let win = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let nav = win.navigator();
    let media = nav
        .media_devices()
        .map_err(|e| format!("media_devices: {e:?}"))?;
    let constraints = web_sys::MediaStreamConstraints::new();
    constraints.set_audio(&wasm_bindgen::JsValue::TRUE);
    constraints.set_video(&wasm_bindgen::JsValue::FALSE);
    let stream_promise = media
        .get_user_media_with_constraints(&constraints)
        .map_err(|e| format!("getUserMedia: {e:?}"))?;
    let stream_value = JsFuture::from(stream_promise)
        .await
        .map_err(|e| format!("getUserMedia await: {e:?}"))?;
    let stream: web_sys::MediaStream = stream_value
        .dyn_into()
        .map_err(|_| "stream cast".to_string())?;
    crate::live_room_whip::publish(publish_url, password, &stream).await
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn publish_audio(
    url: &str,
    password: &str,
) -> Result<crate::live_room_whip::WhipPublisher, String> {
    crate::live_room_native::publish(crate::live_room_native::PublishRequest {
        key: "student-audio".into(),
        url: url.into(),
        password: password.into(),
        element_id: None,
        camera_id: String::new(),
        mic_id: String::new(),
        facing_mode: None,
        screen: false,
        audio_only: true,
        ice_servers: Vec::new(),
    })
    .await?;
    Ok(crate::live_room_whip::WhipPublisher::native(
        "student-audio",
        None,
    ))
}
