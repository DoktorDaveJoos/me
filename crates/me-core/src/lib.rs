//! Platform-independent foundation for ME.
//!
//! The vault, profiles, facts, import jobs, and search belong in this crate.
//! Keep GPUI and platform APIs in `me-app`; keep file I/O and expensive work
//! outside the UI thread. Persistence and encryption are not implemented yet.

/// The product name shared by desktop surfaces.
pub const APP_NAME: &str = "ME";

/// The product's central promise.
pub const TAGLINE: &str = "You. Your Data.";
