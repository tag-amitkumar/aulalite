// crates/features-courses/src/file_picker.rs
//! Generic upload widget. The `validation` submodule is pure and unit-testable;
//! the rest of the file (XHR-based upload flow) lands in Task 20.

pub mod validation {
    /// Mirrors the backend matrix; provides instant client-side feedback.
    pub fn client_side_check(
        purpose: &str,
        content_type: &str,
        size_bytes: i64,
        allowed_types: &[&str],
        max_size_bytes: i64,
    ) -> Result<(), String> {
        if size_bytes < 0 {
            return Err("size must be non-negative".to_string());
        }
        if !allowed_types
            .iter()
            .any(|t| t.eq_ignore_ascii_case(content_type))
        {
            return Err(format!(
                "content type {content_type} not allowed for purpose {purpose}"
            ));
        }
        if size_bytes > max_size_bytes {
            return Err(format!(
                "size {size_bytes} exceeds {max_size_bytes} cap for purpose {purpose}"
            ));
        }
        Ok(())
    }

    pub const COVER_TYPES: &[&str] = &["image/jpeg", "image/png", "image/webp"];
    pub const VIDEO_TYPES: &[&str] = &["video/mp4", "video/webm"];
    pub const ATTACHMENT_TYPES: &[&str] = &[
        "application/pdf",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "application/zip",
        "text/plain",
        "text/csv",
        "image/jpeg",
        "image/png",
        "image/webp",
        "audio/mpeg",
        "video/mp4",
    ];
    pub const COVER_MAX: i64 = 5 * 1024 * 1024;
    pub const VIDEO_MAX: i64 = 500 * 1024 * 1024;
    pub const ATTACHMENT_MAX: i64 = 100 * 1024 * 1024;
}

use design_system::{ProgressBar, Spinner};
use dioxus::prelude::*;

use crate::api;
#[cfg(target_arch = "wasm32")]
use crate::api::ApiError;

#[derive(Props, Clone, PartialEq)]
pub struct FilePickerProps {
    pub purpose: String,
    pub linked_entity_type: String,
    pub linked_entity_id: String,
    pub allowed_types: Vec<String>,
    pub max_size_bytes: i64,
    pub on_uploaded: EventHandler<String>, // asset_id
    #[props(default = "Choose file".to_string())]
    pub button_label: String,
}

#[derive(Clone, PartialEq)]
enum PickerState {
    Idle,
    Validating,
    Beginning,
    Uploading(f32),
    Completing,
    Error(String),
}

