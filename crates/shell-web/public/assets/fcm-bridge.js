// Foreground Firebase Cloud Messaging bridge.
//
// Installs a small global `window.aula.fcm` that Rust (wasm) calls via
// wasm-bindgen. Mirrors the structure of firebase-bridge.js (auth): the global
// is installed synchronously, and the Firebase Messaging SDK is lazily imported
// on first use.
//
// `requestToken()` is the only entry point the app needs:
//   1. bail out with a status object if push is unconfigured or unsupported
//   2. register the background service worker (/firebase-messaging-sw.js)
//   3. ask the browser for Notification permission
//   4. on grant, fetch an FCM registration token with the configured VAPID key
// It returns { status: "token", token } or a graceful failure status object.
(function () {
  const firebaseConfig = {
    apiKey: window.__AULALITE_FIREBASE_API_KEY__,
    authDomain: window.__AULALITE_FIREBASE_AUTH_DOMAIN__,
    projectId: window.__AULALITE_FIREBASE_PROJECT_ID__,
  };

  let messagingPromise;

  // Lazily import firebase-app + firebase-messaging (ESM builds) and return a
  // { messaging, messagingModule } pair. Reuses the singleton across calls.
  function loadMessaging() {
    if (!messagingPromise) {
      messagingPromise = Promise.all([
        import("https://www.gstatic.com/firebasejs/10.13.0/firebase-app.js"),
        import("https://www.gstatic.com/firebasejs/10.13.0/firebase-messaging.js"),
      ]).then(([appModule, messagingModule]) => {
        const app = appModule.getApps().length
          ? appModule.getApp()
          : appModule.initializeApp(firebaseConfig);
        const messaging = messagingModule.getMessaging(app);
        // Best-effort: surface foreground messages as a window event so the app
        // can react (e.g. refresh the bell). Optional — failures are ignored.
        try {
          messagingModule.onMessage(messaging, (payload) => {
            window.dispatchEvent(
              new CustomEvent("aula:fcm-message", { detail: payload })
            );
          });
        } catch (_) {
          /* onMessage wiring is best-effort */
        }
        return { messaging, messagingModule };
      });
    }
    return messagingPromise;
  }

  window.aula = window.aula || {};
  window.aula.fcm = {
    // Returns an FCM registration token status object, preserving graceful setup
    // failures so the UI can show a specific message.
    async requestToken() {
      const vapidKey = window.__AULALITE_FCM_VAPID_KEY__;
      // No VAPID key => push is not configured. Degrade gracefully.
      if (!vapidKey) {
        return { status: "missing_vapid_key" };
      }
      // Feature support check (older browsers / no service worker support).
      if (
        typeof Notification === "undefined" ||
        !("serviceWorker" in navigator)
      ) {
        return { status: "unsupported" };
      }

      // Register the background service worker at the origin root.
      let registration;
      try {
        registration = await navigator.serviceWorker.register(
          "/firebase-messaging-sw.js",
          { scope: "/firebase-cloud-messaging-push-scope" }
        );
      } catch (error) {
        console.error("FCM service worker registration failed", error);
        return { status: "service_worker_failed" };
      }

      // Ask for (or confirm) notification permission.
      const permission = await Notification.requestPermission();
      if (permission !== "granted") {
        return { status: "permission_denied" };
      }

      try {
        const { messaging, messagingModule } = await loadMessaging();
        const token = await messagingModule.getToken(messaging, {
          vapidKey,
          serviceWorkerRegistration: registration,
        });
        return token ? { status: "token", token } : { status: "token_failed" };
      } catch (error) {
        console.error("FCM getToken failed", error);
        return { status: "token_failed" };
      }
    },
  };
})();
