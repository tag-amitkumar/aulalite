#!/usr/bin/env bash
# Vendor MediaPipe Tasks-Vision (ImageSegmenter / selfie segmentation) into the
# shell-web public assets so background blur / virtual backgrounds work fully
# OFFLINE — no CDN dependency at class time. Re-run to update the pinned version.
#
# Usage:  bash tools/vendor-mediapipe.sh
# Output: crates/shell-web/public/vendor/mediapipe/{vision_bundle.mjs,wasm/*,selfie_segmenter.tflite}
#
# The blur bridge (public/assets/blur-bridge.js) imports vision_bundle.mjs and
# points FilesetResolver at ./wasm. If these files are absent the bridge reports
# unsupported and the app falls back to publishing the raw camera (no blur),
# so a missing vendor step degrades gracefully rather than breaking video.
set -euo pipefail

VER="0.10.18"
CDN="https://cdn.jsdelivr.net/npm/@mediapipe/tasks-vision@${VER}"
MODEL="https://storage.googleapis.com/mediapipe-models/image_segmenter/selfie_segmenter/float16/latest/selfie_segmenter.tflite"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/crates/shell-web/public/vendor/mediapipe"
mkdir -p "$DEST/wasm"

fetch() { # url dest
  echo "  fetch $(basename "$2")"
  curl -fsSL -o "$2" "$1"
}

echo "Vendoring @mediapipe/tasks-vision@${VER} -> $DEST"
fetch "$CDN/vision_bundle.mjs"                      "$DEST/vision_bundle.mjs"
fetch "$CDN/wasm/vision_wasm_internal.js"           "$DEST/wasm/vision_wasm_internal.js"
fetch "$CDN/wasm/vision_wasm_internal.wasm"         "$DEST/wasm/vision_wasm_internal.wasm"
fetch "$CDN/wasm/vision_wasm_nosimd_internal.js"    "$DEST/wasm/vision_wasm_nosimd_internal.js"
fetch "$CDN/wasm/vision_wasm_nosimd_internal.wasm"  "$DEST/wasm/vision_wasm_nosimd_internal.wasm"
fetch "$MODEL"                                      "$DEST/selfie_segmenter.tflite"

echo "Done. Vendored files:"
( cd "$DEST" && find . -type f -exec ls -l {} \; | awk '{print "  " $5 "\t" $9}' )
