pub mod config;
pub mod plugin;
pub mod proxy;
pub mod router;

/// Returns the crate version baked in at compile time.
pub const fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
