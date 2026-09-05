// crates/shell-web/public/assets/scorm-bridge.js
//
// SCORM 1.2 + 2004 JS runtime bridge (Tool side).
//
// A SCORM SCO (Sharable Content Object) running in an iframe discovers the LMS
// runtime by walking up the frame hierarchy looking for `window.API` (SCORM 1.2)
// or `window.API_1484_11` (SCORM 2004). This script installs BOTH on the page
// that hosts the player iframe, so a SAME-ORIGIN-served SCO finds them via
// `window.parent.API` / `window.parent.API_1484_11`.
//
// The runtime keeps the CMI model in memory as a flat key→value map and persists
// it to the AulaLite backend (`PUT /v1/scorm/:id/cmi`) on LMSCommit / LMSFinish,
// hydrating from `GET /v1/scorm/:id/cmi` at init. Configure it before loading the
// SCO by setting:
//
//   window.__SCORM__ = {
//     cmiUrl:   "/v1/scorm/<id>/cmi",  // backend CMI load/save endpoint
//     token:    "<bearer>",            // Firebase/app bearer for the fetch
//     initial:  { ... },               // optional pre-fetched CMI blob
//     onStatus: function (s) {}        // optional UI status callback
//   };
//
// then call `window.aulaScorm.install()`. The Dioxus ScormPlayer component does
// this wiring; absent config the bridge degrades to a pure in-memory store (the
// SCO still runs; nothing is persisted).
(function () {
    "use strict";

    function cfg() {
        return window.__SCORM__ || {};
    }

    function status(msg) {
        var c = cfg();
        if (typeof c.onStatus === "function") {
            try { c.onStatus(msg); } catch (_) {}
        }
    }

    // --- CMI store -------------------------------------------------------
    // Flat element-path → string map (e.g. "cmi.core.lesson_status": "completed").
    function makeStore() {
        var data = {};
        var dirty = false;

        // Seed sensible SCORM defaults so an SCO's first GetValue doesn't fail.
        function seedDefaults() {
            if (!("cmi.core.lesson_status" in data)) {
                data["cmi.core.lesson_status"] = "not attempted";
            }
            if (!("cmi.completion_status" in data)) {
                data["cmi.completion_status"] = "unknown";
            }
        }

        return {
            hydrate: function (blob) {
                if (blob && typeof blob === "object") {
                    Object.keys(blob).forEach(function (k) {
                        data[k] = String(blob[k]);
                    });
                }
                seedDefaults();
            },
            get: function (key) {
                return key in data ? data[key] : "";
            },
            set: function (key, val) {
                data[key] = String(val);
                dirty = true;
            },
            snapshot: function () {
                return data;
            },
            isDirty: function () {
                return dirty;
            },
            markClean: function () {
                dirty = false;
            }
        };
    }

    var store = makeStore();
    var initialized = false;
    var lastError = "0";

    // --- backend sync ----------------------------------------------------
    function loadFromBackend() {
        var c = cfg();
        if (c.initial) {
            store.hydrate(c.initial);
            return Promise.resolve();
        }
        if (!c.cmiUrl) {
            store.hydrate({});
            return Promise.resolve();
        }
        var headers = { "content-type": "application/json" };
        if (c.token) headers["authorization"] = "Bearer " + c.token;
        return fetch(c.cmiUrl, { method: "GET", headers: headers })
            .then(function (r) { return r.ok ? r.json() : {}; })
            .then(function (blob) { store.hydrate(blob); })
            .catch(function () { store.hydrate({}); });
    }

    function saveToBackend() {
        var c = cfg();
        if (!c.cmiUrl) return Promise.resolve(true);
        if (!store.isDirty()) return Promise.resolve(true);
        var headers = { "content-type": "application/json" };
        if (c.token) headers["authorization"] = "Bearer " + c.token;
        var payload = JSON.stringify(store.snapshot());
        // Use keepalive so a commit fired during page unload still flushes.
        return fetch(c.cmiUrl, {
            method: "PUT",
            headers: headers,
            body: payload,
            keepalive: true
        })
            .then(function (r) {
                if (r.ok) { store.markClean(); status("saved"); return true; }
                status("save-failed");
                return false;
            })
            .catch(function () { status("save-failed"); return false; });
    }

    // --- SCORM 1.2 API (window.API) -------------------------------------
    var API = {
        LMSInitialize: function () {
            initialized = true;
            lastError = "0";
            status("initialized");
            return "true";
        },
        LMSFinish: function () {
            saveToBackend();
            initialized = false;
            status("finished");
            return "true";
        },
        LMSGetValue: function (key) {
            lastError = "0";
            return store.get(key);
        },
        LMSSetValue: function (key, val) {
            store.set(key, val);
            lastError = "0";
            return "true";
        },
        LMSCommit: function () {
            saveToBackend();
            lastError = "0";
            return "true";
        },
        LMSGetLastError: function () { return lastError; },
        LMSGetErrorString: function () { return lastError === "0" ? "No error" : "Error"; },
        LMSGetDiagnostic: function () { return ""; }
    };

    // --- SCORM 2004 API (window.API_1484_11) ----------------------------
    var API_1484_11 = {
        Initialize: function () {
            initialized = true;
            lastError = "0";
            status("initialized");
            return "true";
        },
        Terminate: function () {
            saveToBackend();
            initialized = false;
            status("finished");
            return "true";
        },
        GetValue: function (key) {
            lastError = "0";
            return store.get(key);
        },
        SetValue: function (key, val) {
            store.set(key, val);
            lastError = "0";
            return "true";
        },
        Commit: function () {
            saveToBackend();
            lastError = "0";
            return "true";
        },
        GetLastError: function () { return lastError; },
        GetErrorString: function () { return lastError === "0" ? "No error" : "Error"; },
        GetDiagnostic: function () { return ""; }
    };

    function install() {
        return loadFromBackend().then(function () {
            window.API = API;            // SCORM 1.2 discovery target
            window.API_1484_11 = API_1484_11; // SCORM 2004 discovery target
            status("ready");
            // Best-effort flush if the learner closes the tab mid-activity.
            window.addEventListener("pagehide", function () { saveToBackend(); });
            return true;
        });
    }

    window.aulaScorm = {
        install: install,
        // Exposed for the player UI / debugging.
        snapshot: function () { return store.snapshot(); },
        isInitialized: function () { return initialized; }
    };
})();
