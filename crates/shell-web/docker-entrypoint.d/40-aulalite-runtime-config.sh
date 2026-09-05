#!/bin/sh
set -eu

# This image is production-strict by default. Local `dx serve` does not execute
# this entrypoint and uses public/runtime-config.js's empty defaults. A local
# container may explicitly opt out with AULALITE_REQUIRE_FIREBASE_CONFIG=false.
require_config=${AULALITE_REQUIRE_FIREBASE_CONFIG:-true}

api_base_url=${AULALITE_API_BASE_URL:-}
app_origin=${AULALITE_APP_ORIGIN:-}
admin_origin=${AULALITE_ADMIN_ORIGIN:-}

api_key=${FIREBASE_WEB_API_KEY:-}
auth_domain=${FIREBASE_AUTH_DOMAIN:-}
project_id=${FIREBASE_PROJECT_ID:-}
storage_bucket=${FIREBASE_STORAGE_BUCKET:-}
messaging_sender_id=${FIREBASE_MESSAGING_SENDER_ID:-}
app_id=${FIREBASE_APP_ID:-}
measurement_id=${FIREBASE_MEASUREMENT_ID:-}

# FIREBASE_WEB_VAPID_KEY is the preferred name. The aliases make existing
# deployments easier to migrate without ever treating the value as a secret.
vapid_key=${FIREBASE_WEB_VAPID_KEY:-${FCM_WEB_VAPID_KEY:-${FCM_VAPID_KEY:-}}}

if [ "$require_config" != "false" ]; then
  missing=""
  [ -n "$api_key" ] || missing="$missing FIREBASE_WEB_API_KEY"
  [ -n "$auth_domain" ] || missing="$missing FIREBASE_AUTH_DOMAIN"
  [ -n "$project_id" ] || missing="$missing FIREBASE_PROJECT_ID"
  [ -n "$api_base_url" ] || missing="$missing AULALITE_API_BASE_URL"
  [ -n "$app_origin" ] || missing="$missing AULALITE_APP_ORIGIN"
  [ -n "$admin_origin" ] || missing="$missing AULALITE_ADMIN_ORIGIN"
  if [ -n "$missing" ]; then
    echo >&2 "AulaLite web startup refused: missing required publishable Firebase config:$missing"
    echo >&2 "Set the variables above, or use AULALITE_REQUIRE_FIREBASE_CONFIG=false only for local-login development."
    exit 1
  fi

  for origin in "$api_base_url" "$app_origin" "$admin_origin"; do
    case "$origin" in
      https://*) ;;
      *)
        echo >&2 "AulaLite web startup refused: public runtime origins must use HTTPS: $origin"
        exit 1
        ;;
    esac
  done
fi

# Push remains optional. If a VAPID key is supplied, fail early on the Firebase
# fields Messaging requires instead of exposing a button that can never work.
if [ -n "$vapid_key" ]; then
  missing_push=""
  [ -n "$messaging_sender_id" ] || missing_push="$missing_push FIREBASE_MESSAGING_SENDER_ID"
  [ -n "$app_id" ] || missing_push="$missing_push FIREBASE_APP_ID"
  if [ -n "$missing_push" ]; then
    echo >&2 "AulaLite web startup refused: a VAPID key was provided but push config is missing:$missing_push"
    exit 1
  fi
fi

config_path=${AULALITE_RUNTIME_CONFIG_PATH:-/usr/share/nginx/html/runtime-config.js}
index_template=${AULALITE_INDEX_TEMPLATE_PATH:-/usr/share/nginx/html/index.template.html}
index_path=${AULALITE_INDEX_PATH:-/usr/share/nginx/html/index.html}

# Values are base64-encoded before entering JavaScript source. This is not
# secrecy—the values are intentionally publishable—but prevents quotes,
# backslashes, newlines, or JS syntax in an environment value from becoming
# executable source.
base64_value() {
  printf '%s' "$1" | base64 | tr -d '\r\n'
}

api_key_b64=$(base64_value "$api_key")
auth_domain_b64=$(base64_value "$auth_domain")
project_id_b64=$(base64_value "$project_id")
storage_bucket_b64=$(base64_value "$storage_bucket")
messaging_sender_id_b64=$(base64_value "$messaging_sender_id")
app_id_b64=$(base64_value "$app_id")
measurement_id_b64=$(base64_value "$measurement_id")
vapid_key_b64=$(base64_value "$vapid_key")
api_base_url_b64=$(base64_value "$api_base_url")
app_origin_b64=$(base64_value "$app_origin")
admin_origin_b64=$(base64_value "$admin_origin")

cat > "$config_path" <<EOF
// Generated at container startup. Values are publishable Firebase web config.
(function (global) {
  "use strict";
  function decode(value) { return value ? global.atob(value) : ""; }
  global.__AULALITE_FIREBASE_API_KEY__ = decode("$api_key_b64");
  global.__AULALITE_FIREBASE_AUTH_DOMAIN__ = decode("$auth_domain_b64");
  global.__AULALITE_FIREBASE_PROJECT_ID__ = decode("$project_id_b64");
  global.__AULALITE_FIREBASE_STORAGE_BUCKET__ = decode("$storage_bucket_b64");
  global.__AULALITE_FIREBASE_MESSAGING_SENDER_ID__ = decode("$messaging_sender_id_b64");
  global.__AULALITE_FIREBASE_APP_ID__ = decode("$app_id_b64");
  global.__AULALITE_FIREBASE_MEASUREMENT_ID__ = decode("$measurement_id_b64");
  global.__AULALITE_FCM_VAPID_KEY__ = decode("$vapid_key_b64");
  global.__AULALITE_API_BASE_URL__ = decode("$api_base_url_b64");
  global.__AULALITE_APP_ORIGIN__ = decode("$app_origin_b64");
  global.__AULALITE_ADMIN_ORIGIN__ = decode("$admin_origin_b64");
  if (global.location && global.history &&
      global.location.origin === global.__AULALITE_ADMIN_ORIGIN__ &&
      global.location.pathname === "/") {
    global.history.replaceState({}, "", "/platform");
  }
})(globalThis);
EOF

# Couple the runtime-config and app-shell fingerprints. The resulting query is
# also used for service-worker registration, so a new frontend image rotates
# browser caches even when its Firebase settings did not change.
app_root=$(dirname "$index_template")
asset_version=$(
  find "$app_root" -type f \
    ! -name 'index.html' \
    ! -name 'index.template.html' \
    ! -name 'runtime-config.js' \
    -exec cksum {} \; | sort | cksum | awk '{print $1}'
)
config_version=$(
  {
    cksum "$config_path"
    cksum "$index_template"
    printf '%s\n' "$asset_version"
  } | cksum | awk '{print $1}'
)
sed "s/__AULALITE_RUNTIME_CONFIG_VERSION__/$config_version/g" "$index_template" > "$index_path"

echo "AulaLite runtime web config rendered (version $config_version; push=$([ -n "$vapid_key" ] && echo enabled || echo disabled))."
