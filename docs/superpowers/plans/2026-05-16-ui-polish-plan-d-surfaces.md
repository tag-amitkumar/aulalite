# UI Polish — Plan D: Surfaces Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Apply Plan A tokens, Plan B primitives, and Plan C patterns to the actual user-facing surfaces of AulaLite. This is the user-visible payoff phase — dashboards, course pages, the live room, auth screens all gain consistent state coverage, refined typography, two-tier density, and polished empty/loading/error states. Backward-compatible at the route boundary — URLs and behaviour preserved.

**Architecture:** In-place rewrites of surface render functions in `crates/features-courses/src/`, `crates/features-auth/src/`, and `crates/shell-web/src/routes/`. Each surface task replaces raw HTML/CSS with design-system components (`Button`, `Field`, `Card`, `Modal`, `Tabs`, `Badge`, `EmptyState`, `Loading`, `Skeleton`, `PageHeader`, `Table`, `CardList`, `Toast`). Page-level CSS that was hand-rolled in surfaces gets consolidated into pattern classes from Plan C.

**Tech Stack:** Rust + Dioxus 0.7, design-system primitives + composites from Plans A–C.

**Vision spec:** `docs/superpowers/specs/2026-05-16-ui-polish-vision-design.md`.
**Plans:** Foundation (`-a-foundation.md`, executed), Primitives (`-b-primitives.md`, executed), Patterns (`-c-patterns.md`, executed).

---

## Conventions (referenced by every task)

### Density application

Each surface explicitly opts into the **roomy** or **compact** tier per vision spec decision 2:

| Tier | Surfaces |
|--|--|
| Roomy | Auth (login/signup/forgot/accept-invite/redeem), Dashboard, Course detail header, Live room hero / lobby / replay |
| Compact | Course builder, Schedule list, Submissions table, Live room sidebars (chat/presence/hand-raise), Assignment editor body |

Roomy density = default `--space-*` scale used as-is. Compact density = step down by one (e.g. where roomy uses `--space-4`, compact uses `--space-3`).

### Page Header adoption

Every page that currently renders an ad-hoc `<header><h1>...</h1></header>` switches to `PageHeader { title, kicker?, subtitle?, actions? }`. The page should NOT also render its own `<h1>` (avoid heading-level collision flagged in Plan C M1).

### Token-only invariant at surface level

Surface CSS that previously hard-coded hex/px values gets migrated to tokens. The audit step in Task 10 greps for violations after each pass.

### Component substitution map

| Hand-rolled element | Replace with |
|--|--|
| `<button class="btn-primary">` | `Button { variant: ButtonVariant::Primary, ... }` |
| `<input>` + label + helper div | `Field { label, helper, error, Input { ... } }` |
| Generic loading `<p>Loading…</p>` | `Loading { message }` |
| Ad-hoc empty-state divs | `EmptyState { title, description, illustration?, cta? }` |
| `<table>` | `Table { compact, sticky_header, head, body }` |
| Course/schedule/assignment cards in a grid | `CardList { columns?, ... }` |
| `dx-toast` (now removed) | `use_toast_sender().push(level, title, message)` |

### Mirror discipline

CSS edits in surfaces typically touch `components.css` only when consolidating patterns. Asset-sync still applies if any CSS changes happen.

### Backward compat

Route URLs, component prop signatures at the route boundary, and API calls preserved. Internal layout/markup may change freely.

---

## Task 0: Cleanup deferred follow-ups

**Files (depending on what we fold in):**
- Modify: `crates/design-system/src/toast.rs` (auto-dismiss timer).
- Modify: `crates/design-system/src/page_header.rs` (`as_tag` option for heading level).
- Modify: `crates/design-system/assets/components.css` (kicker color decision; `--card-list-columns` rename).
- Modify: `crates/shell-web/public/assets/components.css` (mirror).
- Modify: `crates/shell-web/src/routes/course_detail.rs` (OutlineTab error logging from quick-fix Task 4 follow-up).

### Sub-task 0a: Toast auto-dismiss timer

Add `wasm-bindgen` `setTimeout` plumbing to `ToastSender::push`. On push, schedule a future to call `dismiss(id)` after `duration_ms`. Use `web_sys::window().set_timeout_with_callback_and_timeout_and_arguments_0` and an `Arc<Closure>` to keep the callback alive. If `duration_ms` is `None`, skip the timer (sticky toast).

Pattern in `crates/design-system/src/toast.rs`:

```rust
impl ToastSender {
    pub fn push(&mut self, level: ToastLevel, title: impl Into<String>, message: impl Into<String>) {
        let id = next_toast_id();
        let duration_ms = 4000u32;
        self.0.write().push(ToastEntry {
            id,
            level,
            title: title.into(),
            message: message.into(),
            duration_ms: Some(duration_ms),
        });

        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::closure::Closure;
            use wasm_bindgen::JsCast;
            let mut sender = self.clone();
            let closure = Closure::once_into_js(move || {
                sender.dismiss(id);
            });
            if let Some(window) = web_sys::window() {
                let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                    closure.as_ref().unchecked_ref(),
                    duration_ms as i32,
                );
            }
        }
    }
}
```

Note: `wasm-bindgen` and `web-sys` aren't yet in `design-system`'s `Cargo.toml`. Add them with `wasm-bindgen = "0.2"` and `web-sys = { version = "0.3", features = ["Window"] }` gated to `target_arch = "wasm32"`. Use the same versions as `shell-web` (look at `crates/shell-web/Cargo.toml` for canonical versions).

### Sub-task 0b: PageHeader as_tag option

Add an optional `as_tag: HeadingLevel` prop to `PageHeader` so callers can render an `<h2>` instead of `<h1>` when the page already has a hero header elsewhere. Default stays `<h1>`.

```rust
#[derive(Clone, PartialEq, Default)]
pub enum HeadingLevel {
    #[default]
    H1,
    H2,
}
```

Switch the rendering:
```rust
match props.as_tag {
    HeadingLevel::H1 => rsx! { h1 { class: "ds-page-header-title", "{props.title}" } },
    HeadingLevel::H2 => rsx! { h2 { class: "ds-page-header-title", "{props.title}" } },
}
```

### Sub-task 0c: Kicker color decision

Change `.ds-page-header-kicker` CSS rule from `color: var(--color-accent);` to `color: var(--color-gold-700);` — gold reads as a decorative accent and pairs better with the warm-academy palette than the brand green.

### Sub-task 0d: Namespace `--card-list-columns`

Rename `--card-list-columns` to `--ds-card-list-columns` in `card_list.rs` and `components.css`. Update the test assertion accordingly.

### Sub-task 0e: OutlineTab error logging

In `crates/shell-web/src/routes/course_detail.rs::OutlineTab`, the six handlers that follow the `if api::...(...).await.is_ok() { outline.restart(); }` pattern silently drop errors. Replace with explicit `match` that logs via `tracing::warn!` on the `Err` arm:

```rust
match api::patch_module(&api, &course_id, &module_id, &body).await {
    Ok(_) => outline.restart(),
    Err(e) => tracing::warn!("rename module failed: {e}"),
}
```

(For wasm builds, `tracing` needs to be initialized to forward to `console.error`. The shell crate already does this; verify the import path.)

### Tests / verification

- Toast: add an SSR test asserting `duration_ms: Some(4000)` lands in the queued entry. The timer itself can't be unit-tested in SSR; manual smoke check after T1 lands.
- PageHeader: add `page_header_as_h2_renders_h2` test.
- CardList: update existing test asserting the new var name.
- OutlineTab: existing tests still pass.

### Commit (single commit, all sub-tasks)

