# AulaLite Learning Suite And UI Uplift Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **STATUS (2026-06-10): Cycles 1–7 COMPLETE and committed.** Cycle 1 `efc8137`,
> Cycle 2 `02ce666`, Cycle 3 `4c9ea6b`, Cycle 4 `4d01314` (+ parent XP chips,
> settings opt-out), Cycle 5 `919fdf8`, Cycle 6 `c572749`, Cycle 7 split across
> `aaca57e` (charts), `8b8e112` (sortable builder), `75fc77d` (guided tour),
> `82cd995`/`43d353d` (admin/auth/dashboard/live-room polish — owner additionally
> requested and received auth + dashboard hero + live-room passes). Owner
> re-ordered execution (C4 → C7 → C5 → C6) on 2026-06-10. Cycle 8 verification
> in progress: full host test suite green per-cycle; docker rebuild + browser
> walkthrough pending. Schema deltas vs plan: tour flag lives in
> `users.tour_dismissed` (migration …035); certificates issue snapshots +
> stable credential id on re-issue; flashcard review queue caps NEW cards at
> 10/session (due cards uncapped).

**Goal:** Expand AulaLite's product offering with a learning suite (lesson progress, quizzes, gamification, certificates, flashcards) built on the new `ui-learn` family in dioxus-kinetics, adopt full dark-mode theming through the kinetics ThemeProvider and the Apple/Comet material contract, and uplift the remaining raw UI surfaces (analytics charts, drag-reorder builder, onboarding tour, admin pages).

**Architecture:** Keep existing workspace boundaries. Bump the pinned dioxus-kinetics rev once at the workspace root and add the `learn` feature. All new kinetics consumption goes through `crates/design-system` (`kinetics_ui.rs` re-exports, `kinetics_styles.rs` token bridge). New product features each get: SQL migration(s) under `migrations/`, db module under `crates/backend/src/db/`, handler module under `crates/backend/src/handlers/`, DTOs in `core-types`, feature components in `crates/features-courses`, and routes in `crates/shell-web/src/routes/`. RLS policies follow the patterns established in `20260530000030_system_context_rls.sql`.

**Tech Stack:** Rust 2021, Dioxus 0.7.4, Axum 0.7, SQLx/Postgres with RLS, dioxus-kinetics @ `3fa9027` (features: web, tokens, glass, motion, layout-motion, a11y, timeline, runtime, blocks, **learn**), static CSS tokens/components.

**Decisions locked with the product owner (2026-06-09):**

- Quizzes: both module-item and course-level placement; per-quiz mode `graded` (limited attempts, recorded score) or `practice` (unlimited, instant feedback, no grade record).
- Lesson completion: explicit student "Mark as complete"; quizzes auto-complete on submission.
- Gamification: XP/streaks personal and tenant-wide; leaderboards per-course; students can opt out of leaderboards; parents see child XP/streaks.
- Certificates: completion makes a student eligible; **teacher approves and issues**; CertificateCard render + public verify link by credential ID + browser print-to-PDF.
- Flashcards: teacher-authored decks per course; SM-2 review scheduling via kinetics `next_review`.
- Theming: full dark mode; preference **persisted in backend user settings** (follows the user across devices); ThemeProvider supplies reactive mode/density.
- Do not run `cargo fmt --all` (rustfmt version skew reformats ~70 unrelated files). Format only touched hunks.

---

## Scope Check

One cohesive slice per cycle; cycles are ordered by dependency. Cycle 1 (foundations) blocks everything. Progress (Cycle 2) blocks gamification and certificates. Quizzes (Cycle 3) feed gamification XP and graded analytics. AI-assisted study remains out of scope (separate Elementors repo). LTI/SSO, peer review, plagiarism detection remain backlog.

## File Structure

Create (by cycle, representative — exact splits may vary):