#[component]
pub fn FilePicker(props: FilePickerProps) -> Element {
    let mut state = use_signal(|| PickerState::Idle);
    let cx = api::use_api();

    let on_change = {
        let purpose = props.purpose.clone();
        let entity_type = props.linked_entity_type.clone();
        let entity_id = props.linked_entity_id.clone();
        let allowed = props.allowed_types.clone();
        let max_size = props.max_size_bytes;
        let on_uploaded = props.on_uploaded;
        let cx = cx.clone();

        move |evt: FormEvent| {
            #[cfg(target_arch = "wasm32")]
            {
                use wasm_bindgen::JsCast;
                // Downcast the inner event data to web_sys::Event, then get target
                let web_event = evt.data().downcast::<web_sys::Event>().cloned();
                let input_opt = web_event
                    .and_then(|e| e.target())
                    .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
                let input = match input_opt {
                    Some(i) => i,
                    None => return,
                };
                let files = match input.files() {
                    Some(f) => f,
                    None => return,
                };
                if files.length() == 0 {
                    return;
                }
                let file = match files.get(0) {
                    Some(f) => f,
                    None => return,
                };
                let filename = file.name();
                let content_type = file.type_();
                let size_bytes = file.size() as i64;

                let allowed_refs: Vec<&str> = allowed.iter().map(|s| s.as_str()).collect();
                state.set(PickerState::Validating);
                if let Err(e) = validation::client_side_check(
                    &purpose,
                    &content_type,
                    size_bytes,
                    &allowed_refs,
                    max_size,
                ) {
                    state.set(PickerState::Error(e));
                    return;
                }

                let cx = cx.clone();
                let purpose = purpose.clone();
                let entity_type = entity_type.clone();
                let entity_id = entity_id.clone();
                let on_uploaded = on_uploaded;
                let mut state_for_async = state;

                wasm_bindgen_futures::spawn_local(async move {
                    state_for_async.set(PickerState::Beginning);
                    let begin_body = api::UploadBeginBody {
                        filename: &filename,
                        content_type: &content_type,
                        size_bytes,
                        linked_entity_type: &entity_type,
                        linked_entity_id: entity_id.clone(),
                        purpose: &purpose,
                    };
                    let begin_resp = match api::upload_begin(&cx, &begin_body).await {
                        Ok(r) => r,
                        Err(ApiError::Status(_, body)) => {
                            state_for_async.set(PickerState::Error(body));
                            return;
                        }
                        Err(e) => {
                            state_for_async.set(PickerState::Error(e.to_string()));
                            return;
                        }
                    };

                    state_for_async.set(PickerState::Uploading(0.0));
                    if let Err(message) = validate_presigned_put_url(&begin_resp.presigned_put_url)
                    {
                        state_for_async.set(PickerState::Error(message));
                        return;
                    }
                    if let Err(msg) = upload_via_xhr(
                        &begin_resp.presigned_put_url,
                        &content_type,
                        &file,
                        move |frac| state_for_async.set(PickerState::Uploading(frac)),
                    )
                    .await
                    {
                        state_for_async.set(PickerState::Error(msg));
                        return;
                    }

                    state_for_async.set(PickerState::Completing);
                    if let Err(e) = api::upload_complete(&cx, &begin_resp.asset_id).await {
                        state_for_async.set(PickerState::Error(e.to_string()));
                        return;
                    }

                    on_uploaded.call(begin_resp.asset_id);
                    state_for_async.set(PickerState::Idle);
                });
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                use futures_util::StreamExt;

                let Some(file) = evt.files().into_iter().next() else {
                    return;
                };
                let filename = file.name();
                let content_type = content_type_or_guess(file.content_type().as_deref(), &filename);
                let Ok(size_bytes) = i64::try_from(file.size()) else {
                    state.set(PickerState::Error("The selected file is too large.".into()));
                    return;
                };
                let allowed_refs: Vec<&str> = allowed.iter().map(String::as_str).collect();
                state.set(PickerState::Validating);
                if let Err(error) = validation::client_side_check(
                    &purpose,
                    &content_type,
                    size_bytes,
                    &allowed_refs,
                    max_size,
                ) {
                    state.set(PickerState::Error(error));
                    return;
                }

                let cx = cx.clone();
                let purpose = purpose.clone();
                let entity_type = entity_type.clone();
                let entity_id = entity_id.clone();
                let on_uploaded = on_uploaded;
                let mut state_for_async = state;
                spawn(async move {
                    state_for_async.set(PickerState::Beginning);
                    let begin_body = api::UploadBeginBody {
                        filename: &filename,
                        content_type: &content_type,
                        size_bytes,
                        linked_entity_type: &entity_type,
                        linked_entity_id: entity_id,
                        purpose: &purpose,
                    };
                    let begin_resp = match api::upload_begin(&cx, &begin_body).await {
                        Ok(response) => response,
                        Err(error) => {
                            state_for_async.set(PickerState::Error(error.to_string()));
                            return;
                        }
                    };

                    state_for_async.set(PickerState::Uploading(0.0));
                    if let Err(message) = validate_presigned_put_url(&begin_resp.presigned_put_url)
                    {
                        state_for_async.set(PickerState::Error(message));
                        return;
                    }
                    let total = file.size().max(1);
                    let uploaded = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                    let uploaded_for_stream = uploaded.clone();
                    let body_stream = file.byte_stream().map(move |chunk| {
                        chunk
                            .inspect(|bytes| {
                                uploaded_for_stream.fetch_add(
                                    bytes.len() as u64,
                                    std::sync::atomic::Ordering::Relaxed,
                                );
                            })
                            .map_err(|error| std::io::Error::other(error.to_string()))
                    });
                    let client = match reqwest::Client::builder()
                        .connect_timeout(std::time::Duration::from_secs(15))
                        .timeout(std::time::Duration::from_secs(30 * 60))
                        .redirect(reqwest::redirect::Policy::none())
                        .build()
                    {
                        Ok(client) => client,
                        Err(_) => {
                            state_for_async.set(PickerState::Error(
                                "The upload service could not be started.".into(),
                            ));
                            return;
                        }
                    };
                    let upload_request = client
                        .put(&begin_resp.presigned_put_url)
                        .header("content-type", &content_type)
                        .header("content-length", file.size())
                        .body(reqwest::Body::wrap_stream(body_stream))
                        .send();
                    tokio::pin!(upload_request);
                    let upload = loop {
                        tokio::select! {
                            response = &mut upload_request => break response,
                            _ = tokio::time::sleep(std::time::Duration::from_millis(120)) => {
                                let sent = uploaded.load(std::sync::atomic::Ordering::Relaxed);
                                state_for_async.set(PickerState::Uploading(
                                    (sent as f32 / total as f32).clamp(0.0, 1.0),
                                ));
                            }
                        }
                    };
                    match upload {
                        Ok(response) if response.status().is_success() => {}
                        Ok(_) => {
                            state_for_async.set(PickerState::Error(
                                "The storage service rejected the upload.".into(),
                            ));
                            return;
                        }
                        Err(_) => {
                            state_for_async.set(PickerState::Error(
                                "The file could not be uploaded. Check your connection and try again."
                                    .into(),
                            ));
                            return;
                        }
                    }

                    state_for_async.set(PickerState::Completing);
                    if let Err(error) = api::upload_complete(&cx, &begin_resp.asset_id).await {
                        state_for_async.set(PickerState::Error(error.to_string()));
                        return;
                    }
                    on_uploaded.call(begin_resp.asset_id);
                    state_for_async.set(PickerState::Idle);
                });
            }
        }
    };

    let accept = props.allowed_types.join(",");
    rsx! {
        div { class: "ds-file-picker",
            input {
                r#type: "file",
                accept: "{accept}",
                onchange: on_change,
                disabled: !matches!(*state.read(), PickerState::Idle | PickerState::Error(_)),
            }
            match &*state.read() {
                PickerState::Idle => rsx! {},
                PickerState::Validating | PickerState::Beginning => rsx! { Spinner {} },
                PickerState::Uploading(f) => rsx! { ProgressBar { value: *f, label: None } },
                PickerState::Completing => rsx! { Spinner {} },
                PickerState::Error(msg) => rsx! {
                    div { class: "form-error", "{msg}" }
                },
            }
        }
    }
}