```bash
git add -A
git commit -m "$(cat <<'EOF'
fix(plan-d-cleanup): close Plan C follow-ups + quick-fix error logging

- Toast push schedules auto-dismiss via wasm-bindgen setTimeout
  (sticky toasts still supported via duration_ms None).
- PageHeader gains as_tag prop (H1 default, H2 for nested heroes).
- Kicker color switched from --color-accent (green) to
  --color-gold-700 — better fit for the warm-academy palette.
- --card-list-columns renamed to --ds-card-list-columns for
  namespace consistency.
- OutlineTab rename handlers log errors via tracing::warn instead of
  silently swallowing on the .is_ok() branch.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 1: Auth screens (login, signup, forgot, accept-invite, redeem)

**Files:**
- Modify: `crates/features-auth/src/login.rs` (+ signup.rs + forgot.rs as they exist).
- Modify: `crates/features-courses/src/accept_invite.rs`.
- Modify: `crates/features-courses/src/redeem_code.rs`.

### Polish goals

- Wrap each form's inputs in `Field { label, helper, error }`.
- Replace raw `<button>` with `Button { variant: ButtonVariant::Primary, button_type: "submit" }`.
- Use `PageHeader { title, kicker, subtitle, variant: PageHeaderVariant::Hero }` for the login/signup hero.
- Loading state during submit: `Loading { message: "Signing in…" }`.
- Error display: top-level error becomes a `Toast` push (info: form-level; danger: server error) or inline `error` prop on the Field. Pick inline for client-side validation; toast for server errors.
- Accept-invite success state: use `EmptyState { variant: EmptyStateVariant::Accent, illustration: <checkmark>, ... }`.

### Steps (template — repeat per auth surface)

1. Read each existing file; identify the form structure.
2. Replace `<input type="email">` + label markup with `Field { label: "Email", helper: None, error: form_error.email, Input { value, on_input, error: form_error.email.is_some() } }`.
3. Replace submit button with `Button`.
4. If a "Loading…" `<p>` exists during submit, swap with `Loading`.
5. Final verification: form still submits; URLs `/login`, `/signup`, `/forgot`, `/accept-invite/:token`, `/redeem` all reachable.

### Tests

Existing route smoke tests in `crates/shell-web/tests/shell_routes_smoke.rs` must continue to pass (they assert on substring matches like `"email"` or `"Email"`). If a test asserts on a specific class name that no longer renders, update the assertion.

### Commit

```
feat(auth): polish login / signup / forgot / accept-invite / redeem surfaces
```

---

## Task 2: Dashboard (`/`)

**Files:**
- Modify: `crates/features-courses/src/dashboard.rs`.

### Polish goals

- Replace the existing ad-hoc `<header class="page-header dashboard-hero">` with `PageHeader { kicker: "Command center", title: "Welcome back, {display_name}", subtitle: "Courses, live sessions, …", variant: PageHeaderVariant::Hero }`.
- Replace dashboard `Card` blocks (Your Courses, Upcoming) with refined `Card { variant: CardVariant::Default }` consuming Plan C tokens.
- Course list inside dashboard: use `CardList { columns: 2 }` or fall back to the existing `course-list-mini` ul if it's preferred for density. Apply roomy density per convention.
- Empty state when no courses: `EmptyState { variant: EmptyStateVariant::Accent, title: "No courses yet", description: "...", cta: <Button label: "Browse catalog" ...> }`.
- Loading state during initial fetch (if the parent route shows one): `Loading { layout: LoadingLayout::Block }`.

### Tests

`crates/features-courses/src/dashboard.rs::tests` ssr tests should still pass — the asserted strings ("No courses yet", course title, role badge) stay in the rendered output.

### Commit

```
feat(dashboard): apply PageHeader + refined cards + accent empty state
```

---

## Task 3: Course list (`/courses`)

**Files:**
- Modify: `crates/features-courses/src/course_list.rs`.

### Polish goals

- `PageHeader { title: "Courses", kicker: "Catalog", actions: <Button label="New Course" variant=Primary> }`.
- Course cards rendered through `CardList { columns: None }` (defaults to auto-fit 260px min).
- Each card uses `Card { variant: CardVariant::Default, on_click: navigate-to-detail }`.
- Filter chips: `Tabs` or a horizontal chip row using `.chip` class.
- Empty state: `EmptyState`.
- Loading: `Loading`.

### Commit

```
feat(course-list): apply CardList + clickable cards + page header
```

---

## Task 4: Course detail (header + tabs + sub-tabs)

**Files:**
- Modify: `crates/features-courses/src/course_detail.rs`.
- Modify: `crates/shell-web/src/routes/course_detail.rs` (already lightly polished in quick-fix; tighten).

### Polish goals

- Course header: `PageHeader { kicker: "Course", title: "{course.title}", subtitle: course.description, actions: <admin Edit button> }`. Course-title edit affordance from quick-fix Task 6 stays in concept — fold it into the PageHeader's `actions` slot.
- Badge tones: `Badge { tone: BadgeTone::{Neutral|Success|Warning} }` based on course status (already wired; refine).
- Tabs: continue using `Tabs` primitive (already refined in Plan B).
- Hero is *roomy*; tab content is *compact* depending on tab (Outline/People/Edit are roomy; Schedule list within Course Schedule tab is compact).
- Sub-tabs (people, outline, edit, schedule) are owned by `OutlineTab`, `PeopleTab`, etc. in `crates/shell-web/src/routes/course_detail.rs`; polish each.

### Commit

```
feat(course-detail): page header + admin edit action + density tiers
```

---

## Task 5: Course builder

**Files:**
- Modify: `crates/features-courses/src/course_builder.rs`.

### Polish goals

- Module title: already uses `RenamableTitle` (quick-fix Task 4). Keep.
- Lesson title: same. Keep.
- "+ Module" button: `Button { variant: ButtonVariant::Primary, leading_icon: <plus icon> }`.
- "+ Add lesson": `Button { variant: ButtonVariant::Ghost, leading_icon: <plus icon> }` (was `linkish` button — `Ghost` reads better here).
- Empty state when no modules: existing `EmptyState`. Add accent variant: `variant: EmptyStateVariant::Accent`.
- Apply **compact** density to the builder list itself (tighter spacing on module / lesson rows).

### Commit

```
feat(course-builder): apply compact density + ghost buttons + accent empty state
```

---

## Task 6: Schedule view (`/schedule`) + course-schedule tab

**Files:**
- Modify: `crates/features-courses/src/schedule_view.rs`.
- Modify: `crates/features-courses/src/series_scheduler.rs` (for the course-schedule tab admin UI).

### Polish goals

- `PageHeader { title: "Schedule", kicker: "Live sessions" }` on `/schedule`.
- Schedule entries: use `CardList` for the grid OR a `Table` if data-dense (probably `CardList` at student density, `Table { compact: true }` for the admin course-schedule tab).
- Each schedule entry's status: `Badge` with `tone: BadgeTone::{Live|Success|Warning|Neutral}` based on `status` and `diverged` fields.
- Series scheduler form (admin-only inside course-schedule tab): wrap inputs in `Field`. Buttons → `Button`.
- Apply **compact** density inside the course-schedule tab table.

### Commit

```
feat(schedule): apply card list (student) + compact table (admin) + badges
```

---

## Task 7: Assignments (list + editor + detail + grading)

**Files:**
- Modify: `crates/features-courses/src/assignment_list.rs`.
- Modify: `crates/features-courses/src/assignment_editor.rs`.
- Modify: `crates/features-courses/src/assignment_detail.rs`.
- Modify: `crates/features-courses/src/submissions_grading_table.rs`.
- Modify: `crates/features-courses/src/submission_grade_modal.rs`.

### Polish goals

- Assignment list: `PageHeader { title: "Assignments" }` + `CardList` of assignments.
- Editor: `Field`-wrapped inputs throughout; `Button` for submit/draft; `Modal` for file-picker if it exists.
- Detail: `PageHeader { title: "{assignment.title}", subtitle: assignment.instructions_md (truncated) }`.
- Grading table: `Table { sticky_header: true, compact: true }`.
- Submission grade modal: refined via Plan B Modal sizes (Medium); inputs through Field.

### Commit

```
feat(assignments): list + editor + detail + grading surfaces polished
```

---

## Task 8: Live room (lobby / broadcast / in-session / replay / ended)

**Files:**
- Modify: `crates/features-courses/src/live_room_shell.rs`.
- Modify: `crates/features-courses/src/live_room_lobby.rs`.
- Modify: `crates/features-courses/src/live_room_broadcast.rs`.
- Modify: `crates/features-courses/src/live_room_view.rs`.
- Modify: `crates/features-courses/src/live_room_chat.rs`.
- Modify: `crates/features-courses/src/live_room_presence.rs`.
- Modify: `crates/features-courses/src/live_room_hand_raise.rs`.
- Modify: `crates/features-courses/src/live_room_replay.rs`.

### Polish goals

This is the most-visible surface to students. Hero moments are roomy; sidebars are compact.

- **Lobby:** `PageHeader { kicker: "Joining", title: "{course_title}", variant: Hero }` + `Loading { message: "Connecting…" }` + a primary CTA `Button` to Join when ready.
- **Broadcast (teacher):** `Card` for the broadcast surface; primary `Button` for Go Live; `Badge { tone: BadgeTone::Live }` (uses the pulse animation added in Plan B).
- **In-session (student view):** Two-column layout — video left (roomy), sidebars right (compact). Sidebars use the `chat / presence / hand-raise` patterns; each gets a `<aside>` wrapper.
- **Chat:** Replace ad-hoc input + button with `Field` + `Input` + `Button`. List of messages: keep current shape but use spacing tokens.
- **Hand-raise queue:** `Badge` for "pending", `Button` variants for Grant/Revoke.
- **Replay:** `PageHeader { title: "Replay: {course_title}" }`; video pane in a `Card`; transcript-style chat replay using the same chat component.
- **Ended state:** `EmptyState { variant: EmptyStateVariant::Subtle, title: "Class has ended", description: "...", cta: <Button label="Back to course"> }`.

### Tests

Live-room SSR tests in `live_room_smoke.rs` currently have one pre-existing failure (`replay_renders_loading_initially` panics for missing `ApiContext`). Task 8 should NOT attempt to fix that test — it's a harness issue logged in Plan A retro. Verify no NEW regressions.

### Commit

```
feat(live-room): apply density tiers + cards + badges + page headers across lobby, broadcast, in-session, replay, ended
```

---

## Task 9: Page transitions + global motion polish

**Files:**
- Modify: `crates/design-system/assets/components.css` (+ mirror).
- Possibly modify `crates/shell-web/src/lib.rs` or per-route components.

### Polish goals

- Every route's root element gets `class: "motion-page"` (or equivalent) so the page-in animation from Plan A fires on route change.
- Confirm the `.motion-page` animation works with the new Plan A motion tokens (`var(--duration-page-in) var(--ease-page-in)`).
- Add subtle `<main>`-level fade-in on first render for surfaces that don't already trigger `.motion-page`.
- Honor `prefers-reduced-motion` (existing rule in components.css from Plan A; verify).

### Commit

```
feat(motion): page transition consistency across all routes
```

---

## Task 10: Final verification + cross-task review

- [ ] **Step 1: Workspace tests** (`cargo test --workspace --no-fail-fast`).
- [ ] **Step 2: Format check** (`cargo fmt --all -- --check`).
- [ ] **Step 3: Wasm build** (`cargo build -p shell-web --target wasm32-unknown-unknown`).
- [ ] **Step 4: Asset-sync diff**.
- [ ] **Step 5: Token-only audit** — grep for raw hex / rem / px in all surface CSS blocks and primitive consumer files.
- [ ] **Step 6: Dispatch final cross-task code reviewer** over all Plan D commits — focus on consistency across surfaces (PageHeader convention, density tier adherence, primitive usage).
- [ ] **Step 7: Manual smoke test** — load dev server, walk every surface, confirm:
  - Typography renders Source Serif 4 + Inter.
  - Every page has a Page Header.
  - Every primary button is Primary; ghost buttons are Ghost.
  - Empty / loading / error states show with token-driven styling.
  - Toast push works (e.g. on a successful save).
  - Live room renders correctly through all 5 phases.
- [ ] **Step 8: Vision-spec success criteria check** — review the 6 criteria in `docs/superpowers/specs/2026-05-16-ui-polish-vision-design.md` ("Success Criteria" section) and document which are met.
- [ ] **Step 9: Lighthouse pass** — run Lighthouse on dashboard, course-detail, live-room. Confirm Accessibility ≥ 90 and Best Practices ≥ 90 per vision spec.
- [ ] **Step 10: Write closing retro** — a short `docs/superpowers/specs/2026-05-16-ui-polish-retro.md` summarizing what shipped, what was deferred, and known follow-ups.

---

## Spec coverage check

| Vision-spec requirement | Task |
|--|--|
| Auth surfaces | Task 1 |
| Dashboard | Task 2 |
| Course list | Task 3 |
| Course detail + tabs | Task 4 |
| Course builder | Task 5 |
| Schedule view | Task 6 |
| Assignments (editor/detail/grading) | Task 7 |
| Live room (5 phases) | Task 8 |
| Page transitions | Task 9 |
| Density tiers (roomy / compact) | Convention applied per task |
| Token-only audit | Task 10 Step 5 |
| Accessibility floor | Task 10 Step 9 |
| Plan C deferred items | Task 0 |

## Out of scope

- New product features. Polish only.
- Mobile shell (`crates/shell-mobile`) — stays unchanged.
- Desktop shell (`crates/shell-desktop`) — stays unchanged.
- I18n / RTL.
- Dark theme values (placeholder remains empty per vision spec decision 5).

## Risks

- **Plan D is the biggest plan.** ~11 tasks, each touching multiple files. Subagent execution may take several hours. Pause between high-impact tasks (1, 2, 4, 8) for visual review.
- **Live room (Task 8) touches 8+ files.** May reveal coupling issues that require splitting into sub-tasks during execution. If the implementer encounters too much scope, it's acceptable to scope-creep into a separate Task 8a (lobby+broadcast) and 8b (in-session+replay+ended).
- **SSR test assertions may break.** Many existing tests assert on specific class names or substring matches. Each task's verification step needs to handle test breakage gracefully — update the test if the new markup is the correct shape; don't blame the test if it's protecting a regression.
- **Toast auto-dismiss timer.** The `wasm-bindgen` plumbing in Task 0a is the riskiest single change in Plan D. If it doesn't compile cleanly under `target_arch = "wasm32"`, fall back to a simpler approach (e.g., a polling effect in `ToastViewport` that prunes entries whose age exceeds `duration_ms` using `js_sys::Date::now()`).
- **CSS bloat.** `components.css` is ~1800 lines after Plan C cleanup. Plan D may add ~200-400 lines. Splitting the file is a follow-up plan; flagged in Plan B retrospective.
