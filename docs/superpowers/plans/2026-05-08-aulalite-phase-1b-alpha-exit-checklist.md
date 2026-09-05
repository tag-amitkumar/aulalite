# Phase 1b-α Exit Checklist

Run these checks in order from the repository root. Phase 1b-α is complete only
when every required item passes.

## 1. Stack health
- [ ] `docker compose up -d`
- [ ] `curl http://localhost:8080/healthz` returns `ok`
- [ ] MinIO console at `http://localhost:9001` shows the `aulalite` bucket
- [ ] Postgres / Redis / MinIO / backend / mediamtx all running

## 2. Migrations
- [ ] `sqlx migrate info --source migrations` shows `20260508000011_tighten_file_assets_links` as applied.
- [ ] `\d file_assets` shows the `file_assets_linked_entity_type_check` and `file_assets_size_nonneg` constraints.
- [ ] `\d courses` shows `courses_cover_asset_id_fkey`.
- [ ] `\d lessons` shows `lessons_video_asset_id_fkey`.

## 3. Automated verification
- [ ] `cargo test --workspace` (everything green; zero failures)
- [ ] `cargo build -p shell-web --target wasm32-unknown-unknown`
- [ ] `cargo check -p shell-mobile --target aarch64-linux-android`
- [ ] `cargo build -p features-courses --target wasm32-unknown-unknown`
- [ ] `dx build --platform web --package shell-web`

## 4. MinIO CORS check (one-time setup)
- [ ] In MinIO console (or via mc CLI), set the bucket's CORS to allow `PUT` from
      `http://localhost:3000` (dev) and `https://app.elementors.guru` (prod). If
      not set, browser uploads fail with CORS errors. Workaround for dev:
      ```bash
      mc alias set local http://localhost:9000 aulalite changeme123
      mc anonymous set-cors local/aulalite '<<<json>>>'
      ```
      where `<<<json>>>` allows your origins. (Spec section 7 open question 3.)

## 5. Course cover flow (web, real Firebase user)
- [ ] Sign in as teacher; open a course → Edit tab → upload a JPEG <5MB.
- [ ] Confirm preview displays.
- [ ] Confirm course list card shows the cover image.
- [ ] Confirm course detail header banner displays the cover.

## 6. Lesson video flow (web)
- [ ] Create a `'video'` lesson; upload an mp4 ≤500MB.
- [ ] Confirm `<video controls>` plays in lesson outline.
- [ ] Replace the video; confirm new video loads.

## 7. Lesson file_bundle flow (web)
- [ ] Create a `'file_bundle'` lesson; attach 3 files (pdf + zip + image).
- [ ] Confirm file list displays with correct icons + sizes.
- [ ] Click a file; download starts.
- [ ] Delete a file; confirm it's removed from the list and the MinIO object is gone.

## 8. Permissions
- [ ] Sign in as a student; confirm upload-begin returns 403.
- [ ] Cross-tenant probe: GET `/v1/file-assets/{some-tenant-A-asset-id}/url` from tenant B's session returns 404.
- [ ] PATCH course with a `cover_asset_id` from a different tenant returns 4xx.

## 9. Failure path
- [ ] Begin an upload; close the browser tab before completing.
- [ ] Confirm the `file_assets` row stays at `status='pending'`.
- [ ] Manually run the janitor SQL:
      ```sql
      UPDATE file_assets SET status='failed'
       WHERE status='pending' AND created_at < now() - interval '1 hour';
      ```
      (For exit-checklist purposes, override the timestamp with `interval '0 seconds'`.)
- [ ] Confirm the row is now `status='failed'`.

## Completion tag

Only after every required check above passes:

```bash
git tag phase-1b-alpha-complete
git push origin phase-1b-alpha-complete
```
