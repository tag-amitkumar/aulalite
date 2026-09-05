const fs = require("node:fs");
const path = require("node:path");
const { expect, test } = require("@playwright/test");

const baseURL = process.env.BASE_URL || "http://127.0.0.1:3000";
const apiBase = process.env.API_BASE || "http://127.0.0.1:8080";
const screenshotDir = path.join(process.cwd(), "target", "playwright-ui");

const teacherEmail = process.env.LOCAL_LOGIN_EMAIL || "local.teacher@example.test";
const teacherPassword = process.env.LOCAL_LOGIN_TEACHER_PASSWORD || "local-teacher-pass";
const studentEmail = process.env.LOCAL_LOGIN_STUDENT_EMAIL || "local.student@example.test";
const studentPassword = process.env.LOCAL_LOGIN_STUDENT_PASSWORD || "local-student-pass";

test.describe.configure({ mode: "serial" });

let seedCtx = null;

test.beforeAll(async ({ request }) => {
  fs.mkdirSync(screenshotDir, { recursive: true });
  const seedResponse = await request.post(`${apiBase}/v1/dev/audit-seed`);
  expect(seedResponse.ok(), await seedResponse.text()).toBeTruthy();
  const seed = await seedResponse.json();

  const tokenResp = await request.post(`${apiBase}/v1/auth/local-login`, {
    data: { email: teacherEmail, password: teacherPassword },
  });
  expect(tokenResp.ok(), await tokenResp.text()).toBeTruthy();
  const { id_token: teacherToken } = await tokenResp.json();
  const headers = { Authorization: `Bearer ${teacherToken}` };

  const courses = await apiJson(request, `${apiBase}/v1/courses`, headers);
  const course = courses.find((c) => c.slug === seed.course_slug);
  expect(course).toBeTruthy();

  const sessions = await apiJson(request, `${apiBase}/v1/courses/${course.id}/sessions`, headers);
  expect(sessions.length).toBeGreaterThan(0);
  const liveStartsAt = new Date(Date.now() - 60_000).toISOString();
  const patch = await request.patch(`${apiBase}/v1/sessions/${sessions[0].session_id}`, {
    headers,
    data: { starts_at: liveStartsAt, duration_minutes: 45, title: "Audit Live Class", status: "scheduled" },
  });
  expect(patch.ok(), await patch.text()).toBeTruthy();

  const assignments = await apiJson(
    request,
    `${apiBase}/v1/courses/${course.id}/assignments?include_drafts=true`,
    headers,
  );
  expect(assignments.length).toBeGreaterThan(0);

  seedCtx = { seed, course, session: sessions[0], assignment: assignments[0] };
});

test("teacher walks the workspace via email+password login", async ({ page }) => {
  const { seed, course, session, assignment } = seedCtx;
  const consoleErrors = collectConsoleErrors(page);

  await page.goto(`${baseURL}/login`, { waitUntil: "domcontentloaded" });
  await expect(page.locator(".auth-composite")).toBeVisible();
  await screenshot(page, "teacher-login");

  await page.locator("input[type=email]").fill(teacherEmail);
  await page.locator("input[type=password]").fill(teacherPassword);
  await page.getByRole("button", { name: "Sign in" }).click();

  await expect(page.locator(".ds-page-header--hero")).toBeVisible({ timeout: 15000 });
  await assertStyleHealth(page, "teacher-dashboard");
  await screenshot(page, "teacher-dashboard");

  const routes = [
    { name: "courses", path: "/courses", selector: ".course-list-page", text: "Audit Course" },
    { name: "course-detail", path: `/courses/${seed.course_slug}`, selector: ".course-detail", text: "Audit Course" },
    { name: "course-people", path: `/courses/${seed.course_slug}/people`, selector: ".course-people", text: "Members" },
    { name: "course-schedule", path: `/courses/${seed.course_slug}/schedule`, selector: ".schedule-agenda", text: "Audit Live Class" },
    { name: "assignments", path: `/courses/${seed.course_slug}/assignments`, selector: ".assignment-shell", text: assignment.title },
    { name: "assignment-detail", path: `/courses/${seed.course_slug}/assignments/${assignment.id}`, selector: ".assignment-shell", text: assignment.title },
    { name: "redeem", path: "/redeem", selector: ".workflow-page", text: "Redeem an enrollment code" },
    { name: "schedule", path: "/schedule", selector: ".schedule-agenda", text: "Audit Live Class" },
    { name: "live-session", path: `/courses/${seed.course_slug}/sessions/${session.session_id}`, selector: ".live-room-shell", text: "Go Live" },
  ];

  for (const route of routes) {
    await navigateSpa(page, route.path);
    await expect(page.locator(route.selector).first()).toBeVisible({ timeout: 15000 });
    await expect(page.locator("body")).toContainText(route.text);
    await assertStyleHealth(page, `teacher-${route.name}`);
    await screenshot(page, `teacher-${route.name}`);
  }

  await navigateSpa(page, `/courses/${seed.course_slug}`);
  await expect(page).toHaveURL(new RegExp(`/courses/${seed.course_slug}$`));
  await navigateSpa(page, "/");
  await expect(page).toHaveURL(new RegExp("/$"));
  await navigateSpa(page, `/courses/${seed.course_slug}`);
  await expect(page).not.toHaveURL(/\/login$/);

  expect(consoleErrors.filter((m) => !isIgnorableConsoleError(m))).toEqual([]);
});

