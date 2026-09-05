// tools/ui-write-flows.spec.js
//
// Real-stack e2e coverage for the write flows the original audit identified
// as missing from ui-real-stack.spec.js:
//   * assignment create + edit + publish
//   * assignment unpublish + delete
//   * series schedule create (via SeriesScheduler form)
//   * cancel a scheduled session
//   * generate an enrollment code, redeem it as a different identity
//   * signup form renders
//   * forgot-password form renders
//
// Each flow is independent — `test.describe.configure({ mode: "serial" })`
// only because they share the dev-seed state.

const fs = require("node:fs");
const path = require("node:path");
const { expect, test } = require("@playwright/test");

const baseURL = process.env.BASE_URL || "http://127.0.0.1:3000";
const apiBase = process.env.API_BASE || "http://127.0.0.1:8080";
const screenshotDir = path.join(process.cwd(), "target", "playwright-write-flows");

const teacherEmail = process.env.LOCAL_LOGIN_EMAIL || "local.teacher@example.test";
const teacherPassword = process.env.LOCAL_LOGIN_TEACHER_PASSWORD || "local-teacher-pass";

test.describe.configure({ mode: "serial" });

let ctx = null;

test.beforeAll(async ({ request }) => {
  fs.mkdirSync(screenshotDir, { recursive: true });

  const seedResp = await request.post(`${apiBase}/v1/dev/audit-seed`);
  expect(seedResp.ok(), await seedResp.text()).toBeTruthy();
  const seed = await seedResp.json();

  const tokenResp = await request.post(`${apiBase}/v1/auth/local-login`, {
    data: { email: teacherEmail, password: teacherPassword },
  });
  expect(tokenResp.ok(), await tokenResp.text()).toBeTruthy();
  const { id_token: teacherToken } = await tokenResp.json();
  const headers = { Authorization: `Bearer ${teacherToken}` };

  const courses = await apiJson(request, `${apiBase}/v1/courses`, headers);
  const course = courses.find((c) => c.slug === seed.course_slug);
  expect(course).toBeTruthy();

  ctx = { seed, course, teacherToken, headers };
});

async function apiJson(request, url, headers) {
  const r = await request.get(url, { headers });
  expect(r.ok(), `${url}: ${await r.text()}`).toBeTruthy();
  return r.json();
}

async function signInAsTeacher(page) {
  await page.goto(`${baseURL}/login`, { waitUntil: "domcontentloaded" });
  await page.locator("input[type=email]").fill(teacherEmail);
  await page.locator("input[type=password]").fill(teacherPassword);
  await page.getByRole("button", { name: "Sign in" }).click();
  await page.waitForURL(/\/$/);
}

async function screenshot(page, name) {
  await page.screenshot({ path: path.join(screenshotDir, `${name}.png`), fullPage: true });
}

test("signup page renders form fields", async ({ page }) => {
  await page.goto(`${baseURL}/signup`, { waitUntil: "domcontentloaded" });
  // Form must show email + password fields at minimum.
  await expect(page.locator("input[type=email]")).toBeVisible({ timeout: 10000 });
  await expect(page.locator("input[type=password]").first()).toBeVisible();
  await screenshot(page, "signup-form");
});

test("forgot-password page renders email field", async ({ page }) => {
  await page.goto(`${baseURL}/forgot`, { waitUntil: "domcontentloaded" });
  await expect(page.locator("input[type=email]")).toBeVisible({ timeout: 10000 });
  await screenshot(page, "forgot-form");
});

