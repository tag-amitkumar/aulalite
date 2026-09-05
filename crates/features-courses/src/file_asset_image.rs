// crates/features-courses/src/file_asset_image.rs
//! Renders an <img> for a file_asset. Fetches a fresh presigned GET URL on
//! every mount so we never store or cache short-lived URLs.

use dioxus::prelude::*;
use serde::Deserialize;

use crate::api::{self, fetch_json};

#[derive(Deserialize)]
struct UrlResponse {
    url: String,
}

#[derive(Props, Clone, PartialEq)]
pub struct FileAssetImageProps {
    pub asset_id: String,
    #[props(default = "asset".to_string())]
    pub alt: String,
    #[props(default)]
    pub class: Option<String>,
}

#[component]
pub fn FileAssetImage(props: FileAssetImageProps) -> Element {
    let cx = api::use_api();
    let asset_id = props.asset_id.clone();

    let url_resource = use_resource(move || {
        let cx = cx.clone();
        let asset_id = asset_id.clone();
        async move {
            let path = format!("/v1/file-assets/{asset_id}/url");
            fetch_json::<UrlResponse>(&cx, "GET", &path, None::<&()>)
                .await
                .map(|r| r.url)
        }
    });

    let class_attr = props.class.clone().unwrap_or_default();
    let alt = props.alt.clone();

    match &*url_resource.read_unchecked() {
        Some(Ok(url)) => rsx! {
            img {
                class: "{class_attr}",
                src: "{url}",
                alt: "{alt}",
                loading: "lazy",
                decoding: "async",
            }
        },
        // Branded "broken frame" state instead of bare text — a small gold
        // image glyph in an on-brand frame with a short label.
        Some(Err(_)) => rsx! {
            div { class: "ds-img-frame ds-img-error {class_attr}", role: "img", "aria-label": "Image unavailable",
                div {
                    class: "ds-img-frame-glyph",
                    "aria-hidden": "true",
                    dangerous_inner_html: BROKEN_IMAGE_SVG,
                }
                span { class: "ds-img-frame-label", "Image unavailable" }
            }
        },
        // Skeleton shimmer in the same branded frame while the presigned URL is
        // being fetched (replaces the lone "…").
        None => rsx! {
            div { class: "ds-img-frame ds-img-loading {class_attr}", "aria-busy": "true", "aria-label": "Loading image",
                design_system::SkeletonCard { height: "100%".to_string() }
            }
        },
    }
}

/// Small gold image glyph for the unavailable-asset frame. Self-contained inline
/// SVG (no external text); the outline uses `currentColor` so it follows the
/// surrounding text color, with the gold (#b08842) brand accent inline.
const BROKEN_IMAGE_SVG: &str = "<svg xmlns='http://www.w3.org/2000/svg' \
viewBox='0 0 48 48' fill='none' role='img' aria-hidden='true' width='40' \
height='40' stroke='currentColor' stroke-width='2.5' stroke-linecap='round' \
stroke-linejoin='round'>\
<rect x='8' y='10' width='32' height='28' rx='4' stroke-opacity='0.45'/>\
<circle cx='18' cy='20' r='3' fill='#b08842' stroke='#b08842'/>\
<path d='M11 34 L21 24 L29 32' stroke-opacity='0.45'/>\
<path d='M27 30 L33 24 L37 28' stroke='#b08842'/></svg>";
