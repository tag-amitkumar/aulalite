# AulaLite SaaS Readiness And Elite Academy Refresh Design

**Date:** 2026-05-15
**Status:** Approved design, awaiting user review

## Goal

Make the current AulaLite web app feel like a polished SaaS product and an
elite modern academy while keeping this phase focused enough to finish. This
phase upgrades the existing product surface, adds a stronger brand and visual
system, reviews backend consistency broadly, and records larger SaaS expansion
features for the next phase.

## Current Phase Scope

This phase includes:

- Full-page frontend polish across auth, shell, dashboard, courses,
  assignments, schedule, live rooms, replay, invites, redeem, loading, empty,
  error, and permission states.
- A new AulaLite brand system: logo or mark, favicon/app icon direction,
  premium typography, richer color system, page motion, icons, and generated
  academy-style imagery where useful.
- A broad backend consistency audit and defect fixes across auth, permissions,
  tenant boundaries, handlers, migrations, tests, local login/dev seed, and
  live-session flows.
- SaaS-readiness cleanup: consistent API errors, polished local/demo flows,
  reliable route redirects, consistent role behavior, and test coverage for
  defects found.

Out of scope for this phase:

- Billing and subscription plans.
- Tenant settings UI.
- Analytics dashboards.
- Full onboarding flows.
- Admin console expansion.
- Notification preferences and delivery.
- Support/helpdesk flows.
- Large product modules that require their own data model and route design.

Those expansion items belong in the next SaaS phase backlog.

## Architecture

Keep the existing Rust workspace boundaries and make the refresh work through
shared layers instead of one-off page patches.

Frontend ownership stays split as follows:

- `crates/design-system`: visual tokens, shared CSS, reusable Dioxus
  primitives, state styling, and shared asset conventions.
- `crates/shell-web`: route wiring, auth bootstrap, public assets, shell-level
  layout, and app-level behavior.
- `crates/features-auth`: login, signup, forgot password, local/demo account
  affordances, and auth form states.
- `crates/features-courses`: dashboard, courses, assignments, schedule, live
  room, replay, and supporting course workflows.

The design system becomes the main polish layer. It should own expanded
tokens, animations, icon utilities, button/input/table/badge/card state
consistency, branded logo assets, image treatment classes, and responsive
layout patterns. Page components should receive targeted Dioxus markup changes
only where CSS cannot express the desired result cleanly.

Backend work stays in `crates/backend`, `crates/core-types`, migrations, and
integration tests. The review should look for inconsistent tenant checks, role
checks, API shape mismatches, missing validation, fragile dev/local auth
behavior, and route behavior that could cause broken SaaS flows. Fixes should
stay tied to existing product surfaces unless a defect requires a small
supporting change.

## Visual And Interaction Direction

AulaLite should present as an elite modern academy: editorial, premium,
scholarly, and current, with expressive moments that do not reduce daily
course-work efficiency.

Use this visual direction:

- Replace the plain `A` block with a distinctive AulaLite logo and brand mark.
- Use a richer professional palette: ink, ivory, deep academy green,
  oxblood/live accents, restrained gold, and cool blue for information states.
- Use serif display type for identity and page titles, with clean sans-serif
  type for operational UI.
- Use generated imagery in high-impact places: auth hero, dashboard
  masthead/accent, empty states, course cover fallbacks, and live/replay
  atmosphere.
- Add a motion system for page entrances, cards, buttons, panels, live
  indicators, skeleton/loading states, nav focus, and modal transitions.
- Use icons for navigation, actions, and status labels through a
  Dioxus-compatible dependency or repo-native SVG components if dependency
  friction is high.
- Keep dense SaaS surfaces scan-friendly; avoid nested card-heavy page
  layouts and decorative clutter.

The strongest "bells and whistles" belong in auth, dashboard, course cards,
live room, replay, and empty states. Operational views like grading tables and
schedule lists should receive better hierarchy, icons, transitions, sticky
actions where useful, and polished state handling without becoming flashy.

## Page Coverage

Every existing web route should have an intentional finished treatment.

Auth:

- Branded split-screen academy experience.
- Generated hero image.
- Styled local/demo account shortcuts.
- Polished signup and forgot-password states.
- Clear errors, loading, and disabled states.

Shell:

- Logo and responsive navigation.
- Active and hover states.
- Account menu polish.
- Route-specific page framing where needed.

Dashboard:

- Premium SaaS overview with useful stats.
- Next sessions and course momentum.
- Styled loading, empty, and error states.

