#!/usr/bin/env bash
# Copy this file outside the repository, make it executable, and replace the
# final command with the CLI for your chosen encrypted offsite destination.
# postgres-backup.sh supplies all four variables below.

set -Eeuo pipefail

: "${BACKUP_DIRECTORY:?missing BACKUP_DIRECTORY}"
: "${BACKUP_ARCHIVE:?missing BACKUP_ARCHIVE}"
: "${BACKUP_CHECKSUM:?missing BACKUP_CHECKSUM}"
: "${BACKUP_MANIFEST:?missing BACKUP_MANIFEST}"

# Example contract only. The real hook MUST fail when the remote upload or its
# remote checksum/immutability confirmation fails. Do not leave this no-op hook
# configured in production.
printf 'upload hook is not configured; refusing to claim offsite durability for %s\n' \
  "${BACKUP_DIRECTORY}" >&2
exit 64
