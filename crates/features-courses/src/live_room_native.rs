//! Native WebView live-classroom runtime.
//!
//! Dioxus desktop and mobile render into a real WebView. Those targets do not
//! expose `web_sys` to Rust, but their document still provides the browser
//! media, WebRTC, WebSocket and SVG APIs. This module keeps all browser object
//! ownership inside the WebView and talks to it through `document::eval`.
//!
//! The bridge is deliberately request/response based: Rust sends structured
//! JSON through the Dioxus channel (never interpolated into JavaScript), input
//! and SDP bodies are bounded, network operations time out, and errors are
//! reduced to stable user-facing messages so credentials/SDP never leak into
//! logs or UI.

#![cfg(not(target_arch = "wasm32"))]

use dioxus::prelude::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const MAX_REQUEST_BYTES: usize = 64 * 1024;

/// JavaScript installed once per WebView. Re-evaluating the loader is cheap:
/// it reuses `window.__aulaLiveNative` and therefore preserves peer
/// connections and capture streams between Rust calls.
const RUNTIME: &str = r##"
const AULA_LIVE_NATIVE_VERSION = 1;
if (!window.__aulaLiveNative || window.__aulaLiveNative.version !== AULA_LIVE_NATIVE_VERSION) {
  const peers = new Map();
  const viewers = new Map();
  const streams = new Map();
  const hlsPlayers = new Map();
  const MAX_SDP_BYTES = 262144;
  const MAX_MESSAGE_BYTES = 65536;
  const REQUEST_TIMEOUT_MS = 15000;

  const utf8Bytes = (value) => new TextEncoder().encode(String(value || "")).byteLength;
  const bounded = (value, max, label) => {
    const text = String(value || "");
    if (utf8Bytes(text) > max) throw new Error(`${label} exceeded the safe size limit`);
    return text;
  };
  const safeError = (error) => {
    const name = String(error && error.name || "");
    if (name === "NotAllowedError" || name === "SecurityError") {
      return "Camera, microphone, or screen access was denied. Allow it in system settings and try again.";
    }
    if (name === "NotFoundError" || name === "OverconstrainedError") {
      return "The selected camera or microphone is unavailable.";
    }
    if (name === "NotReadableError") {
      return "The camera or microphone is busy in another application.";
    }
    if (name === "AbortError" || name === "TimeoutError") {
      return "The media server did not respond in time.";
    }
    if (name === "NotSupportedError") {
      return "This device does not support the requested live-classroom feature.";
    }
    const message = String(error && error.message || "Live-classroom operation failed");
    const clean = message.replace(/[\r\n\t]+/g, " ");
    return clean.length > 512 ? `${clean.slice(0, 509)}...` : clean;
  };
  const requireMedia = () => {
    if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
      throw new DOMException("Media capture is unavailable", "NotSupportedError");
    }
    return navigator.mediaDevices;
  };
  const stopTracks = (stream) => {
    if (stream && stream.getTracks) stream.getTracks().forEach((track) => track.stop());
  };
  const isLoopbackHost = (hostname) => hostname === "localhost"
    || hostname === "::1"
    || hostname === "10.0.2.2"
    || /^127(?:\.[0-9]{1,3}){3}$/.test(hostname);
  const secureHttpUrl = (value, allowInsecureLoopback, label) => {
    bounded(value, 4096, label);
    let parsed;
    try { parsed = new URL(value); } catch (_) { throw new Error(`${label} is invalid`); }
    const debugLoopback = allowInsecureLoopback === true
      && parsed.protocol === "http:"
      && isLoopbackHost(parsed.hostname);
    if (parsed.protocol !== "https:" && !debugLoopback) {
      throw new Error(`${label} must use HTTPS`);
    }
    return parsed;
  };
  const attach = (elementId, stream, muted) => {
    let element = document.getElementById(elementId);
    if (!element && String(elementId).startsWith("live-room-student-audio-")) {
      element = document.createElement("audio");
      element.id = elementId;
      element.autoplay = true;
      element.setAttribute("aria-hidden", "true");
      element.style.display = "none";
      document.body.appendChild(element);
    }
    if (!(element instanceof HTMLMediaElement)) throw new Error("The live video surface is unavailable");
    element.srcObject = stream;
    element.muted = !!muted;
    element.playsInline = true;
    const started = element.play();
    if (started && started.catch) started.catch(() => {});
  };
  const detach = (elementId) => {
    const element = document.getElementById(elementId);
    if (element instanceof HTMLMediaElement) {
      element.pause();
      element.srcObject = null;
      element.removeAttribute("src");
      element.load();
      if (String(elementId).startsWith("live-room-student-audio-")) element.remove();
    }
  };
  const closeResource = async (entry) => {
    if (!entry) return;
    if (entry.meterTimer) clearInterval(entry.meterTimer);
    if (entry.audioContext) await entry.audioContext.close().catch(() => {});
    stopTracks(entry.stream);
    if (entry.pc) entry.pc.close();
    if (entry.resourceUrl && entry.resourceOrigin && entry.authorization) {
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), 5000);
      try {
        const resource = new URL(entry.resourceUrl);
        if (resource.origin === entry.resourceOrigin) {
          await fetch(resource.href, {
            method: "DELETE",
            headers: { "Authorization": bounded(entry.authorization, 32768, "credential") },
            cache: "no-store",
            signal: controller.signal,
          });
        }
      } catch (_) {}
      clearTimeout(timer);
    }
  };
  const replaceEntry = async (map, key, entry) => {
    await closeResource(map.get(key));
    map.set(key, entry);
  };
  const resolveLocation = (base, location) => {
    if (!location) return null;
    try {
      const baseUrl = new URL(base);
      const resource = new URL(location, baseUrl);
      return resource.origin === baseUrl.origin ? resource.href : null;
    } catch (_) { return null; }
  };
  const basicPassword = (password) => {
    const value = bounded(password, 16384, "credential");
    return `Basic ${btoa(`:${value}`)}`;
  };
  const rtcConfig = (iceServers) => ({
    iceServers: Array.isArray(iceServers) && iceServers.length
      ? iceServers.map((server) => ({
          urls: Array.isArray(server.urls) ? server.urls.slice(0, 8) : [],
          username: server.username || undefined,
          credential: server.credential || undefined,
        }))
      : [{ urls: ["stun:stun.l.google.com:19302", "stun:stun1.l.google.com:19302"] }],
  });
  const waitForIce = (pc) => new Promise((resolve) => {
    if (pc.iceGatheringState === "complete") return resolve();
    const done = () => {
      if (pc.iceGatheringState === "complete") {
        pc.removeEventListener("icegatheringstatechange", done);
        resolve();
      }
    };
    pc.addEventListener("icegatheringstatechange", done);
    setTimeout(() => { pc.removeEventListener("icegatheringstatechange", done); resolve(); }, 5000);
  });
  const exchangeSdp = async (url, password, sdp, allowInsecureLoopback) => {
    const parsed = secureHttpUrl(url, allowInsecureLoopback, "The media URL");
    bounded(sdp, MAX_SDP_BYTES, "SDP offer");
    const authorization = basicPassword(password);
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(new DOMException("timeout", "TimeoutError")), REQUEST_TIMEOUT_MS);
    let response;
    try {
      response = await fetch(parsed.href, {
        method: "POST",
        headers: { "Content-Type": "application/sdp", "Authorization": authorization },
        body: sdp,
        cache: "no-store",
        signal: controller.signal,
      });
    } finally {
      clearTimeout(timer);
    }
    if (!response.ok) throw new Error(`Media negotiation failed (${response.status})`);
    const answer = bounded(await response.text(), MAX_SDP_BYTES, "SDP answer");
    return {
      answer,
      resourceUrl: resolveLocation(parsed.href, response.headers.get("Location")),
      resourceOrigin: parsed.origin,
      authorization,
    };
  };
  const constraints = (cameraId, micId, facingMode, audioOnly) => ({
    video: audioOnly ? false : (cameraId
      ? { deviceId: { exact: cameraId }, width: { ideal: 1280 }, height: { ideal: 720 } }
      : { facingMode: facingMode ? { ideal: facingMode } : undefined, width: { ideal: 1280 }, height: { ideal: 720 } }),
    audio: micId
      ? { deviceId: { exact: micId }, echoCancellation: true, noiseSuppression: true, autoGainControl: true }
      : { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
  });
  const mapDevice = (device) => ({
    device_id: bounded(device.deviceId, 2048, "device id"),
    label: bounded(device.label, 512, "device label"),
    kind: device.kind,
  });

  const api = {
    version: AULA_LIVE_NATIVE_VERSION,
    capabilities(args) {
      const media = !!(navigator.mediaDevices && navigator.mediaDevices.getUserMedia);
      const mobile = args.platform === "android" || args.platform === "ios";
      return {
        camera: media,
        microphone: media,
        webrtc: typeof RTCPeerConnection === "function",
        websocket: typeof WebSocket === "function",
        // Mobile WebViews need an explicit MediaProjection/ReplayKit host
        // adapter. Do not expose a browser method that cannot complete there.
        screen_share: !mobile && !!(navigator.mediaDevices && navigator.mediaDevices.getDisplayMedia),
        picture_in_picture: !mobile && !!document.pictureInPictureEnabled,
        hls_native: !!document.createElement("video").canPlayType("application/vnd.apple.mpegurl"),
      };
    },
    async prejoin(args) {
      const media = requireMedia();
      await closeResource(streams.get("prejoin"));
      const stream = await media.getUserMedia(constraints(args.camera_id, args.mic_id, null, false));
      try { attach(args.element_id || "prejoin-preview-video", stream, true); }
      catch (error) { stopTracks(stream); throw error; }
      const entry = { stream, meterTimer: null, audioContext: null };
      const AudioContextClass = window.AudioContext || window.webkitAudioContext;
      if (AudioContextClass && stream.getAudioTracks().length) {
        const audioContext = new AudioContextClass();
        const analyser = audioContext.createAnalyser();
        analyser.fftSize = 512;
        audioContext.createMediaStreamSource(stream).connect(analyser);
        const values = new Uint8Array(analyser.frequencyBinCount);
        entry.audioContext = audioContext;
        entry.meterTimer = setInterval(() => {
          analyser.getByteTimeDomainData(values);
          let sum = 0;
          for (const value of values) { const centered = value - 128; sum += centered * centered; }
          const level = Math.min(100, Math.round(Math.sqrt(sum / Math.max(1, values.length)) / 128 * 220));
          const meter = document.querySelector(".prejoin-mic-meter-fill");
          if (meter instanceof HTMLElement) meter.style.width = `${level}%`;
        }, 100);
      }
      streams.set("prejoin", entry);
      const devices = await media.enumerateDevices();
      return { devices: devices.slice(0, 64).map(mapDevice) };
    },
    async stopPrejoin() {
      await closeResource(streams.get("prejoin"));
      streams.delete("prejoin");
      detach("prejoin-preview-video");
      return true;
    },
    async enumerate() {
      const media = requireMedia();
      const devices = await media.enumerateDevices();
      return { devices: devices.slice(0, 64).map(mapDevice) };
    },
    async publish(args) {
      if (typeof RTCPeerConnection !== "function") throw new DOMException("WebRTC unavailable", "NotSupportedError");
      const key = bounded(args.key, 128, "publisher key");
      secureHttpUrl(args.url, args.allow_insecure_loopback, "The media URL");
      if (args.screen && (args.platform === "android" || args.platform === "ios")) {
        throw new DOMException("Mobile screen sharing requires a platform capture adapter", "NotSupportedError");
      }
      const media = requireMedia();
      const stream = args.screen
        ? await (() => {
            if (!media.getDisplayMedia) throw new DOMException("Screen sharing unavailable", "NotSupportedError");
            return media.getDisplayMedia({ video: true, audio: true });
          })()
        : await media.getUserMedia(constraints(args.camera_id, args.mic_id, args.facing_mode, !!args.audio_only));
      let pc = null;
      let exchanged = null;
      try {
        if (args.element_id && document.getElementById(args.element_id)) attach(args.element_id, stream, true);
        pc = new RTCPeerConnection(rtcConfig(args.ice_servers));
        stream.getTracks().forEach((track) => pc.addTrack(track, stream));
        const offer = await pc.createOffer();
        await pc.setLocalDescription(offer);
        await waitForIce(pc);
        const local = pc.localDescription;
        if (!local || !local.sdp) throw new Error("The device did not create a media offer");
        exchanged = await exchangeSdp(args.url, args.password, local.sdp, args.allow_insecure_loopback);
        await pc.setRemoteDescription({ type: "answer", sdp: exchanged.answer });
        await replaceEntry(peers, key, {
          pc,
          stream,
          resourceUrl: exchanged.resourceUrl,
          resourceOrigin: exchanged.resourceOrigin,
          authorization: exchanged.authorization,
        });
        return { key };
      } catch (error) {
        await closeResource({
          pc,
          stream,
          resourceUrl: exchanged && exchanged.resourceUrl,
          resourceOrigin: exchanged && exchanged.resourceOrigin,
          authorization: exchanged && exchanged.authorization,
        });
        if (args.element_id) detach(args.element_id);
        throw error;
      }
    },
    async stopPublisher(args) {
      const key = bounded(args.key, 128, "publisher key");
      await closeResource(peers.get(key));
      peers.delete(key);
      if (args.element_id) detach(args.element_id);
      return true;
    },
    async attachPublisher(args) {
      const entry = peers.get(bounded(args.key, 128, "publisher key"));
      if (!entry) throw new Error("The live publisher is not active");
      attach(args.element_id, entry.stream, true);
      return true;
    },
    async setTrackEnabled(args) {
      const entry = peers.get(bounded(args.key, 128, "publisher key"));
      if (!entry) throw new Error("The live publisher is not active");
      const wanted = args.kind === "audio" ? entry.stream.getAudioTracks() : entry.stream.getVideoTracks();
      wanted.forEach((track) => { track.enabled = !!args.enabled; });
      return true;
    },
    async publisherStats(args) {
      const entry = peers.get(bounded(args.key, 128, "publisher key"));
      if (!entry || !entry.pc) return { loss_fraction: 0, jitter_secs: 0, rtt_secs: 0, has_data: false };
      const report = await entry.pc.getStats();
      let packetsLost = 0;
      let packetsTotal = 0;
      let jitter = 0;
      let rtt = 0;
      let hasData = false;
      report.forEach((stat) => {
        if (stat.type === "inbound-rtp" || stat.type === "outbound-rtp") {
          const lost = Math.max(0, Number(stat.packetsLost || 0));
          const moved = Math.max(0, Number(stat.packetsReceived || stat.packetsSent || 0));
          packetsLost += lost;
          packetsTotal += lost + moved;
          jitter = Math.max(jitter, Number(stat.jitter || 0));
          hasData = true;
        } else if (stat.type === "remote-inbound-rtp") {
          jitter = Math.max(jitter, Number(stat.jitter || 0));
          rtt = Math.max(rtt, Number(stat.roundTripTime || 0));
          hasData = true;
        } else if (stat.type === "candidate-pair" && stat.nominated) {
          rtt = Math.max(rtt, Number(stat.currentRoundTripTime || 0));
        }
      });
      return {
        loss_fraction: packetsTotal > 0 ? Math.min(1, packetsLost / packetsTotal) : 0,
        jitter_secs: Number.isFinite(jitter) ? jitter : 0,
        rtt_secs: Number.isFinite(rtt) ? rtt : 0,
        has_data: hasData,
      };
    },
    async switchCamera(args) {
      const key = bounded(args.key, 128, "publisher key");
      const entry = peers.get(key);
      if (!entry) throw new Error("The live publisher is not active");
      const media = requireMedia();
      const next = await media.getUserMedia({
        video: args.camera_id
          ? { deviceId: { exact: args.camera_id } }
          : { facingMode: { ideal: args.facing_mode || "user" } },
        audio: false,
      });
      const track = next.getVideoTracks()[0];
      if (!track) { stopTracks(next); throw new Error("The selected camera returned no video"); }
      const sender = entry.pc.getSenders().find((candidate) => candidate.track && candidate.track.kind === "video");
      if (!sender) { stopTracks(next); throw new Error("The video sender is unavailable"); }
      await sender.replaceTrack(track);
      entry.stream.getVideoTracks().forEach((old) => { entry.stream.removeTrack(old); old.stop(); });
      entry.stream.addTrack(track);
      if (args.element_id) attach(args.element_id, entry.stream, true);
      return true;
    },
    async view(args) {
      if (typeof RTCPeerConnection !== "function") throw new DOMException("WebRTC unavailable", "NotSupportedError");
      const key = bounded(args.key, 128, "viewer key");
      secureHttpUrl(args.url, args.allow_insecure_loopback, "The media URL");
      const pc = new RTCPeerConnection(rtcConfig(args.ice_servers));
      const stream = new MediaStream();
      let exchanged = null;
      try {
        pc.addTransceiver("video", { direction: "recvonly" });
        pc.addTransceiver("audio", { direction: "recvonly" });
        pc.ontrack = (event) => { stream.addTrack(event.track); attach(args.element_id, stream, false); };
        const offer = await pc.createOffer();
        await pc.setLocalDescription(offer);
        await waitForIce(pc);
        const local = pc.localDescription;
        if (!local || !local.sdp) throw new Error("The device did not create a media offer");
        exchanged = await exchangeSdp(args.url, args.password, local.sdp, args.allow_insecure_loopback);
        await pc.setRemoteDescription({ type: "answer", sdp: exchanged.answer });
        await replaceEntry(viewers, key, {
          pc,
          stream,
          resourceUrl: exchanged.resourceUrl,
          resourceOrigin: exchanged.resourceOrigin,
          authorization: exchanged.authorization,
        });
        attach(args.element_id, stream, false);
        return { key };
      } catch (error) {
        await closeResource({
          pc,
          stream,
          resourceUrl: exchanged && exchanged.resourceUrl,
          resourceOrigin: exchanged && exchanged.resourceOrigin,
          authorization: exchanged && exchanged.authorization,
        });
        detach(args.element_id);
        throw error;
      }
    },
    async stopViewer(args) {
      const key = bounded(args.key, 128, "viewer key");
      await closeResource(viewers.get(key));
      viewers.delete(key);
      if (args.element_id) detach(args.element_id);
      return true;
    },
    async pictureInPicture(args) {
      if (args.platform === "android" || args.platform === "ios") {
        throw new DOMException("Mobile picture in picture requires a platform adapter", "NotSupportedError");
      }
      const video = document.getElementById(args.element_id);
      if (!(video instanceof HTMLVideoElement) || !video.requestPictureInPicture) {
        throw new DOMException("Picture in picture unavailable", "NotSupportedError");
      }
      await video.requestPictureInPicture();
      return true;
    },
    async promptText(args) {
      const value = window.prompt(String(args.message || "Text:"));
      if (value === null) return { value: null };
      return { value: String(value).slice(0, 500) };
    },
    async exportWhiteboard(args) {
      const svg = document.getElementById(args.element_id);
      if (!(svg instanceof SVGElement)) throw new Error("The whiteboard surface is unavailable");
      const source = new XMLSerializer().serializeToString(svg);
      if (utf8Bytes(source) > 4 * 1024 * 1024) throw new Error("The whiteboard is too large to export");
      const image = new Image();
      const svgBlob = new Blob([source], { type: "image/svg+xml;charset=utf-8" });
      const svgUrl = URL.createObjectURL(svgBlob);
      try {
        await new Promise((resolve, reject) => { image.onload = resolve; image.onerror = reject; image.src = svgUrl; });
        const canvas = document.createElement("canvas");
        canvas.width = 1600; canvas.height = 960;
        const context = canvas.getContext("2d");
        context.fillStyle = "#ffffff"; context.fillRect(0, 0, canvas.width, canvas.height);
        context.drawImage(image, 0, 0, canvas.width, canvas.height);
        const dataUrl = canvas.toDataURL("image/png");
        const base64 = String(dataUrl).split(",", 2)[1] || "";
        if (!base64 || utf8Bytes(base64) > 16 * 1024 * 1024) {
          throw new Error("The whiteboard image could not be created safely");
        }
        return { base64 };
      } finally { URL.revokeObjectURL(svgUrl); }
    },
    async attachHls(args) {
      const video = document.getElementById(args.element_id);
      if (!(video instanceof HTMLVideoElement)) throw new Error("The live video surface is unavailable");
      const mediaUrl = secureHttpUrl(args.url, args.allow_insecure_loopback, "The HLS URL");
      await api.detachHls(args);
      if (video.canPlayType("application/vnd.apple.mpegurl")) {
        video.src = mediaUrl.href;
        await video.play().catch(() => {});
        hlsPlayers.set(args.element_id, { mode: "native", video });
        return { mode: "native" };
      }
      if (!window.Hls) {
        await new Promise((resolve, reject) => {
          const existing = document.querySelector('script[data-aula-hls="true"]');
          if (existing) {
            existing.addEventListener("load", resolve, { once: true });
            existing.addEventListener("error", reject, { once: true });
            if (window.Hls) resolve();
            return;
          }
          const script = document.createElement("script");
          script.src = "/vendor/hls.js";
          script.dataset.aulaHls = "true";
          script.onload = resolve;
          script.onerror = () => reject(new Error("HLS player could not be loaded"));
          document.head.appendChild(script);
        });
      }
      if (!window.Hls || !window.Hls.isSupported()) throw new DOMException("HLS unavailable", "NotSupportedError");
      const player = new window.Hls({ lowLatencyMode: true, backBufferLength: 30 });
      player.loadSource(mediaUrl.href);
      player.attachMedia(video);
      hlsPlayers.set(args.element_id, { mode: "hls.js", video, player });
      return { mode: "hls.js" };
    },
    async detachHls(args) {
      const entry = hlsPlayers.get(args.element_id);
      if (entry && entry.player) entry.player.destroy();
      if (entry && entry.video) { entry.video.pause(); entry.video.removeAttribute("src"); entry.video.load(); }
      hlsPlayers.delete(args.element_id);
      return true;
    },
    async delay(args) {
      const ms = Math.max(0, Math.min(30000, Number(args.ms || 0)));
      await new Promise((resolve) => setTimeout(resolve, ms));
      return true;
    },
    async closeAll() {
      for (const entry of [...peers.values(), ...viewers.values(), ...streams.values()]) await closeResource(entry);
      peers.clear(); viewers.clear(); streams.clear();
      for (const [element_id] of hlsPlayers) await api.detachHls({ element_id });
      return true;
    },
    async execute(request) {
      const args = Object.assign({}, request && request.args || {}, {
        allow_insecure_loopback: request && request.allow_insecure_loopback === true,
        platform: String(request && request.platform || "unknown"),
      });
      switch (request && request.op) {
        case "capabilities": return api.capabilities(args);
        case "prejoin": return api.prejoin(args);
        case "stop_prejoin": return api.stopPrejoin();
        case "enumerate": return api.enumerate();
        case "publish": return api.publish(args);
        case "stop_publisher": return api.stopPublisher(args);
        case "attach_publisher": return api.attachPublisher(args);
        case "set_track_enabled": return api.setTrackEnabled(args);
        case "publisher_stats": return api.publisherStats(args);
        case "switch_camera": return api.switchCamera(args);
        case "view": return api.view(args);
        case "stop_viewer": return api.stopViewer(args);
        case "picture_in_picture": return api.pictureInPicture(args);
        case "prompt_text": return api.promptText(args);
        case "export_whiteboard": return api.exportWhiteboard(args);
        case "attach_hls": return api.attachHls(args);
        case "detach_hls": return api.detachHls(args);
        case "delay": return api.delay(args);
        case "close_all": return api.closeAll();
        default: throw new Error("Unknown native live-classroom operation");
      }
    },
    safeError,
    maxMessageBytes: MAX_MESSAGE_BYTES,
  };
  window.__aulaLiveNative = api;
}
const request = await dioxus.recv();
try {
  return { ok: true, value: await window.__aulaLiveNative.execute(request) };
} catch (error) {
  return { ok: false, error: window.__aulaLiveNative.safeError(error) };
}
"##;

