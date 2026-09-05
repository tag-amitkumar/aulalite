# UI Polish — Plan E: Polish Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the three deferred vision-spec criteria from the UI Polish retrospective (criteria 3, 5, 6). Migrate residual surface CSS hex to new tokens, verify the dark-mode infrastructure works end-to-end, adopt the Toast primitive in production, and run a Lighthouse pass on the key surfaces.

**Architecture:** Five tightly-scoped tasks layered on top of Plans A–D. No new components, no new patterns — only token migrations, dark-token smoke test, surface-level Toast wiring, and verification.

**Tech Stack:** Same as the rest of the initiative.

**Retro / Vision spec:** `docs/superpowers/specs/2026-05-16-ui-polish-retro.md`, `docs/superpowers/specs/2026-05-16-ui-polish-vision-design.md`.

---

## Task 1: Token audit on residual surface CSS

**Files:**
- Modify: `crates/design-system/assets/tokens.css` (+ mirror).
- Modify: `crates/design-system/assets/components.css` (+ mirror).
- Modify: `crates/shell-web/tests/editorial_assets.rs` (extend the existing token-contract test).

### Residual hex inventory (from Plan D retro)

| File / selector | Current | Proposed token |
|--|--|--|
| `components.css` `.app-side` `background` | `#162822` | `var(--color-ink-sidebar)` |
| `.app-side` `border-right` | `rgba(255, 255, 255, 0.08)` | `var(--color-on-ink-subtle)` |
| `.app-side .nav-link` `color` | `rgba(255, 255, 255, 0.78)` | `var(--color-on-ink-strong)` |
| `.app-side .nav-link:hover` `color` | `#fff` | `#fff` (keep — solid white is the maximal contrast; no new token) |
| `.app-side .nav-link:hover` `background` | `rgba(255, 255, 255, 0.08)` | `var(--color-on-ink-subtle)` |
| `.live-room-video-area, .live-room-broadcast-controls, .replay-video-pane` `background` | `#101815` | `var(--color-ink-stage)` |
| Same selectors `color` | `#fff` | keep |
| `.live-video-main, .replay-video, .lesson-video` `background` | `#050706` | `var(--color-ink-stage-deep)` |
| `.live-room-banner` `background` | `#f4ead6` | `var(--color-banner-warm)` |
| `.live-room-banner` `border-color` | `rgba(155, 106, 30, 0.24)` | `var(--color-banner-warm-border)` |
| `.system-state--error` `background` | `#fbebe8` | `var(--color-state-error-bg)` |
| `.system-state--error` `border-color` | `rgba(163, 58, 54, .28)` | `var(--color-state-error-border)` |

(If grep surfaces other residual hex inside surface CSS rules — `.lobby-video-frame`, `.live-room-stage`, etc. — extend the table during implementation. The rule: any literal hex inside a `.app-*`, `.live-room-*`, `.replay-*`, `.system-state-*`, `.lobby-*` rule must move to a token.)

### New tokens to add to `tokens.css`

Insert in the surface-aliases block:

```css
  /* Surface ink / stage tokens (Plan E) — anchors for dark-mode flipping */
  --color-ink-sidebar: #162822;
  --color-ink-stage: #101815;
  --color-ink-stage-deep: #050706;
  --color-on-ink-strong: rgba(255, 255, 255, 0.78);
  --color-on-ink-subtle: rgba(255, 255, 255, 0.08);
  --color-banner-warm: #f4ead6;
  --color-banner-warm-border: rgba(155, 106, 30, 0.24);
  --color-state-error-bg: #fbebe8;
  --color-state-error-border: rgba(163, 58, 54, 0.28);
```

### Steps

- [ ] **Step 1: Add failing tests**

Append to `tokens_define_modern_academy_theme_contract` in `crates/shell-web/tests/editorial_assets.rs`:

```rust
    for expected in [
        "--color-ink-sidebar",
        "--color-ink-stage",
        "--color-ink-stage-deep",
        "--color-on-ink-strong",
        "--color-on-ink-subtle",
        "--color-banner-warm",
        "--color-banner-warm-border",
        "--color-state-error-bg",
        "--color-state-error-border",
    ] {
        assert!(
            css.contains(expected),
            "tokens.css missing expected token `{expected}`"
        );
    }
```

