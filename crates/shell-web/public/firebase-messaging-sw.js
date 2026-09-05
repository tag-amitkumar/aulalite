// Firebase Cloud Messaging service worker (background push).
//
// Served at the origin root (/firebase-messaging-sw.js) so it can control the
// whole scope. Uses the Firebase *compat* SDK because service workers cannot
// use ES-module imports portably; importScripts loads the classic builds.
//
// The config below is the same PUBLISHABLE runtime config the page loads before
// its auth bridge. There are no server secrets here. The production container
// generates /runtime-config.js from environment variables at startup; direct
// local serving keeps an empty placeholder and disables background push.
//
// Background messages (received while the tab is closed/backgrounded) are
// surfaced via showNotification. Foreground messages are handled by the page
// itself (see fcm-bridge.js onMessage).
/* eslint-disable no-undef */

importScripts("/runtime-config.js");

const firebaseConfig = {
  apiKey: self.__AULALITE_FIREBASE_API_KEY__,
  authDomain: self.__AULALITE_FIREBASE_AUTH_DOMAIN__,
  projectId: self.__AULALITE_FIREBASE_PROJECT_ID__,
  storageBucket: self.__AULALITE_FIREBASE_STORAGE_BUCKET__,
  messagingSenderId: self.__AULALITE_FIREBASE_MESSAGING_SENDER_ID__,
  appId: self.__AULALITE_FIREBASE_APP_ID__,
  measurementId: self.__AULALITE_FIREBASE_MEASUREMENT_ID__,
};

const firebaseConfigured =
  firebaseConfig.apiKey &&
  firebaseConfig.authDomain &&
  firebaseConfig.projectId;

if (firebaseConfigured) {
  importScripts(
    "https://www.gstatic.com/firebasejs/10.13.0/firebase-app-compat.js"
  );
  importScripts(
    "https://www.gstatic.com/firebasejs/10.13.0/firebase-messaging-compat.js"
  );

  firebase.initializeApp(firebaseConfig);
  const messaging = firebase.messaging();

  messaging.onBackgroundMessage(function (payload) {
    const notification = payload.notification || {};
    const title = notification.title || "AulaLite";
    const options = {
      body: notification.body || "",
      icon: "/assets/brand/aulalite-mark.svg",
      badge: "/assets/brand/aulalite-mark.svg",
      data: payload.data || {},
    };
    return self.registration.showNotification(title, options);
  });
} else {
  // Direct local `dx serve` intentionally ships an empty runtime config. Avoid
  // throwing during worker evaluation; the page reports push as unconfigured.
  console.info("AulaLite background push is not configured.");
}

function sameOriginNotificationPath(data) {
  const candidate = data && (data.link || data.url || data.path);
  if (!candidate) return "/";

  try {
    const url = new URL(candidate, self.location.origin);
    if (url.origin !== self.location.origin) return "/";
    return `${url.pathname}${url.search}${url.hash}`;
  } catch (_) {
    return "/";
  }
}

self.addEventListener("notificationclick", function (event) {
  event.notification.close();
  const targetPath = sameOriginNotificationPath(event.notification.data || {});
  const targetUrl = new URL(targetPath, self.location.origin).href;

  event.waitUntil(
    self.clients
      .matchAll({ type: "window", includeUncontrolled: true })
      .then(async function (windows) {
        const exact = windows.find((client) => client.url === targetUrl);
        if (exact && "focus" in exact) return exact.focus();

        const existing = windows.find((client) => {
          try {
            return new URL(client.url).origin === self.location.origin;
          } catch (_) {
            return false;
          }
        });
        if (existing && "navigate" in existing && "focus" in existing) {
          await existing.navigate(targetUrl);
          return existing.focus();
        }
        if (self.clients.openWindow) return self.clients.openWindow(targetPath);
        return undefined;
      })
  );
});
