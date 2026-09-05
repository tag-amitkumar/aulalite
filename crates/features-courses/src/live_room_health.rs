//! Frontend live-room health DTOs, merge model, and SSR-renderable health UI.

use crate::live_room_socket::ConnStatus;
use crate::live_room_stats::{NetQuality, NetworkQualityBadge};
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Ok,
    Warning,
    Error,
    Unknown,
    NotApplicable,
}

impl HealthStatus {
    pub fn label(self) -> &'static str {
        match self {
            HealthStatus::Ok => "OK",
            HealthStatus::Warning => "Warning",
            HealthStatus::Error => "Error",
            HealthStatus::Unknown => "Unknown",
            HealthStatus::NotApplicable => "N/A",
        }
    }

    pub fn class(self) -> &'static str {
        match self {
            HealthStatus::Ok => "ok",
            HealthStatus::Warning => "warning",
            HealthStatus::Error => "error",
            HealthStatus::Unknown => "unknown",
            HealthStatus::NotApplicable => "not-applicable",
        }
    }

    fn rank(self) -> u8 {
        match self {
            HealthStatus::NotApplicable => 0,
            HealthStatus::Ok => 1,
            HealthStatus::Unknown => 2,
            HealthStatus::Warning => 3,
            HealthStatus::Error => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthCheckDto {
    pub status: HealthStatus,
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLifecycleHealthDto {
    pub status: HealthStatus,
    pub lifecycle: String,
    pub scheduled_starts_at: String,
    pub actual_started_at: Option<String>,
    pub actual_ended_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingHealthDto {
    pub enabled: bool,
    pub status: HealthStatus,
    pub processing_status: Option<String>,
    pub processing_error: Option<String>,
    pub retry_eligible: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveSessionHealthDto {
    pub session_id: String,
    pub checked_at: String,
    pub session: SessionLifecycleHealthDto,
    pub media_server: HealthCheckDto,
    pub main_stream: HealthCheckDto,
    pub screen_stream: HealthCheckDto,
    pub recording: RecordingHealthDto,
}

pub async fn fetch_session_health(
    cx: &crate::api::ApiContext,
    session_id: &str,
) -> Result<LiveSessionHealthDto, crate::api::ApiError> {
    crate::api::fetch_json(
        cx,
        "GET",
        &format!("/v1/sessions/{session_id}/health"),
        None::<&()>,
    )
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketHealthStatus {
    Connected,
    Reconnecting,
    Disconnected,
}

impl From<ConnStatus> for SocketHealthStatus {
    fn from(status: ConnStatus) -> Self {
        match status {
            ConnStatus::Connected => SocketHealthStatus::Connected,
            ConnStatus::Reconnecting => SocketHealthStatus::Reconnecting,
            ConnStatus::Disconnected => SocketHealthStatus::Disconnected,
        }
    }
}

impl SocketHealthStatus {
    fn health_status(self) -> HealthStatus {
        match self {
            SocketHealthStatus::Connected => HealthStatus::Ok,
            SocketHealthStatus::Reconnecting => HealthStatus::Warning,
            SocketHealthStatus::Disconnected => HealthStatus::Error,
        }
    }

    fn detail(self) -> &'static str {
        match self {
            SocketHealthStatus::Connected => "Room connection is live",
            SocketHealthStatus::Reconnecting => "Room connection is reconnecting",
            SocketHealthStatus::Disconnected => "Room connection is disconnected",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrowserRoomHealth {
    pub devices_captured: bool,
    pub publish_active: bool,
    pub screen_publish_active: bool,
    pub socket_status: SocketHealthStatus,
    pub quality: NetQuality,
    pub video_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthItem {
    pub label: String,
    pub status: HealthStatus,
    pub detail: String,
}

impl HealthItem {
    pub fn new(label: impl Into<String>, status: HealthStatus, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            status,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiveRoomHealthModel {
    pub overall_status: HealthStatus,
    pub devices: HealthItem,
    pub media_server: HealthItem,
    pub stream: HealthItem,
    pub screen: HealthItem,
    pub room: HealthItem,
    pub recording: HealthItem,
    pub quality: NetQuality,
    pub checked_at: Option<String>,
    pub retry_recording: bool,
    pub screen_publish_active: bool,
}

pub fn merge_live_room_health(
    server: Option<&LiveSessionHealthDto>,
    browser: BrowserRoomHealth,
) -> LiveRoomHealthModel {
    let devices = if browser.devices_captured {
        HealthItem::new(
            "Devices",
            HealthStatus::Ok,
            "Camera and microphone are captured",
        )
    } else {
        HealthItem::new(
            "Devices",
            HealthStatus::Warning,
            "Camera or microphone capture needs attention",
        )
    };

    let media_server = server
        .map(|health| item_from_check("Media server", &health.media_server))
        .unwrap_or_else(|| {
            HealthItem::new(
                "Media server",
                HealthStatus::Unknown,
                "Media server health has not loaded yet",
            )
        });

    let stream = merge_stream_item(server, &browser);
    let screen = merge_screen_item(server, &browser);
    let room = HealthItem::new(
        "Room",
        browser.socket_status.health_status(),
        browser.socket_status.detail(),
    );
    let recording = merge_recording_item(server);

    let overall_status = worst_status([
        devices.status,
        media_server.status,
        stream.status,
        screen.status,
        room.status,
        recording.status,
    ]);

    LiveRoomHealthModel {
        overall_status,
        devices,
        media_server,
        stream,
        screen,
        room,
        recording,
        quality: browser.quality,
        checked_at: server.map(|health| health.checked_at.clone()),
        retry_recording: server
            .map(|health| health.recording.retry_eligible)
            .unwrap_or(false),
        screen_publish_active: browser.screen_publish_active,
    }
}

fn item_from_check(label: &str, check: &HealthCheckDto) -> HealthItem {
    HealthItem::new(label, check.status, check.detail.clone())
}

fn merge_stream_item(
    server: Option<&LiveSessionHealthDto>,
    browser: &BrowserRoomHealth,
) -> HealthItem {
    let server_check = server.map(|health| &health.main_stream);
    let server_status = server_check
        .map(|check| check.status)
        .unwrap_or(HealthStatus::Unknown);
    let local_status = if browser.video_error.is_some() {
        HealthStatus::Error
    } else if browser.publish_active {
        HealthStatus::Ok
    } else {
        HealthStatus::Warning
    };
    let status = worst_status([server_status, local_status]);

    let detail = if status == HealthStatus::Error {
        browser
            .video_error
            .clone()
            .or_else(|| {
                server_check
                    .filter(|check| check.status == HealthStatus::Error)
                    .map(|check| check.detail.clone())
            })
            .unwrap_or_else(|| "Teacher stream is not publishing".to_string())
    } else if status == server_status && server_status != HealthStatus::Unknown {
        server_check
            .map(|check| check.detail.clone())
            .unwrap_or_else(|| "Stream health is not checked yet".to_string())
    } else if browser.publish_active {
        "Teacher stream is publishing from this browser".to_string()
    } else {
        "Teacher stream is not publishing from this browser".to_string()
    };

    HealthItem::new("Stream", status, detail)
}

fn merge_screen_item(
    server: Option<&LiveSessionHealthDto>,
    browser: &BrowserRoomHealth,
) -> HealthItem {
    let Some(health) = server else {
        return if browser.screen_publish_active {
            HealthItem::new(
                "Screen",
                HealthStatus::Unknown,
                "Screen share health has not loaded yet",
            )
        } else {
            HealthItem::new(
                "Screen",
                HealthStatus::NotApplicable,
                "Screen sharing is not active",
            )
        };
    };

    if browser.screen_publish_active && health.screen_stream.status == HealthStatus::NotApplicable {
        return HealthItem::new(
            "Screen",
            HealthStatus::Warning,
            "Screen share is active but is not reaching students yet",
        );
    }

    item_from_check("Screen", &health.screen_stream)
}

fn merge_recording_item(server: Option<&LiveSessionHealthDto>) -> HealthItem {
    let Some(health) = server else {
        return HealthItem::new(
            "Recording",
            HealthStatus::Unknown,
            "Recording health has not loaded yet",
        );
    };

    let detail = health
        .recording
        .processing_error
        .clone()
        .filter(|msg| !msg.trim().is_empty())
        .unwrap_or_else(|| health.recording.detail.clone());

    HealthItem::new("Recording", health.recording.status, detail)
}

fn worst_status<const N: usize>(statuses: [HealthStatus; N]) -> HealthStatus {
    statuses
        .into_iter()
        .max_by_key(|status| status.rank())
        .unwrap_or(HealthStatus::Unknown)
}

#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomHealthStripProps {
    pub model: LiveRoomHealthModel,
}

#[component]
pub fn LiveRoomHealthStrip(props: LiveRoomHealthStripProps) -> Element {
    let model = props.model;
    let overall_class = format!(
        "live-room-health-strip health-status--{}",
        model.overall_status.class()
    );

    rsx! {
        section {
            class: "{overall_class}",
            role: "status",
            "aria-label": "Live room health",
            div { class: "live-room-health-strip__items",
                HealthStripItem { item: model.devices.clone() }
                HealthStripItem { item: model.stream.clone() }
                HealthStripItem { item: model.screen.clone() }
                HealthStripItem { item: model.room.clone() }
                HealthStripItem { item: model.recording.clone() }
            }
            div { class: "live-room-health-strip__quality",
                NetworkQualityBadge {
                    quality: model.quality,
                    show_label: false,
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct HealthStripItemProps {
    item: HealthItem,
}

#[component]
fn HealthStripItem(props: HealthStripItemProps) -> Element {
    let item = props.item;
    let class = format!(
        "live-room-health-strip__item health-status--{}",
        item.status.class()
    );

    rsx! {
        span { class: "{class}", title: "{item.detail}",
            span { class: "live-room-health-strip__label", "{item.label}" }
            span { class: "live-room-health-strip__status", "{item.status.label()}" }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomDiagnosticsSheetProps {
    pub model: LiveRoomHealthModel,
    pub on_refresh: EventHandler<()>,
    pub on_recheck_devices: EventHandler<()>,
    pub on_retry_publish: EventHandler<()>,
    pub on_restart_screen_share: EventHandler<()>,
    pub on_retry_recording: EventHandler<()>,
}

#[component]
pub fn LiveRoomDiagnosticsSheet(props: LiveRoomDiagnosticsSheetProps) -> Element {
    let model = props.model;
    let checked_at = model
        .checked_at
        .clone()
        .unwrap_or_else(|| "not checked yet".to_string());
    let refresh = props.on_refresh;
    let recheck_devices = props.on_recheck_devices;
    let retry_publish = props.on_retry_publish;
    let restart_screen = props.on_restart_screen_share;
    let retry_recording = props.on_retry_recording;

    rsx! {
        section { class: "live-room-diagnostics-sheet", "aria-label": "Live room diagnostics",
            header { class: "live-room-diagnostics-sheet__header",
                div {
                    h3 { "Live room diagnostics" }
                    p { class: "live-room-diagnostics-sheet__freshness", "Last checked: {checked_at}" }
                }
                button {
                    r#type: "button",
                    class: "live-room-diagnostics-sheet__refresh",
                    onclick: move |_| refresh.call(()),
                    "Refresh room health"
                }
            }
            div { class: "live-room-diagnostics-sheet__rows",
                HealthCheckRow {
                    item: model.devices.clone(),
                    checked_at: Some(checked_at.clone()),
                    action_label: if model.devices.status != HealthStatus::Ok {
                        Some("Recheck devices".to_string())
                    } else {
                        None
                    },
                    on_action: recheck_devices,
                }
                HealthCheckRow {
                    item: model.stream.clone(),
                    checked_at: Some(checked_at.clone()),
                    action_label: if matches!(model.stream.status, HealthStatus::Warning | HealthStatus::Error) {
                        Some("Retry publish".to_string())
                    } else {
                        None
                    },
                    on_action: retry_publish,
                }
                HealthCheckRow {
                    item: model.media_server.clone(),
                    checked_at: Some(checked_at.clone()),
                }
                HealthCheckRow {
                    item: model.screen.clone(),
                    checked_at: Some(checked_at.clone()),
                    action_label: if model.screen_publish_active
                        && matches!(model.screen.status, HealthStatus::Warning | HealthStatus::Error)
                    {
                        Some("Restart screen share".to_string())
                    } else {
                        None
                    },
                    on_action: restart_screen,
                }
                HealthCheckRow {
                    item: model.room.clone(),
                    checked_at: Some(checked_at.clone()),
                }
                HealthCheckRow {
                    item: model.recording.clone(),
                    checked_at: Some(checked_at.clone()),
                    action_label: if model.retry_recording {
                        Some("Retry recording".to_string())
                    } else {
                        None
                    },
                    on_action: retry_recording,
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct HealthCheckRowProps {
    pub item: HealthItem,
    #[props(default)]
    pub checked_at: Option<String>,
    #[props(default)]
    pub action_label: Option<String>,
    #[props(default)]
    pub on_action: EventHandler<()>,
}

#[component]
pub fn HealthCheckRow(props: HealthCheckRowProps) -> Element {
    let item = props.item;
    let checked_at = props.checked_at.clone();
    let action_label = props.action_label.clone();
    let on_action = props.on_action;
    let row_class = format!("health-check-row health-status--{}", item.status.class());

    rsx! {
        article { class: "{row_class}",
            div { class: "health-check-row__main",
                span { class: "health-check-row__status", "{item.status.label()}" }
                div {
                    h4 { class: "health-check-row__label", "{item.label}" }
                    p { class: "health-check-row__detail", "{item.detail}" }
                    if let Some(checked_at) = checked_at {
                        p { class: "health-check-row__checked", "Last checked: {checked_at}" }
                    }
                }
            }
            if let Some(label) = action_label {
                button {
                    r#type: "button",
                    class: "health-check-row__action",
                    onclick: move |_| on_action.call(()),
                    "{label}"
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamNoticeState {
    Waiting,
    Connecting,
    Retrying,
    Failed,
}

impl StreamNoticeState {
    pub fn class(self) -> &'static str {
        match self {
            StreamNoticeState::Waiting => "waiting",
            StreamNoticeState::Connecting => "connecting",
            StreamNoticeState::Retrying => "retrying",
            StreamNoticeState::Failed => "failed",
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct StreamStateNoticeProps {
    pub state: StreamNoticeState,
    pub title: String,
    pub detail: String,
    #[props(default)]
    pub on_retry: EventHandler<()>,
}

#[component]
pub fn StreamStateNotice(props: StreamStateNoticeProps) -> Element {
    let state_class = props.state.class();
    let class = format!("stream-state-notice stream-state-notice--{state_class}");
    let show_retry = matches!(
        props.state,
        StreamNoticeState::Retrying | StreamNoticeState::Failed
    );
    let on_retry = props.on_retry;

    rsx! {
        section { class: "{class}", role: "status",
            h3 { class: "stream-state-notice__title", "{props.title}" }
            p { class: "stream-state-notice__detail", "{props.detail}" }
            if show_retry {
                button {
                    r#type: "button",
                    class: "stream-state-notice__retry",
                    onclick: move |_| on_retry.call(()),
                    "Retry"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live_room_socket::ConnStatus;
    use crate::live_room_stats::NetQuality;

    fn check(status: HealthStatus, label: &str, detail: &str) -> HealthCheckDto {
        HealthCheckDto {
            status,
            label: label.into(),
            detail: detail.into(),
        }
    }

    fn server_health() -> LiveSessionHealthDto {
        LiveSessionHealthDto {
            session_id: "session-1".into(),
            checked_at: "2026-07-01T12:00:00Z".into(),
            session: SessionLifecycleHealthDto {
                status: HealthStatus::Ok,
                lifecycle: "live".into(),
                scheduled_starts_at: "2026-07-01T11:55:00Z".into(),
                actual_started_at: Some("2026-07-01T12:00:00Z".into()),
                actual_ended_at: None,
            },
            media_server: check(
                HealthStatus::Ok,
                "Media server",
                "Media server API is reachable",
            ),
            main_stream: check(HealthStatus::Ok, "Main stream", "Main stream is active"),
            screen_stream: check(
                HealthStatus::NotApplicable,
                "Screen stream",
                "Screen sharing is not active",
            ),
            recording: RecordingHealthDto {
                enabled: true,
                status: HealthStatus::Ok,
                processing_status: Some("available".into()),
                processing_error: None,
                retry_eligible: false,
                detail: "Recording is available".into(),
            },
        }
    }

    fn browser_health() -> BrowserRoomHealth {
        BrowserRoomHealth {
            devices_captured: true,
            publish_active: true,
            screen_publish_active: false,
            socket_status: SocketHealthStatus::Connected,
            quality: NetQuality::Good,
            video_error: None,
        }
    }

    #[test]
    fn merge_promotes_main_stream_error_over_local_ok() {
        let mut server = server_health();
        server.main_stream = check(
            HealthStatus::Error,
            "Main stream",
            "Main stream is inactive while class is live",
        );
        let model = merge_live_room_health(Some(&server), browser_health());

        assert_eq!(model.stream.status, HealthStatus::Error);
        assert_eq!(model.overall_status, HealthStatus::Error);
        assert!(model.stream.detail.contains("inactive"));
    }

    #[test]
    fn merge_promotes_media_server_error_to_overall() {
        let mut server = server_health();
        server.media_server = check(
            HealthStatus::Error,
            "Media server",
            "Media server API check failed",
        );

        let model = merge_live_room_health(Some(&server), browser_health());

        assert_eq!(model.media_server.status, HealthStatus::Error);
        assert_eq!(model.overall_status, HealthStatus::Error);
    }

    #[test]
    fn merge_marks_inactive_screen_warning_only_when_browser_is_sharing() {
        let server = server_health();
        let mut browser = browser_health();
        browser.screen_publish_active = true;

        let model = merge_live_room_health(Some(&server), browser);
        assert_eq!(model.screen.status, HealthStatus::Warning);
        assert!(model.screen.detail.contains("not reaching"));

        let model = merge_live_room_health(Some(&server), browser_health());
        assert_eq!(model.screen.status, HealthStatus::NotApplicable);
    }

    #[test]
    fn health_strip_renders_statuses() {
        fn app() -> Element {
            let mut server = server_health();
            server.recording.status = HealthStatus::Warning;
            server.recording.processing_status = Some("remuxing".into());
            server.recording.detail = "Recording is being processed".into();
            let model = merge_live_room_health(Some(&server), browser_health());
            rsx! { LiveRoomHealthStrip { model } }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);

        assert!(
            html.contains("live-room-health-strip"),
            "strip missing: {html}"
        );
        assert!(html.contains("Devices"), "devices status missing: {html}");
        assert!(html.contains("Stream"), "stream status missing: {html}");
        assert!(
            html.contains("Recording"),
            "recording status missing: {html}"
        );
        assert!(
            html.contains("health-status--warning"),
            "warning class missing: {html}"
        );
        assert!(
            html.contains("net-quality--good"),
            "quality badge missing: {html}"
        );
    }

    #[test]
    fn diagnostics_sheet_renders_retry_recording_action() {
        fn app() -> Element {
            let mut server = server_health();
            server.recording.status = HealthStatus::Error;
            server.recording.processing_status = Some("failed".into());
            server.recording.processing_error = Some("Transcode failed".into());
            server.recording.retry_eligible = true;
            server.recording.detail = "Recording failed".into();
            let model = merge_live_room_health(Some(&server), browser_health());
            rsx! {
                LiveRoomDiagnosticsSheet {
                    model,
                    on_refresh: move |_| {},
                    on_recheck_devices: move |_| {},
                    on_retry_publish: move |_| {},
                    on_restart_screen_share: move |_| {},
                    on_retry_recording: move |_| {},
                }
            }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);

        assert!(
            html.contains("live-room-diagnostics-sheet"),
            "sheet missing: {html}"
        );
        assert!(
            html.contains("Transcode failed"),
            "recording error missing: {html}"
        );
        assert!(
            html.contains("Retry recording"),
            "retry action missing: {html}"
        );
        assert!(html.contains("Last checked"), "freshness missing: {html}");
    }

    #[test]
    fn stream_notice_renders_retry() {
        fn app() -> Element {
            rsx! {
                StreamStateNotice {
                    state: StreamNoticeState::Retrying,
                    title: "Reconnecting video".to_string(),
                    detail: "Trying the stream again.".to_string(),
                    on_retry: move |_| {},
                }
            }
        }

        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);

        assert!(
            html.contains("stream-state-notice--retrying"),
            "stable class missing: {html}"
        );
        assert!(html.contains("Reconnecting video"), "title missing: {html}");
        assert!(html.contains("Retry"), "retry button missing: {html}");
    }

    #[test]
    fn socket_status_maps_from_connection_status() {
        assert_eq!(
            SocketHealthStatus::from(ConnStatus::Connected),
            SocketHealthStatus::Connected
        );
        assert_eq!(
            SocketHealthStatus::from(ConnStatus::Reconnecting),
            SocketHealthStatus::Reconnecting
        );
        assert_eq!(
            SocketHealthStatus::from(ConnStatus::Disconnected),
            SocketHealthStatus::Disconnected
        );
    }
}