fn validate_presigned_put_url(raw: &str) -> Result<(), String> {
    let parsed =
        url::Url::parse(raw).map_err(|_| "The upload destination is invalid.".to_string())?;
    let debug_loopback_http = cfg!(debug_assertions)
        && parsed.scheme() == "http"
        && matches!(
            parsed.host_str(),
            Some("localhost" | "127.0.0.1" | "10.0.2.2")
        );
    if (parsed.scheme() != "https" && !debug_loopback_http)
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err("The upload destination must use HTTPS and cannot contain credentials.".into());
    }
    Ok(())
}

fn content_type_or_guess(reported: Option<&str>, filename: &str) -> String {
    if let Some(reported) = reported.map(str::trim).filter(|value| !value.is_empty()) {
        return reported.to_ascii_lowercase();
    }
    let extension = filename
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    match extension.as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("zip") => "application/zip",
        Some("txt") => "text/plain",
        Some("csv") => "text/csv",
        Some("mp3") => "audio/mpeg",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    }
    .to_string()
}

#[cfg(test)]
mod native_upload_tests {
    use super::validate_presigned_put_url;

    #[test]
    fn presigned_upload_destination_rejects_downgrades_and_credentials() {
        assert!(validate_presigned_put_url(
            "https://storage.elementors.guru/bucket/object?X-Amz-Signature=abc"
        )
        .is_ok());
        assert!(validate_presigned_put_url("ftp://storage.example/object").is_err());
        assert!(validate_presigned_put_url("https://user:secret@storage.example/object").is_err());
    }
}

