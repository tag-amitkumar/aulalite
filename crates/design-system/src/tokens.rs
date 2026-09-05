//! Design tokens. Mirrored 1:1 in `assets/tokens.css`.

pub mod color {
    pub const BG: &str = "var(--color-bg)";
    pub const SURFACE: &str = "var(--color-surface)";
    pub const TEXT: &str = "var(--color-text)";
    pub const TEXT_MUTED: &str = "var(--color-text-muted)";
    pub const PRIMARY: &str = "var(--color-primary)";
    pub const PRIMARY_HOVER: &str = "var(--color-primary-hover)";
    pub const SUCCESS: &str = "var(--color-success)";
    pub const WARNING: &str = "var(--color-warning)";
    pub const DANGER: &str = "var(--color-danger)";
    pub const LIVE: &str = "var(--color-live)";
}

pub mod space {
    pub const S1: &str = "var(--space-1)";
    pub const S2: &str = "var(--space-2)";
    pub const S3: &str = "var(--space-3)";
    pub const S4: &str = "var(--space-4)";
    pub const S5: &str = "var(--space-5)";
    pub const S6: &str = "var(--space-6)";
}

pub mod radius {
    pub const SM: &str = "var(--radius-sm)";
    pub const MD: &str = "var(--radius-md)";
    pub const LG: &str = "var(--radius-lg)";
}
