mod csharp;
mod dart;
mod elixir;
mod ffi;
mod gleam;
mod go;
mod java;
mod jni;
mod kotlin;
#[cfg(test)]
mod migration_containment_tests;
mod node;
pub(crate) mod php;
mod poly;
mod poly_migrations;
mod python;
mod r;
mod ruby;
mod swift;
mod wasm;
mod zig;
mod zig_migrations;

pub(crate) use csharp::scaffold_csharp;
pub use csharp::{
    PUBLISHED_RUNTIME_IDENTIFIERS, render_csharp_csproj, render_csharp_runtime_csproj,
    render_csharp_runtime_json_template,
};
pub(crate) use dart::{migrate_dart_placeholder_test, migrate_dart_pubignore, scaffold_dart};
pub(crate) use elixir::{elixir_native_crate_dir, scaffold_elixir, scaffold_elixir_cargo};
pub(crate) use ffi::scaffold_ffi;
pub(crate) use gleam::scaffold_gleam;
pub(crate) use go::scaffold_go;
pub(crate) use java::{migrate_java_checkstyle_line_length, scaffold_java};
pub(crate) use jni::scaffold_jni;
pub(crate) use kotlin::{migrate_kotlin_build_gradle, scaffold_kotlin};
pub(crate) use node::{migrate_node_package_json_service_export, scaffold_node, scaffold_node_cargo};
pub(crate) use php::{migrate_php_composer_phpunit_constraint, scaffold_php, scaffold_php_cargo};
pub(crate) use poly::scaffold_poly_config;
pub(crate) use poly_migrations::{
    migrate_poly_toml_drop_snippet_hook, migrate_poly_toml_drop_unrunnable_snapshot_hooks,
};
pub(crate) use python::{scaffold_python, scaffold_python_cargo};
pub(crate) use r::{scaffold_r, scaffold_r_cargo};
pub(crate) use ruby::{ruby_native_manifest_path, scaffold_ruby, scaffold_ruby_cargo};
pub(crate) use swift::{migrate_swift_placeholder_test, scaffold_swift};
#[cfg(test)]
pub(crate) use wasm::STALE_WASM_CARGO_CONFIG;
pub(crate) use wasm::{
    migrate_wasm_cargo_config_allow_multiple_definition, migrate_wasm_package_json, scaffold_wasm,
    wasm_cargo_config_file,
};
pub(crate) use zig::scaffold_zig;
pub(crate) use zig_migrations::{
    migrate_build_zig_test_target, migrate_zig_build_ffi_include_default, migrate_zig_example,
};