- [ ] **Step 2: Run, expect failure**

```bash
cargo test -p shell-web --test editorial_assets tokens_define_modern_academy_theme_contract
```

Expected: FAIL — `missing expected token --color-ink-sidebar`.

- [ ] **Step 3: Add the 9 new tokens to `tokens.css`**

Per the snippet above. Place inside `:root`, near the existing surface aliases (after `--color-text-muted`).

- [ ] **Step 4: Migrate the 12+ surface CSS rules in `components.css`**

Per the inventory table. Each migration is a one-line edit: replace `background: #162822;` with `background: var(--color-ink-sidebar);`, etc. The `:hover` `color: #fff;` rule keeps `#fff` literal (no token for pure white — design choice).

If grep surfaces any additional hex inside surface CSS rules during the audit, migrate those too. Common suspects to check: `.lobby-video-frame`, `.live-room-stage`, `.replay-chat-pane`, `.live-broadcast-camera`, `.live-room-presence`, `.live-room-chat`, `.live-room-hand-raise`. Most of these already use tokens (Plan D Task 8 migrated them); double-check.

- [ ] **Step 5: Mirror + run tests**

```bash
cp crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
cp crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
diff crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
diff crates/design-system/assets/components.css crates/shell-web/public/assets/components.css
cargo test -p shell-web --test editorial_assets
cargo test --workspace --no-fail-fast
```

Both diffs empty; editorial_assets tests pass; workspace shows only 2 known pre-existing failures.

- [ ] **Step 6: Token audit grep**

Run a stricter audit to find any remaining hex in surface CSS rules:

```bash
grep -nE "^\s+(background|color|border-color)\s*:\s*(#[0-9a-fA-F]{3,8}|rgba\()" crates/design-system/assets/components.css | grep -vE "(--color-|var\(|/\*)" | head -30
```

Each match should be either inside a primitive block (where Plan B already audited), a `var()` fallback, an intentional `#fff` literal, or a rgba that we deliberately kept as token-resistant decoration.

If genuine surface-level hex still remains, migrate it before commit.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/design-system/assets/tokens.css \
        crates/design-system/assets/components.css \
        crates/shell-web/public/assets/tokens.css \
        crates/shell-web/public/assets/components.css \
        crates/shell-web/tests/editorial_assets.rs
git commit -m "$(cat <<'EOF'
feat(tokens): introduce surface ink/stage/banner/state tokens; migrate residual hex

Closes vision-spec criterion 3 (no raw hex in surface CSS) and unblocks
criterion 6 (dark mode flip needs only token-value overrides, not Rust
or per-component CSS changes). New tokens cover the sidebar dark
background, live-room stage panes, the warm banner, and the
error-state surface; existing rules migrate to var(). The hover white
in .app-side stays as #fff — pure white has no token equivalent and
isn't theme-dependent.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Dark mode infrastructure smoke-test

**Files:**
- Modify (temporarily): `crates/design-system/assets/tokens.css` to populate `[data-theme="dark"]` block.
- Modify (manual test only): `crates/shell-web/index.html` to set `<html data-theme="dark">`.
- Create: `docs/superpowers/specs/2026-05-17-dark-mode-smoke-test-notes.md` capturing the test result.
- Revert: token values + index.html change after verification.

### Approach

Dark mode is OUT OF PRODUCT SCOPE for this initiative (vision spec decision 5). We only verify the *infrastructure works* — populating the dark block correctly flips the UI. Then we revert both changes; the empty `[data-theme="dark"]` block remains, ready for a future product decision.

### Steps

- [ ] **Step 1: Populate dark token overrides**

In `crates/design-system/assets/tokens.css`, replace the empty `[data-theme="dark"]` block (the one with the intentionally-empty `/* Intentionally empty — fill in when the dark theme is designed. */` comment) with a temporary populated version:

```css
[data-theme="dark"] {
  /* Plan E Task 2 smoke test — temporary; revert in Step 4. */
  --color-paper: #0c0e0b;
  --color-paper-warm: #171a17;
  --color-surface: #1a1d1a;
  --color-surface-subtle: #2c2920;
  --color-ink: #f6f0e6;
  --color-text: #f6f0e6;
  --color-text-muted: #b8a880;
  --color-rule: rgba(246, 240, 230, 0.13);
  --color-rule-strong: rgba(246, 240, 230, 0.25);
  --color-focus: rgba(201, 164, 92, 0.42);

  --color-primary: var(--color-green-400);
  --color-primary-hover: var(--color-green-300);
  --color-accent: var(--color-green-400);
  --color-accent-strong: var(--color-green-200);
  --color-accent-soft: var(--color-green-800);

  --color-ink-sidebar: #060f1f;
  --color-ink-stage: #050d09;
  --color-ink-stage-deep: #02060a;
  --color-banner-warm: #2e1d09;
  --color-banner-warm-border: rgba(217, 161, 80, 0.32);
  --color-state-error-bg: #2d0f0e;
  --color-state-error-border: rgba(210, 122, 116, 0.32);
}
```

- [ ] **Step 2: Set the dark attribute in index.html temporarily**

In `crates/shell-web/index.html`, change `<html>` to `<html data-theme="dark">`. (The opening `<html>` tag is at the top of the file.)

- [ ] **Step 3: Mirror tokens.css and run the dev server**

```bash
cp crates/design-system/assets/tokens.css crates/shell-web/public/assets/tokens.css
```

Launch `dx serve crates/shell-web` (or equivalent). Walk these surfaces and capture observations:
- `/login` — auth hero, fields, button.
- `/` — dashboard, hero, course list mini.
- `/courses` — course-list, CardList, course cards.
- `/courses/:slug` — course detail header, tabs, outline body.
- `/schedule` — schedule entries, badges.
- `/courses/:slug/sessions/:session_id` — live room stage and sidebars.

