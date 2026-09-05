-- The runtime application identity must never be able to read or modify SQLx's
-- migration ledger. Migration 20260517000020 granted DML on every existing
-- public table so the initial least-privilege role could operate the product;
-- that broad bootstrap grant also included this operator-owned metadata table.
-- Keep schema history exclusively behind MIGRATION_DATABASE_URL.

DO $$
BEGIN
    IF to_regclass('public._sqlx_migrations') IS NOT NULL
       AND EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'aulalite_app') THEN
        REVOKE ALL PRIVILEGES ON TABLE public._sqlx_migrations FROM aulalite_app;
    END IF;
END;
$$;
