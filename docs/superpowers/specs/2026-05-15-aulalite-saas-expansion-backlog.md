# AulaLite SaaS Expansion Backlog

**Date:** 2026-05-15
**Status:** Backlog created from SaaS readiness refresh

## Current Phase Fixes

- Schedule frontend links use `/schedule` instead of the backend API path `/v1/me/schedule`.
- `/v1/me/schedule?days=` accepts only values from 1 through 180.
- Live-room sockets can authenticate local/demo users through an `access_token` query parameter.
- WebRTC viewer URLs carry the MediaMTX read JWT in the WHEP URL.

## Backend Audit Checklist

| Area | Current-phase result | Next action |
| --- | --- | --- |
| Auth bootstrap and local login | Reviewed during implementation | Keep local/demo profile behavior documented in ops runbook |
| Tenant isolation | Existing tests cover core courses, RLS, assignments, submissions, and live room paths | Expand with admin console tests in SaaS expansion phase |
| Role permissions | Existing permission tests remain part of final verification | Add tenant admin UI permission matrix in SaaS expansion phase |
| API validation | Schedule days validation fixed in this phase | Normalize API error codes and user-facing messages in SaaS expansion phase |
| Live sessions and live room | Socket auth, viewer JWT propagation, and WHEP/WHIP setup reviewed | Add instructor operations dashboard in SaaS expansion phase |
| Migrations | Existing ordering preserved | Add migration linting/checksum workflow in SaaS expansion phase |

## Next SaaS Expansion Candidates

- Billing and subscription plan management.
- Tenant settings and tenant branding controls.
- Admin console for tenant, user, and role operations.
- Analytics and reporting for course activity, attendance, assignment progress, and live sessions.
- Onboarding and guided setup for new organizations.
- Notification settings and delivery.
- Support, help, and operational tooling.
- Deeper audit, compliance, export, and retention workflows.
