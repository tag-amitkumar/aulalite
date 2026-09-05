// Security-policy-compatible document bootstrap. This file intentionally runs
// synchronously in <head> so the first paint uses the saved theme and locale.
(function () {
  "use strict";

  let theme = "system";
  let locale = "en";
  try {
    theme = localStorage.getItem("aula-theme") || "system";
    locale = localStorage.getItem("aula-locale") || "en";
  } catch (_) {
    // Storage can be unavailable in hardened/private browser contexts.
  }

  const media = window.matchMedia
    ? window.matchMedia("(prefers-color-scheme: dark)")
    : null;
  function applyTheme() {
    const dark = theme === "dark" || (theme !== "light" && media && media.matches);
    document.documentElement.setAttribute("data-ui-theme", dark ? "dark" : "light");
  }
  applyTheme();
  if (media) {
    media.addEventListener("change", function () {
      try {
        theme = localStorage.getItem("aula-theme") || "system";
      } catch (_) {
        theme = "system";
      }
      if (theme === "system") applyTheme();
    });
  }

  const rtlLocales = new Set(["ar"]);
  document.documentElement.setAttribute("lang", locale);
  document.documentElement.setAttribute("dir", rtlLocales.has(locale) ? "rtl" : "ltr");

  document.addEventListener("click", function (event) {
    if (event.defaultPrevented || event.button !== 0) return;
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    const anchor = event.target && event.target.closest && event.target.closest("a");
    if (!anchor || (anchor.target && anchor.target !== "_self")) return;
    if (anchor.hasAttribute("download")) return;
    const rel = anchor.getAttribute("rel") || "";
    if (rel.split(/\s+/).includes("external")) return;
    const href = anchor.getAttribute("href");
    if (!href || href.startsWith("#")) return;
    let url;
    try {
      url = new URL(href, location.href);
    } catch (_) {
      return;
    }
    if (url.origin !== location.origin) return;
    event.preventDefault();
    const path = url.pathname + url.search + url.hash;
    if (path !== location.pathname + location.search + location.hash) {
      history.pushState({}, "", path);
    }
    window.dispatchEvent(new PopStateEvent("popstate"));
  });

  if ("serviceWorker" in navigator) {
    const version = new URL(document.currentScript.src).searchParams.get("v") || "dev";
    window.addEventListener("load", function () {
      navigator.serviceWorker
        .register(`/service-worker.js?v=${encodeURIComponent(version)}`, { scope: "/" })
        .catch(function (error) {
          console.warn("SW registration failed", error);
        });
    });
  }
})();
