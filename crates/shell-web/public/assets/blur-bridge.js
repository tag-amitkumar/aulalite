// blur-bridge.js — background blur / virtual background for the live room.
//
// Exposes `window.aula.blur` (mirrors the window.aula.* bridge convention used
// by firebase-bridge.js / fcm-bridge.js). The Rust side (live_room_video_fx.rs)
// calls it via js_sys::Reflect, exactly like render_hls() drives window.Hls.
//
// Pipeline: a hidden <video> plays the raw camera MediaStream; MediaPipe
// ImageSegmenter (selfie segmentation, vendored under /vendor/mediapipe so it
// works OFFLINE) produces a person mask each frame; we composite onto an
// offscreen <canvas> — person kept sharp, background blurred or replaced with
// an image — and hand back `canvas.captureStream()` carrying the processed
// video track plus the ORIGINAL audio track(s) untouched.
//
// Robustness: every failure path (no WebGL, model missing, API absent) returns
// the ORIGINAL stream so video keeps publishing — background FX degrades to a
// no-op rather than breaking the broadcast.
(function () {
  "use strict";
  window.aula = window.aula || {};
  if (window.aula.blur) return; // idempotent

  var BASE = "/vendor/mediapipe";
  var MODEL = BASE + "/selfie_segmenter.tflite";

  // Lazy singletons — one pipeline (the teacher has one camera).
  var mod = null;        // the imported ESM module
  var vision = null;     // FilesetResolver result
  var segmenter = null;  // ImageSegmenter

  // Active session state.
  var S = null;

  function isSupported() {
    try {
      return (
        typeof WebAssembly === "object" &&
        typeof HTMLCanvasElement !== "undefined" &&
        typeof HTMLCanvasElement.prototype.captureStream === "function" &&
        typeof document !== "undefined"
      );
    } catch (_e) {
      return false;
    }
  }

  async function ensureSegmenter() {
    if (segmenter) return segmenter;
    if (!mod) mod = await import(BASE + "/vision_bundle.mjs");
    if (!vision) vision = await mod.FilesetResolver.forVisionTasks(BASE + "/wasm");
    // Try GPU first (fast); fall back to CPU so older machines still work.
    try {
      segmenter = await mod.ImageSegmenter.createFromOptions(vision, {
        baseOptions: { modelAssetPath: MODEL, delegate: "GPU" },
        runningMode: "VIDEO",
        outputCategoryMask: true,
        outputConfidenceMasks: false,
      });
    } catch (_gpuErr) {
      segmenter = await mod.ImageSegmenter.createFromOptions(vision, {
        baseOptions: { modelAssetPath: MODEL, delegate: "CPU" },
        runningMode: "VIDEO",
        outputCategoryMask: true,
        outputConfidenceMasks: false,
      });
    }
    return segmenter;
  }

  function makeVideo(stream) {
    var v = document.createElement("video");
    v.autoplay = true;
    v.muted = true;
    v.playsInline = true;
    v.srcObject = stream;
    return v;
  }

  // Composite one frame: blurred/replaced background + sharp person.
  function renderFrame() {
    if (!S || S.stopped) return;
    var v = S.video;
    var w = v.videoWidth | 0;
    var h = v.videoHeight | 0;
    if (w === 0 || h === 0) {
      S.raf = requestAnimationFrame(renderFrame);
      return;
    }
    if (S.canvas.width !== w || S.canvas.height !== h) {
      S.canvas.width = w;
      S.canvas.height = h;
      S.sharp.width = w;
      S.sharp.height = h;
      S.person.width = w;
      S.person.height = h;
    }
    var ctx = S.ctx;

    // Passthrough when off (or while the segmenter is still loading): just copy
    // the camera frame so the published canvas track stays live and identical.
    if (S.mode === "off" || !segmenter) {
      ctx.filter = "none";
      ctx.globalCompositeOperation = "source-over";
      ctx.drawImage(v, 0, 0, w, h);
      S.raf = requestAnimationFrame(renderFrame);
      return;
    }

    var ts = performance.now();
    var result;
    try {
      result = segmenter.segmentForVideo(v, ts);
    } catch (_e) {
      // Transient segmenter hiccup — passthrough this frame.
      ctx.filter = "none";
      ctx.drawImage(v, 0, 0, w, h);
      S.raf = requestAnimationFrame(renderFrame);
      return;
    }

    var mask = result && result.categoryMask;
    if (!mask) {
      ctx.filter = "none";
      ctx.drawImage(v, 0, 0, w, h);
      if (result && result.close) result.close();
      S.raf = requestAnimationFrame(renderFrame);
      return;
    }

    var maskW = mask.width;
    var maskH = mask.height;
    var data = mask.getAsUint8Array(); // category index per pixel
    // Build an alpha mask: person -> opaque, background -> transparent. The
    // selfie segmenter marks background as category 0; `invert` flips this if a
    // model/build reports the opposite polarity.
    var md = S.maskCtx.createImageData(maskW, maskH);
    var px = md.data;
    var invert = S.invert;
    for (var i = 0; i < data.length; i++) {
      var isPerson = invert ? data[i] === 0 : data[i] !== 0;
      px[i * 4 + 3] = isPerson ? 255 : 0;
    }
    if (S.maskCanvas.width !== maskW || S.maskCanvas.height !== maskH) {
      S.maskCanvas.width = maskW;
      S.maskCanvas.height = maskH;
    }
    S.maskCtx.putImageData(md, 0, 0);
    if (mask.close) mask.close();

    // 1) Background layer onto the output canvas.
    ctx.globalCompositeOperation = "source-over";
    if (S.mode === "image" && S.bgImg && S.bgImg.complete && S.bgImg.naturalWidth > 0) {
      ctx.filter = "none";
      drawCover(ctx, S.bgImg, w, h);
    } else {
      ctx.filter = "blur(" + S.blurPx + "px)";
      ctx.drawImage(v, 0, 0, w, h);
      ctx.filter = "none";
    }

    // 2) Build the sharp, person-only layer = sharp frame masked by the mask.
    var pc = S.personCtx;
    pc.globalCompositeOperation = "source-over";
    pc.filter = "none";
    pc.clearRect(0, 0, w, h);
    pc.drawImage(v, 0, 0, w, h);
    pc.globalCompositeOperation = "destination-in";
    pc.imageSmoothingEnabled = true; // feathers the upscaled mask edge
    pc.drawImage(S.maskCanvas, 0, 0, maskW, maskH, 0, 0, w, h);
    pc.globalCompositeOperation = "source-over";

    // 3) Composite the sharp person over the background.
    ctx.globalCompositeOperation = "source-over";
    ctx.drawImage(S.person, 0, 0, w, h);

    if (result.close) result.close();
    S.raf = requestAnimationFrame(renderFrame);
  }

  function drawCover(ctx, img, w, h) {
    var ir = img.naturalWidth / img.naturalHeight;
    var cr = w / h;
    var dw, dh, dx, dy;
    if (ir > cr) {
      dh = h;
      dw = h * ir;
      dx = (w - dw) / 2;
      dy = 0;
    } else {
      dw = w;
      dh = w / ir;
      dx = 0;
      dy = (h - dh) / 2;
    }
    ctx.drawImage(img, dx, dy, dw, dh);
  }

  // start(stream, mode, bgUrl) -> Promise<MediaStream>
  // mode: "off" | "blur" | "image". Returns a processed stream, or the original
  // stream if FX can't be initialised (so video always publishes).
  async function start(stream, mode, bgUrl) {
    if (!isSupported()) return stream;
    try {
      var video = makeVideo(stream);
      await video.play().catch(function () {});

      var canvas = document.createElement("canvas");
      var sharp = document.createElement("canvas"); // unused scratch retained for parity
      var person = document.createElement("canvas");
      var maskCanvas = document.createElement("canvas");
      S = {
        stream: stream,
        video: video,
        canvas: canvas,
        ctx: canvas.getContext("2d", { willReadFrequently: false }),
        sharp: sharp,
        person: person,
        personCtx: person.getContext("2d", { willReadFrequently: false }),
        maskCanvas: maskCanvas,
        maskCtx: maskCanvas.getContext("2d", { willReadFrequently: true }),
        mode: mode || "off",
        blurPx: 12,
        invert: false,
        bgImg: null,
        raf: 0,
        stopped: false,
      };
      if (bgUrl) setBackground(bgUrl);

      // Kick the render loop immediately (passthrough) so the canvas track has
      // frames, then load the segmenter in the background.
      S.raf = requestAnimationFrame(renderFrame);
      // Begin loading the model unless we're starting in "off".
      if (S.mode !== "off") {
        ensureSegmenter().catch(function (e) {
          console.warn("[aula.blur] segmenter init failed; passthrough only:", e);
        });
      }

      var fps = 30;
      var out = canvas.captureStream(fps);
      // Keep the ORIGINAL audio untouched.
      stream.getAudioTracks().forEach(function (t) {
        out.addTrack(t);
      });
      S.out = out;
      return out;
    } catch (e) {
      console.warn("[aula.blur] start failed; publishing raw stream:", e);
      return stream;
    }
  }

  function setMode(mode) {
    if (!S) return;
    S.mode = mode || "off";
    if (S.mode !== "off") {
      ensureSegmenter().catch(function () {});
    }
  }

  // setSource(newStream) — swap the INPUT camera stream feeding the
  // segmentation / passthrough <video> WITHOUT touching the output canvas
  // track. This is the FX-aware camera switch: the published track is the
  // canvas.captureStream() video track, which never changes — only the pixels
  // the render loop reads from change — so there is NO WebRTC renegotiation.
  //
  // Audio handling mirrors start(): the output stream carries the camera's
  // audio track directly (the bridge only processes video). On a video-only
  // camera switch the new stream has no audio track, so the published audio
  // is untouched. If the new stream DOES carry audio (full getUserMedia), we
  // reconcile the output stream's audio track to match the new source so the
  // bridge stays consistent with what start() would have produced.
  function setSource(newStream) {
    if (!S || S.stopped || !newStream) return;
    var oldVideo = S.video;
    // Build a fresh hidden <video> bound to the new stream and start it before
    // swapping, so the render loop never reads a half-torn-down element.
    var nv = makeVideo(newStream);
    nv.play().catch(function () {});
    S.stream = newStream;
    S.video = nv;
    // Detach the previous hidden <video> from its source so the browser can
    // release it. We do NOT stop the old stream's tracks here: the raw camera
    // lifecycle (stopping the prior camera's video track) is owned by the Rust
    // caller, which knows whether the track is shared with anything else.
    try {
      if (oldVideo) oldVideo.srcObject = null;
    } catch (_e) {}
    // Reconcile audio on the published output stream, if one exists. The output
    // canvas video track is left in place (no renegotiation). Only audio tracks
    // are reconciled to the new source's audio, matching start()'s contract.
    if (S.out) {
      try {
        var newAudio = newStream.getAudioTracks();
        if (newAudio.length > 0) {
          S.out.getAudioTracks().forEach(function (t) {
            S.out.removeTrack(t);
          });
          newAudio.forEach(function (t) {
            S.out.addTrack(t);
          });
        }
      } catch (_e) {}
    }
  }

  // isActive() — true when a pipeline is currently running (start() has been
  // called and stop() has not). Lets the Rust caller decide between the
  // FX-aware setSource() path and the raw replaceTrack() path.
  function isActive() {
    return !!(S && !S.stopped);
  }

  function setBackground(url) {
    if (!S) return;
    if (!url) {
      S.bgImg = null;
      return;
    }
    var img = new Image();
    img.crossOrigin = "anonymous";
    img.onload = function () {
      S.bgImg = img;
    };
    img.onerror = function () {
      console.warn("[aula.blur] background image failed to load:", url);
    };
    img.src = url;
  }

  function setInvert(on) {
    if (S) S.invert = !!on;
  }

  function setBlurStrength(px) {
    if (S && px > 0) S.blurPx = px | 0;
  }

  function stop() {
    if (!S) return;
    S.stopped = true;
    if (S.raf) cancelAnimationFrame(S.raf);
    try {
      if (S.out) S.out.getVideoTracks().forEach(function (t) { t.stop(); });
    } catch (_e) {}
    try {
      S.video.srcObject = null;
    } catch (_e) {}
    S = null;
    // Keep the segmenter cached for a fast re-start within the session.
  }

  window.aula.blur = {
    isSupported: isSupported,
    start: start,
    setSource: setSource,
    isActive: isActive,
    setMode: setMode,
    setBackground: setBackground,
    setInvert: setInvert,
    setBlurStrength: setBlurStrength,
    stop: stop,
  };
})();
