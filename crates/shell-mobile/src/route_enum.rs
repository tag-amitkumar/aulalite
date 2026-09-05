// crates/shell-mobile/src/route_enum.rs
/// Mobile and desktop use the same type-safe route graph as web. Keeping this
/// alias in the mobile crate provides a stable native-facing name without
/// duplicating (and eventually drifting from) dozens of route declarations.
pub use shell_web::route_enum::Route as MobileRoute;