test("student walks the visible subset", async ({ page }) => {
  const { seed, session, assignment } = seedCtx;
  const consoleErrors = collectConsoleErrors(page);

  await page.goto(`${baseURL}/login`, { waitUntil: "domcontentloaded" });
  await page.locator("input[type=email]").fill(studentEmail);
  await page.locator("input[type=password]").fill(studentPassword);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.locator(".ds-page-header--hero")).toBeVisible({ timeout: 15000 });
  await screenshot(page, "student-dashboard");

  const routes = [
    { name: "courses", path: "/courses", selector: ".course-list-page", text: "Audit Course" },
    { name: "course-detail", path: `/courses/${seed.course_slug}`, selector: ".course-detail", text: "Audit Course" },
    { name: "assignments", path: `/courses/${seed.course_slug}/assignments`, selector: ".assignment-shell", text: assignment.title },
    { name: "assignment-detail", path: `/courses/${seed.course_slug}/assignments/${assignment.id}`, selector: ".assignment-shell", text: assignment.title },
    { name: "redeem", path: "/redeem", selector: ".workflow-page", text: "Redeem an enrollment code" },
    { name: "schedule", path: "/schedule", selector: ".schedule-agenda", text: "Audit Live Class" },
    { name: "live-session", path: `/courses/${seed.course_slug}/sessions/${session.session_id}`, selector: ".live-room-shell", text: "Audit Course" },
  ];

  for (const route of routes) {
    await navigateSpa(page, route.path);
    await expect(page.locator(route.selector).first()).toBeVisible({ timeout: 15000 });
    await expect(page.locator("body")).toContainText(route.text);
    await assertStyleHealth(page, `student-${route.name}`);
    await screenshot(page, `student-${route.name}`);
  }

  await navigateSpa(page, `/courses/${seed.course_slug}/sessions/${session.session_id}`);
  await expect(page.getByRole("button", { name: "Go Live" })).toHaveCount(0);
  await navigateSpa(page, "/courses");
  await expect(page.getByRole("button", { name: "New Course" })).toHaveCount(0);

  expect(consoleErrors.filter((m) => !isIgnorableConsoleError(m))).toEqual([]);
});

async function apiJson(request, url, headers) {
  const r = await request.get(url, { headers });
  expect(r.ok(), `${url}: ${await r.text()}`).toBeTruthy();
  return r.json();
}

async function navigateSpa(page, target) {
  await page.evaluate((p) => {
    history.pushState({}, "", p);
    window.dispatchEvent(new PopStateEvent("popstate"));
  }, target);
  await page.waitForLoadState("networkidle").catch(() => {});
}

async function assertStyleHealth(page, name) {
  const result = await page.evaluate(() => {
    const doc = document.documentElement;
    const body = document.body;
    const viewportWidth = doc.clientWidth;
    const viewportHeight = window.innerHeight;
    const els = Array.from(
      document.querySelectorAll("h1,h2,h3,p,a,button,label,input,textarea,select,.ds-card,.dashboard-stat,.schedule-item,.assignment-list__row,.course-card-art"),
    );
    const badBounds = [];
    for (const el of els) {
      if (el.closest("[class^='dx-'], [class*=' dx-']")) continue;
      const rect = el.getBoundingClientRect();
      const style = window.getComputedStyle(el);
      if (style.visibility === "hidden" || style.display === "none" || rect.width < 1 || rect.height < 1 || rect.bottom < 0 || rect.top > viewportHeight) continue;
      if (rect.left < -2 || rect.right > viewportWidth + 2) {
        badBounds.push({ tag: el.tagName.toLowerCase(), className: String(el.className || ""), left: Math.round(rect.left), right: Math.round(rect.right), viewportWidth });
      }
    }
    return {
      overflowX: Math.max(doc.scrollWidth, body.scrollWidth) - viewportWidth,
      bodyTextLength: (body.innerText || "").trim().length,
      badBounds,
    };
  });
  expect(result.bodyTextLength, `${name} text`).toBeGreaterThan(20);
  expect(result.overflowX, `${name} overflow`).toBeLessThanOrEqual(2);
  expect(result.badBounds, `${name} bounds`).toEqual([]);
}

