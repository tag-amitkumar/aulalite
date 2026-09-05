# AulaLite Editorial UI Refresh Design

**Date:** 2026-05-12
**Status:** Approved design, awaiting implementation plan

## Goal

Replace the current plain browser-default web presentation with an elegant,
functional editorial interface for the main AulaLite web flow. The refresh
should make the existing app feel like a premium learning workspace while
preserving the current routes, backend contracts, and feature behavior.

## Scope

The first pass covers the high-traffic web surface:

- Login and local testing.
- App shell, sidebar, and topbar.
- Dashboard.
- Course list.
- Course detail tabs and outline.
- Schedule views.
- Live session lobby, teacher broadcast view, student live view, and live-room sidebars.

Assignments and other forms inherit shared styling from tokens, buttons,
inputs, cards, tables, badges, empty states, and modal styles. Full assignment
editor polish is out of scope for this pass unless it falls out naturally from
shared CSS.

Out of scope:

- New product features.
- Backend or API behavior changes.
- Route changes.
- Major component rewrites.
- Separate mobile or desktop shell design.
- Custom visual mockup tooling.

## Visual Direction

Use a **Modern Academy** editorial theme:

- Warm paper background, white/off-white surfaces, ink text.
- Restrained deep green and navy accents.
- Refined serif display headings paired with a readable sans-serif body.
- Thin rules, subtle shadows, and calm status colors.
- Dense but readable layouts suited to repeated course-management work.

The app should feel editorial and premium without becoming decorative. It is a
learning operations tool, so scanability, spacing discipline, and clear actions
matter more than dramatic illustration.

## Approach

Use a CSS-first redesign with targeted markup cleanup.

Primary files:

- `crates/shell-web/public/assets/tokens.css`
- `crates/shell-web/public/assets/components.css`
- `crates/design-system/assets/tokens.css`
- `crates/design-system/assets/components.css`

Targeted Rust/Dioxus edits are allowed when CSS alone cannot produce the
desired result:

- Replace raw local-testing `card` and `btn-primary` classes with the existing
  design-system look or equivalent shell classes.
- Add wrapper/header classes where pages need styleable structure.
- Keep existing component boundaries for auth, app shell, dashboard, courses,
  schedule, and live room.

Avoid route-by-route visual rewrites that duplicate styling logic. Shared
tokens and shared component classes should carry most of the visual language.

## Layout System

Desktop uses a fixed-responsive application frame:

- Left sidebar around `260px`.
- Fluid main content area.
- Compact topbar.
- Main content constrained around `1180px` where appropriate.

Tablet and mobile collapse into a single-column content flow with navigation
that wraps or stacks instead of squeezing text.

Cards stay restrained:

- Radius no larger than `8px`.
- Thin borders.
- Subtle elevation.
- No nested card-heavy page sections.

Page headers follow an editorial pattern:

- Optional small section label.
- Large serif title.
- Short supporting line or action group.

## Page Treatments

### Login

The first screen becomes an editorial auth page:

- AulaLite is visually clear as the product identity.
- Local testing appears as styled account shortcuts, not plain buttons.
- Firebase sign-in remains available in the same visual system.
- Inputs, labels, actions, errors, and loading states are polished.

### App Shell

The shell should feel composed and useful:

- Left navigation rail with brand, route links, and active/hover states.
- Topbar with signed-in identity and sign-out affordance.
- Main area uses consistent spacing and responsive constraints.

### Dashboard

Dashboard emphasizes quick orientation:

- Welcome header with user name.
- Courses and upcoming sessions in a balanced grid on desktop.
- Course rows scan like editorial index entries: title, role/status, next session.
- Empty/loading/error states use styled panels rather than raw text.

### Courses

Course list:

- Course cards include stable cover slots, title, status, description, and clear
  click affordance.
- Filter chips behave visually like segmented controls.

Course detail:

- Header combines title, status, and cover image if present.
- Tabs are prominent but quiet.
- Outline rows use hierarchy, spacing, type pills, and clear links.

### Schedule

Schedule entries should read like a clean agenda:

- Time/date, course title, session title, status, and actions are visually
  grouped.
- Live or scheduled status is obvious without overpowering the page.

### Live Room

The live room is operational, not decorative:

- Main video area gets visual priority.
- Lobby state feels intentional and calm.
- Teacher broadcast controls make the live state obvious.
- Chat, presence, and hand-raise areas become compact side panels.
- Error and rate-limit banners are visible but not disruptive.

## Interaction And States

Shared interaction rules:

- Buttons use the design-system style everywhere possible.
- Links remain visibly clickable but quieter than buttons.
- Inputs have labels, spacing, focus rings, disabled states, and error states.
- Empty/loading/error states never fall back to raw unstyled text.
- Status badges use consistent tones:
  - `draft`: neutral.
  - `published`: success.
  - `scheduled`: info.
  - `live`: danger/live.
  - `ended`: muted.
  - `cancelled`: warning/danger depending on context.

## Testing And Verification

Verification for implementation:

- Rebuild the frontend Docker image.
- Confirm `http://localhost:3000` renders styled login/local testing.
- Use local dev login to verify dashboard, courses, course detail, schedule, and
  live-session pages are styled.
- Run a headless browser check for fatal JavaScript runtime errors.
- Capture a desktop screenshot.
- Run existing smoke tests, including `cargo test -p shell-web --test shell_routes_smoke`.

## Risks

- Some pages currently use ad hoc class names. The implementation should
  centralize common styling without requiring every component to be rewritten.
- The generated Dioxus output must continue to include the updated CSS assets in
  Docker builds.
- Overly decorative styling could reduce usability. The theme must stay
  restrained and work-focused.