Courses:

- Modern course cards and generated fallback covers.
- Clear filters.
- Course detail hero treatment.
- Outline hierarchy.
- Polished people, edit, and schedule tabs.

Assignments:

- List, detail, editor, grading, and submission states should look
  production-ready.
- Forms should share consistent labels, validation, focus rings, button groups,
  and feedback states.

Schedule, invites, and redeem:

- Branded, focused workflows.
- Consistent confirmations, failures, empty states, and redirects.

Live room and replay:

- Professional broadcast/watch layout.
- Animated live status.
- Stronger side panels for chat, presence, and hand raise.
- Error banners and loading states that are visible without disrupting the
  session.
- Operational controls that remain clear under teacher and student roles.

System states:

- Skeletons, empty states, forbidden/not-found redirects, toast or inline
  feedback, and permission states should use one coherent visual language.

## Assets

Create or wire the following project-bound assets:

- AulaLite logo mark and wordmark, implemented in a crisp repo-native form
  where practical and reused in the shell, auth surface, and browser metadata.
- Raster generated academy hero imagery for the auth surface.
- Raster generated or code-native academy imagery for dashboard accents and
  high-impact empty states.
- Course fallback imagery or patterned covers that avoid a generic stock feel.
- Favicons and app icons wired through the current `shell-web` HTML/static
  asset setup.

Generated raster assets must be copied into the workspace before they are
referenced by the app. Do not leave project-referenced images only in the
image-generation output directory.

## Backend Consistency Review

The backend review is broad but defect-driven. Inspect routing, handlers, DB
access, migrations, and tests for issues that affect SaaS readiness:

- Auth bootstrap, local login bypass, token refresh, and redirect behavior.
- Tenant isolation and role permission consistency across courses, lessons,
  assignments, submissions, schedule, invitations, enrollment codes, file
  assets, live sessions, live room, recordings, and audit/dev seed flows.
- API validation consistency for status enums, required fields, IDs,
  tenant/course mismatches, and useful error responses.
- Frontend/backend DTO alignment so UI states do not silently collapse to empty
  lists on API failures.
- Live session and live room reliability, including current uncommitted work in
  socket, WHIP/WHEP, go-live, and local-login tests.
- Migration ordering, default values, constraints, and test coverage for known
  edge cases.

Fix defects found during the review when they are tightly related to current
functionality and can be verified in this phase. Larger SaaS feature gaps
should be recorded for the next phase instead of being partially implemented.

## Dependency Policy

Dependencies are allowed when they materially improve quality and fit the
Rust/Dioxus/WASM stack cleanly. Prefer small, focused additions for icons,
animation helpers, or asset handling. Do not add a heavy frontend framework or
state-management layer when the existing Dioxus architecture can support the
change.

Dependency additions must have a clear reason, work in the web target, and not
break the desktop/mobile crates unnecessarily.

## Testing And Verification

Verification should cover both the design refresh and SaaS consistency work:

- Run existing Rust tests for backend, shell-web, features-auth,
  features-courses, and design-system where relevant.
- Add or update SSR and smoke tests for key classes, logo/asset references,
  route rendering, role-aware UI, empty/error/loading states, and any fixed
  backend behavior.
- Add backend integration tests for auth, permission, tenant isolation,
  validation, or live-session defects fixed in this phase.
- Keep asset sync tests updated because `shell-web/public/assets` and
  `design-system/assets` currently mirror each other.
- Verify the browser app locally where feasible: auth screen, dashboard,
  courses, course detail, assignments, schedule, and live-session route.
- Check responsive breakpoints, text wrapping, reduced-motion handling, and
  that generated assets render correctly.

## Completion Criteria

This phase is complete when:

- Every current web route looks intentionally styled.
- The logo and generated imagery are present and wired from project assets.
- Loading, empty, error, permission, and redirect states are consistently
  styled.
- Backend review findings are fixed in this phase or recorded as next-phase
  backlog items.
- Tests pass, or environmental blockers are clearly documented with the exact
  commands attempted.
- No unrelated user work in the dirty worktree is reverted or overwritten.

## Next Phase Backlog

Use the review results to plan the next SaaS expansion phase. Likely candidates:

- Billing and subscription plan management.
- Tenant settings and branding controls.
- Admin console for tenant and user operations.
- Analytics and reporting.
- Onboarding and guided setup.
- Notification settings and delivery.
- Support, help, and operational tooling.
- Deeper audit and compliance workflows.