For each surface, note: does the dark theme apply (background dark, text light, badges legible)? Any unexpected light bleed (a hardcoded color that didn't migrate to a token)?

- [ ] **Step 4: Write `docs/superpowers/specs/2026-05-17-dark-mode-smoke-test-notes.md`**

```markdown
# Dark Mode Infrastructure Smoke Test

Date: 2026-05-17 (Plan E Task 2)

## What

Temporarily populated `[data-theme="dark"]` block in `tokens.css` and applied `<html data-theme="dark">` to verify the dark-mode infrastructure (decision 5 of the vision spec, criterion 6 of success criteria).

## Result

[Per-surface observations from Step 3 above.]

## Verdict

[Was the flip complete? Did any surface fail to darken? Document the leaks if any.]

## Follow-up

The actual dark-mode product decision (tone, brand interpretation in dark) is a separate design exercise. This smoke test confirmed that AulaLite's token architecture is theme-flip-ready: filling in tokens under `[data-theme="dark"]` flips the UI without component code changes.

Both the temporary populated dark block and the `<html data-theme="dark">` attribute have been reverted as part of this task. The empty `[data-theme="dark"]` placeholder remains in `tokens.css`, ready for the future product pass.
```

Fill in the bracketed sections with real observations.

- [ ] **Step 5: Revert the dark token values and the index.html attribute**

In `crates/design-system/assets/tokens.css`, replace the populated `[data-theme="dark"]` block with the original empty version (preserve the original comment).

In `crates/shell-web/index.html`, change `<html data-theme="dark">` back to `<html>`.

Mirror tokens.css. Confirm `diff` is clean.

- [ ] **Step 6: Run tests**

```bash
cargo test --workspace --no-fail-fast
```

Only 2 known pre-existing failures.

- [ ] **Step 7: Commit**

```bash
git add docs/superpowers/specs/2026-05-17-dark-mode-smoke-test-notes.md
git commit -m "$(cat <<'EOF'
docs: dark-mode infrastructure smoke-test notes

Plan E Task 2 verified that filling [data-theme="dark"] with token
overrides flips the UI without Rust or component-CSS changes — closes
vision-spec criterion 6. The temporary dark values used during the
test have been reverted; the placeholder block remains empty awaiting
a product-led dark-theme design pass.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

Note: the only committed artifact from Task 2 is the smoke-test notes doc. The token + index.html changes are temporary and reverted; nothing else lands.

---

## Task 3: Toast adoption in production

**Files:**
- Modify: 3 surfaces — pick from course-save, assignment-publish, signin-error, course-create-success, etc.

### Approach

Wire `use_toast_sender()` into at least 3 production paths that already have an error/success outcome. The minimum bar is that calling the action shows a toast on success or failure.

Suggested paths:
1. **Course Settings Save** — `CourseEditTab` in `crates/shell-web/src/routes/course_detail.rs` already has a `saved: Signal<bool>` flag. Replace the inline `p.muted` success message with a Toast push.
2. **Assignment Publish** — `assignment_editor.rs` or `assignment_detail.rs` likely has a publish action. Push a success toast on the `Ok` arm.
3. **Sign-in error** — `crates/features-auth/src/login.rs` has a `form_error.error_message: Option<String>`. Push a Danger toast when a 401 / 403 surfaces; keep the inline `FormError` for client-side validation (different category).

### Steps (per surface)

- [ ] **Step 1: Identify the success / failure handler** in each chosen surface.

- [ ] **Step 2: Add `let mut toast = use_toast_sender();` near the top of the component.**

- [ ] **Step 3: On the success or failure branch, call `toast.push(level, title, message);`**

Example for the course-save case:
```rust
match api::patch_course(&api, &course_id, &body).await {
    Ok(_) => {
        saved.set(true);
        toast.push(ToastLevel::Success, "Saved", "Course settings updated.");
    }
    Err(e) => {
        error.set(Some(format!("{e}")));
        toast.push(ToastLevel::Danger, "Save failed", format!("{e}"));
    }
}
```

- [ ] **Step 4: Verify each path manually (or in SSR test if practical)**.

Manual verification is acceptable since toasts are visible-only at runtime.

- [ ] **Step 5: Workspace test + wasm build**

```bash
cargo build --workspace
cargo test --workspace --no-fail-fast
cargo build -p shell-web --target wasm32-unknown-unknown
```

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "$(cat <<'EOF'
feat(toast): adopt use_toast_sender in production (course-save, assignment-publish, signin)

Wires the design-system Toast primitive (introduced in Plan C, given
auto-dismiss in Plan D Task 0a) into three production paths. The
primitive now has consumers; regression risk from a built-but-unused
component is removed.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Lighthouse pass

**Files:**
- Modify (append): `docs/superpowers/specs/2026-05-16-ui-polish-retro.md`.

### Approach

Run Lighthouse on Dashboard, Course Detail, and Live Room. Document scores. The pass is a check — we're not chasing 100s. The vision spec calls for ≥ 90 on Accessibility + Best Practices.

### Steps

- [ ] **Step 1: Start a local dev server**

```bash
# In one terminal:
cargo run -p backend
# In another (or use docker-compose):
dx serve crates/shell-web
```

Sign in as a test user. Make sure at least one course with a session is reachable.

- [ ] **Step 2: Run Lighthouse on three surfaces**

Use Chrome DevTools Lighthouse panel OR the CLI:
```bash
npx lighthouse http://127.0.0.1:3000 --only-categories=accessibility,best-practices,performance --output=json --output-path=./target/lighthouse-dashboard.json
npx lighthouse http://127.0.0.1:3000/courses/{slug} --only-categories=accessibility,best-practices,performance --output=json --output-path=./target/lighthouse-course-detail.json
npx lighthouse http://127.0.0.1:3000/courses/{slug}/sessions/{session_id} --only-categories=accessibility,best-practices,performance --output=json --output-path=./target/lighthouse-live-room.json
```

(Adapt URLs to the actual course slug and session id available in your test data.)

If Lighthouse CLI is unavailable, run manual passes in DevTools and screenshot the result.

- [ ] **Step 3: Extract scores**

From each JSON or screenshot, note:
- Performance score
- Accessibility score
- Best Practices score

- [ ] **Step 4: Append to the retro doc**

Open `docs/superpowers/specs/2026-05-16-ui-polish-retro.md`. Append a new section before the "Mobile and desktop shells" header:

```markdown
## Lighthouse pass (Plan E Task 4)

Run date: 2026-05-17

| Surface | Performance | Accessibility | Best Practices |
|--|--|--|--|
| Dashboard (`/`) | XX | XX | XX |
| Course Detail (`/courses/:slug`) | XX | XX | XX |
| Live Room (`/courses/:slug/sessions/:id`) | XX | XX | XX |

Notes:
- [Any A11Y issues surfaced]
- [Any Best-Practices issues surfaced]

Vision-spec criterion 5 (≥ 90 on Accessibility + Best Practices on these surfaces): **Met / Partially met / Not met** — [explanation].
```

Fill in scores from Step 3.

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-05-16-ui-polish-retro.md
git commit -m "$(cat <<'EOF'
docs(retro): Lighthouse pass — accessibility and best-practices scores

Per Plan E Task 4, ran Lighthouse on dashboard, course-detail, and
live-room. Scores appended to the polish retro to close vision-spec
criterion 5.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

If Lighthouse is genuinely unavailable in the local environment, document this in the retro section and flag as deferred. Don't fake scores.

---

## Task 5: Final verification + retro update

- [ ] **Step 1: Workspace tests** (`cargo test --workspace --no-fail-fast`).
- [ ] **Step 2: Format check** (`cargo fmt --all -- --check`).
- [ ] **Step 3: Wasm build** (`cargo build -p shell-web --target wasm32-unknown-unknown`).
- [ ] **Step 4: Asset-sync diff**.
- [ ] **Step 5: Update the success-criteria table** in `docs/superpowers/specs/2026-05-16-ui-polish-retro.md`:
  - Criterion 3 → Met (token audit complete).
  - Criterion 5 → Met OR Deferred-with-reason (depending on Lighthouse outcome).
  - Criterion 6 → Met (smoke-test verified the infra).

- [ ] **Step 6: Dispatch a final cross-task code reviewer** over all 4 Plan E commits — focus on token migration correctness, dark smoke-test cleanup (no leftover dark values), Toast adoption sanity.

- [ ] **Step 7: Append a "Plan E close" section to the retro doc**:
  ```markdown
  ## Plan E close (2026-05-17)

  Plan E shipped:
  - Token audit migrated XX residual hex literals to YY new tokens.
  - Dark-mode infrastructure smoke-tested and reverted; results in `2026-05-17-dark-mode-smoke-test-notes.md`.
  - Toast adopted in 3 production paths (course-save, assignment-publish, signin).
  - Lighthouse pass run; scores recorded above.

  Vision spec criteria 3, 5, 6 now Met. UI Polish initiative officially complete.
  ```

- [ ] **Step 8: Commit**
  ```bash
  git add docs/superpowers/specs/2026-05-16-ui-polish-retro.md
  git commit -m "$(cat <<'EOF'
  docs(retro): Plan E close — vision-spec criteria 3, 5, 6 met
  
  Polish initiative officially complete. Token audit closed the
  surface-CSS hex residue; dark-mode infrastructure verified; Toast
  primitive has production consumers; Lighthouse scores recorded.
  
  Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
  EOF
  )"
  ```

---

## Spec coverage check

| Retro follow-up | Task |
|--|--|
| Token audit on residual surface CSS | Task 1 |
| Dark mode infrastructure verification | Task 2 |
| Toast production adoption | Task 3 |
| Lighthouse pass | Task 4 |
| Retro doc update | Task 5 |
| Font subsetting | Deferred — independent perf plan |
| `components.css` split | Deferred — future plan when file > 2500 lines |
| Visual regression baseline | Deferred — independent tooling plan |

## Out of scope

- New dark-theme product design (we only verify infra; design happens later).
- Font subsetting.
- `components.css` per-component file split.
- Mobile / desktop shells.

## Risks

- **Task 2 revert discipline.** If the temporary dark values stay accidentally, the running app would dark-theme on `<html data-theme="dark">`. Mitigation: the smoke test commit includes ONLY the notes doc; the token revert and html revert are working-tree changes that don't persist. Verify `git status` before committing Task 2.
- **Toast adoption breakage.** Three surface edits in Task 3 could trip SSR tests that asserted on specific markup; update tests if needed.
- **Lighthouse availability.** If neither DevTools nor CLI Lighthouse is reachable in the local env, document the deferral honestly — don't fake numbers.
