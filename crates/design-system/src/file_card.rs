// crates/design-system/src/file_card.rs
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct FileCardProps {
    pub filename: String,
    pub size_bytes: i64,
    pub content_type: String,
    pub on_open: EventHandler<()>,
    #[props(default)]
    pub on_delete: Option<EventHandler<()>>,
}

fn format_size(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

fn icon_for(content_type: &str) -> &'static str {
    match content_type {
        "application/pdf" => "📄",
        ct if ct.starts_with("image/") => "🖼",
        ct if ct.starts_with("video/") => "🎬",
        ct if ct.starts_with("audio/") => "🎧",
        "application/zip" => "🗜",
        _ => "📎",
    }
}

#[component]
pub fn FileCard(props: FileCardProps) -> Element {
    let on_open = props.on_open;
    let on_delete = props.on_delete;
    rsx! {
        div { class: "ds-file-card",
            span { class: "ds-file-icon", "{icon_for(&props.content_type)}" }
            div { class: "ds-file-meta",
                a {
                    class: "ds-file-name",
                    onclick: move |_| on_open.call(()),
                    "{props.filename}"
                }
                span { class: "ds-file-size", "{format_size(props.size_bytes)}" }
            }
            if on_delete.is_some() {
                button {
                    class: "ds-file-delete",
                    "aria-label": "Delete",
                    onclick: move |_| {
                        if let Some(ref h) = on_delete {
                            h.call(());
                        }
                    },
                    "×"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::format_size;

    #[test]
    fn format_size_picks_unit() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1500), "1.5 KB");
        assert_eq!(format_size(2_500_000), "2.4 MB");
        assert_eq!(format_size(3_000_000_000), "2.8 GB");
    }
}
