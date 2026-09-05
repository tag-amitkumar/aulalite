// playwright.config.js
module.exports = {
  testDir: ".",
  use: { headless: true, viewport: { width: 1440, height: 1000 } },
  reporter: [["list"], ["html", { open: "never" }]],
};