- `migrations/2026XXXXXXXXXX_user_preferences.sql` — theme/density preference columns (Cycle 1)
- `migrations/2026XXXXXXXXXX_lesson_progress.sql` — lesson_completions + RLS (Cycle 2)
- `migrations/2026XXXXXXXXXX_quizzes.sql` — quizzes, quiz_questions, quiz_attempts, quiz_attempt_answers + RLS (Cycle 3)
- `migrations/2026XXXXXXXXXX_gamification.sql` — xp_events, learner_stats, achievements, achievement_unlocks, leaderboard opt-out + RLS (Cycle 4)
- `migrations/2026XXXXXXXXXX_certificates.sql` — certificates + RLS + public verify view (Cycle 5)
- `migrations/2026XXXXXXXXXX_flashcards.sql` — flashcard_decks, flashcards, flashcard_review_state + RLS (Cycle 6)
- `crates/backend/src/db/{progress,quizzes,gamification,certificates,flashcards}.rs`
- `crates/backend/src/handlers/{progress,quizzes,gamification,certificates,flashcards}.rs`
- `crates/features-courses/src/{course_progress,quiz_take,quiz_results,quiz_editor,quiz_list,gamify_panel,leaderboard_view,certificate_view,certificates_admin,flashcards_review,flashcards_editor}.rs`
- `crates/shell-web/src/routes/{quizzes_list,quizzes_take,quizzes_edit,certificates,certificate_verify,flashcards,flashcards_edit,leaderboard}.rs`
- `crates/design-system/src/theme_toggle.rs` — light/dark toggle wired to backend preference

Modify (high-traffic):

- `Cargo.toml` — kinetics rev bump + `learn` feature
- `crates/design-system/src/kinetics_ui.rs` — re-export ui-learn, charts, sortable, tour, ThemeProvider, hooks
- `crates/design-system/src/kinetics_styles.rs` — extend `--ui-*` bridge to material contract (glass highlights, elevation scale, press/density tokens), light + dark values
- `crates/design-system/assets/tokens.css` + `crates/shell-web/public/assets/tokens.css` — `[data-ui-theme="dark"]` Elite Academy dark palette
- `crates/design-system/assets/components.css` + mirror — dark-theme component rules, new surface classes
- `crates/features-courses/src/app_shell.rs` — ThemeProvider wrap, theme toggle in account area, new nav entries
- `crates/features-courses/src/dashboard.rs` — ResumeLearning, CourseProgressCard, XpBar, StreakBadge
- `crates/features-courses/src/course_detail.rs` — Outline/progress tab, quizzes tab, leaderboard tab, certificates admin tab
- `crates/backend/src/handlers/me.rs` — theme/density preference read/patch
- `crates/backend/src/handlers/{mod,analytics,parent}.rs`, `crates/backend/src/lib.rs` router — new routes + graded-quiz + XP surfacing
- `crates/shell-web/src/route_enum.rs` + `routes/mod.rs` — new routes
- `crates/shell-web/src/routes/{admin_analytics,onboarding,admin_billing,admin_branding,admin_audit,admin_tenant,admin_files,search,notification_settings}.rs` — Cycle 7 uplift
- `crates/features-courses/src/course_builder.rs` (or equivalent module editor) — SortableList reorder

Test:

- Backend: unit tests in each new handler/db module; integration tests `crates/backend/tests/{progress,quizzes,gamification,certificates,flashcards}.rs` following `crates/backend/tests/attendance.rs` patterns (RLS, tenant isolation, role checks)
- Frontend: SSR smoke tests in `crates/features-courses/tests/` and `crates/shell-web/tests/` following `assignments_ssr.rs` patterns
- Grading/SM-2 math: rely on kinetics' own tested `grade_answer`/`next_review`; test AulaLite's persistence and authorization seams

---

## Cycle 1: Foundations — Kinetics Bump, Material Contract Bridge, Dark Mode

