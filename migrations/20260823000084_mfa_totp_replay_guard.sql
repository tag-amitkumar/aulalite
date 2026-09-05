-- migrations/20260823000084_mfa_totp_replay_guard.sql
--
-- TOTP replay prevention (RFC 6238 §5.2): track the newest time-step whose
-- code has been consumed for a step-up challenge. A presented code is only
-- accepted when its matched step is STRICTLY newer than the stored value,
-- claimed atomically, so a captured code cannot be replayed within its ±1
-- step validity window (~90 s).
--
-- The column is per-account (user_mfa is global, like MFA itself), and -1
-- means "no code consumed yet". Enrollment confirmation does not consume a
-- step: it completes once and is guarded by confirm_enrollment's row lock.

ALTER TABLE user_mfa
    ADD COLUMN IF NOT EXISTS last_used_totp_step BIGINT NOT NULL DEFAULT -1;
