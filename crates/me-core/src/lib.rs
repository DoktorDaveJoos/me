//! Platform-independent foundation for ME.
//!
//! The vault, profiles, facts, import jobs, and search belong in this crate.
//! Keep GPUI and platform APIs in `me-app`; keep file I/O and expensive work
//! outside the UI thread. The desktop calls the vault on a background worker.

mod imports;
pub use imports::*;
mod data_filter;
pub use data_filter::*;
mod workspace;
pub use workspace::*;
mod checkpoints;
mod credentials;
mod crypto;
pub use credentials::*;
mod grounding;
pub use checkpoints::extraction_fingerprint;
pub use grounding::*;
mod documents;
mod extraction_fields;
mod knowledge;
pub use extraction_fields::document_field_label;
mod model;
mod review_questions;
mod settings;
pub use review_questions::{ReviewQuestion, question_warning};
mod vault;

pub use documents::*;
pub use knowledge::*;
pub use model::*;
pub use settings::*;
pub use vault::Vault;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Couldn't access the file. Check storage space and permissions.")]
    Io(#[from] std::io::Error),
    #[error("Couldn't process the vault data.")]
    Database(#[from] rusqlite::Error),
    #[error("Incorrect password or damaged vault.")]
    Authentication,
    #[error("The vault format is unknown or damaged.")]
    Format,
    #[error("This vault is open in another instance of ME.")]
    InUse,
    #[error("{0}")]
    Validation(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

/// The product name shared by desktop surfaces.
pub const APP_NAME: &str = "ME.";

/// The product's central promise.
pub const TAGLINE: &str = "It's about you. Your data. Your fingerprints.";