**Files:** `Cargo.toml`, `crates/design-system/src/kinetics_ui.rs`, `crates/design-system/src/kinetics_styles.rs`, `crates/design-system/src/theme_toggle.rs` (new), `crates/design-system/src/lib.rs`, `crates/design-system/assets/tokens.css`, `crates/design-system/assets/components.css`, mirrors in `crates/shell-web/public/assets/`, `crates/features-courses/src/app_shell.rs`, `migrations/*_user_preferences.sql`, `crates/backend/src/handlers/me.rs`.

- [ ] **Step 1: Bump kinetics rev and enable `learn`.** Update workspace `Cargo.toml` kinetics dep to rev `3fa9027` and add `"learn"` to features. `cargo update -p kinetics` (or refresh lockfile) and `cargo check -p design-system` to surface breakage. Known upstream changes since `c915ca0`: `Theme::default()` palette darkened for WCAG AA (`#0066cc`→`#0058b3` etc.), additive prelude exports, no renames.
- [ ] **Step 2: Re-export new families.** In `kinetics_ui.rs`, re-export: all ui-learn surfaces + vocabulary (`CourseOutline`, `CourseModule`, `CourseLesson`, `LessonState`, `CourseProgressCard`, `ResumeLearning`, `FlipCard`, `Flashcard`, `FlashcardDeck`, `ReviewRating`, `ReviewState`, `QuestionCard`, `QuizQuestion`, `QuizPrompt`, `QuizChoice`, `QuizAnswer`, `QuizResults`, `QuizTimer`, `XpBar`, `StreakBadge`, `AchievementUnlock`, `Leaderboard`, `LeaderboardEntry`, `CertificateCard`, plus helpers `grade_answer`, `normalize_short_answer`, `next_review`, `course_progress`), charts (`BarChart`, `LineChart`, `Sparkline`, `DonutGauge`, `ChartSeries`, `ChartTone`), sortable (`SortableList`, `SortableItem`, `KanbanBoard`), tour (`Tour`, `TourStep`, `TourPlacement`), and runtime theming (`ThemeProvider`, `use_theme_mode`, `use_density`).
- [ ] **Step 3: Extend the token bridge to the material contract.** In `kinetics_styles.rs`, map the new `--ui-*` variables to Elite Academy values for light theme: `--ui-glass-highlight`, `--ui-glass-highlight-bottom`, `--ui-glass-blur`, `--ui-glass-saturate`, `--ui-surface`, `--ui-surface-muted`, `--ui-surface-strong`, `--ui-elevation-0..3`, `--ui-press-scale`, `--ui-control-height`, `--ui-focus`. Keep warm-cream/deep-green brand mapping; add `[data-ui-theme="dark"]` block with the dark Elite Academy palette.
- [ ] **Step 4: Dark Elite Academy palette in tokens.css.** Design dark counterparts: deep green-charcoal background (#101a16 family), elevated surfaces (#18241f), cream text (#f1e9da), gold accent unchanged (#c9a45c), adjusted semantic tones meeting WCAG AA. Add `[data-ui-theme="dark"]` overrides for `--color-*`, `--shadow-*` variables. Mirror to shell-web public assets. Audit `components.css` for hardcoded colors that must become token references.
- [ ] **Step 5: User preference migration + API.** Migration adds `theme_preference text not null default 'system'` (check: system|light|dark) and `density_preference text not null default 'comfortable'` to the users (or profiles) table. Extend `/v1/me` GET payload and add `PATCH /v1/me/preferences`. Unit-test validation.
- [ ] **Step 6: ThemeProvider + toggle.** Wrap the app shell subtree in `ThemeProvider`. New `theme_toggle.rs` design-system component (system/light/dark segmented control); shell account area hosts it; on change, set `data-ui-theme` and PATCH preference; on login bootstrap, apply stored preference (system → omit attribute so `prefers-color-scheme` rules).
- [ ] **Step 7: Verify.** `cargo check` workspace; design-system + shell-web + features-courses SSR tests; visual pass on `/dev/components` in both themes; fix dark-mode regressions in polished pages (dashboard, course detail, live room, login).
- [ ] **Step 8: Commit** `feat(theme): adopt kinetics material contract with full dark mode`.

## Cycle 2: Lesson Progress Tracking

**Files:** `migrations/*_lesson_progress.sql`, `crates/backend/src/db/progress.rs`, `crates/backend/src/handlers/progress.rs`, `core-types` DTOs, `crates/features-courses/src/{course_progress.rs,dashboard.rs,course_detail.rs}`, lesson view component, shell routes.

- [ ] **Step 1: Schema.** `lesson_completions(tenant_id, course_id, lesson_id, user_id, completed_at)` PK (lesson_id, user_id); RLS: students read/write own rows, teachers/admins read course-scoped. Indexes for per-course-per-user aggregation.
- [ ] **Step 2: API.** `PUT/DELETE /v1/courses/:slug/lessons/:lesson_id/completion` (student, must be enrolled); `GET /v1/courses/:slug/progress` (per-lesson states + aggregate for current user); `GET /v1/me/progress` (all enrolled courses: completed/total, current lesson = first incomplete, last activity). Teacher variant: per-student progress for the course.
- [ ] **Step 3: Frontend.** Lesson view gains "Mark as complete" (and undo). Course detail modules tab renders kinetics `CourseOutline` with `LessonState::{Available,Current,Completed,Locked}` (locked unused for now — all self-paced). Dashboard: `ResumeLearning` strip (most recent in-progress course) + `CourseProgressCard` per enrolled course using `course_progress` helper. Parent home shows child progress.
- [ ] **Step 4: Tests + commit** `feat(progress): lesson completion tracking with course outline and resume`.

## Cycle 3: Quizzes (Graded + Practice)

**Files:** `migrations/*_quizzes.sql`, `crates/backend/src/db/quizzes.rs`, `crates/backend/src/handlers/quizzes.rs`, `core-types` quiz DTOs (mirror kinetics `QuizPrompt`/`QuizAnswer` shapes for serde), `crates/features-courses/src/{quiz_editor,quiz_take,quiz_results,quiz_list}.rs`, shell routes `quizzes_*`, `course_detail.rs` (Quizzes tab + module items), analytics + parent handlers.

- [ ] **Step 1: Schema.** `quizzes(id, tenant_id, course_id, module_id nullable, position nullable, title, description, mode graded|practice, time_limit_seconds nullable, max_attempts nullable, status draft|published, created_by, timestamps)` — `module_id` set = module item; null = course-level. `quiz_questions(id, quiz_id, position, prompt jsonb, points)` storing the five `QuizPrompt` shapes as tagged JSON. `quiz_attempts(id, quiz_id, user_id, started_at, submitted_at, score_points, max_points, passed bool nullable)`. `quiz_attempt_answers(attempt_id, question_id, answer jsonb, correct bool)`. RLS: students CRUD own attempts on published quizzes in enrolled courses; teachers author within their courses; correct answers never leak to students pre-submission (separate student-safe question projection).
- [ ] **Step 2: Authoring API + UI.** CRUD for quizzes/questions; publish action validates ≥1 question. `quiz_editor.rs`: question list with type picker (choice, multi-select, true/false, ordering, short answer), per-question points, quiz settings (mode, time limit, attempts). Teacher sees quizzes in a course-level Quizzes tab and inline in module editing.
- [ ] **Step 3: Taking flow.** `POST .../attempts` starts (server stamps started_at; enforces max_attempts for graded); student-safe questions fetched without answers; `quiz_take.rs` renders one `QuestionCard` at a time with progress + `QuizTimer` (host ticks remaining; auto-submit at zero); `POST .../attempts/:id/submit` grades server-side (port/reuse `grade_answer` semantics in backend — kinetics helpers are frontend; backend re-grades authoritatively), stores per-question correctness.
- [ ] **Step 4: Results + records.** `quiz_results.rs` renders `QuizResults` (score gauge, per-question dots, retry for practice or remaining graded attempts). Graded scores appear in course analytics and parent view. Quiz submission auto-marks the module-item quiz complete for progress (Cycle 2 integration).
- [ ] **Step 5: Tests + commit** `feat(quizzes): graded and practice quizzes with authoring, timer, and auto-grading`.

## Cycle 4: Gamification

**Files:** `migrations/*_gamification.sql`, `crates/backend/src/db/gamification.rs`, `crates/backend/src/handlers/gamification.rs`, award hooks in progress/quizzes/attendance paths, `crates/features-courses/src/{gamify_panel.rs,leaderboard_view.rs,dashboard.rs}`, notification settings page (leaderboard opt-out), parent handler/view.

- [ ] **Step 1: Schema.** `xp_events(id, tenant_id, user_id, course_id nullable, kind, points, created_at, dedup_key unique)` (kinds: lesson_completed=10, quiz_passed=25, quiz_perfect=+15, session_attended=15, streak_day=5; dedup_key prevents double-award). `learner_stats(tenant_id, user_id, total_xp, level, current_streak_days, longest_streak, last_activity_date)` maintained transactionally. `achievements` seed catalog (first lesson, 7-day streak, first perfect quiz, course completed, …) + `achievement_unlocks(user_id, achievement_id, unlocked_at, seen bool)`. `leaderboard_opt_out(user_id, tenant_id)`. Level curve: simple quadratic (level n needs 100·n XP).
- [ ] **Step 2: Award engine.** Service called from lesson-completion, quiz-submit, and attendance-reconciliation paths; updates streak on any qualifying activity per local date; returns newly unlocked achievements in the triggering response so the frontend can fire `AchievementUnlock`.
- [ ] **Step 3: API.** `GET /v1/me/gamification` (xp, level, streak, recent unlocks, unseen flag); `POST /v1/me/gamification/seen`; `GET /v1/courses/:slug/leaderboard` (rank by course XP, opt-outs excluded, current user pinned); `PUT/DELETE /v1/me/leaderboard-opt-out`. Parent endpoint surfaces child xp/streak.
- [ ] **Step 4: Frontend.** Dashboard hero row gains `XpBar` + `StreakBadge`; `AchievementUnlock` celebration rendered on unlock events; course detail Leaderboard tab via kinetics `Leaderboard` with "hide me" toggle; settings page hosts the opt-out; parent home shows child XP/streak chips.
- [ ] **Step 5: Tests + commit** `feat(gamify): xp, streaks, achievements, and per-course leaderboards`.

## Cycle 5: Certificates (Teacher-Approved)

**Files:** `migrations/*_certificates.sql`, `crates/backend/src/db/certificates.rs`, `crates/backend/src/handlers/certificates.rs`, `crates/features-courses/src/{certificate_view.rs,certificates_admin.rs}`, shell routes `certificates.rs` + public `certificate_verify.rs`, course detail (Certificates admin tab), dashboard/parent surfacing.

- [ ] **Step 1: Schema.** `certificates(id, tenant_id, course_id, user_id, credential_id text unique (e.g. AULA-XXXX-XXXX), status eligible|issued|revoked, issued_by, issued_at, recipient_name, course_title snapshot)`. Eligibility row created automatically when course progress hits 100% (lessons + published required quizzes passed for graded). RLS: student reads own; teacher manages course-scoped; verify endpoint uses a security-definer function or system context returning only public fields.
- [ ] **Step 2: API.** `GET /v1/courses/:slug/certificates` (teacher: eligible + issued list); `POST /v1/courses/:slug/certificates/:user_id/issue`; `POST .../revoke`; `GET /v1/me/certificates`; public unauthenticated `GET /v1/verify/:credential_id`.
- [ ] **Step 3: Frontend.** Course detail Certificates tab: eligible students table with Issue buttons (and revoke). Student `/certificates` page lists earned certs; detail renders kinetics `CertificateCard` (recipient, course, date, issuer, credential id) with print stylesheet for PDF export and copyable verify link. Public `/verify/:credential_id` route renders validity + certificate. Notification on issue.
- [ ] **Step 4: Tests + commit** `feat(certificates): teacher-issued completion certificates with public verification`.

## Cycle 6: Flashcards (Teacher Decks + SM-2)

**Files:** `migrations/*_flashcards.sql`, `crates/backend/src/db/flashcards.rs`, `crates/backend/src/handlers/flashcards.rs`, `crates/features-courses/src/{flashcards_editor.rs,flashcards_review.rs}`, shell routes, course detail tab, dashboard due-count chip.

- [ ] **Step 1: Schema.** `flashcard_decks(id, tenant_id, course_id, title, description, status, created_by)`; `flashcards(id, deck_id, position, front, back)`; `flashcard_review_state(card_id, user_id, ease, interval_days, repetitions, due_date, last_rating)` matching kinetics `ReviewState`/SM-2 vocabulary. RLS course-scoped; review state per-student.
- [ ] **Step 2: API.** Deck/card CRUD for teachers; `GET /v1/courses/:slug/decks/:id/review` returns due cards (due_date ≤ today, new cards capped per session); `POST .../review/:card_id` records `ReviewRating`, computes next state server-side mirroring kinetics `next_review`.
- [ ] **Step 3: Frontend.** Teacher deck editor (card list, front/back markdown-lite). Student review session with `FlashcardDeck` (flip, rate Again/Hard/Good/Easy), session summary, due-count badge on course card/dashboard.
- [ ] **Step 4: Tests + commit** `feat(flashcards): teacher decks with SM-2 spaced review`.

## Cycle 7: UI Uplift — Charts, Sortable Builder, Tour, Admin Polish

**Files:** `crates/shell-web/src/routes/{admin_analytics,onboarding,admin_billing,admin_branding,admin_audit,admin_tenant,admin_files,search,notification_settings}.rs`, course analytics view, module editor in `course_detail.rs`/builder component, `components.css` + mirror.

- [ ] **Step 1: Analytics charts.** Replace raw tables: attendance over time → `LineChart`; per-course engagement → `BarChart`; completion/score rates → `DonutGauge`; KPI cards gain `Sparkline` trends. Course analytics adds quiz-score distribution and progress funnel. Keep accessible table fallback (toggle or detail expander).
- [ ] **Step 2: Drag-reorder builder.** Module/lesson (and quiz item) reordering with `SortableList` (keyboard a11y built in); persist via existing reorder endpoints or add `PATCH .../modules/:id/reorder`.
- [ ] **Step 3: Onboarding + tour.** Bring `/onboarding` to design-system standard; add a first-run `GuidedTour` for teacher/admin (dashboard → create course → invite members → go live) with per-user dismissed flag (extend preferences from Cycle 1).
- [ ] **Step 4: Admin/search/notifications polish.** Apply PageHeader/Card/DataTable/EmptyState patterns and dark-theme support to `/admin/billing`, `/admin/branding`, `/admin/audit`, `/admin/tenant`, `/admin/files`, `/search`, `/settings/notifications`.
- [ ] **Step 5: Commit per surface group** (`feat(analytics): chart visualizations`, `feat(builder): drag-reorder modules`, `feat(onboarding): guided tour`, `style(admin): design-system polish`).

## Cycle 8: Final Verification

- [ ] Full workspace `cargo check` + `cargo test` (backend integration suite needs the local Postgres; use the docker stack per `local-docker-stack` notes, backend on 18080).
- [ ] Docker compose stack build (gh-token build secret for the private kinetics dep) and browser walkthrough: every new route in light and dark, student + teacher + parent roles.
- [ ] Update `docs/` feature inventory; record any cut scope as a backlog spec like the SaaS expansion doc.