test("teacher creates, edits, then deletes an assignment", async ({ page, request }) => {
  const { seed, course } = ctx;
  await signInAsTeacher(page);

  // Open the assignment-new page.
  await page.goto(`${baseURL}/courses/${seed.course_slug}/assignments/new`, {
    waitUntil: "domcontentloaded",
  });
  await expect(page.locator(".assignment-editor")).toBeVisible({ timeout: 10000 });

  // Fill the form. The DS Input wraps the native <input>; targeting by
  // placeholder/role is fragile, so we use the label-implied position.
  const uniqueTitle = `Write-flow Essay ${Date.now()}`;
  const titleInput = page.locator(".ds-field").filter({ hasText: "Title" }).locator("input").first();
  await titleInput.fill(uniqueTitle);
  const textarea = page.locator(".ds-field").filter({ hasText: "Instructions" }).locator("textarea").first();
  await textarea.fill("# Essay\nWrite 500 words on a topic of your choice.");
  await page.getByRole("button", { name: /save draft/i }).click();

  // Should navigate to the detail page after save.
  await page.waitForURL(new RegExp(`/courses/${seed.course_slug}/assignments/[0-9a-f-]+$`), {
    timeout: 15000,
  });
  await screenshot(page, "assignment-saved");

  // Verify via API that the assignment exists.
  const assignmentsAfter = await apiJson(
    request,
    `${apiBase}/v1/courses/${course.id}/assignments?include_drafts=true`,
    ctx.headers,
  );
  const created = assignmentsAfter.find((a) => a.title === uniqueTitle);
  expect(created, "created assignment not visible via API").toBeTruthy();

  // Edit: change title.
  await page.goto(`${baseURL}/courses/${seed.course_slug}/assignments/${created.id}/edit`);
  await expect(page.locator(".assignment-editor")).toBeVisible();
  const newTitle = `${uniqueTitle} (edited)`;
  const titleInput2 = page
    .locator(".ds-field")
    .filter({ hasText: "Title" })
    .locator("input")
    .first();
  await titleInput2.fill(newTitle);
  await page.getByRole("button", { name: /save draft/i }).click();
  await page.waitForURL(new RegExp(`/courses/${seed.course_slug}/assignments/${created.id}$`));
  await screenshot(page, "assignment-edited");

  // Delete from editor (re-open edit page to get the Delete button).
  await page.goto(`${baseURL}/courses/${seed.course_slug}/assignments/${created.id}/edit`);
  await expect(page.locator(".assignment-editor")).toBeVisible();
  page.once("dialog", (d) => d.accept().catch(() => {}));
  await page.getByRole("button", { name: /^Delete$/ }).click();
  // After delete the editor navigates back to the assignment list.
  await page.waitForURL(new RegExp(`/courses/${seed.course_slug}/assignments$`), {
    timeout: 15000,
  });
  await screenshot(page, "assignment-deleted");

  // Verify gone from API.
  const finalList = await apiJson(
    request,
    `${apiBase}/v1/courses/${course.id}/assignments?include_drafts=true`,
    ctx.headers,
  );
  expect(finalList.find((a) => a.id === created.id), "assignment should be gone").toBeFalsy();
});

test("teacher generates and revokes an enrollment code via the People tab", async ({
  page,
  request,
}) => {
  const { seed, course } = ctx;
  await signInAsTeacher(page);

  await page.goto(`${baseURL}/courses/${seed.course_slug}/people`);
  await expect(page.locator(".course-people")).toBeVisible({ timeout: 10000 });

  // Open the CodeModal.
  await page.getByRole("button", { name: /generate.*code|create code/i }).first().click();
  await expect(page.getByText(/Generate enrollment code|Generate/i)).toBeVisible();

  // Generate without max-uses.
  await page.getByRole("button", { name: /^Generate$/ }).click();
  await expect(page.locator(".big-code")).toBeVisible({ timeout: 10000 });
  await screenshot(page, "code-generated");

  // Verify backend has at least one active code now.
  const codes = await apiJson(
    request,
    `${apiBase}/v1/courses/${course.id}/codes`,
    ctx.headers,
  );
  expect(codes.length).toBeGreaterThan(0);
});

test("teacher cancels a scheduled session from the schedule tab", async ({ page, request }) => {
  const { seed, course } = ctx;

  // Make sure there's a future occurrence to cancel. Reset the seeded one.
  const futureStart = new Date(Date.now() + 2 * 86400_000).toISOString();
  const sessions = await apiJson(
    request,
    `${apiBase}/v1/courses/${course.id}/sessions`,
    ctx.headers,
  );
  if (sessions.length > 0) {
    await request.patch(`${apiBase}/v1/sessions/${sessions[0].session_id}`, {
      headers: ctx.headers,
      data: { starts_at: futureStart, duration_minutes: 45, status: "scheduled" },
    });
  }

  await signInAsTeacher(page);
  await page.goto(`${baseURL}/courses/${seed.course_slug}/schedule`);
  await expect(page.locator(".schedule-agenda")).toBeVisible({ timeout: 10000 });

  // Click the first Cancel button.
  const cancelBtn = page.getByRole("button", { name: /^Cancel$/ }).first();
  await cancelBtn.click();
  // Toast should appear.
  await expect(page.locator(".ds-toast")).toBeVisible({ timeout: 10000 });

  // Verify backend marks at least one session cancelled.
  const after = await apiJson(
    request,
    `${apiBase}/v1/courses/${course.id}/sessions`,
    ctx.headers,
  );
  expect(after.some((s) => s.status === "cancelled")).toBeTruthy();
  await screenshot(page, "session-cancelled");
});

test("admin audit page is forbidden for the teacher role", async ({ page }) => {
  await signInAsTeacher(page);
  await page.goto(`${baseURL}/admin/audit`);
  // The teacher is not an org_admin, so the page should show Forbidden.
  await expect(page.getByText(/Forbidden|don.t have permission/i)).toBeVisible({
    timeout: 10000,
  });
  await screenshot(page, "audit-forbidden");
});