#[cfg(target_arch = "wasm32")]
async fn upload_via_xhr(
    url: &str,
    content_type: &str,
    file: &web_sys::File,
    mut on_progress: impl FnMut(f32) + 'static,
) -> Result<(), String> {
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;

    let xhr = web_sys::XmlHttpRequest::new().map_err(|e| format!("xhr init: {e:?}"))?;
    xhr.open("PUT", url)
        .map_err(|e| format!("xhr open: {e:?}"))?;
    xhr.set_request_header("Content-Type", content_type)
        .map_err(|e| format!("xhr header: {e:?}"))?;

    let upload = xhr.upload().map_err(|e| format!("xhr upload: {e:?}"))?;
    let progress_cb =
        Closure::<dyn FnMut(web_sys::ProgressEvent)>::new(move |evt: web_sys::ProgressEvent| {
            if evt.length_computable() {
                let frac = (evt.loaded() / evt.total()) as f32;
                on_progress(frac.clamp(0.0, 1.0));
            }
        });
    upload.set_onprogress(Some(progress_cb.as_ref().unchecked_ref()));

    let (tx, rx) = futures_channel::oneshot::channel::<Result<(), String>>();
    let tx_load = std::cell::RefCell::new(Some(tx));

    let load_cb = Closure::<dyn FnMut(web_sys::Event)>::new({
        let tx_load = tx_load;
        let xhr_clone = xhr.clone();
        move |_| {
            let status = xhr_clone.status().unwrap_or(0);
            let result = if (200..300).contains(&status) {
                Ok(())
            } else {
                Err(format!("PUT returned status {status}"))
            };
            if let Some(tx) = tx_load.borrow_mut().take() {
                let _ = tx.send(result);
            }
        }
    });
    xhr.set_onloadend(Some(load_cb.as_ref().unchecked_ref()));

    xhr.send_with_opt_blob(Some(file))
        .map_err(|e| format!("xhr send: {e:?}"))?;

    let result = rx.await.map_err(|_| "xhr cancelled".to_string())?;
    drop(progress_cb);
    drop(load_cb);
    result
}

#[cfg(test)]
mod tests {
    use super::validation::*;

    #[test]
    fn cover_accepts_jpeg() {
        assert!(client_side_check("cover", "image/jpeg", 1000, COVER_TYPES, COVER_MAX).is_ok());
    }

    #[test]
    fn cover_rejects_svg() {
        let err =
            client_side_check("cover", "image/svg+xml", 1000, COVER_TYPES, COVER_MAX).unwrap_err();
        assert!(err.contains("not allowed"));
    }

    #[test]
    fn cover_rejects_oversized() {
        let err = client_side_check("cover", "image/png", COVER_MAX + 1, COVER_TYPES, COVER_MAX)
            .unwrap_err();
        assert!(err.contains("exceeds"));
    }

    #[test]
    fn video_accepts_mp4_under_cap() {
        assert!(
            client_side_check("video", "video/mp4", 100_000_000, VIDEO_TYPES, VIDEO_MAX).is_ok()
        );
    }

    #[test]
    fn attachment_rejects_executable() {
        let err = client_side_check(
            "attachment",
            "application/x-msdownload",
            1000,
            ATTACHMENT_TYPES,
            ATTACHMENT_MAX,
        )
        .unwrap_err();
        assert!(err.contains("not allowed"));
    }
}
