-- PKCE (RFC 7636): persist the per-login code_verifier alongside the CSRF
-- state + nonce so /v1/sso/callback can replay it on the token exchange.
-- Nullable so rows written before this migration still resolve;
-- db::sso::take_login_state COALESCEs a NULL to '' (the exchange then omits a
-- verifier the IdP never received a challenge for). Reached EXCLUSIVELY via the
-- existing system_context policies on sso_login_states (no new policy/grant).
ALTER TABLE sso_login_states ADD COLUMN code_verifier TEXT;
