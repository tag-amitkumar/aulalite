// tools/design_system_smoke.spec.js
// Smoke spec for the v1 design system kitchen sink. Requires the shell-web dev
// server to be running on the URL set in DS_KITCHEN_URL (default http://localhost:8080).
//
// Run: npx playwright test tools/design_system_smoke.spec.js

const { test, expect } = require("@playwright/test");

const BASE = process.env.DS_KITCHEN_URL || "http://localhost:8080";

test.describe("design system kitchen sink", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(`${BASE}/dev/components`, { waitUntil: "load" });
  });

  test("page renders all primitive sections", async ({ page }) => {
    await expect(page.locator("h1", { hasText: "Design System Kitchen Sink" })).toBeVisible();
    for (const heading of [
      "Buttons",
      "Cards",
      "Inputs & fields",
      "Badges",
      "Table",
      "Tabs",
      "Dropdown menu",
      "Sheet",
      "Skeletons",
      "Toast",
    ]) {
      await expect(page.locator("h2", { hasText: heading })).toBeVisible();
    }
  });

  test("primary button has the v1 class", async ({ page }) => {
    await expect(page.locator(".ds-button--primary").first()).toBeVisible();
  });

  test("focus ring becomes visible on tab to first button", async ({ page }) => {
    await page.locator(".ds-button--primary").first().focus();
    const shadow = await page.locator(".ds-button--primary").first().evaluate(
      (el) => getComputedStyle(el).boxShadow,
    );
    expect(shadow).not.toBe("none");
  });

  test("dropdown opens and emits content with data-state", async ({ page }) => {
    const trigger = page.locator("button", { hasText: "Open menu" }).first();
    await trigger.click();
    await expect(page.locator(".ds-dropdown-content[data-state=\"open\"]")).toBeVisible();
    await expect(page.locator(".ds-dropdown-item", { hasText: "Sign out" })).toBeVisible();
  });

  test("sheet opens, escape closes", async ({ page }) => {
    const open = page.locator("button", { hasText: "Open sheet" }).first();
    await open.click();
    const panel = page.locator(".ds-sheet").first();
    await expect(panel).toBeVisible();
    await panel.focus();
    await page.keyboard.press("Escape");
    await expect(panel).toBeHidden();
  });

  test("reduced motion honoured", async ({ browser }) => {
    const context = await browser.newContext({ reducedMotion: "reduce" });
    const page = await context.newPage();
    await page.goto(`${BASE}/dev/components`, { waitUntil: "load" });
    const dur = await page.locator(".ds-button--primary").first().evaluate(
      (el) => getComputedStyle(el).animationDuration,
    );
    // Reduced motion CSS forces animation-duration: 0.01ms !important.
    expect(dur).toBe("0.01ms");
    await context.close();
  });

  test("kitchen sink screenshot", async ({ page }) => {
    await page.locator("h1").first().waitFor();
    await page.waitForFunction(() => {
      const els = document.querySelectorAll(".ds-page-enter > [data-stagger]");
      return Array.from(els).every((el) => getComputedStyle(el).opacity === "1");
    }, null, { timeout: 5000 });
    expect(await page.screenshot({ fullPage: true })).toMatchSnapshot("design-system-kitchen-sink.png", {
      maxDiffPixelRatio: 0.02,
    });
  });
});
