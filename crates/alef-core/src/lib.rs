//! Core types and configuration for alef polyglot binding generator.
//! Defines IR types, config schema, and backend trait.

pub mod backend;
pub mod config;
pub mod error;
pub mod hash;
pub mod ir;
pub mod keywords;
pub mod template_versions;
pub mod version;

pub use backend::{Backend, Capabilities, GeneratedFile};
pub use config::{AlefConfig, resolve_output_dir};
pub use error::AlefError;
pub use ir::ApiSurface;