async function screenshot(page, name) {
  await page.screenshot({ path: path.join(screenshotDir, `${name}.png`), fullPage: true });
}

function collectConsoleErrors(page) {
  const errors = [];
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  page.on("pageerror", (e) => errors.push(e.message));
  return errors;
}

function isIgnorableConsoleError(message) {
  return /favicon|Firebase auth initialization failed|ERR_ABORTED|ResizeObserver loop limit exceeded|WebSocket connection to.*failed|HTTP Authentication failed|^unreachable$/i.test(message);
}

test("teacher starts an instant session from the course page", async ({ page, request }) => {
  const { course, seed } = seedCtx;
  const consoleErrors = collectConsoleErrors(page);

  // Sign in as teacher via the existing local-login UI.
  await page.goto(`${baseURL}/login`, { waitUntil: "domcontentloaded" });
  await page.locator("input[type=email]").fill(teacherEmail);
  await page.locator("input[type=password]").fill(teacherPassword);
  await page.getByRole("button", { name: "Sign in" }).click();
  await page.waitForURL(/\/$/);

  // End any in-progress session from earlier tests so the conflict
  // check doesn't trip. Best-effort.
  const tokenResp = await request.post(`${apiBase}/v1/auth/local-login`, {
    data: { email: teacherEmail, password: teacherPassword },
  });
  const { id_token: teacherToken } = await tokenResp.json();
  const active = await apiJson(
    request,
    `${apiBase}/v1/courses/${course.id}/active-session`,
    { Authorization: `Bearer ${teacherToken}` },
  );
  if (active && active.active) {
    await request.post(`${apiBase}/v1/sessions/${active.active.session_id}/end-class`, {
      headers: { Authorization: `Bearer ${teacherToken}` },
    });
  }

  await page.goto(`${baseURL}/courses/${seed.course_slug}`);
  await expect(page.getByRole("button", { name: "Start session now" })).toBeVisible();
  await screenshot(page, "instant-session-button");

  // Click the primary; expect navigation to /courses/{slug}/sessions/{id}.
  await page.getByRole("button", { name: "Start session now" }).click();
  await page.waitForURL(new RegExp(`/courses/${seed.course_slug}/sessions/[0-9a-f-]+`));
  await screenshot(page, "instant-session-broadcast");

  // Verify a `live` row exists for this course.
  const verify = await apiJson(
    request,
    `${apiBase}/v1/courses/${course.id}/active-session`,
    { Authorization: `Bearer ${teacherToken}` },
  );
  expect(verify.active).not.toBeNull();
  expect(verify.active.title).toContain("Quick session");

  expect(consoleErrors, consoleErrors.join("\n")).toEqual([]);
});

test("student sees Live now banner within ~20s of teacher starting", async ({ browser, request }) => {
  const { course, seed } = seedCtx;
  const tokenResp = await request.post(`${apiBase}/v1/auth/local-login`, {
    data: { email: teacherEmail, password: teacherPassword },
  });
  const { id_token: teacherToken } = await tokenResp.json();

  // Reset to a known state: end any in-progress session.
  const cur = await apiJson(
    request,
    `${apiBase}/v1/courses/${course.id}/active-session`,
    { Authorization: `Bearer ${teacherToken}` },
  );
  if (cur && cur.active) {
    await request.post(`${apiBase}/v1/sessions/${cur.active.session_id}/end-class`, {
      headers: { Authorization: `Bearer ${teacherToken}` },
    });
  }

  // Student context: open the course page first, before the teacher starts.
  const studentCtx = await browser.newContext();
  const studentPage = await studentCtx.newPage();
  await studentPage.goto(`${baseURL}/login`, { waitUntil: "domcontentloaded" });
  await studentPage.locator("input[type=email]").fill(studentEmail);
  await studentPage.locator("input[type=password]").fill(studentPassword);
  await studentPage.getByRole("button", { name: "Sign in" }).click();
  await studentPage.waitForURL(/\/$/);
  await studentPage.goto(`${baseURL}/courses/${seed.course_slug}`);

  // Confirm banner is NOT visible at first.
  await expect(studentPage.locator(".live-now-banner")).toHaveCount(0);

  // Teacher triggers start-now via API (faster than driving the UI).
  const startResp = await request.post(
    `${apiBase}/v1/courses/${course.id}/sessions/start-now`,
    { headers: { Authorization: `Bearer ${teacherToken}` }, data: {} },
  );
  expect(startResp.ok(), await startResp.text()).toBeTruthy();

  // Banner should appear within ~20s (15s poll + slack).
  await expect(studentPage.locator(".live-now-banner")).toBeVisible({ timeout: 25_000 });
  await expect(studentPage.getByText("Live now")).toBeVisible();
  await screenshot(studentPage, "live-now-banner");

  await studentCtx.close();
});
