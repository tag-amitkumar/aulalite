(function () {
  "use strict";

  var ROOT_SELECTOR = ".marketing-page";
  var GSAP_SOURCE = "/assets/vendor/gsap.min.js";
  var SCROLL_TRIGGER_SOURCE = "/assets/vendor/ScrollTrigger.min.js";
  var activeRoot = null;
  var activeCleanup = null;
  var loadPromise = null;
  var generation = 0;

  function loadScript(source, marker) {
    var existing = document.querySelector('script[data-aulalite-motion="' + marker + '"]');
    if (existing) {
      if (existing.dataset.loaded === "true") return Promise.resolve();
      return new Promise(function (resolve, reject) {
        existing.addEventListener("load", resolve, { once: true });
        existing.addEventListener("error", reject, { once: true });
      });
    }

    return new Promise(function (resolve, reject) {
      var script = document.createElement("script");
      script.src = source;
      script.async = true;
      script.dataset.aulaliteMotion = marker;
      script.addEventListener(
        "load",
        function () {
          script.dataset.loaded = "true";
          resolve();
        },
        { once: true }
      );
      script.addEventListener("error", reject, { once: true });
      document.head.appendChild(script);
    });
  }

  function ensureMotionLibrary() {
    if (window.gsap && window.ScrollTrigger) return Promise.resolve();
    if (loadPromise) return loadPromise;

    loadPromise = loadScript(GSAP_SOURCE, "gsap")
      .then(function () {
        return loadScript(SCROLL_TRIGGER_SOURCE, "scroll-trigger");
      })
      .catch(function (error) {
        loadPromise = null;
        throw error;
      });

    return loadPromise;
  }

  function initialize(root, currentGeneration) {
    var reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
    if (reducedMotion.matches) {
      root.dataset.motionState = "reduced";
      root.classList.add("marketing-reduced-motion");
      return;
    }

    root.dataset.motionState = "loading";

    ensureMotionLibrary()
      .then(function () {
        if (
          currentGeneration !== generation ||
          root !== activeRoot ||
          !root.isConnected
        ) {
          return;
        }

        var gsap = window.gsap;
        var ScrollTrigger = window.ScrollTrigger;
        gsap.registerPlugin(ScrollTrigger);

        var media = gsap.matchMedia();
        var imageListeners = [];
        var context = gsap.context(function () {
          var intro = gsap.timeline({ defaults: { ease: "power3.out" } });
          intro
            .from(".marketing-nav", {
              y: -24,
              opacity: 0,
              duration: 0.8,
            })
            .from(
              ".marketing-hero__headline",
              { y: 64, opacity: 0, duration: 1.05 },
              "-=0.48"
            )
            .from(
              ".marketing-hero__lede, .marketing-hero__actions",
              { y: 28, opacity: 0, duration: 0.8, stagger: 0.1 },
              "-=0.62"
            )
            .from(
              ".marketing-hero__visual",
              { x: 90, scale: 0.9, opacity: 0, duration: 1.15 },
              "-=0.9"
            );

          gsap.utils
            .toArray("[data-motion-image]", root)
            .filter(function (element) {
              return !element.closest(".marketing-hero");
            })
            .forEach(function (element) {
              gsap
                .timeline({
                  scrollTrigger: {
                    trigger: element,
                    start: "top 92%",
                    end: "bottom 8%",
                    scrub: 1.1,
                  },
                })
                .fromTo(
                  element,
                  { scale: 0.84, opacity: 0.34 },
                  { scale: 1, opacity: 1, duration: 0.56, ease: "none" }
                )
                .to(element, {
                  scale: 1.025,
                  opacity: 0.22,
                  duration: 0.44,
                  ease: "none",
                });
            });

          media.add("(min-width: 833px)", function () {
            var cards = gsap.utils.toArray(".marketing-stack-card", root);

            cards.forEach(function (card, index) {
              gsap.fromTo(
                card,
                { y: 120, scale: 0.92, opacity: 0.4 },
                {
                  y: 0,
                  scale: 1,
                  opacity: 1,
                  ease: "none",
                  scrollTrigger: {
                    trigger: card,
                    start: "top 94%",
                    end: "top 58%",
                    scrub: 1,
                  },
                }
              );

              var nextCard = cards[index + 1];
              if (nextCard) {
                gsap.to(card, {
                  scale: 0.94,
                  opacity: 0.62,
                  filter: "brightness(0.72)",
                  ease: "none",
                  scrollTrigger: {
                    trigger: nextCard,
                    start: "top 76%",
                    end: "top 24%",
                    scrub: 1,
                  },
                });
              }
            });
          });
        }, root);

        function refresh() {
          if (root.isConnected) ScrollTrigger.refresh();
        }

        root.querySelectorAll("img").forEach(function (image) {
          if (image.complete) return;
          image.addEventListener("load", refresh, { once: true });
          image.addEventListener("error", refresh, { once: true });
          imageListeners.push(image);
        });

        if (document.fonts && document.fonts.ready) {
          document.fonts.ready.then(refresh);
        }

        root.dataset.motionState = "ready";
        ScrollTrigger.refresh();

        activeCleanup = function () {
          imageListeners.forEach(function (image) {
            image.removeEventListener("load", refresh);
            image.removeEventListener("error", refresh);
          });
          media.revert();
          context.revert();
          root.removeAttribute("data-motion-state");
        };
      })
      .catch(function () {
        if (root.isConnected) root.dataset.motionState = "unavailable";
      });
  }

  function clearActiveRoot() {
    generation += 1;
    if (activeCleanup) activeCleanup();
    activeCleanup = null;
    activeRoot = null;
  }

  function scan() {
    if (activeRoot && !activeRoot.isConnected) clearActiveRoot();

    var root = document.querySelector(ROOT_SELECTOR);
    if (!root || root === activeRoot) return;

    clearActiveRoot();
    activeRoot = root;
    generation += 1;
    initialize(root, generation);
  }

  function boot() {
    scan();
    var observer = new MutationObserver(scan);
    observer.observe(document.documentElement, { childList: true, subtree: true });
    window.addEventListener("pagehide", clearActiveRoot, { once: true });
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", boot, { once: true });
  } else {
    boot();
  }
})();
