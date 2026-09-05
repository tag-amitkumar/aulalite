//! Ephemeral whiteboard reducer and SVG component for live rooms.

use dioxus::events::PointerEvent;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

pub const MAX_STROKES: usize = 256;
pub const MAX_POINTS_PER_STROKE: usize = 512;

/// DOM id of the SVG drawing surface. Used by the window-resize listener to
/// re-measure the rendered canvas (see `LiveRoomWhiteboard`). Only one
/// whiteboard renders per room (teacher xor student view), so a static id is
/// unambiguous.
pub const WHITEBOARD_CANVAS_ID: &str = "live-room-whiteboard-canvas";

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WhiteboardTool {
    #[default]
    Pen,
    Eraser,
    Line,
    Rect,
    Ellipse,
    Arrow,
    Text,
}

impl WhiteboardTool {
    /// The geometric kind this UI tool produces. Pen/Eraser draw freehand
    /// polylines; the rest map one-to-one onto a [`WhiteboardKind`].
    fn kind(&self) -> WhiteboardKind {
        match self {
            WhiteboardTool::Pen | WhiteboardTool::Eraser => WhiteboardKind::Freehand,
            WhiteboardTool::Line => WhiteboardKind::Line,
            WhiteboardTool::Rect => WhiteboardKind::Rect,
            WhiteboardTool::Ellipse => WhiteboardKind::Ellipse,
            WhiteboardTool::Arrow => WhiteboardKind::Arrow,
            WhiteboardTool::Text => WhiteboardKind::Text,
        }
    }

    /// The "ink mode" stored on the wire. Only Pen/Eraser exist on the wire
    /// `tool` field (the geometry lives in `kind`), so every drawing tool but
    /// the eraser collapses to `Pen`. This keeps the socket/backend/core-types
    /// `WhiteboardTool` enums Pen/Eraser-only and the JSON backward-compatible.
    fn ink(&self) -> WhiteboardTool {
        match self {
            WhiteboardTool::Eraser => WhiteboardTool::Eraser,
            _ => WhiteboardTool::Pen,
        }
    }
}

impl WhiteboardKind {
    /// True for the two-anchor shape kinds (drag start → drag end).
    fn is_shape(&self) -> bool {
        matches!(
            self,
            WhiteboardKind::Line
                | WhiteboardKind::Rect
                | WhiteboardKind::Ellipse
                | WhiteboardKind::Arrow
        )
    }
}

