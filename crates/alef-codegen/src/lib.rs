//! Shared code generation utilities for all language backends.
//! Provides struct/enum/function generators, type mapping, and conversion helpers.

pub mod builder;
pub mod c_consumer;
pub mod config_gen;
pub mod conversions;
pub mod doc_emission;
pub mod error_gen;
pub mod generators;
pub mod keywords;
pub mod naming;
pub mod shared;
pub mod type_mapper;
