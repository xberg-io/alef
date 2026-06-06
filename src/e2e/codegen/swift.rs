//! Swift e2e test generator using XCTest.
//!
//! Generates a standalone Swift package at `e2e/swift_e2e/` that depends on the
//! binding at `packages/swift/` via `.package(path:)`.
//!
//! IMPORTANT: SwiftPM 6.0 derives the identity of path-based dependencies from
//! the path's *basename* and ignores any explicit `name:` override. If the
//! consumer (`e2e/swift/`) and the dep (`packages/swift/`) share the same path
//! basename `swift`, SwiftPM treats them as the same package and fails
//! resolution with: `product '<X>' required by package 'swift' target '...' not
//! found in package 'swift'`. The e2e package is therefore emitted under
//! `swift_e2e/` to guarantee a distinct identity from any sibling
//! `packages/swift/` dep.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;
use heck::ToUpperCamelCase;
use std::collections::HashMap;
use std::path::PathBuf;

use super::E2eCodegen;

// Empty `result_field_accessor` map shared across calls that don't configure
// one. Using a `OnceLock` lets `render_test_method` hand out a stable
// reference without rebuilding the empty `HashMap` for every fixture.
static EMPTY_FIELD_ACCESSOR_MAP: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();

fn empty_field_accessor_map() -> &'static HashMap<String, String> {
    EMPTY_FIELD_ACCESSOR_MAP.get_or_init(HashMap::new)
}

/// Swift e2e code generator.
pub struct SwiftE2eCodegen;

impl E2eCodegen for SwiftE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        // Emit under `<output>/swift_e2e/` so the consumer's SwiftPM identity
        // (derived from path basename) does not collide with the dep at
        // `packages/swift/` (also basename `swift`). SwiftPM 6.0 deprecated the
        // `name:` parameter on `.package(path:)` and uses the path basename as
        // the package's identity unconditionally, so disambiguation must happen
        // at the filesystem level. Consumers of the alef-emitted e2e must
        // `cd e2e/swift_e2e/` to run `swift test`.
        let output_base = PathBuf::from(e2e_config.effective_output()).join("swift_e2e");

        let mut files = Vec::new();

        // Check if any fixture is an HTTP test (needs app harness and HTTP framework).
        let has_http_fixtures = groups.iter().any(|g| g.fixtures.iter().any(|f| f.is_http_test()));

        // Resolve call config with overrides.
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let function_name = overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call.function.clone());
        let result_var = &call.result_var;
        let result_is_simple = overrides.is_some_and(|o| o.result_is_simple);

        // Resolve package config.
        let swift_pkg = e2e_config.resolve_package("swift");
        let pkg_name = swift_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| config.name.to_upper_camel_case());
        let pkg_path = swift_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/swift".to_string());
        let pkg_version = swift_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .or_else(|| config.resolved_version())
            .unwrap_or_else(|| "0.1.0".to_string());

        // The Swift module name: UpperCamelCase of the package name.
        // Swift convention requires module names to be PascalCase (capitalized).
        // Ensure capitalization even if pkg_name was provided explicitly as lowercase.
        let module_name_str = pkg_name.to_upper_camel_case();
        let module_name = module_name_str.as_str();

        // Resolve the registry URL: derive from the configured repository when
        // available (with a `.git` suffix per SwiftPM convention). Falls back
        // to a vendor-neutral placeholder when no repo is configured.
        let registry_url = config
            .try_github_repo()
            .map(|repo| {
                let base = repo.trim_end_matches('/').trim_end_matches(".git");
                format!("{base}.git")
            })
            .unwrap_or_else(|_| format!("https://example.invalid/{module_name}.git"));

        // Generate Package.swift for the standalone e2e consumer at
        // `<output>/swift_e2e/`. `swift test` is run from that directory.
        files.push(GeneratedFile {
            path: output_base.join("Package.swift"),
            content: project::render_package_swift(
                module_name,
                &registry_url,
                &pkg_path,
                &pkg_version,
                e2e_config.dep_mode,
                has_http_fixtures,
            ),
            generated_header: false,
        });

        // For registry mode, SwiftPM fetches the package directly from GitHub.
        // No pre-test artifact download script is needed.

        // Generate the app harness executable that runs the SUT server for tests.
        // Only emit when there are HTTP fixtures; consumers without HTTP tests
        // don't need the harness or its HTTP framework dependency.
        if has_http_fixtures {
            let app_harness_body = project::render_app_harness(e2e_config, groups, module_name);
            let app_harness_content = format!("{}{}", hash::header(CommentStyle::DoubleSlash), app_harness_body);
            files.push(GeneratedFile {
                path: output_base.join("Sources").join("Harness").join("main.swift"),
                content: app_harness_content,
                generated_header: false,
            });
        }

        // Tests are placed alongside Package.swift under `<output>/swift_e2e/Tests/...`.
        let tests_base = output_base.clone();

        // Build the Swift first-class/opaque classification map for per-segment
        // dispatch in `render_swift_with_first_class_map`. A TypeDef is treated
        // as first-class (Codable struct → property access) when it's not opaque,
        // has serde derives, and every binding field is primitive/optional. This
        // mirrors `can_emit_first_class_struct` in alef-backend-swift.
        let swift_first_class_map = values::build_swift_first_class_map(type_defs, enums, e2e_config);

        let swift_first_class_map_ref = swift_first_class_map;

        // Resolve client_factory override for swift (enables client-instance dispatch).
        let client_factory: Option<&str> = overrides.and_then(|o| o.client_factory.as_deref());

        // Emit a shared TestHelpers.swift that gives `RustString` a
        // `CustomStringConvertible` conformance. swift-bridge generates the
        // `RustString` opaque class but does NOT make it print readably — so
        // any error thrown from a bridge function (the `throw RustString(...)`
        // branches) surfaces in XCTest's failure output as the bare type name
        // `"RustBridge.RustString"`, with the actual Rust error message
        // hidden inside the unprinted instance. The retroactive extension
        // here pulls `.toString()` into `.description` so failures print
        // something diagnostic. Single file per test target; idempotent
        // across regens.
        files.push(GeneratedFile {
            path: tests_base
                .join("Tests")
                .join(format!("{module_name}E2ETests"))
                .join("TestHelpers.swift"),
            content: project::render_test_helpers_swift(),
            generated_header: true,
        });

        // One test file per fixture group.
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                .collect();

            if active.is_empty() {
                continue;
            }

            let class_name = format!("{}Tests", sanitize_filename(&group.category).to_upper_camel_case());
            let filename = format!("{class_name}.swift");
            let content = test_file::render_test_file(
                &group.category,
                &active,
                e2e_config,
                module_name,
                &class_name,
                &function_name,
                result_var,
                &e2e_config.call.args,
                result_is_simple,
                client_factory,
                &swift_first_class_map_ref,
                config,
                type_defs,
                has_http_fixtures,
            );
            files.push(GeneratedFile {
                path: tests_base
                    .join("Tests")
                    .join(format!("{module_name}E2ETests"))
                    .join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "swift"
    }
}

mod accessors;
mod args;
mod assertions;
mod http;
mod project;
mod stubs;
mod test_file;
mod test_method;
mod values;

pub use stubs::emit_test_backend;

#[cfg(test)]
mod tests;
