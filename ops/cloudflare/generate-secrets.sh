#!/usr/bin/env bash
# Generate the cryptographic material APP_ENV=production hard-fails without.
# These are self-contained secrets — no third-party account is involved — so
# they are generated locally rather than obtained from a provider.
#
# Outputs (both mode 600, both gitignored):
#   secrets.generated.env  single-line KEY=VALUE pairs
#   jwt-rs256.key          the viewer-JWT signing key, kept as a real PEM
#
# The PEM lives in its own file on purpose: `docker compose --env-file`
# cannot represent a multi-line value, and JwtSigner::from_pem parses the
# PEM directly without unescaping "\n". deploy.sh therefore exports it into
# the shell environment, where real newlines survive, and compose
# interpolates ${JWT_RS256_PRIVATE_KEY_PEM} from there.
set -euo pipefail
cd "$(dirname "$0")"
umask 077

openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out jwt-rs256.key 2>/dev/null
openssl rsa -in jwt-rs256.key -noout -check >/dev/null

{
  echo "# Generated $(date -u +%Y-%m-%dT%H:%M:%SZ) by generate-secrets.sh"
  echo "# Self-contained secrets only. Provider credentials are NOT here."
  echo "# The viewer-JWT PEM is in jwt-rs256.key (multi-line; see deploy.sh)."
  echo "SSO_SESSION_SECRET=$(openssl rand -hex 32)"
  echo "AULALITE_DATA_ENCRYPTION_KEY=$(openssl rand -base64 32)"
  echo "MEDIAMTX_AUTH_SHARED_HEADER=$(openssl rand -hex 32)"
  echo "POSTGRES_PASSWORD=$(openssl rand -hex 24)"
  echo "POSTGRES_RUNTIME_PASSWORD=$(openssl rand -hex 24)"
  echo "AWS_ACCESS_KEY_ID=aulalite-$(openssl rand -hex 6)"
  echo "AWS_SECRET_ACCESS_KEY=$(openssl rand -hex 32)"
} > secrets.generated.env

chmod 600 secrets.generated.env jwt-rs256.key
echo "Wrote secrets.generated.env ($(grep -c '^[A-Z]' secrets.generated.env) vars) and jwt-rs256.key."
echo "NOT generated (provider accounts required): FIREBASE_*, STRIPE_*, RESEND_*"