/// Geometric vocabulary for a whiteboard element. `Freehand` is the original
/// pen/eraser polyline; shape kinds anchor on the first/last point of
/// `points`; `Text` carries a `text` body; `Image` references an uploaded file
/// asset (`asset_id`) and anchors on two points (top-left + bottom-right
/// bounds), like a rect. `#[serde(default)]` on the stroke's `kind` field keeps
/// older pen-only strokes parsing as `Freehand`.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WhiteboardKind {
    #[default]
    Freehand,
    Line,
    Rect,
    Ellipse,
    Arrow,
    Text,
    Image,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct WhiteboardPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct WhiteboardStroke {
    pub id: String,
    pub points: Vec<WhiteboardPoint>,
    pub color: String,
    pub width: f32,
    pub tool: WhiteboardTool,
    /// Geometric kind. Omitted on the wire by older pen-only clients, which
    /// `#[serde(default)]` parses as `WhiteboardKind::Freehand`.
    #[serde(default)]
    pub kind: WhiteboardKind,
    /// Body for `WhiteboardKind::Text` elements; omitted for every other kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// File-asset id for `WhiteboardKind::Image` elements. The uploaded image is
    /// referenced by id (not embedded as a data-URL) so the socket payload stays
    /// small; the renderer resolves a presigned GET URL on mount. Omitted for
    /// every other kind; `#[serde(default)]` keeps older strokes parsing as
    /// `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    /// User who authored this stroke (UUID string). Added additively for
    /// per-author undo/redo; `#[serde(default)]` keeps older strokes (no
    /// `author`) parsing as `None`. The server stamps this on every received
    /// stroke, so locally-originated strokes leave it `None` until they echo
    /// back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

/// Maximum length (in chars) of a whiteboard text-box body. Mirrors the
/// server-side `MAX_WHITEBOARD_TEXT_LEN`.
pub const MAX_WHITEBOARD_TEXT_LEN: usize = 280;

/// Maximum length (in chars) of an image element's `asset_id`. Mirrors the
/// server-side `MAX_WHITEBOARD_ASSET_ID_LEN` (file-asset ids are 36-char UUIDs;
/// the cap is a backstop).
pub const MAX_WHITEBOARD_ASSET_ID_LEN: usize = 96;

/// A remote participant's live cursor over the board. Ephemeral: never
/// persisted, never part of a snapshot. `x`/`y` are normalized board
/// coordinates in `0.0..=1.0`. Passed in as a prop so the view/broadcast
/// shells own the cursor map (keyed by `user_id`, refreshed on each
/// `WhiteboardCursor` event).
#[derive(Clone, PartialEq, Debug)]
pub struct RemoteCursor {
    pub user_id: String,
    pub display_name: String,
    pub x: f32,
    pub y: f32,
}

/// Maximum number of distinct remote cursors tracked at once. A safety cap so
/// a large room (or a hostile flood that slipped past the rate limit) can't
/// grow the overlay unbounded. Oldest-inserted entries are evicted first.
pub const MAX_REMOTE_CURSORS: usize = 64;

/// Upsert a remote cursor into `cursors` (keyed by `user_id`), evicting the
/// oldest when over [`MAX_REMOTE_CURSORS`]. Pure so the view/broadcast shells
/// can call it from their `WhiteboardCursor` dispatch without owning the merge
/// logic, and so it is unit-testable. Out-of-range coords are clamped.
pub fn apply_cursor_event(
    cursors: &mut Vec<RemoteCursor>,
    user_id: String,
    display_name: String,
    x: f32,
    y: f32,
) {
    let x = if x.is_finite() {
        x.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let y = if y.is_finite() {
        y.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if let Some(existing) = cursors.iter_mut().find(|c| c.user_id == user_id) {
        existing.display_name = display_name;
        existing.x = x;
        existing.y = y;
        return;
    }
    if cursors.len() >= MAX_REMOTE_CURSORS {
        cursors.remove(0);
    }
    cursors.push(RemoteCursor {
        user_id,
        display_name,
        x,
        y,
    });
}

/// Drop a participant's cursor (e.g. on a `Demoted`/`Kicked`/leave event, if
/// the shell wires it). Safe no-op when the id is absent.
pub fn remove_cursor(cursors: &mut Vec<RemoteCursor>, user_id: &str) {
    cursors.retain(|c| c.user_id != user_id);
}

/// Derive a stable, readable cursor/label color from a user id. A tiny FNV-1a
/// hash picks a hue on a fixed palette so the same user always gets the same
/// color across clients without any coordination. Kept pure for unit testing.
pub fn cursor_color_for(user_id: &str) -> &'static str {
    // 8-stop palette — distinct hues, all legible on the light board surface.
    const CURSOR_COLORS: [&str; 8] = [
        "#b8324a", "#2d5f88", "#2f6b45", "#b08842", "#6b4ea0", "#1f7a8c", "#c2570c", "#8a2d4d",
    ];
    // FNV-1a over the bytes — small, allocation-free, good enough spread.
    let mut hash: u32 = 0x811c_9dc5;
    for b in user_id.as_bytes() {
        hash ^= *b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    CURSOR_COLORS[(hash as usize) % CURSOR_COLORS.len()]
}

/// Decode a base64url (no-padding) segment into bytes. JWT segments are
/// base64url-encoded without `=` padding. Returns `None` on any invalid
/// character or a malformed trailing group. Self-contained (no `base64` crate)
/// so the call is identical and unit-testable on the host target — `base64`
/// is shared by browser JWT parsing and native PNG export in this crate.
fn b64url_decode(input: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut nbits: u32 = 0;
    for &b in bytes {
        let v = val(b)? as u32;
        acc = (acc << 6) | v;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
        }
    }
    Some(out)
}

/// Extract the `sub` claim (the signed-in user's id) from a JWT id-token. The
/// token is `header.payload.signature`; we base64url-decode the payload and
/// read its `sub` string. Returns `None` for a malformed token, a non-JSON
/// payload, or a missing/blank `sub`. We deliberately do NOT verify the
/// signature — this is a client-side read of the user's own token purely to
/// scope per-author whiteboard undo/redo; the server independently re-derives
/// and trusts the authenticated principal (it stamps `author` itself). Pure, so
/// it is unit-tested on the host target.
pub fn user_id_from_jwt(token: &str) -> Option<String> {
    let payload_b64 = token.split('.').nth(1)?;
    let payload_bytes = b64url_decode(payload_b64)?;
    let value: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
    let sub = value.get("sub")?.as_str()?.trim();
    if sub.is_empty() {
        None
    } else {
        Some(sub.to_string())
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct WhiteboardState {
    pub strokes: Vec<WhiteboardStroke>,
    pub active_stroke: Option<WhiteboardStroke>,
    pub tool: WhiteboardTool,
    pub color: String,
    pub width: f32,
}

impl Default for WhiteboardState {
    fn default() -> Self {
        Self {
            strokes: Vec::new(),
            active_stroke: None,
            tool: WhiteboardTool::Pen,
            color: "#111827".to_string(),
            width: 4.0,
        }
    }
}

pub fn append_stroke(state: &mut WhiteboardState, stroke: WhiteboardStroke) -> Result<(), String> {
    validate_stroke(&stroke)?;
    if state.strokes.len() >= MAX_STROKES {
        return Err("clear the board before adding more strokes".to_string());
    }
    state.strokes.push(stroke);
    Ok(())
}

pub fn clear_board(state: &mut WhiteboardState) {
    state.strokes.clear();
    state.active_stroke = None;
}

pub fn validate_stroke(stroke: &WhiteboardStroke) -> Result<(), String> {
    if stroke.id.trim().is_empty() || stroke.id.len() > 96 {
        return Err("stroke id is invalid".to_string());
    }
    // Point-count rules vary by kind: text anchors on a single point; shapes
    // anchor on exactly two; freehand polylines need at least two.
    match stroke.kind {
        WhiteboardKind::Text => {
            if stroke.points.is_empty() {
                return Err("stroke must contain at least one point".to_string());
            }
            match &stroke.text {
                Some(text) => {
                    if text.trim().is_empty() {
                        return Err("text is empty".to_string());
                    }
                    if text.chars().count() > MAX_WHITEBOARD_TEXT_LEN {
                        return Err("text is too long".to_string());
                    }
                }
                None => return Err("text element requires a body".to_string()),
            }
        }
        WhiteboardKind::Line
        | WhiteboardKind::Rect
        | WhiteboardKind::Ellipse
        | WhiteboardKind::Arrow => {
            if stroke.points.len() != 2 {
                return Err("shape must contain exactly two points".to_string());
            }
        }
        WhiteboardKind::Image => {
            if stroke.points.len() != 2 {
                return Err("image must contain exactly two points".to_string());
            }
            match &stroke.asset_id {
                Some(id) => {
                    if id.trim().is_empty() || id.len() > MAX_WHITEBOARD_ASSET_ID_LEN {
                        return Err("image asset id is invalid".to_string());
                    }
                }
                None => return Err("image element requires an asset id".to_string()),
            }
        }
        WhiteboardKind::Freehand => {
            if stroke.points.len() < 2 {
                return Err("stroke must contain at least two points".to_string());
            }
        }
    }
    if stroke.points.len() > MAX_POINTS_PER_STROKE {
        return Err("stroke contains too many points".to_string());
    }

    let color = stroke.color.as_bytes();
    let hex = color.strip_prefix(b"#").unwrap_or_default();
    if !matches!(hex.len(), 3 | 6 | 8) || !hex.iter().all(u8::is_ascii_hexdigit) {
        return Err("stroke color is invalid".to_string());
    }

    if !stroke.width.is_finite() || stroke.width < 1.0 || stroke.width > 32.0 {
        return Err("stroke width is invalid".to_string());
    }

    for point in &stroke.points {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err("stroke point is invalid".to_string());
        }
        if !(0.0..=1.0).contains(&point.x) || !(0.0..=1.0).contains(&point.y) {
            return Err("stroke point is outside the board".to_string());
        }
    }

    Ok(())
}

/// Teacher pen palette. Ink, oxblood, navy, green, gold — the brand ramps.
pub const PEN_COLORS: [&str; 5] = ["#111827", "#b8324a", "#2d5f88", "#2f6b45", "#b08842"];

/// Pen width presets (Fine / Medium / Bold).
pub const PEN_WIDTHS: [(f32, &str); 3] = [(2.5, "Fine"), (4.0, "Medium"), (8.0, "Bold")];

/// Eraser strokes paint with the board surface color at a wider gauge.
pub const ERASER_WIDTH_FACTOR: f32 = 3.0;

fn points_attr(points: &[WhiteboardPoint]) -> String {
    points
        .iter()
        .map(|point| {
            format!(
                "{:.1},{:.1}",
                point.x.clamp(0.0, 1.0) * 1000.0,
                point.y.clamp(0.0, 1.0) * 600.0
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Normalize a mouse position to 0..1 board coordinates using the rendered
/// canvas size. Dividing by the SVG viewBox (1000×600) instead — as this did
/// originally — distorts strokes on every viewport whose rendered size
/// differs from the viewBox: a teacher drawing at the right edge of a 640px
/// canvas produced x=0.64, which students saw as a stroke stopping at 64%.
fn event_point(evt: &PointerEvent, canvas_size: (f64, f64)) -> WhiteboardPoint {
    let position = evt.element_coordinates();
    let (w, h) = canvas_size;
    let w = if w > 1.0 { w } else { 1000.0 };
    let h = if h > 1.0 { h } else { 600.0 };
    WhiteboardPoint {
        x: ((position.x / w) as f32).clamp(0.0, 1.0),
        y: ((position.y / h) as f32).clamp(0.0, 1.0),
    }
}

/// SVG viewBox dimensions. Board coordinates are 0..1; rendering scales by
/// these. Kept as constants so geometry helpers and the renderer agree.
const VIEW_W: f32 = 1000.0;
const VIEW_H: f32 = 600.0;

/// Scale a normalized point into viewBox coordinates.
fn scaled(point: &WhiteboardPoint) -> (f32, f32) {
    (
        point.x.clamp(0.0, 1.0) * VIEW_W,
        point.y.clamp(0.0, 1.0) * VIEW_H,
    )
}

/// Squared distance from point `p` to the segment `a`–`b`, in normalized
/// board units. Used by the geometric eraser hit-test. Squared to avoid a
/// `sqrt` in the inner loop.
fn point_segment_dist_sq(p: &WhiteboardPoint, a: &WhiteboardPoint, b: &WhiteboardPoint) -> f32 {
    let (px, py) = (p.x, p.y);
    let (ax, ay) = (a.x, a.y);
    let (bx, by) = (b.x, b.y);
    let dx = bx - ax;
    let dy = by - ay;
    let len_sq = dx * dx + dy * dy;
    if len_sq <= f32::EPSILON {
        // Degenerate segment: distance to the single point.
        let ex = px - ax;
        let ey = py - ay;
        return ex * ex + ey * ey;
    }
    let t = (((px - ax) * dx + (py - ay) * dy) / len_sq).clamp(0.0, 1.0);
    let cx = ax + t * dx;
    let cy = ay + t * dy;
    let ex = px - cx;
    let ey = py - cy;
    ex * ex + ey * ey
}

/// True if the normalized point `p` falls within `radius` (board units) of any
/// segment of `stroke`. For freehand/shape elements this walks the polyline;
/// for shapes the two anchor points still describe the relevant segment (a
/// good-enough proxy that also matches the rectangle/ellipse bounding span).
/// For text the single anchor point is treated as a degenerate target.
fn stroke_hit_by(stroke: &WhiteboardStroke, p: &WhiteboardPoint, radius: f32) -> bool {
    let r_sq = radius * radius;
    match stroke.points.as_slice() {
        [] => false,
        [only] => {
            let ex = p.x - only.x;
            let ey = p.y - only.y;
            ex * ex + ey * ey <= r_sq
        }
        pts => pts
            .windows(2)
            .any(|w| point_segment_dist_sq(p, &w[0], &w[1]) <= r_sq),
    }
}

/// Geometric-eraser hit radius in normalized board units. Roughly a fifth of
/// the board height — generous enough to catch elements under a dragging
/// eraser without requiring pixel-perfect aim.
pub const ERASER_HIT_RADIUS: f32 = 0.02;

/// Return the ids of every committed element hit by an eraser pass at `p`.
/// (The component erases these locally and asks the server to remove them.)
pub fn elements_hit_at(state: &WhiteboardState, p: &WhiteboardPoint) -> Vec<String> {
    state
        .strokes
        .iter()
        .filter(|s| stroke_hit_by(s, p, ERASER_HIT_RADIUS))
        .map(|s| s.id.clone())
        .collect()
}

/// Remove a stroke by id (server `whiteboard_stroke_removed`, teacher undo).
pub fn remove_stroke(state: &mut WhiteboardState, stroke_id: &str) {
    state.strokes.retain(|s| s.id != stroke_id);
}

/// Index of the stroke an undo for `user_id` should drop: this user's
/// most-recent stroke (matched by server-stamped `author`, or a not-yet-echoed
/// local stroke with `author == None`). When `user_id` is empty (the shell
/// didn't supply the local id) it falls back to the board's most-recent stroke
/// so undo still removes something. Pure, so it is unit-tested directly.
pub fn last_stroke_index_for_author(strokes: &[WhiteboardStroke], user_id: &str) -> Option<usize> {
    if user_id.is_empty() {
        return strokes.len().checked_sub(1);
    }
    strokes
        .iter()
        .rposition(|s| s.author.as_deref() == Some(user_id) || s.author.is_none())
}

/// Geometric eraser: hit-test the pointer against committed elements, remove
/// every hit from the local board, and notify the server (per id) so it drops
/// them too. A free fn (taking the `Copy` `Signal` + `EventHandler`) rather than
/// a closure, so both the pointerdown and pointermove handlers can call it
/// without moving a mutating closure into two places.
fn erase_at(
    mut board: Signal<WhiteboardState>,
    on_erase: EventHandler<String>,
    point: &WhiteboardPoint,
) {
    let hits = board.with(|state| elements_hit_at(state, point));
    if hits.is_empty() {
        return;
    }
    board.with_mut(|state| {
        for id in &hits {
            remove_stroke(state, id);
        }
    });
    for id in hits {
        on_erase.call(id);
    }
}

/// Content types accepted for a pasted/dropped whiteboard image. Mirrors the
/// image subset of the server's `attachment`/`whiteboard` purpose allow-list.
pub const WHITEBOARD_IMAGE_TYPES: [&str; 3] = ["image/jpeg", "image/png", "image/webp"];

/// Size cap (bytes) for a pasted/dropped whiteboard image. Conservative (8 MiB)
/// so a stray large paste is rejected client-side before any upload begins; the
/// server re-checks against the purpose cap.
pub const WHITEBOARD_IMAGE_MAX_BYTES: i64 = 8 * 1024 * 1024;

/// Default normalized bounds (top-left + bottom-right) for a freshly
/// pasted/dropped image: a centered box ~40% of the board, clamped to 0..1.
/// Pure so it is unit-testable; the author can move/resize later (future work).
pub fn default_image_bounds() -> [WhiteboardPoint; 2] {
    [
        WhiteboardPoint { x: 0.30, y: 0.30 },
        WhiteboardPoint { x: 0.70, y: 0.70 },
    ]
}

/// Build a `WhiteboardKind::Image` stroke referencing an uploaded `asset_id`,
/// anchored on `bounds`. The `color`/`width` carry the current tool defaults
/// (unused by the image renderer but required by the shared stroke shape and
/// validated by `validate_stroke`). Pure + unit-tested.
pub fn image_stroke(asset_id: String, bounds: [WhiteboardPoint; 2]) -> WhiteboardStroke {
    WhiteboardStroke {
        id: uuid::Uuid::new_v4().to_string(),
        points: bounds.to_vec(),
        // Renderer ignores these for images; kept valid for the shared shape.
        color: PEN_COLORS[0].to_string(),
        width: 4.0,
        tool: WhiteboardTool::Pen,
        kind: WhiteboardKind::Image,
        text: None,
        asset_id: Some(asset_id),
        author: None,
    }
}

/// Minimum gap (ms) between two emitted cursor positions. Keeps a smooth drag
/// from spamming the socket; the server caps it again as a backstop.
pub const CURSOR_THROTTLE_MS: f64 = 60.0;

/// Current wall-clock time in ms since the epoch (`Date.now()` in a browser,
/// `SystemTime` in native Rust). Both renderers emit pointer/cursor events; the
/// throttle logic is unit-tested via [`should_emit_cursor`].
#[cfg(target_arch = "wasm32")]
fn now_ms() -> f64 {
    js_sys::Date::now()
}
#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64() * 1000.0)
        .unwrap_or_default()
}

/// Pure throttle predicate: emit if at least [`CURSOR_THROTTLE_MS`] elapsed
/// since `last_ms`. Extracted so it can be unit-tested without a clock.
pub fn should_emit_cursor(now: f64, last_ms: f64) -> bool {
    now - last_ms >= CURSOR_THROTTLE_MS
}

/// Emit the local cursor position through `on_cursor` at most once per
/// [`CURSOR_THROTTLE_MS`]. Updates `last_cursor_ms` on each emit. `x`/`y` are
/// normalized board coords (already clamped by `event_point`).
fn emit_cursor_throttled(
    on_cursor: &EventHandler<(f32, f32)>,
    last_cursor_ms: &mut Signal<f64>,
    x: f32,
    y: f32,
) {
    let now = now_ms();
    if should_emit_cursor(now, *last_cursor_ms.read()) {
        last_cursor_ms.set(now);
        on_cursor.call((x, y));
    }
}

/// Replace the board with a server snapshot (socket hydration for late
/// joiners / reconnects). Strokes beyond the cap are dropped oldest-first.
pub fn apply_snapshot(state: &mut WhiteboardState, mut strokes: Vec<WhiteboardStroke>) {
    strokes.retain(|s| validate_stroke(s).is_ok());
    let len = strokes.len();
    if len > MAX_STROKES {
        strokes.drain(..len - MAX_STROKES);
    }
    state.strokes = strokes;
    state.active_stroke = None;
}

fn sync_board_from_props(
    board: &mut WhiteboardState,
    last_props: &mut WhiteboardState,
    incoming: &WhiteboardState,
) {
    if last_props != incoming {
        *board = incoming.clone();
        *last_props = incoming.clone();
    }
}

fn finish_active_stroke(
    state: &mut WhiteboardState,
    release_point: WhiteboardPoint,
) -> Option<WhiteboardStroke> {
    let mut stroke = state.active_stroke.take()?;
    if stroke.kind.is_shape() {
        // Shapes are two-anchor: keep the drag start (first point) and the
        // release point, discarding the intermediate move samples.
        let start = stroke.points.first().cloned().unwrap_or(WhiteboardPoint {
            x: release_point.x,
            y: release_point.y,
        });
        stroke.points = vec![start, release_point];
    } else {
        stroke.points.push(release_point);
    }
    if append_stroke(state, stroke.clone()).is_ok() {
        Some(stroke)
    } else {
        None
    }
}

/// Render a single committed/active element as the appropriate SVG primitive.
/// Freehand pen + eraser stay polylines (preserving the existing markup and
/// `whiteboard-stroke--eraser` class); shapes become line/rect/ellipse/path;
/// text becomes an SVG `<text>`. The `key` is the stroke id so Dioxus diffs
/// elements stably across snapshots.
fn render_element(stroke: &WhiteboardStroke) -> Element {
    let id = stroke.id.clone();
    let color = stroke.color.clone();
    let width = stroke.width.to_string();
    match stroke.kind {
        WhiteboardKind::Freehand => {
            let points = points_attr(&stroke.points);
            let class = if matches!(stroke.tool, WhiteboardTool::Eraser) {
                "whiteboard-stroke whiteboard-stroke--eraser"
            } else {
                "whiteboard-stroke"
            };
            rsx! {
                polyline {
                    key: "{id}",
                    class: "{class}",
                    points: "{points}",
                    fill: "none",
                    stroke: "{color}",
                    stroke_width: "{width}",
                    stroke_linecap: "round",
                    stroke_linejoin: "round",
                }
            }
        }
        WhiteboardKind::Line => {
            let (x1, y1) = stroke.points.first().map(scaled).unwrap_or((0.0, 0.0));
            let (x2, y2) = stroke.points.get(1).map(scaled).unwrap_or((x1, y1));
            rsx! {
                line {
                    key: "{id}",
                    class: "whiteboard-shape",
                    x1: "{x1}", y1: "{y1}", x2: "{x2}", y2: "{y2}",
                    stroke: "{color}",
                    stroke_width: "{width}",
                    stroke_linecap: "round",
                }
            }
        }
        WhiteboardKind::Rect => {
            let (x1, y1) = stroke.points.first().map(scaled).unwrap_or((0.0, 0.0));
            let (x2, y2) = stroke.points.get(1).map(scaled).unwrap_or((x1, y1));
            let x = x1.min(x2);
            let y = y1.min(y2);
            let w = (x2 - x1).abs();
            let h = (y2 - y1).abs();
            rsx! {
                rect {
                    key: "{id}",
                    class: "whiteboard-shape",
                    x: "{x}", y: "{y}", width: "{w}", height: "{h}",
                    fill: "none",
                    stroke: "{color}",
                    stroke_width: "{width}",
                }
            }
        }
        WhiteboardKind::Ellipse => {
            let (x1, y1) = stroke.points.first().map(scaled).unwrap_or((0.0, 0.0));
            let (x2, y2) = stroke.points.get(1).map(scaled).unwrap_or((x1, y1));
            let cx = (x1 + x2) / 2.0;
            let cy = (y1 + y2) / 2.0;
            let rx = (x2 - x1).abs() / 2.0;
            let ry = (y2 - y1).abs() / 2.0;
            rsx! {
                ellipse {
                    key: "{id}",
                    class: "whiteboard-shape",
                    cx: "{cx}", cy: "{cy}", rx: "{rx}", ry: "{ry}",
                    fill: "none",
                    stroke: "{color}",
                    stroke_width: "{width}",
                }
            }
        }
        WhiteboardKind::Arrow => {
            let (x1, y1) = stroke.points.first().map(scaled).unwrap_or((0.0, 0.0));
            let (x2, y2) = stroke.points.get(1).map(scaled).unwrap_or((x1, y1));
            // Arrowhead: two short barbs at the tip, rotated to the segment
            // direction. Built as a single polyline path so no <marker> defs
            // are needed (markers don't inherit the stroke cleanly here).
            let dx = x2 - x1;
            let dy = y2 - y1;
            let len = (dx * dx + dy * dy).sqrt().max(0.001);
            let (ux, uy) = (dx / len, dy / len);
            let head = (12.0 + stroke.width * 1.5).min(len);
            // Barb base point, stepped back along the shaft.
            let bx = x2 - ux * head;
            let by = y2 - uy * head;
            // Perpendicular offset for the two barbs.
            let (px, py) = (-uy, ux);
            let half = head * 0.5;
            let lx = bx + px * half;
            let ly = by + py * half;
            let rxx = bx - px * half;
            let ryy = by - py * half;
            let shaft = format!("{x1:.1},{y1:.1} {x2:.1},{y2:.1}");
            let barbs = format!("{lx:.1},{ly:.1} {x2:.1},{y2:.1} {rxx:.1},{ryy:.1}");
            rsx! {
                g { key: "{id}", class: "whiteboard-shape",
                    polyline {
                        points: "{shaft}",
                        fill: "none",
                        stroke: "{color}",
                        stroke_width: "{width}",
                        stroke_linecap: "round",
                    }
                    polyline {
                        points: "{barbs}",
                        fill: "none",
                        stroke: "{color}",
                        stroke_width: "{width}",
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                    }
                }
            }
        }
        WhiteboardKind::Text => {
            let (x, y) = stroke.points.first().map(scaled).unwrap_or((0.0, 0.0));
            let body = stroke.text.clone().unwrap_or_default();
            // Font size scales with the chosen stroke width so the text-size
            // control is the width control, like the other tools.
            let font_size = (stroke.width * 4.0).clamp(12.0, 96.0);
            rsx! {
                text {
                    key: "{id}",
                    class: "whiteboard-text",
                    x: "{x}", y: "{y}",
                    fill: "{color}",
                    font_size: "{font_size}",
                    "{body}"
                }
            }
        }
        WhiteboardKind::Image => {
            // Two-anchor bounds (top-left + bottom-right), normalized → viewBox.
            let (x1, y1) = stroke.points.first().map(scaled).unwrap_or((0.0, 0.0));
            let (x2, y2) = stroke.points.get(1).map(scaled).unwrap_or((x1, y1));
            let x = x1.min(x2);
            let y = y1.min(y2);
            let w = (x2 - x1).abs().max(1.0);
            let h = (y2 - y1).abs().max(1.0);
            let asset_id = stroke.asset_id.clone().unwrap_or_default();
            // Delegated to a component so it can resolve a fresh presigned GET
            // URL via `use_resource` (a plain fn cannot use hooks).
            rsx! {
                WhiteboardImageElement {
                    key: "{id}",
                    asset_id,
                    x, y, width: w, height: h,
                }
            }
        }
    }
}

/// Renders a whiteboard `Image` element as an SVG `<image>`. Resolves a fresh
/// presigned GET URL for `asset_id` on mount (same pattern as
/// `FileAssetImage`), so short-lived URLs are never persisted on the board or
/// shipped over the socket. While the URL is loading or unavailable, a faint
/// placeholder rect keeps the element's bounds visible.
#[component]
fn WhiteboardImageElement(asset_id: String, x: f32, y: f32, width: f32, height: f32) -> Element {
    // `try_consume_context` (not the panicking `use_api`) so SSR/tests without
    // an ApiContext provider render the placeholder instead of panicking.
    let cx = try_consume_context::<Signal<crate::api::ApiContext>>()
        .map(|s| s.read().clone())
        .unwrap_or_else(|| crate::api::ApiContext {
            base_url: String::new(),
            id_token: String::new(),
        });
    let asset_for_resource = asset_id.clone();
    let url_resource = use_resource(move || {
        let cx = cx.clone();
        let asset_id = asset_for_resource.clone();
        async move {
            crate::api::file_asset_get_url(&cx, &asset_id)
                .await
                .map(|r| r.url)
        }
    });

    match &*url_resource.read_unchecked() {
        Some(Ok(url)) => rsx! {
            image {
                class: "whiteboard-image",
                x: "{x}", y: "{y}", width: "{width}", height: "{height}",
                href: "{url}",
                preserve_aspect_ratio: "none",
            }
        },
        // Loading or error: a faint outlined placeholder so the element's
        // footprint is still visible (and erasable) on the board.
        _ => rsx! {
            rect {
                class: "whiteboard-image whiteboard-image--placeholder",
                x: "{x}", y: "{y}", width: "{width}", height: "{height}",
                fill: "none",
                stroke: "#9ca3af",
                stroke_width: "2",
                stroke_dasharray: "6 6",
            }
        },
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct LiveRoomWhiteboardProps {
    pub state: WhiteboardState,
    pub is_teacher: bool,
    pub on_emit_stroke: EventHandler<WhiteboardStroke>,
    pub on_clear: EventHandler<()>,
    /// Teacher undo: ask the server to drop the most recent stroke. The
    /// removal lands back as a `whiteboard_stroke_removed` event.
    #[props(default)]
    pub on_undo: EventHandler<()>,
    /// Geometric eraser: ask the server to drop a specific element by id (the
    /// component hit-tests the eraser path locally and emits one call per hit
    /// element). The authoritative removal lands back as
    /// `whiteboard_stroke_removed`. Defaulted so the student call site, which
    /// never erases, need not pass it.
    #[props(default)]
    pub on_erase: EventHandler<String>,
    /// This client's own user id (UUID string), used to (a) filter the local
    /// user out of the remote-cursor overlay and (b) author the per-user
    /// undo/redo request. Empty when unknown (SSR / tests): cursors still
    /// render and undo falls back to the global path.
    #[props(default)]
    pub local_user_id: String,
    /// Live remote cursors to overlay (already excludes the local user upstream
    /// is fine too; the component double-checks against `local_user_id`).
    /// Keyed/refreshed by the view/broadcast shell from `WhiteboardCursor`
    /// events. Defaulted so call sites that don't wire cursors compile.
    #[props(default)]
    pub cursors: Vec<RemoteCursor>,
    /// Emit the local pointer position (normalized board coords) so the shell
    /// can broadcast it as a `whiteboard_cursor`. Throttled inside the
    /// component (~60ms). Defaulted to a no-op.
    #[props(default)]
    pub on_cursor: EventHandler<(f32, f32)>,
    /// Whether non-teachers may currently draw. Drives the toolbar visibility
    /// for students and the teacher's toggle state. Hydrated/updated by the
    /// shell from `DrawPermissionChanged`. Defaulted to `false` (teacher-only).
    #[props(default)]
    pub draw_open: bool,
    /// Teacher-only: toggle whether students may draw. Defaulted to a no-op so
    /// the student call site need not pass it.
    #[props(default)]
    pub on_set_draw_permission: EventHandler<bool>,
    /// The live session id this board belongs to. Used as the `linked_entity_id`
    /// when uploading a pasted/dropped image via the presigned-upload path
    /// (`linked_entity_type = "live_session"`, `purpose = "whiteboard"`).
    /// Defaulted to empty so SSR/test call sites compile. Clipboard/drop image
    /// ingestion currently requires browser `DataTransfer` and is unavailable
    /// in native builds; drawing, text, shapes and PNG export remain active.
    #[props(default)]
    pub session_id: String,
}

#[allow(non_snake_case)]
pub fn LiveRoomWhiteboard(props: LiveRoomWhiteboardProps) -> Element {
    let mut board = use_signal(|| props.state.clone());
    let mut last_props = use_signal(|| props.state.clone());

    // Tool preferences are purely local UI state — kept out of `board` so a
    // props sync (which carries the default tool/color) can't reset the
    // teacher's selection mid-class.
    let mut tool = use_signal(WhiteboardTool::default);
    let mut color = use_signal(|| PEN_COLORS[0].to_string());
    let mut width = use_signal(|| PEN_WIDTHS[1].0);

    // Per-author redo stack: strokes this user undid, newest last. A redo pops
    // the top and re-emits it as a normal stroke (the server re-appends and
    // fans it out). Pushing a fresh stroke clears the stack (standard undo/redo
    // semantics: a new edit invalidates the redo history).
    let mut redo_stack = use_signal(Vec::<WhiteboardStroke>::new);

    // Whether this client may draw at all: the teacher always may; a non-teacher
    // only while the room's draw-permission flag is open.
    let can_draw = props.is_teacher || props.draw_open;
    let local_user_id = props.local_user_id.clone();

    // Rendered canvas size, captured at mount, used to normalize mouse
    // positions (see `event_point`). The viewBox stays 1000×600 for drawing.
    let mut canvas_size = use_signal(|| (1000.0_f64, 600.0_f64));

    // Cursor throttle: last emit time (ms since epoch). Capped at ~60ms so a
    // smooth drag doesn't spam the socket. Kept as a signal so the (Copy)
    // pointer-move handler can read/update it without borrowing `props`.
    let mut last_cursor_ms = use_signal(|| 0.0_f64);

    let incoming_state = props.state.clone();
    use_effect(move || {
        board.with_mut(|state| {
            last_props.with_mut(|last| sync_board_from_props(state, last, &incoming_state));
        });
    });

    // Re-measure the rendered canvas on window resize. `canvas_size` is captured
    // once at mount (the `onmounted` handler below), but layout changes — opening
    // a sidebar, rotating a phone, the responsive min-height breakpoint — change
    // the SVG's pixel size. Without re-measuring, `event_point` divides by a
    // stale size and every new stroke is normalized incorrectly relative to what
    // students and late-joiners see, reintroducing the exact distortion the
    // mount-time measurement was added to fix. Listener ownership + teardown
    // mirror the command-palette / live_room_socket pattern (use_hook cell,
    // detached in use_drop) so the closure is not dropped early.
    #[cfg(target_arch = "wasm32")]
    {
        use std::cell::RefCell;
        use std::rc::Rc;
        use wasm_bindgen::closure::Closure;
        use wasm_bindgen::JsCast;

        type ResizeClosure = Closure<dyn FnMut(web_sys::Event)>;
        let listener: Rc<RefCell<Option<ResizeClosure>>> = use_hook(|| Rc::new(RefCell::new(None)));
        let listener_setup = listener.clone();
        let listener_drop = listener.clone();

        use_effect(move || {
            if listener_setup.borrow().is_some() {
                return;
            }
            let Some(win) = web_sys::window() else {
                return;
            };
            let mut canvas_size = canvas_size;
            let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |_evt: web_sys::Event| {
                if let Some(el) = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.get_element_by_id(WHITEBOARD_CANVAS_ID))
                {
                    let rect = el.get_bounding_client_rect();
                    let (w, h) = (rect.width(), rect.height());
                    if w > 1.0 && h > 1.0 {
                        canvas_size.set((w, h));
                    }
                }
            });
            if win
                .add_event_listener_with_callback("resize", closure.as_ref().unchecked_ref())
                .is_ok()
            {
                *listener_setup.borrow_mut() = Some(closure);
            }
        });

        use_drop(move || {
            if let Some(closure) = listener_drop.borrow_mut().take() {
                if let Some(win) = web_sys::window() {
                    let _ = win.remove_event_listener_with_callback(
                        "resize",
                        closure.as_ref().unchecked_ref(),
                    );
                }
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut canvas_size = canvas_size;
        use_future(move || async move {
            let mut observer = document::eval(
                r#"
const element = document.getElementById("live-room-whiteboard-canvas");
if (!element || typeof ResizeObserver !== "function") return false;
const sendSize = () => {
  const rect = element.getBoundingClientRect();
  if (rect.width > 1 && rect.height > 1) dioxus.send([rect.width, rect.height]);
};
const resizeObserver = new ResizeObserver(sendSize);
resizeObserver.observe(element);
// Capture in the WebView's own event turn. Calling through an async Rust eval
// after `pointerdown` is too late for fast stylus/touch drags.
const capturePointer = (event) => {
  try { element.setPointerCapture(event.pointerId); } catch (_) {}
};
element.addEventListener("pointerdown", capturePointer, true);
sendSize();
try { await new Promise(() => {}); }
finally {
  element.removeEventListener("pointerdown", capturePointer, true);
  resizeObserver.disconnect();
}
"#,
            );
            while let Ok((width, height)) = observer.recv::<(f64, f64)>().await {
                if width > 1.0 && height > 1.0 {
                    canvas_size.set((width, height));
                }
            }
        });
    }

    let on_clear = props.on_clear;
    let clear = move |_| {
        board.with_mut(clear_board);
        on_clear.call(());
    };

    // Undo: drop this user's most-recent stroke locally, push it onto the redo
    // stack, then ask the server to remove it (per-author on the wire). Falls
    // back to the board's global last stroke when no author info is available
    // (keeps the legacy teacher global-undo working). `mine` matches strokes
    // the server stamped with our id OR not-yet-echoed local strokes
    // (`author == None`).
    let on_undo = props.on_undo;
    let undo_user_id = local_user_id.clone();
    let undo = move |_| {
        let uid = undo_user_id.clone();
        let undone = board.with_mut(|state| {
            // Prefer this user's most-recent stroke (server-stamped author, or a
            // not-yet-echoed local stroke with `author == None`); falls back to
            // the board's most-recent stroke when our id is unknown.
            last_stroke_index_for_author(&state.strokes, &uid).map(|i| state.strokes.remove(i))
        });
        if let Some(stroke) = undone {
            redo_stack.with_mut(|stack| {
                stack.push(stroke);
                // Bound the redo history so it can't grow without limit.
                if stack.len() > MAX_STROKES {
                    let overflow = stack.len() - MAX_STROKES;
                    stack.drain(..overflow);
                }
            });
        }
        // Tell the server to remove our most-recent stroke (it authoritatively
        // re-broadcasts a `whiteboard_stroke_removed`, which is idempotent with
        // the local removal above).
        on_undo.call(());
    };

    // Redo: pop the most-recently-undone stroke and re-emit it as a fresh
    // stroke. The server re-appends + fans it out; the echo lands it back on
    // the local board (so we don't add it locally here to avoid a duplicate).
    let on_emit_redo = props.on_emit_stroke;
    let redo = move |_| {
        let stroke = redo_stack.with_mut(|stack| stack.pop());
        if let Some(stroke) = stroke {
            on_emit_redo.call(stroke);
        }
    };

    // Renderer-specific PNG export: web delegates to the bundled `window.aula`
    // helper; native rasterizes in the WebView and sends bounded bytes through
    // the installed-app save/share path.
    let export_png = move |_| {
        imp::export_whiteboard_png(WHITEBOARD_CANVAS_ID);
    };

    let on_emit_stroke = props.on_emit_stroke;
    let on_erase = props.on_erase;

    // Geometric eraser lives in the free fn `erase_at` (below), called from
    // pointerdown + pointermove. It's a free fn (not a closure) so both `move`
    // handlers can call it without the closure-Copy / FnMut borrow issues that
    // arise from moving a mutating closure into two places.

    let start_stroke = move |evt: PointerEvent| {
        if !can_draw {
            return;
        }
        let stroke_tool = tool.read().clone();
        let point = event_point(&evt, *canvas_size.read());

        // The text tool places a single element at the click point; it does
        // not drag. Prompt for the body via the browser (wasm only).
        if stroke_tool == WhiteboardTool::Text {
            #[cfg(target_arch = "wasm32")]
            {
                let body = web_sys::window()
                    .and_then(|w| w.prompt_with_message("Text:").ok().flatten())
                    .unwrap_or_default();
                let trimmed = body.trim();
                if trimmed.is_empty() {
                    return;
                }
                let mut text: String = trimmed.chars().take(MAX_WHITEBOARD_TEXT_LEN).collect();
                text = text.trim().to_string();
                let stroke = WhiteboardStroke {
                    id: uuid::Uuid::new_v4().to_string(),
                    points: vec![point],
                    color: color.read().clone(),
                    width: *width.read(),
                    // Wire ink mode is Pen; the `kind` carries "text".
                    tool: WhiteboardTool::Pen,
                    kind: WhiteboardKind::Text,
                    text: Some(text),
                    asset_id: None,
                    // Author is stamped server-side on echo; left None locally.
                    author: None,
                };
                let appended = board.with_mut(|state| append_stroke(state, stroke.clone()).is_ok());
                if appended {
                    // A fresh edit invalidates the redo history.
                    redo_stack.with_mut(Vec::clear);
                    on_emit_stroke.call(stroke);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let mut board = board;
                let mut redo_stack = redo_stack;
                let color = color.read().clone();
                let stroke_width = *width.read();
                let on_emit_stroke = on_emit_stroke;
                spawn(async move {
                    let Ok(Some(body)) = crate::live_room_native::prompt_text("Text:").await else {
                        return;
                    };
                    let mut text: String =
                        body.trim().chars().take(MAX_WHITEBOARD_TEXT_LEN).collect();
                    text = text.trim().to_string();
                    if text.is_empty() {
                        return;
                    }
                    let stroke = WhiteboardStroke {
                        id: uuid::Uuid::new_v4().to_string(),
                        points: vec![point],
                        color,
                        width: stroke_width,
                        tool: WhiteboardTool::Pen,
                        kind: WhiteboardKind::Text,
                        text: Some(text),
                        asset_id: None,
                        author: None,
                    };
                    let appended =
                        board.with_mut(|state| append_stroke(state, stroke.clone()).is_ok());
                    if appended {
                        redo_stack.with_mut(Vec::clear);
                        on_emit_stroke.call(stroke);
                    }
                });
            }
            return;
        }

        // The geometric eraser removes whole elements rather than painting a
        // surface-colored stroke. Erase on press and keep erasing on drag.
        if stroke_tool == WhiteboardTool::Eraser {
            #[cfg(target_arch = "wasm32")]
            {
                let pid = evt.pointer_id();
                if let Some(el) = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.get_element_by_id(WHITEBOARD_CANVAS_ID))
                {
                    let _ = el.set_pointer_capture(pid);
                }
            }
            erase_at(board, on_erase, &point);
            return;
        }

        // Capture the pointer so a fast drag that briefly leaves the SVG keeps
        // delivering move/up events to it (instead of firing pointerleave and
        // dropping the in-progress stroke). Pointer events also bring touch +
        // stylus support that the old mouse handlers lacked.
        #[cfg(target_arch = "wasm32")]
        {
            let pid = evt.pointer_id();
            if let Some(el) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id(WHITEBOARD_CANVAS_ID))
            {
                let _ = el.set_pointer_capture(pid);
            }
        }
        let stroke_width = *width.read();
        let kind = stroke_tool.kind();
        // Wire ink mode is Pen for every shape/pen tool; geometry is in `kind`.
        let ink = stroke_tool.ink();
        board.with_mut(|state| {
            state.active_stroke = Some(WhiteboardStroke {
                id: uuid::Uuid::new_v4().to_string(),
                points: vec![point],
                color: color.read().clone(),
                width: stroke_width,
                tool: ink,
                kind,
                text: None,
                asset_id: None,
                author: None,
            });
        });
    };

    let on_cursor = props.on_cursor;
    let extend_stroke = move |evt: PointerEvent| {
        if !can_draw {
            return;
        }
        let point = event_point(&evt, *canvas_size.read());
        // Broadcast the live cursor position (throttled ~60ms) so other
        // participants see where this user is pointing. Best-effort and
        // independent of whether a stroke is in progress.
        emit_cursor_throttled(&on_cursor, &mut last_cursor_ms, point.x, point.y);
        if *tool.read() == WhiteboardTool::Eraser {
            erase_at(board, on_erase, &point);
            return;
        }
        board.with_mut(|state| {
            if let Some(stroke) = state.active_stroke.as_mut() {
                if stroke.kind.is_shape() {
                    // Live preview: keep the start anchor, update the second
                    // anchor to the current pointer.
                    if stroke.points.len() < 2 {
                        stroke.points.push(point);
                    } else {
                        stroke.points[1] = point;
                    }
                } else if stroke.points.len() < MAX_POINTS_PER_STROKE {
                    stroke.points.push(point);
                }
            }
        });
    };

    let finish_stroke = move |evt: PointerEvent| {
        if !can_draw {
            return;
        }
        if *tool.read() == WhiteboardTool::Eraser {
            return;
        }
        let release_point = event_point(&evt, *canvas_size.read());
        let finalized = board.with_mut(|state| finish_active_stroke(state, release_point));
        if let Some(stroke) = finalized {
            // A fresh edit invalidates the redo history.
            redo_stack.with_mut(Vec::clear);
            on_emit_stroke.call(stroke);
        }
    };

    let cancel_stroke = move |_| {
        // Native installs immediate WebView pointer capture above, so crossing
        // the SVG edge must not discard the active touch/stylus stroke. Browser
        // capture is synchronous in `start_stroke`; retaining its historical
        // leave cancellation also handles engines that reject capture.
        #[cfg(target_arch = "wasm32")]
        if can_draw {
            board.with_mut(|state| state.active_stroke = None);
        }
    };

    let rendered_strokes = board.with(|state| {
        let mut strokes = state.strokes.clone();
        if let Some(active) = &state.active_stroke {
            strokes.push(active.clone());
        }
        strokes
    });

    let current_tool = tool.read().clone();
    let current_color = color.read().clone();
    let current_width = *width.read();

    // Remote cursors to overlay: everyone except the local user (we never draw
    // our own cursor — the browser already shows it). Filtering here makes the
    // component robust even if the shell forgets to exclude self.
    let cursor_self = local_user_id.clone();
    let remote_cursors: Vec<RemoteCursor> = props
        .cursors
        .iter()
        .filter(|c| c.user_id != cursor_self)
        .cloned()
        .collect();

    // Teacher draw-permission toggle state + handler.
    let draw_open = props.draw_open;
    let on_set_draw_permission = props.on_set_draw_permission;
    let toggle_draw = move |_| {
        on_set_draw_permission.call(!draw_open);
    };

    // Image paste/drop: when an image is pasted (Ctrl/Cmd-V) or dropped onto the
    // board, upload it via the existing presigned-upload path and emit a
    // `WhiteboardKind::Image` stroke referencing the asset id. The whole flow is
    // wasm-only (DOM clipboard / drag APIs); a no-op on the host target and when
    // the user cannot draw or no session id is wired.
    let session_id = props.session_id.clone();
    let on_emit_image = props.on_emit_stroke;
    // Toast + API context are read via `try_consume_context` (not the panicking
    // `use_context` wrappers) so SSR / unit tests that render this component
    // without the providers don't panic — image paste/drop simply no-ops there.
    // The API context is captured during render so the paste/drop handlers
    // (which fire outside the render cycle) never call a hook out of order. The
    // fallback toast signal is created unconditionally to keep hook order
    // stable across renders (Rules of Hooks); it's only used when no provider
    // is mounted (tests).
    let fallback_toast = use_signal(design_system::ToastQueue::new);
    let toast = design_system::ToastSender(
        try_consume_context::<Signal<design_system::ToastQueue>>().unwrap_or(fallback_toast),
    );
    let api_cx = try_consume_context::<Signal<crate::api::ApiContext>>()
        .map(|s| s.read().clone())
        .unwrap_or_else(|| crate::api::ApiContext {
            base_url: String::new(),
            id_token: String::new(),
        });

    // onpaste handler.
    let paste_session = session_id.clone();
    let paste_cx = api_cx.clone();
    let on_paste = move |_evt: Event<ClipboardData>| {
        if !can_draw {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let file = _evt
                .data()
                .downcast::<web_sys::ClipboardEvent>()
                .and_then(|ce| ce.clipboard_data())
                .and_then(|dt| imp_img::first_image_file(&dt));
            if let Some(file) = file {
                imp_img::upload_and_emit(
                    file,
                    paste_session.clone(),
                    paste_cx.clone(),
                    on_emit_image,
                    toast,
                    board,
                    redo_stack,
                );
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (&paste_session, &paste_cx);
    };

    // ondrop handler. dragover must preventDefault for drop to fire (handled in
    // the rsx `ondragover` below).
    let drop_session = session_id.clone();
    let drop_cx = api_cx.clone();
    let on_drop = move |_evt: Event<DragData>| {
        if !can_draw {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        {
            _evt.stop_propagation();
            let file = _evt
                .data()
                .downcast::<web_sys::DragEvent>()
                .and_then(|de| de.data_transfer())
                .and_then(|dt| imp_img::first_image_file(&dt));
            if let Some(file) = file {
                imp_img::upload_and_emit(
                    file,
                    drop_session.clone(),
                    drop_cx.clone(),
                    on_emit_image,
                    toast,
                    board,
                    redo_stack,
                );
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (&drop_session, &drop_cx);
    };

    // On the host target the paste/drop closures never touch these captures
    // (the upload path is wasm-only); keep the bindings used so SSR builds clean.
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (&session_id, &api_cx, &on_emit_image, &toast);

    rsx! {
        section { class: "live-room-whiteboard live-panel",
            div { class: "live-panel-header",
                h3 { class: "live-panel-title", "Whiteboard" }
            }
            if can_draw {
                div { class: "whiteboard-toolbar", role: "toolbar", "aria-label": "Whiteboard tools",
                    div { class: "whiteboard-toolbar-group", "aria-label": "Pen color",
                        for swatch in PEN_COLORS {
                            button {
                                r#type: "button",
                                class: if swatch == current_color && current_tool == WhiteboardTool::Pen {
                                    "whiteboard-swatch whiteboard-swatch--active"
                                } else {
                                    "whiteboard-swatch"
                                },
                                style: "--swatch-color: {swatch};",
                                title: "Pen color {swatch}",
                                onclick: move |_| {
                                    color.set(swatch.to_string());
                                    tool.set(WhiteboardTool::Pen);
                                },
                            }
                        }
                    }
                    div { class: "whiteboard-toolbar-group", "aria-label": "Pen width",
                        for (preset, label) in PEN_WIDTHS {
                            button {
                                r#type: "button",
                                class: if (preset - current_width).abs() < f32::EPSILON {
                                    "whiteboard-width whiteboard-width--active"
                                } else {
                                    "whiteboard-width"
                                },
                                title: "{label} stroke",
                                onclick: move |_| width.set(preset),
                                span {
                                    class: "whiteboard-width-dot",
                                    style: "--dot-size: {preset * 1.5}px;",
                                }
                            }
                        }
                    }
                    div { class: "whiteboard-toolbar-group", "aria-label": "Tool",
                        for (variant, label) in [
                            (WhiteboardTool::Pen, "Pen"),
                            (WhiteboardTool::Line, "Line"),
                            (WhiteboardTool::Rect, "Rect"),
                            (WhiteboardTool::Ellipse, "Ellipse"),
                            (WhiteboardTool::Arrow, "Arrow"),
                            (WhiteboardTool::Text, "Text"),
                            (WhiteboardTool::Eraser, "Eraser"),
                        ] {
                            {
                                let is_active = current_tool == variant;
                                let pick = variant.clone();
                                rsx! {
                                    button {
                                        r#type: "button",
                                        class: if is_active {
                                            "whiteboard-tool whiteboard-tool--active"
                                        } else {
                                            "whiteboard-tool"
                                        },
                                        title: "{label} tool",
                                        "aria-pressed": "{is_active}",
                                        onclick: move |_| tool.set(pick.clone()),
                                        "{label}"
                                    }
                                }
                            }
                        }
                    }
                    div { class: "whiteboard-toolbar-spacer" }
                    button {
                        r#type: "button",
                        class: "whiteboard-tool",
                        onclick: export_png,
                        title: "Download the board as a PNG",
                        "Export"
                    }
                    button {
                        r#type: "button",
                        class: "whiteboard-tool",
                        onclick: undo,
                        "Undo"
                    }
                    button {
                        r#type: "button",
                        class: "whiteboard-tool",
                        onclick: redo,
                        title: "Redo the last undone stroke",
                        "Redo"
                    }
                    if props.is_teacher {
                        button {
                            r#type: "button",
                            class: if draw_open {
                                "whiteboard-tool whiteboard-tool--active"
                            } else {
                                "whiteboard-tool"
                            },
                            "aria-pressed": "{draw_open}",
                            title: if draw_open {
                                "Students can draw — click to lock the board"
                            } else {
                                "Only you can draw — click to let students draw"
                            },
                            onclick: toggle_draw,
                            if draw_open { "Students: on" } else { "Students: off" }
                        }
                        button {
                            r#type: "button",
                            class: "whiteboard-clear",
                            onclick: clear,
                            "Clear"
                        }
                    }
                }
            }
            svg {
                id: WHITEBOARD_CANVAS_ID,
                class: "whiteboard-canvas",
                view_box: "0 0 1000 600",
                preserve_aspect_ratio: "none",
                role: "img",
                "aria-label": "Live whiteboard",
                onmounted: move |evt| {
                    spawn(async move {
                        if let Ok(rect) = evt.data().get_client_rect().await {
                            let (w, h) = (rect.size.width, rect.size.height);
                            if w > 1.0 && h > 1.0 {
                                canvas_size.set((w, h));
                            }
                        }
                    });
                },
                // Make the board focusable so a paste (Ctrl/Cmd-V) targets it.
                tabindex: if can_draw { "0" } else { "-1" },
                onpointerdown: start_stroke,
                onpointermove: extend_stroke,
                onpointerup: finish_stroke,
                onpointerleave: cancel_stroke,
                onpaste: on_paste,
                ondrop: on_drop,
                // A drop target must cancel the default dragover for `ondrop` to
                // fire; without this the browser navigates to the dropped file.
                ondragover: move |evt: Event<DragData>| evt.prevent_default(),
                for stroke in rendered_strokes.iter() {
                    {render_element(stroke)}
                }
                for cursor in remote_cursors.iter() {
                    {render_cursor(cursor)}
                }
            }
        }
    }
}

/// Render a single remote cursor as a small labeled dot in board (viewBox)
/// coordinates. The label sits to the upper-right of the dot; the color is
/// derived deterministically from the user id so the same user is the same
/// color for everyone. `key`ed on the user id so Dioxus diffs them stably.
fn render_cursor(cursor: &RemoteCursor) -> Element {
    let (cx, cy) = scaled(&WhiteboardPoint {
        x: cursor.x,
        y: cursor.y,
    });
    let color = cursor_color_for(&cursor.user_id);
    let uid = cursor.user_id.clone();
    // Trim absurdly long names so a crafted display_name can't blow out the
    // overlay; the wire already bounds these but defense in depth is cheap.
    let label: String = cursor.display_name.chars().take(24).collect();
    let label = if label.trim().is_empty() {
        "Guest".to_string()
    } else {
        label
    };
    let label_x = cx + 14.0;
    let label_y = cy - 8.0;
    rsx! {
        g { key: "{uid}", class: "whiteboard-cursor", "aria-hidden": "true",
            circle {
                cx: "{cx}", cy: "{cy}", r: "6",
                fill: "{color}",
                stroke: "#ffffff",
                stroke_width: "2",
            }
            text {
                class: "whiteboard-cursor-label",
                x: "{label_x}", y: "{label_y}",
                fill: "{color}",
                "{label}"
            }
        }
    }
}

// PNG-export bridge. Web delegates to the bundled `window.aula` helper. Native
// asks the WebView to rasterize the SVG, then passes the bounded PNG bytes to
// `platform_bridge::native_files` for a desktop save dialog or mobile private
// export/share-sheet adapter. Native export is therefore an installed-app path,
// not a synthetic WebView download.
#[cfg(target_arch = "wasm32")]
mod imp {
    use wasm_bindgen::{JsCast, JsValue};

    fn aula_obj() -> Option<js_sys::Object> {
        let win = web_sys::window()?;
        let aula = js_sys::Reflect::get(&win, &JsValue::from_str("aula")).ok()?;
        if aula.is_undefined() || aula.is_null() {
            return None;
        }
        aula.dyn_into::<js_sys::Object>().ok()
    }

    /// Ask the JS bridge to export the SVG with DOM id `svg_id` as a PNG
    /// download. Silently degrades to a no-op if the bridge isn't present.
    pub fn export_whiteboard_png(svg_id: &str) {
        let Some(obj) = aula_obj() else {
            return;
        };
        let Ok(func) = js_sys::Reflect::get(&obj, &JsValue::from_str("exportWhiteboardPng")) else {
            return;
        };
        let Ok(func) = func.dyn_into::<js_sys::Function>() else {
            return;
        };
        let args = js_sys::Array::new();
        args.push(&JsValue::from_str(svg_id));
        let _ = js_sys::Reflect::apply(&func, &obj, &args);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    pub fn export_whiteboard_png(svg_id: &str) {
        let element_id = svg_id.to_string();
        dioxus::prelude::spawn(async move {
            if let Err(error) = crate::live_room_native::export_whiteboard(&element_id).await {
                tracing::warn!(%error, "native whiteboard export failed");
            }
        });
    }
}

// Image paste/drop upload bridge (wasm only). Extracts an image `File` from a
// clipboard/drag `DataTransfer`, uploads it via the existing presigned-upload
// path (`upload_begin` → PUT to the presigned URL → `upload_complete`), then
// emits a `WhiteboardKind::Image` stroke referencing the asset id. Mirrors
// `file_picker`'s XHR-PUT flow but inlined here so the whiteboard owns its own
// small upload routine (no cross-module private coupling).
#[cfg(target_arch = "wasm32")]
mod imp_img {
    use super::*;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;

    /// First image `File` in a `DataTransfer`'s file list (paste or drop), or
    /// `None` if there is no image among the transferred files.
    pub fn first_image_file(dt: &web_sys::DataTransfer) -> Option<web_sys::File> {
        let files = dt.files()?;
        for i in 0..files.length() {
            if let Some(file) = files.get(i) {
                if WHITEBOARD_IMAGE_TYPES.contains(&file.type_().as_str()) {
                    return Some(file);
                }
            }
        }
        None
    }

    /// Kick off the upload-then-emit flow on the local task queue. Validates
    /// type/size client-side first, surfaces failures as a toast, and on success
    /// emits the image stroke through `on_emit` (the parent broadcasts it; the
    /// server echo lands it on every board, including ours). Adding a fresh
    /// element clears the redo stack, matching the draw/finish path.
    pub fn upload_and_emit(
        file: web_sys::File,
        session_id: String,
        cx: crate::api::ApiContext,
        on_emit: EventHandler<WhiteboardStroke>,
        mut toast: design_system::ToastSender,
        mut board: Signal<WhiteboardState>,
        mut redo_stack: Signal<Vec<WhiteboardStroke>>,
    ) {
        if session_id.trim().is_empty() {
            return;
        }
        let content_type = file.type_();
        let size_bytes = file.size() as i64;
        if !WHITEBOARD_IMAGE_TYPES.contains(&content_type.as_str()) {
            toast.push(
                design_system::ToastLevel::Danger,
                "Unsupported image",
                "Paste or drop a PNG, JPEG, or WebP image.",
            );
            return;
        }
        if size_bytes > WHITEBOARD_IMAGE_MAX_BYTES {
            toast.push(
                design_system::ToastLevel::Danger,
                "Image too large",
                "Whiteboard images must be 8 MB or smaller.",
            );
            return;
        }

        let filename = {
            let n = file.name();
            if n.trim().is_empty() {
                "pasted-image".to_string()
            } else {
                n
            }
        };

        wasm_bindgen_futures::spawn_local(async move {
            let begin_body = crate::api::UploadBeginBody {
                filename: &filename,
                content_type: &content_type,
                size_bytes,
                linked_entity_type: "live_session",
                linked_entity_id: session_id.clone(),
                purpose: "whiteboard",
            };
            let begin = match crate::api::upload_begin(&cx, &begin_body).await {
                Ok(r) => r,
                Err(e) => {
                    toast.push(
                        design_system::ToastLevel::Danger,
                        "Image upload failed",
                        e.to_string(),
                    );
                    return;
                }
            };
            if let Err(msg) = put_via_xhr(&begin.presigned_put_url, &content_type, &file).await {
                toast.push(
                    design_system::ToastLevel::Danger,
                    "Image upload failed",
                    msg,
                );
                return;
            }
            if let Err(e) = crate::api::upload_complete(&cx, &begin.asset_id).await {
                toast.push(
                    design_system::ToastLevel::Danger,
                    "Image upload failed",
                    e.to_string(),
                );
                return;
            }
            // Add locally for an instant preview, clear redo (a fresh edit), and
            // emit so the server records + fans it out to everyone.
            let stroke = image_stroke(begin.asset_id, default_image_bounds());
            let appended = board.with_mut(|state| append_stroke(state, stroke.clone()).is_ok());
            if appended {
                redo_stack.with_mut(Vec::clear);
                on_emit.call(stroke);
            }
        });
    }

    /// PUT a `File` to a presigned URL, resolving once the request completes.
    /// Mirrors `file_picker::upload_via_xhr` (sans progress reporting).
    async fn put_via_xhr(
        url: &str,
        content_type: &str,
        file: &web_sys::File,
    ) -> Result<(), String> {
        let xhr = web_sys::XmlHttpRequest::new().map_err(|e| format!("xhr init: {e:?}"))?;
        xhr.open("PUT", url)
            .map_err(|e| format!("xhr open: {e:?}"))?;
        xhr.set_request_header("Content-Type", content_type)
            .map_err(|e| format!("xhr header: {e:?}"))?;

        let (tx, rx) = futures_channel::oneshot::channel::<Result<(), String>>();
        let tx_load = std::cell::RefCell::new(Some(tx));
        let load_cb = Closure::<dyn FnMut(web_sys::Event)>::new({
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
        drop(load_cb);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_stroke() -> WhiteboardStroke {
        WhiteboardStroke {
            id: "stroke-1".to_string(),
            points: vec![
                WhiteboardPoint { x: 0.1, y: 0.2 },
                WhiteboardPoint { x: 0.3, y: 0.4 },
            ],
            color: "#111827".to_string(),
            width: 4.0,
            tool: WhiteboardTool::Pen,
            kind: WhiteboardKind::Freehand,
            text: None,
            asset_id: None,
            author: None,
        }
    }

    fn shape_stroke(kind: WhiteboardKind, tool: WhiteboardTool) -> WhiteboardStroke {
        WhiteboardStroke {
            id: "shape-1".to_string(),
            points: vec![
                WhiteboardPoint { x: 0.1, y: 0.2 },
                WhiteboardPoint { x: 0.5, y: 0.6 },
            ],
            color: "#111827".to_string(),
            width: 4.0,
            tool,
            kind,
            text: None,
            asset_id: None,
            author: None,
        }
    }

    #[test]
    fn live_room_whiteboard_validates_normal_stroke() {
        assert_eq!(validate_stroke(&sample_stroke()), Ok(()));
    }

    #[test]
    fn live_room_whiteboard_accepts_short_and_alpha_hex_colors() {
        let mut short = sample_stroke();
        short.color = "#abc".to_string();
        assert_eq!(validate_stroke(&short), Ok(()));

        let mut alpha = sample_stroke();
        alpha.color = "#111827cc".to_string();
        assert_eq!(validate_stroke(&alpha), Ok(()));
    }

    #[test]
    fn live_room_whiteboard_rejects_invalid_stroke_fields() {
        let mut stroke = sample_stroke();
        stroke.id = " ".to_string();
        assert_eq!(
            validate_stroke(&stroke),
            Err("stroke id is invalid".to_string())
        );

        let mut stroke = sample_stroke();
        stroke.points = vec![WhiteboardPoint { x: 0.1, y: 0.2 }];
        assert_eq!(
            validate_stroke(&stroke),
            Err("stroke must contain at least two points".to_string())
        );

        let mut stroke = sample_stroke();
        stroke.color = "#zzzzzz".to_string();
        assert_eq!(
            validate_stroke(&stroke),
            Err("stroke color is invalid".to_string())
        );

        let mut stroke = sample_stroke();
        stroke.width = 0.5;
        assert_eq!(
            validate_stroke(&stroke),
            Err("stroke width is invalid".to_string())
        );

        let mut stroke = sample_stroke();
        stroke.points[0].x = 1.1;
        assert_eq!(
            validate_stroke(&stroke),
            Err("stroke point is outside the board".to_string())
        );
    }

    #[test]
    fn live_room_whiteboard_append_rejects_stroke_cap() {
        let mut state = WhiteboardState::default();
        for i in 0..MAX_STROKES {
            let mut stroke = sample_stroke();
            stroke.id = format!("stroke-{i}");
            append_stroke(&mut state, stroke).unwrap();
        }

        let err = append_stroke(&mut state, sample_stroke()).unwrap_err();
        assert_eq!(err, "clear the board before adding more strokes");
        assert_eq!(state.strokes.len(), MAX_STROKES);
        assert_eq!(state.strokes.first().unwrap().id, "stroke-0");
    }

    #[test]
    fn live_room_whiteboard_finish_active_stroke_adds_release_point() {
        let mut state = WhiteboardState {
            active_stroke: Some(WhiteboardStroke {
                id: "active".to_string(),
                points: vec![WhiteboardPoint { x: 0.1, y: 0.2 }],
                color: "#111827".to_string(),
                width: 4.0,
                tool: WhiteboardTool::Pen,
                kind: WhiteboardKind::Freehand,
                text: None,
                asset_id: None,
                author: None,
            }),
            ..WhiteboardState::default()
        };

        let emitted = finish_active_stroke(&mut state, WhiteboardPoint { x: 0.3, y: 0.4 })
            .expect("active stroke should finalize");

        assert_eq!(emitted.points.len(), 2);
        assert_eq!(state.strokes.len(), 1);
        assert!(state.active_stroke.is_none());
    }

    #[test]
    fn live_room_whiteboard_finish_shape_keeps_two_anchor_points() {
        // A dragged shape accumulates many intermediate move samples; the
        // finalizer must collapse them to the start + release anchors.
        let mut state = WhiteboardState {
            active_stroke: Some(WhiteboardStroke {
                id: "rect".to_string(),
                points: vec![
                    WhiteboardPoint { x: 0.1, y: 0.1 },
                    WhiteboardPoint { x: 0.2, y: 0.2 },
                    WhiteboardPoint { x: 0.3, y: 0.3 },
                ],
                color: "#111827".to_string(),
                width: 4.0,
                tool: WhiteboardTool::Rect,
                kind: WhiteboardKind::Rect,
                text: None,
                asset_id: None,
                author: None,
            }),
            ..WhiteboardState::default()
        };

        let emitted = finish_active_stroke(&mut state, WhiteboardPoint { x: 0.6, y: 0.7 })
            .expect("active shape should finalize");

        assert_eq!(emitted.points.len(), 2);
        assert_eq!(emitted.points[0], WhiteboardPoint { x: 0.1, y: 0.1 });
        assert_eq!(emitted.points[1], WhiteboardPoint { x: 0.6, y: 0.7 });
        assert_eq!(state.strokes.len(), 1);
    }

    #[test]
    fn live_room_whiteboard_syncs_when_incoming_state_changes() {
        let mut local = WhiteboardState::default();
        let mut last_props = WhiteboardState::default();
        let incoming = WhiteboardState {
            strokes: vec![sample_stroke()],
            ..WhiteboardState::default()
        };

        sync_board_from_props(&mut local, &mut last_props, &incoming);

        assert_eq!(local, incoming);
        assert_eq!(last_props, incoming);
    }

    #[test]
    fn live_room_whiteboard_remove_stroke_drops_only_the_matching_id() {
        let mut state = WhiteboardState::default();
        let mut a = sample_stroke();
        a.id = "a".into();
        let mut b = sample_stroke();
        b.id = "b".into();
        append_stroke(&mut state, a).unwrap();
        append_stroke(&mut state, b).unwrap();

        remove_stroke(&mut state, "a");

        assert_eq!(state.strokes.len(), 1);
        assert_eq!(state.strokes[0].id, "b");
        // Removing an unknown id is a no-op, never a panic.
        remove_stroke(&mut state, "missing");
        assert_eq!(state.strokes.len(), 1);
    }

    #[test]
    fn live_room_whiteboard_snapshot_replaces_board_and_filters_invalid() {
        let mut state = WhiteboardState {
            strokes: vec![sample_stroke()],
            active_stroke: Some(sample_stroke()),
            ..WhiteboardState::default()
        };
        let mut good = sample_stroke();
        good.id = "good".into();
        let mut bad = sample_stroke();
        bad.id = "bad".into();
        bad.color = "#zzz".into();

        apply_snapshot(&mut state, vec![good, bad]);

        assert_eq!(state.strokes.len(), 1);
        assert_eq!(state.strokes[0].id, "good");
        assert!(state.active_stroke.is_none());
    }

    #[test]
    fn live_room_whiteboard_clear_resets_strokes_and_active_stroke() {
        let mut state = WhiteboardState {
            strokes: vec![sample_stroke()],
            active_stroke: Some(sample_stroke()),
            ..WhiteboardState::default()
        };

        clear_board(&mut state);

        assert!(state.strokes.is_empty());
        assert!(state.active_stroke.is_none());
    }

    #[test]
    fn legacy_pen_stroke_json_defaults_kind_to_freehand() {
        // Backward-compat: a stroke serialized before the `kind`/`text` fields
        // existed must still deserialize, defaulting to freehand.
        let raw = r##"{"id":"s1","points":[{"x":0.1,"y":0.2},{"x":0.3,"y":0.4}],"color":"#111827","width":4.0,"tool":"pen"}"##;
        let stroke: WhiteboardStroke = serde_json::from_str(raw).unwrap();
        assert_eq!(stroke.kind, WhiteboardKind::Freehand);
        assert_eq!(stroke.text, None);
        assert_eq!(validate_stroke(&stroke), Ok(()));
    }

    #[test]
    fn shape_stroke_round_trips_json_and_validates() {
        for (kind, tool) in [
            (WhiteboardKind::Line, WhiteboardTool::Line),
            (WhiteboardKind::Rect, WhiteboardTool::Rect),
            (WhiteboardKind::Ellipse, WhiteboardTool::Ellipse),
            (WhiteboardKind::Arrow, WhiteboardTool::Arrow),
        ] {
            let stroke = shape_stroke(kind.clone(), tool);
            assert_eq!(validate_stroke(&stroke), Ok(()), "kind {kind:?}");
            let s = serde_json::to_string(&stroke).unwrap();
            let d: WhiteboardStroke = serde_json::from_str(&s).unwrap();
            assert_eq!(d.kind, kind);
        }
    }

    #[test]
    fn shape_with_one_point_fails_validation() {
        let mut stroke = shape_stroke(WhiteboardKind::Line, WhiteboardTool::Line);
        stroke.points.truncate(1);
        assert_eq!(
            validate_stroke(&stroke),
            Err("shape must contain exactly two points".to_string())
        );
    }

    #[test]
    fn text_element_validates_body_length_and_presence() {
        let mut stroke = WhiteboardStroke {
            id: "t1".into(),
            points: vec![WhiteboardPoint { x: 0.2, y: 0.3 }],
            color: "#111827".into(),
            width: 4.0,
            tool: WhiteboardTool::Text,
            kind: WhiteboardKind::Text,
            text: None,
            asset_id: None,
            author: None,
        };
        assert_eq!(
            validate_stroke(&stroke),
            Err("text element requires a body".to_string())
        );

        stroke.text = Some("  ".into());
        assert_eq!(validate_stroke(&stroke), Err("text is empty".to_string()));

        stroke.text = Some("x".repeat(MAX_WHITEBOARD_TEXT_LEN + 1));
        assert_eq!(
            validate_stroke(&stroke),
            Err("text is too long".to_string())
        );

        stroke.text = Some("Hello".into());
        assert_eq!(validate_stroke(&stroke), Ok(()));

        // Text round-trips with its body.
        let s = serde_json::to_string(&stroke).unwrap();
        assert!(s.contains("\"kind\":\"text\""));
        let d: WhiteboardStroke = serde_json::from_str(&s).unwrap();
        assert_eq!(d.text.as_deref(), Some("Hello"));
    }

    #[test]
    fn elements_hit_at_finds_only_elements_under_the_eraser() {
        let mut state = WhiteboardState::default();
        // A diagonal freehand stroke through the board middle.
        let mut near = sample_stroke();
        near.id = "near".into();
        near.points = vec![
            WhiteboardPoint { x: 0.4, y: 0.4 },
            WhiteboardPoint { x: 0.6, y: 0.6 },
        ];
        // A far-away stroke in the corner.
        let mut far = sample_stroke();
        far.id = "far".into();
        far.points = vec![
            WhiteboardPoint { x: 0.9, y: 0.05 },
            WhiteboardPoint { x: 0.95, y: 0.1 },
        ];
        append_stroke(&mut state, near).unwrap();
        append_stroke(&mut state, far).unwrap();

        // Eraser pressed right on the diagonal hits only "near".
        let hits = elements_hit_at(&state, &WhiteboardPoint { x: 0.5, y: 0.5 });
        assert_eq!(hits, vec!["near".to_string()]);

        // Eraser pressed in empty space hits nothing.
        let hits = elements_hit_at(&state, &WhiteboardPoint { x: 0.1, y: 0.9 });
        assert!(hits.is_empty());
    }

    #[test]
    fn point_segment_dist_handles_degenerate_segment() {
        let p = WhiteboardPoint { x: 0.0, y: 0.0 };
        let a = WhiteboardPoint { x: 0.3, y: 0.4 };
        // a == b → distance is just |p - a| = 0.5 → squared 0.25.
        let d = point_segment_dist_sq(&p, &a, &a);
        assert!((d - 0.25).abs() < 1e-5, "got {d}");
    }

    #[test]
    fn render_element_emits_rect_primitive_via_component() {
        // SSR-render the teacher board so the shape renderer path runs end to
        // end. (Mirrors the `VirtualDom::new(app)` SSR pattern used elsewhere
        // in this crate.)
        fn app() -> Element {
            let mut state = WhiteboardState::default();
            append_stroke(
                &mut state,
                shape_stroke(WhiteboardKind::Rect, WhiteboardTool::Rect),
            )
            .unwrap();
            rsx! {
                LiveRoomWhiteboard {
                    state: state,
                    is_teacher: true,
                    on_emit_stroke: EventHandler::new(|_| {}),
                    on_clear: EventHandler::new(|_| {}),
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("<rect"), "rect markup missing: {html}");
        assert!(
            html.contains("whiteboard-shape"),
            "shape class missing: {html}"
        );
    }

    #[test]
    fn render_element_emits_text_primitive_via_component() {
        fn app() -> Element {
            let mut state = WhiteboardState::default();
            append_stroke(
                &mut state,
                WhiteboardStroke {
                    id: "txt".into(),
                    points: vec![WhiteboardPoint { x: 0.2, y: 0.3 }],
                    color: "#111827".into(),
                    width: 4.0,
                    tool: WhiteboardTool::Text,
                    kind: WhiteboardKind::Text,
                    text: Some("Geometry".into()),
                    asset_id: None,
                    author: None,
                },
            )
            .unwrap();
            rsx! {
                LiveRoomWhiteboard {
                    state: state,
                    is_teacher: false,
                    on_emit_stroke: EventHandler::new(|_| {}),
                    on_clear: EventHandler::new(|_| {}),
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("<text"), "text markup missing: {html}");
        assert!(html.contains("Geometry"), "text body missing: {html}");
    }

    #[test]
    fn cursor_color_is_deterministic_and_from_palette() {
        // Same id → same color across calls.
        let a1 = cursor_color_for("user-abc");
        let a2 = cursor_color_for("user-abc");
        assert_eq!(a1, a2);
        // It's one of the known palette stops.
        const PALETTE: [&str; 8] = [
            "#b8324a", "#2d5f88", "#2f6b45", "#b08842", "#6b4ea0", "#1f7a8c", "#c2570c", "#8a2d4d",
        ];
        assert!(PALETTE.contains(&a1));
        // An empty id still yields a stable palette color (no panic / OOB).
        assert!(PALETTE.contains(&cursor_color_for("")));
    }

    #[test]
    fn apply_cursor_event_upserts_clamps_and_caps() {
        let mut cursors = Vec::new();
        // Insert, then update the same user in place (no duplicate).
        apply_cursor_event(&mut cursors, "u1".into(), "Ada".into(), 0.5, 0.5);
        apply_cursor_event(&mut cursors, "u1".into(), "Ada B".into(), 0.7, 0.2);
        assert_eq!(cursors.len(), 1);
        assert_eq!(cursors[0].display_name, "Ada B");
        assert!((cursors[0].x - 0.7).abs() < 1e-6);

        // Out-of-range coords clamp into 0..=1.
        apply_cursor_event(&mut cursors, "u2".into(), "Bo".into(), 2.0, -1.0);
        let u2 = cursors.iter().find(|c| c.user_id == "u2").unwrap();
        assert_eq!(u2.x, 1.0);
        assert_eq!(u2.y, 0.0);

        // Cap: filling past MAX evicts the oldest.
        let mut many = Vec::new();
        for i in 0..(MAX_REMOTE_CURSORS + 5) {
            apply_cursor_event(&mut many, format!("u{i}"), "x".into(), 0.1, 0.1);
        }
        assert_eq!(many.len(), MAX_REMOTE_CURSORS);
        // The first few ids were evicted.
        assert!(many.iter().all(|c| c.user_id != "u0"));
    }

    #[test]
    fn last_stroke_index_for_author_picks_users_own_or_unstamped() {
        let mk = |id: &str, author: Option<&str>| WhiteboardStroke {
            author: author.map(|a| a.to_string()),
            ..{
                let mut s = sample_stroke();
                s.id = id.to_string();
                s
            }
        };
        let strokes = vec![
            mk("a1", Some("alice")),
            mk("b1", Some("bob")),
            mk("a2", Some("alice")),
            mk("local", None), // not-yet-echoed local stroke
        ];
        // Alice: most recent matching hers OR the unstamped local one — the
        // unstamped "local" is last, so it wins (we treat None as "mine").
        assert_eq!(last_stroke_index_for_author(&strokes, "alice"), Some(3));
        // Bob: his only stroke is "b1" (index 1); the unstamped local also
        // counts as "mine" for whoever asks, and it's later → index 3.
        assert_eq!(last_stroke_index_for_author(&strokes, "bob"), Some(3));
        // Empty id → board's most-recent stroke.
        assert_eq!(last_stroke_index_for_author(&strokes, ""), Some(3));
        // Empty board → None.
        assert_eq!(last_stroke_index_for_author(&[], "alice"), None);

        // A board with only other authors' (stamped) strokes and no unstamped
        // ones: a stranger finds nothing of their own.
        let others = vec![mk("b1", Some("bob")), mk("c1", Some("carol"))];
        assert_eq!(last_stroke_index_for_author(&others, "alice"), None);
    }

    #[test]
    fn remove_cursor_drops_matching_id_only() {
        let mut cursors = vec![
            RemoteCursor {
                user_id: "a".into(),
                display_name: "A".into(),
                x: 0.1,
                y: 0.1,
            },
            RemoteCursor {
                user_id: "b".into(),
                display_name: "B".into(),
                x: 0.2,
                y: 0.2,
            },
        ];
        remove_cursor(&mut cursors, "a");
        assert_eq!(cursors.len(), 1);
        assert_eq!(cursors[0].user_id, "b");
        remove_cursor(&mut cursors, "missing"); // no-op
        assert_eq!(cursors.len(), 1);
    }

    #[test]
    fn cursor_throttle_predicate_respects_gap() {
        // Below the gap → suppress.
        assert!(!should_emit_cursor(1000.0, 1000.0));
        assert!(!should_emit_cursor(
            1000.0 + CURSOR_THROTTLE_MS - 1.0,
            1000.0
        ));
        // At/above the gap → emit.
        assert!(should_emit_cursor(1000.0 + CURSOR_THROTTLE_MS, 1000.0));
        assert!(should_emit_cursor(5000.0, 1000.0));
    }

    #[test]
    fn render_cursor_emits_dot_and_label_via_component() {
        // SSR-render a student board with a remote cursor in props; the overlay
        // must render the labeled dot and skip the local user's own cursor.
        fn app() -> Element {
            rsx! {
                LiveRoomWhiteboard {
                    state: WhiteboardState::default(),
                    is_teacher: false,
                    on_emit_stroke: EventHandler::new(|_| {}),
                    on_clear: EventHandler::new(|_| {}),
                    local_user_id: "me".to_string(),
                    cursors: vec![
                        RemoteCursor {
                            user_id: "me".to_string(),
                            display_name: "Should be hidden".to_string(),
                            x: 0.5,
                            y: 0.5,
                        },
                        RemoteCursor {
                            user_id: "other".to_string(),
                            display_name: "Ada".to_string(),
                            x: 0.5,
                            y: 0.5,
                        },
                    ],
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("whiteboard-cursor"),
            "cursor markup missing: {html}"
        );
        assert!(html.contains("Ada"), "remote label missing: {html}");
        assert!(
            !html.contains("Should be hidden"),
            "local user's own cursor must not render: {html}"
        );
    }

    #[test]
    fn empty_display_name_cursor_falls_back_to_guest() {
        fn app() -> Element {
            rsx! {
                LiveRoomWhiteboard {
                    state: WhiteboardState::default(),
                    is_teacher: false,
                    on_emit_stroke: EventHandler::new(|_| {}),
                    on_clear: EventHandler::new(|_| {}),
                    local_user_id: "me".to_string(),
                    cursors: vec![RemoteCursor {
                        user_id: "u1".to_string(),
                        display_name: "   ".to_string(),
                        x: 0.1,
                        y: 0.1,
                    }],
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("Guest"),
            "blank name should fall back to Guest: {html}"
        );
    }

    #[test]
    fn user_id_from_jwt_reads_sub_claim() {
        // A JWT is header.payload.signature; only the payload matters here. The
        // payload below is base64url(no-pad) of {"sub":"user-42","aud":"x"}.
        // header / signature are arbitrary non-empty segments.
        fn b64url_nopad(bytes: &[u8]) -> String {
            const ALPHABET: &[u8; 64] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
            let mut out = String::new();
            for chunk in bytes.chunks(3) {
                let b0 = chunk[0] as u32;
                let b1 = *chunk.get(1).unwrap_or(&0) as u32;
                let b2 = *chunk.get(2).unwrap_or(&0) as u32;
                let n = (b0 << 16) | (b1 << 8) | b2;
                out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
                out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
                if chunk.len() > 1 {
                    out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
                }
                if chunk.len() > 2 {
                    out.push(ALPHABET[(n & 63) as usize] as char);
                }
            }
            out
        }
        let payload = b64url_nopad(br#"{"sub":"user-42","aud":"x"}"#);
        let token = format!("aGVhZGVy.{payload}.c2ln");
        assert_eq!(user_id_from_jwt(&token).as_deref(), Some("user-42"));

        // Round-trips through the per-author undo selector: a stroke stamped
        // with this id is selected for that user.
        let uid = user_id_from_jwt(&token).unwrap();
        let mut mine = sample_stroke();
        mine.author = Some(uid.clone());
        let strokes = vec![mine];
        assert_eq!(last_stroke_index_for_author(&strokes, &uid), Some(0));
    }

    #[test]
    fn user_id_from_jwt_rejects_malformed_tokens() {
        // Not three segments / no payload.
        assert_eq!(user_id_from_jwt(""), None);
        assert_eq!(user_id_from_jwt("onlyonepart"), None);
        // Payload is not valid base64url.
        assert_eq!(user_id_from_jwt("h.!!!!.s"), None);
        // Payload decodes but is not JSON with a `sub`.
        // base64url(no-pad) of {"x":1} -> "eyJ4IjoxfQ".
        assert_eq!(user_id_from_jwt("h.eyJ4IjoxfQ.s"), None);
    }

    #[test]
    fn validate_stroke_image_requires_two_points_and_asset_id() {
        let mut stroke = sample_stroke();
        stroke.kind = WhiteboardKind::Image;

        // No asset id → rejected.
        stroke.asset_id = None;
        assert_eq!(
            validate_stroke(&stroke),
            Err("image element requires an asset id".to_string())
        );

        // Asset id present, wrong point count → rejected.
        stroke.asset_id = Some("a".into());
        stroke.points = vec![WhiteboardPoint { x: 0.1, y: 0.2 }];
        assert_eq!(
            validate_stroke(&stroke),
            Err("image must contain exactly two points".to_string())
        );

        // Valid image element built by the helper.
        let img = image_stroke("asset-1".into(), default_image_bounds());
        assert_eq!(img.kind, WhiteboardKind::Image);
        assert_eq!(img.points.len(), 2);
        assert_eq!(img.asset_id.as_deref(), Some("asset-1"));
        assert_eq!(validate_stroke(&img), Ok(()));
    }

    #[test]
    fn default_image_bounds_are_in_range_and_two_points() {
        let bounds = default_image_bounds();
        assert_eq!(bounds.len(), 2);
        for p in &bounds {
            assert!((0.0..=1.0).contains(&p.x));
            assert!((0.0..=1.0).contains(&p.y));
        }
        // Top-left strictly above-left of bottom-right (a non-degenerate box).
        assert!(bounds[0].x < bounds[1].x);
        assert!(bounds[0].y < bounds[1].y);
    }

    #[test]
    fn render_element_emits_image_placeholder_when_url_unresolved() {
        // SSR has no async URL resolution, so the image element renders its
        // dashed placeholder rect (the load/error fallback). This exercises the
        // Image render arm end to end without a live presigned URL.
        fn app() -> Element {
            let mut state = WhiteboardState::default();
            append_stroke(
                &mut state,
                image_stroke("asset-1".into(), default_image_bounds()),
            )
            .unwrap();
            rsx! {
                LiveRoomWhiteboard {
                    state: state,
                    is_teacher: true,
                    on_emit_stroke: EventHandler::new(|_| {}),
                    on_clear: EventHandler::new(|_| {}),
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(
            html.contains("whiteboard-image"),
            "image element markup missing: {html}"
        );
    }
}
