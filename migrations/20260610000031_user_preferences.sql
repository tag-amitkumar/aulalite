-- UI preferences persisted per user (cross-device). `system` defers the theme
-- to the OS `prefers-color-scheme`; density maps to the kinetics
-- `data-ui-density` contract.
ALTER TABLE users
    ADD COLUMN theme_preference TEXT NOT NULL DEFAULT 'system'
        CONSTRAINT users_theme_preference_check
        CHECK (theme_preference IN ('system', 'light', 'dark')),
    ADD COLUMN density_preference TEXT NOT NULL DEFAULT 'comfortable'
        CONSTRAINT users_density_preference_check
        CHECK (density_preference IN ('compact', 'comfortable', 'spacious'));