#[derive(Debug, Serialize)]
struct Request<'a, T> {
    op: &'a str,
    args: T,
    allow_insecure_loopback: bool,
    platform: &'static str,
}

#[derive(Debug, Deserialize)]
struct Reply<T> {
    ok: bool,
    value: Option<T>,
    error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct NativeCapabilities {
    pub camera: bool,
    pub microphone: bool,
    pub webrtc: bool,
    pub websocket: bool,
    pub screen_share: bool,
    pub picture_in_picture: bool,
    pub hls_native: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct NativeMediaDevice {
    pub device_id: String,
    pub label: String,
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceList {
    pub devices: Vec<NativeMediaDevice>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeIceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublishRequest {
    pub key: String,
    pub url: String,
    pub password: String,
    pub element_id: Option<String>,
    pub camera_id: String,
    pub mic_id: String,
    pub facing_mode: Option<String>,
    pub screen: bool,
    pub audio_only: bool,
    pub ice_servers: Vec<NativeIceServer>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ViewRequest {
    pub key: String,
    pub url: String,
    pub password: String,
    pub element_id: String,
    pub ice_servers: Vec<NativeIceServer>,
}

#[derive(Debug, Deserialize)]
struct KeyResult {
    #[allow(dead_code)]
    key: String,
}

#[derive(Debug, Serialize)]
struct PrejoinArgs<'a> {
    camera_id: &'a str,
    mic_id: &'a str,
    element_id: &'a str,
}

#[derive(Debug, Serialize)]
struct KeyArgs<'a> {
    key: &'a str,
    element_id: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct TrackArgs<'a> {
    key: &'a str,
    kind: &'a str,
    enabled: bool,
}

#[derive(Debug, Serialize)]
struct SwitchCameraArgs<'a> {
    key: &'a str,
    camera_id: &'a str,
    facing_mode: Option<&'a str>,
    element_id: &'a str,
}

#[derive(Debug, Serialize)]
struct ElementArgs<'a> {
    element_id: &'a str,
}

#[derive(Debug, Serialize)]
struct HlsArgs<'a> {
    element_id: &'a str,
    url: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct PromptResult {
    pub value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WhiteboardExportResult {
    base64: String,
}

#[derive(Debug, Deserialize)]
struct NativeStatsSample {
    loss_fraction: f64,
    jitter_secs: f64,
    rtt_secs: f64,
    has_data: bool,
}

fn native_platform() -> &'static str {
    if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "ios") {
        "ios"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}

fn is_debug_loopback_http(url: &reqwest::Url) -> bool {
    if !cfg!(debug_assertions) || url.scheme() != "http" {
        return false;
    }
    matches!(url.host_str(), Some("localhost" | "::1" | "10.0.2.2"))
        || url.host_str().is_some_and(|host| host.starts_with("127."))
}

fn validate_media_url(value: &str, label: &str) -> Result<(), String> {
    if value.len() > 4096 {
        return Err(format!("{label} exceeded the safe size limit"));
    }
    let url = reqwest::Url::parse(value).map_err(|_| format!("{label} is invalid"))?;
    if url.scheme() == "https" || is_debug_loopback_http(&url) {
        Ok(())
    } else {
        Err(format!("{label} must use HTTPS"))
    }
}

async fn call<T, A>(op: &str, args: A) -> Result<T, String>
where
    T: DeserializeOwned,
    A: Serialize,
{
    let request = Request {
        op,
        args,
        allow_insecure_loopback: cfg!(debug_assertions),
        platform: native_platform(),
    };
    let encoded =
        serde_json::to_vec(&request).map_err(|_| "Invalid live-classroom request".to_string())?;
    if encoded.len() > MAX_REQUEST_BYTES {
        return Err("Live-classroom request exceeded the safe size limit".into());
    }

    let eval = document::eval(RUNTIME);
    eval.send(request)
        .map_err(|_| "The native media bridge is unavailable".to_string())?;
    let reply: Reply<T> = eval
        .join()
        .await
        .map_err(|_| "The native media bridge did not respond".to_string())?;
    if reply.ok {
        reply
            .value
            .ok_or_else(|| "The native media bridge returned no result".to_string())
    } else {
        Err(reply
            .error
            .unwrap_or_else(|| "Live-classroom operation failed".into()))
    }
}

pub async fn capabilities() -> Result<NativeCapabilities, String> {
    call("capabilities", serde_json::json!({})).await
}

pub async fn start_prejoin(camera_id: &str, mic_id: &str) -> Result<DeviceList, String> {
    call(
        "prejoin",
        PrejoinArgs {
            camera_id,
            mic_id,
            element_id: "prejoin-preview-video",
        },
    )
    .await
}

pub async fn stop_prejoin() -> Result<(), String> {
    let _: bool = call("stop_prejoin", serde_json::json!({})).await?;
    Ok(())
}

pub async fn enumerate() -> Result<DeviceList, String> {
    call("enumerate", serde_json::json!({})).await
}

pub async fn publish(request: PublishRequest) -> Result<(), String> {
    validate_media_url(&request.url, "The media URL")?;
    let _: KeyResult = call("publish", request).await?;
    Ok(())
}

pub async fn stop_publisher(key: &str, element_id: Option<&str>) -> Result<(), String> {
    let _: bool = call("stop_publisher", KeyArgs { key, element_id }).await?;
    Ok(())
}

pub async fn attach_publisher(key: &str, element_id: &str) -> Result<(), String> {
    let _: bool = call(
        "attach_publisher",
        KeyArgs {
            key,
            element_id: Some(element_id),
        },
    )
    .await?;
    Ok(())
}

pub async fn set_track_enabled(key: &str, kind: &str, enabled: bool) -> Result<(), String> {
    let _: bool = call("set_track_enabled", TrackArgs { key, kind, enabled }).await?;
    Ok(())
}

pub async fn publisher_quality(key: &str) -> crate::live_room_stats::NetQuality {
    let sample: Result<NativeStatsSample, _> = call(
        "publisher_stats",
        KeyArgs {
            key,
            element_id: None,
        },
    )
    .await;
    let Ok(sample) = sample else {
        return crate::live_room_stats::NetQuality::Unknown;
    };
    crate::live_room_stats::classify(&crate::live_room_stats::StatsSample {
        loss_fraction: sample.loss_fraction.clamp(0.0, 1.0),
        jitter_secs: sample.jitter_secs.max(0.0),
        rtt_secs: sample.rtt_secs.max(0.0),
        has_data: sample.has_data,
    })
}

pub async fn switch_camera(
    camera_id: &str,
    facing_mode: Option<&str>,
    element_id: &str,
) -> Result<(), String> {
    let _: bool = call(
        "switch_camera",
        SwitchCameraArgs {
            key: "main",
            camera_id,
            facing_mode,
            element_id,
        },
    )
    .await?;
    Ok(())
}

pub async fn view(request: ViewRequest) -> Result<(), String> {
    validate_media_url(&request.url, "The media URL")?;
    let _: KeyResult = call("view", request).await?;
    Ok(())
}

pub async fn stop_viewer(key: &str, element_id: Option<&str>) -> Result<(), String> {
    let _: bool = call("stop_viewer", KeyArgs { key, element_id }).await?;
    Ok(())
}

pub async fn request_picture_in_picture(element_id: &str) -> Result<(), String> {
    let _: bool = call("picture_in_picture", ElementArgs { element_id }).await?;
    Ok(())
}

pub async fn prompt_text(message: &str) -> Result<Option<String>, String> {
    let result: PromptResult =
        call("prompt_text", serde_json::json!({ "message": message })).await?;
    Ok(result.value)
}

pub async fn export_whiteboard(
    element_id: &str,
) -> Result<platform_bridge::native_files::SaveOutcome, String> {
    let result: WhiteboardExportResult =
        call("export_whiteboard", ElementArgs { element_id }).await?;
    if result.base64.len() > 16 * 1024 * 1024 {
        return Err("The whiteboard image exceeded the safe size limit".into());
    }
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(result.base64)
        .map_err(|_| "The whiteboard image was invalid".to_string())?;
    platform_bridge::native_files::save_bytes("aulalite-whiteboard.png", "image/png", &bytes)
        .await
        .map_err(|error| error.to_string())
}

pub async fn attach_hls(element_id: &str, url: &str) -> Result<(), String> {
    validate_media_url(url, "The HLS URL")?;
    let _: serde_json::Value = call("attach_hls", HlsArgs { element_id, url }).await?;
    Ok(())
}

pub async fn detach_hls(element_id: &str) -> Result<(), String> {
    let _: bool = call("detach_hls", ElementArgs { element_id }).await?;
    Ok(())
}

pub async fn close_all() -> Result<(), String> {
    let _: bool = call("close_all", serde_json::json!({})).await?;
    Ok(())
}

pub async fn delay(ms: u32) {
    let _: Result<bool, _> = call("delay", serde_json::json!({ "ms": ms.min(30_000) })).await;
}

pub fn map_ice_servers(servers: &[crate::live_room_whip::IceServerConfig]) -> Vec<NativeIceServer> {
    servers
        .iter()
        .map(|server| NativeIceServer {
            urls: server.urls.iter().take(8).cloned().collect(),
            username: server.username.clone(),
            credential: server.credential.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_has_every_live_transport_and_bounded_network_contract() {
        for needle in [
            "getUserMedia",
            "enumerateDevices",
            "getDisplayMedia",
            "RTCPeerConnection",
            "application/sdp",
            "MAX_SDP_BYTES",
            "AbortController",
            "requestPictureInPicture",
            "attachHls",
            "publisherStats",
        ] {
            assert!(
                RUNTIME.contains(needle),
                "missing native bridge primitive {needle}"
            );
        }
    }

    #[test]
    fn bridge_never_interpolates_credentials_into_javascript() {
        assert!(RUNTIME.contains("await dioxus.recv()"));
        assert!(!RUNTIME.contains("console.log"));
        assert!(!RUNTIME.contains("console.error"));
    }

    #[test]
    fn bridge_enforces_secure_transport_and_failure_cleanup() {
        for contract in [
            "secureHttpUrl",
            "must use HTTPS",
            "isLoopbackHost",
            "resource.origin === entry.resourceOrigin",
            "headers: { \"Authorization\"",
            "catch (error)",
            "await closeResource({",
        ] {
            assert!(
                RUNTIME.contains(contract),
                "missing cleanup/security contract {contract}"
            );
        }
        assert!(RUNTIME.contains("case \"capabilities\": return api.capabilities(args)"));
    }

    #[test]
    fn media_transport_policy_allows_only_tls_or_debug_loopback() {
        assert!(validate_media_url("https://media.example.test/live/whep", "media").is_ok());
        assert!(validate_media_url("http://media.example.test/live/whep", "media").is_err());
        assert!(validate_media_url("file:///tmp/stream.m3u8", "media").is_err());
        if cfg!(debug_assertions) {
            assert!(validate_media_url("http://127.0.0.1:8889/live/whep", "media").is_ok());
            assert!(validate_media_url("http://10.0.2.2:8889/live/whep", "media").is_ok());
        }
    }

    #[test]
    fn mobile_capabilities_are_explicitly_host_adapter_gated() {
        assert!(RUNTIME.contains("args.platform === \"android\""));
        assert!(RUNTIME.contains("args.platform === \"ios\""));
        assert!(RUNTIME.contains("screen_share: !mobile"));
        assert!(RUNTIME.contains("picture_in_picture: !mobile"));
    }

    #[test]
    fn ice_mapping_caps_untrusted_server_lists() {
        let server = crate::live_room_whip::IceServerConfig {
            urls: (0..20)
                .map(|index| format!("turn:relay-{index}.example"))
                .collect(),
            username: Some("user".into()),
            credential: Some("secret".into()),
        };
        let mapped = map_ice_servers(&[server]);
        assert_eq!(mapped[0].urls.len(), 8);
        assert_eq!(mapped[0].username.as_deref(), Some("user"));
    }
}
