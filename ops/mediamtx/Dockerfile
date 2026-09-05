# syntax=docker/dockerfile:1.7

# MediaMTX configuration keys and the backend auth callback are version-coupled,
# so keep both the image and config immutable for push deployments.
FROM bluenviron/mediamtx:latest@sha256:f8628851106cc053f9175b248050bb5f362a6e65abd72297c167a1cb5a9a3be2

COPY ops/mediamtx/mediamtx.yml /mediamtx.yml
