# syntax=docker/dockerfile:1.7

# Pin the complete multi-platform manifest. Bump deliberately after testing a
# PostgreSQL 16 minor release and a backup/restore drill.
FROM postgres:16-alpine@sha256:57c72fd2a128e416c7fcc499958864df5301e940bca0a56f58fddf30ffc07777

# The official image executes this only while initializing an empty PGDATA.
COPY --chmod=0555 ops/postgres/10-init-runtime-role.sh /docker-entrypoint-initdb.d/10-init-runtime-role.sh
