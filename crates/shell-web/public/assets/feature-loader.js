// Route-level browser feature loader.
//
// Keep large, specialist dependencies out of the initial app shell. Rust calls
// these small stable functions only when the corresponding experience is
// mounted. A single promise is shared across callers so concurrent renders do
// not download or evaluate the same module twice.
(function () {
  "use strict";

  window.aula = window.aula || {};
  if (window.aula.features) return;

  let hlsModulePromise;
  const hlsPlayers = new Map();

  function loadHlsModule() {
    if (!hlsModulePromise) {
      hlsModulePromise = import("/vendor/hls.js")
        .then((module) => {
          const Hls = module.default;
          window.Hls = Hls;
          return Hls;
        })
        .catch((error) => {
          // Let a later reconnect retry a transient module fetch failure.
          hlsModulePromise = undefined;
          throw error;
        });
    }
    return hlsModulePromise;
  }

  function detachHls(videoId) {
    const current = hlsPlayers.get(videoId);
    if (!current) return;

    hlsPlayers.delete(videoId);
    if (current.hls) {
      current.hls.destroy();
    }

    // Release decoder/network resources when the live-room surface unmounts.
    const video = current.video;
    if (video && video.isConnected) {
      video.pause();
      video.removeAttribute("src");
      video.load();
    }
  }

  async function attachHls(videoId, source) {
    const video = document.getElementById(videoId);
    if (!(video instanceof HTMLVideoElement)) {
      throw new Error(`HLS video element not found: ${videoId}`);
    }
    if (!source) {
      throw new Error("HLS source is empty");
    }

    const existing = hlsPlayers.get(videoId);
    if (existing && existing.video === video && existing.source === source) {
      return existing.mode;
    }
    detachHls(videoId);

    // Safari and iOS have a more efficient native HLS pipeline; avoid loading
    // the JavaScript player entirely there.
    if (video.canPlayType("application/vnd.apple.mpegurl")) {
      video.src = source;
      video.load();
      hlsPlayers.set(videoId, { video, source, mode: "native" });
      return "native";
    }

    // Mark this request while the module downloads. If the component unmounts
    // or the source changes, its token is removed/replaced and we abort safely.
    const token = {};
    hlsPlayers.set(videoId, { video, source, mode: "loading", token });
    const Hls = await loadHlsModule();
    const pending = hlsPlayers.get(videoId);
    if (!pending || pending.token !== token) return "cancelled";
    if (!Hls || !Hls.isSupported()) {
      hlsPlayers.delete(videoId);
      throw new Error("HLS playback is unsupported in this browser");
    }

    const hls = new Hls({
      enableWorker: true,
      lowLatencyMode: true,
      backBufferLength: 30,
      maxBufferLength: 30,
    });
    hlsPlayers.set(videoId, { video, source, mode: "hls.js", hls });
    hls.loadSource(source);
    hls.attachMedia(video);
    return "hls.js";
  }

  window.aula.features = Object.freeze({ attachHls, detachHls });
})();
