// whiteboard-bridge.js — exposes window.aula.exportWhiteboardPng(svgId), used
// by the live-room whiteboard's "Export" button (live_room_whiteboard.rs calls
// it via js_sys::Reflect, mirroring the blur bridge). Serializes the board's
// SVG to a canvas and triggers a PNG download. Absent → the Rust side no-ops.
(function () {
  "use strict";
  window.aula = window.aula || {};
  if (window.aula.exportWhiteboardPng) return; // idempotent

  window.aula.exportWhiteboardPng = function (svgId) {
    try {
      var svg = document.getElementById(svgId);
      if (!svg) return;
      var xml = new XMLSerializer().serializeToString(svg);
      var svg64 = btoa(unescape(encodeURIComponent(xml)));
      var img = new Image();
      img.onload = function () {
        var rect = svg.getBoundingClientRect();
        var w = Math.max(1, Math.round(rect.width)) || 1000;
        var h = Math.max(1, Math.round(rect.height)) || 600;
        var canvas = document.createElement("canvas");
        canvas.width = w;
        canvas.height = h;
        var ctx = canvas.getContext("2d");
        // Paint the board surface so transparent areas aren't black in the PNG.
        ctx.fillStyle = "#fffdf8";
        ctx.fillRect(0, 0, w, h);
        ctx.drawImage(img, 0, 0, w, h);
        var a = document.createElement("a");
        a.download = "whiteboard.png";
        a.href = canvas.toDataURL("image/png");
        a.click();
      };
      img.onerror = function () {
        console.warn("[aula.whiteboard] export failed to rasterize the SVG");
      };
      img.src = "data:image/svg+xml;base64," + svg64;
    } catch (e) {
      console.warn("[aula.whiteboard] exportWhiteboardPng failed:", e);
    }
  };
})();
