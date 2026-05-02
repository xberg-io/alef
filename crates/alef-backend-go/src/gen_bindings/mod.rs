mod functions;
mod methods;
pub(super) mod types;

use functions::gen_function_wrapper;
use methods::gen_method_wrapper;
use types::{
    gen_config_options, gen_enum_type, gen_last_error_helper, gen_opaque_type, gen_opaque_type_free_only,
    gen_struct_type, gen_unmarshal_bytes_helper,
};

use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{AdapterPattern, Language, ResolvedCrateConfig, resolve_output_dir};
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, TypeRef};
use heck::ToPascalCase;
use std::collections::HashSet;
use std::fmt::Write;
use std::path::PathBuf;

pub struct GoBackend;

impl GoBackend {
    /// Extract the package name from module path (last segment).
    /// Sanitize by removing hyphens and converting to lowercase.
    fn package_name(module_path: &str) -> String {
        module_path
            .split('/')
            .next_back()
            .unwrap_or("binding")
            .replace('-', "")
            .to_lowercase()
    }
}

impl Backend for GoBackend {
    fn name(&self) -> &str {
        "go"
    }

    fn language(&self) -> Language {
        Language::Go
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: true,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let module_path = config.go_module();
        let pkg_name = config
            .go
            .as_ref()
            .and_then(|g| g.package_name.clone())
            .unwrap_or_else(|| Self::package_name(&module_path));
        let ffi_prefix = config.ffi_prefix();

        let output_dir = resolve_output_dir(config.output_paths.get("go"), &config.name, "packages/go/");

        let ffi_lib_name = config.ffi_lib_name();
        let ffi_header = config.ffi_header_name();
        // Derive the FFI crate directory from the output path (e.g., "crates/html-to-markdown-ffi/src/" → "crates/html-to-markdown-ffi")
        let ffi_crate_dir = config
            .output_paths
            .get("ffi")
            .and_then(|p| {
                let path = p.as_path();
                path.ancestors()
                    .find(|a| {
                        a.file_name()
                            .is_some_and(|n| n != "src" && n != "lib" && n != "include")
                    })
                    .map(|a| a.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| format!("crates/{ffi_lib_name}"));
        // Collect bridge param names from trait_bridges config so we can strip them
        // from generated function signatures and emit ConvertWithVisitor instead.
        let bridge_param_names: HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.param_name.clone())
            .collect();
        // Also collect type aliases used as bridge params (e.g. "VisitorHandle").
        let bridge_type_aliases: HashSet<String> = config
            .trait_bridges
            .iter()
            .filter_map(|b| b.type_alias.clone())
            .collect();
        // Determine if any bridge is configured for the visitor pattern.
        // Options-field bridges generate visitor.go regardless of visitor_callbacks.
        let visitor_callbacks_enabled = config.ffi.as_ref().is_some_and(|f| f.visitor_callbacks);
        let has_options_field_bridge = config
            .trait_bridges
            .iter()
            .any(|b| b.bind_via == alef_core::config::BridgeBinding::OptionsField);
        let has_visitor_bridge =
            has_options_field_bridge || (!config.trait_bridges.is_empty() && visitor_callbacks_enabled);

        // Determine if any plugin-style bridges (with register_fn) are configured.
        // These are independent of visitor_callbacks and generate trait_bridges.go.
        let has_plugin_bridges = config.trait_bridges.iter().any(|b| b.register_fn.is_some());

        // Collect streaming adapter method names — their FFI signature uses callbacks
        // which Go's CGO wrappers can't call directly.
        let streaming_methods: HashSet<String> = config
            .adapters
            .iter()
            .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
            .map(|a| a.name.clone())
            .collect();

        // Collect functions excluded from FFI generation. Go bindings call C symbols directly
        // via cgo, so any function excluded from the FFI header must also be excluded here.
        let ffi_exclude_functions: HashSet<String> = config
            .ffi
            .as_ref()
            .map(|f| f.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();

        let content = format_go_code(&strip_trailing_whitespace(&gen_go_file(
            api,
            config,
            &ffi_prefix,
            &pkg_name,
            &ffi_lib_name,
            &ffi_header,
            &ffi_crate_dir,
            &output_dir,
            &bridge_param_names,
            &bridge_type_aliases,
            &streaming_methods,
            &ffi_exclude_functions,
        )));

        // Build adapter body map (consumed by generators via body substitution)
        let _adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::Go)?;

        // Compute relative path from Go output dir to project root.
        let depth = output_dir.trim_end_matches('/').matches('/').count() + 1;
        let to_root = "../".repeat(depth);

        let mut files = vec![GeneratedFile {
            path: PathBuf::from(&output_dir).join("binding.go"),
            content,
            generated_header: true,
        }];

        // Generate visitor.go when a visitor bridge is configured.
        if has_visitor_bridge {
            // Derive vtable_trait_name and options_field from the first options-field bridge,
            // falling back to sensible defaults for legacy function-param bridges.
            let (vtable_trait_name, options_field) = config
                .trait_bridges
                .iter()
                .find(|b| b.bind_via == alef_core::config::BridgeBinding::OptionsField)
                .and_then(|b| {
                    let field = b.resolved_options_field()?;
                    Some((b.trait_name.clone(), field.to_string()))
                })
                .unwrap_or_else(|| ("HtmlVisitor".to_string(), "visitor".to_string()));

            let visitor_content = strip_trailing_whitespace(&crate::gen_visitor::gen_visitor_file(
                &pkg_name,
                &ffi_prefix,
                &ffi_header,
                &ffi_crate_dir,
                &to_root,
                &vtable_trait_name,
                &options_field,
            ));
            files.push(GeneratedFile {
                path: PathBuf::from(&output_dir).join("visitor.go"),
                content: visitor_content,
                generated_header: true,
            });
        }

        // Generate trait_bridges.go for plugin-style bridges (with register_fn).
        // Per-call bridges (no register_fn) use visitor.go callbacks via convert() instead.
        // This is independent of visitor_callbacks, which only affects per-call bridges.
        if has_plugin_bridges {
            let trait_bridges_content = strip_trailing_whitespace(&super::trait_bridge::gen_trait_bridges_file(
                api,
                config,
                &pkg_name,
                &ffi_prefix,
                &ffi_header,
                &ffi_crate_dir,
                &to_root,
                &config.name,
            ));
            if !trait_bridges_content.trim().is_empty() && trait_bridges_content.len() > 100 {
                files.push(GeneratedFile {
                    path: PathBuf::from(&output_dir).join("trait_bridges.go"),
                    content: trait_bridges_content,
                    generated_header: true,
                });
            }
        }

        Ok(files)
    }

    /// Go bindings are already the public API (single .go file wrapping C FFI).
    /// This returns empty since the binding.go file serves as both the FFI layer
    /// and the high-level public API for consumers.
    fn generate_public_api(
        &self,
        _api: &ApiSurface,
        _config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        // Go's binding.go IS the public API — no additional wrapper needed.
        Ok(vec![])
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "go",
            crate_suffix: "",
            build_dep: BuildDependency::Ffi,
            post_build: vec![],
        })
    }
}

/// Strip trailing whitespace from every line and ensure the file ends with a single newline.
fn strip_trailing_whitespace(content: &str) -> String {
    let mut result: String = content
        .lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Run `gofmt -s` on generated Go code. Falls back to the original if gofmt is unavailable.
fn format_go_code(code: &str) -> String {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let child = Command::new("gofmt")
        .arg("-s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    match child {
        Ok(mut c) => {
            if let Some(ref mut stdin) = c.stdin.take() {
                let _ = stdin.write_all(code.as_bytes());
            }
            match c.wait_with_output() {
                Ok(output) if output.status.success() => {
                    String::from_utf8(output.stdout).unwrap_or_else(|_| code.to_string())
                }
                _ => code.to_string(),
            }
        }
        Err(_) => code.to_string(),
    }
}

/// Returns true if a `TypeRef::Named` type comes from `api.enums` (either unit or data enum)
/// and therefore does not have `_from_json`/`_to_json`/`_free` FFI helpers.
///
/// Only types in `api.types` (non-opaque struct types) have these helpers in the C header.
fn is_ffi_enum_type(name: &str, ffi_enum_names: &HashSet<String>) -> bool {
    ffi_enum_names.contains(name)
}

/// Returns true if a function references an enum type (from `api.enums`) as a parameter type
/// or return type, for which the FFI header lacks `_from_json`/`_to_json`/`_free` helpers.
///
/// Such functions cannot be generated correctly and must be skipped.
fn uses_ffi_enum_type(
    func_params: &[alef_core::ir::ParamDef],
    return_type: &TypeRef,
    ffi_enum_names: &HashSet<String>,
    opaque_names: &std::collections::HashSet<&str>,
) -> bool {
    let named_is_problem = |n: &str| is_ffi_enum_type(n, ffi_enum_names) && !opaque_names.contains(n);
    let return_uses = match return_type {
        TypeRef::Named(n) => named_is_problem(n),
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n) if named_is_problem(n)),
        _ => false,
    };
    if return_uses {
        return true;
    }
    func_params.iter().any(|p| match &p.ty {
        TypeRef::Named(n) => named_is_problem(n),
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n) if named_is_problem(n)),
        _ => false,
    })
}

/// Generate the complete Go binding file wrapping the C FFI layer.
#[allow(clippy::too_many_arguments)]
fn gen_go_file(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    ffi_prefix: &str,
    pkg_name: &str,
    ffi_lib_name: &str,
    ffi_header: &str,
    ffi_crate_dir: &str,
    go_output_dir: &str,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    streaming_methods: &HashSet<String>,
    ffi_exclude_functions: &HashSet<String>,
) -> String {
    let mut out = String::with_capacity(4096);

    // Go convention: generated file marker must appear before package declaration.
    // Blank line after header prevents revive from treating it as package doc.
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    out.push('\n');

    // Compute relative path from Go output dir to project root.
    // go_output_dir is like "packages/go/", so we need "../../" to reach root.
    let depth = go_output_dir.trim_end_matches('/').matches('/').count() + 1;
    let to_root = "../".repeat(depth);

    // Package header and cgo directives.
    // The package comment must immediately precede the package declaration with no blank line.
    writeln!(
        out,
        "// Package {} provides Go bindings for the {} library.",
        pkg_name, config.name
    )
    .ok();
    writeln!(out, "package {}\n", pkg_name).ok();
    writeln!(out, "/*").ok();
    writeln!(out, "#cgo CFLAGS: -I${{SRCDIR}}/{to_root}{ffi_crate_dir}/include").ok();
    writeln!(
        out,
        "#cgo LDFLAGS: -L${{SRCDIR}}/{to_root}target/release -l{ffi_lib_name}"
    )
    .ok();
    writeln!(out, "#include \"{}\"", ffi_header).ok();
    writeln!(out, "*/\nimport \"C\"").ok();
    writeln!(out).ok();
    // Determine which imports are needed based on generated code.
    let has_opaque_types = api.types.iter().any(|t| t.is_opaque);
    // Functions that are not skipped (non-async or with non-Named returns) need json + unsafe.
    // Opaque-returning functions are no longer skipped, so check all non-async functions.
    let has_sync_functions = api.functions.iter().any(|f| !f.is_async);
    let has_non_static_methods = api.types.iter().any(|t| t.methods.iter().any(|m| !m.is_static));
    let needs_json_and_unsafe = has_sync_functions || has_non_static_methods;

    let mut imports = vec!["\"fmt\""];
    if needs_json_and_unsafe {
        imports.insert(0, "\"encoding/json\"");
        imports.push("\"unsafe\"");
    } else if has_opaque_types {
        // Opaque types need unsafe for pointer wrapping even without JSON serialization.
        imports.push("\"unsafe\"");
    }
    if !api.errors.is_empty() {
        imports.insert(1.min(imports.len()), "\"errors\"");
    }
    writeln!(
        out,
        "import (\n{}\n)\n",
        imports
            .iter()
            .map(|i| format!("\t{}", i))
            .collect::<Vec<_>>()
            .join("\n")
    )
    .ok();

    // Error helper functions
    writeln!(out, "{}\n", gen_last_error_helper(ffi_prefix)).ok();

    // Bytes helper: emitted once per package, used by every method/function
    // returning `TypeRef::Bytes`. Defining it here (rather than inline at each
    // call site) avoids repeated declarations and keeps a single place to
    // adjust ownership semantics.
    writeln!(out, "{}\n", gen_unmarshal_bytes_helper()).ok();

    // Generate trait bridge exports (//export trampolines called by C)
    let has_plugin_bridges = config.trait_bridges.iter().any(|b| b.register_fn.is_some());
    if has_plugin_bridges {
        writeln!(out, "// Trait bridge trampolines (exported to C)\n").ok();
        for bridge_cfg in &config.trait_bridges {
            if let Some(trait_def) = api.types.iter().find(|t| t.name == bridge_cfg.trait_name) {
                // IR type names are already PascalCase from Rust source. Avoid
                // ToPascalCase here — it mangles all-caps acronyms (GraphQL ->
                // GraphQl) and disagrees with cbindgen's emitted C type name.
                let pascal = trait_def.name.clone();
                // Trait method trampolines
                for method in &trait_def.methods {
                    let export_name = format!("go{}{}", pascal, method.name.to_pascal_case());
                    writeln!(out, "//export {}", export_name).ok();
                }
                // Plugin lifecycle trampolines
                writeln!(out, "//export go{}Name", pascal).ok();
                writeln!(out, "//export go{}Version", pascal).ok();
                writeln!(out, "//export go{}Initialize", pascal).ok();
                writeln!(out, "//export go{}Shutdown", pascal).ok();
                writeln!(out, "//export go{}FreeUserData", pascal).ok();
            }
        }
        writeln!(out).ok();
    }

    // Generate error types: a single consolidated sentinel `var (...)` block
    // across all ErrorDefs (variant-name collisions are disambiguated by
    // qualifying with the parent error's base name, e.g.
    // `ErrGraphQLValidationError` vs `ErrSchemaValidationError`), followed by
    // the per-error structured error struct + Error() method.
    if !api.errors.is_empty() {
        writeln!(
            out,
            "{}\n",
            alef_codegen::error_gen::gen_go_sentinel_errors(&api.errors)
        )
        .ok();
        for error in &api.errors {
            writeln!(
                out,
                "{}\n",
                alef_codegen::error_gen::gen_go_error_struct(error, pkg_name)
            )
            .ok();
        }
    }

    // When a visitor bridge is active, visitor.go defines NodeContext and VisitResult
    // with FFI-compatible fields. Skip them in binding.go to avoid redeclarations.
    let visitor_types: std::collections::HashSet<&str> = if !bridge_param_names.is_empty() {
        ["NodeContext", "VisitResult"].into_iter().collect()
    } else {
        std::collections::HashSet::new()
    };

    // Generate enum types and constants
    // Only unit enums map to `type X string` — data enums are generated as Go structs below.
    let unit_enum_names: std::collections::HashSet<&str> = api
        .enums
        .iter()
        .filter(|e| e.variants.iter().all(|v| v.fields.is_empty()))
        .map(|e| e.name.as_str())
        .collect();
    for enum_def in api.enums.iter().filter(|e| !visitor_types.contains(e.name.as_str())) {
        writeln!(out, "{}\n", gen_enum_type(enum_def)).ok();
    }

    // Error type names that are also opaque types — in this case the error struct emitted by
    // gen_go_error_types is the Go-side type and the opaque handle definition below would be a
    // duplicate. Skip re-generating the struct for such opaque types; the Free() method is still
    // generated separately.
    let error_names: std::collections::HashSet<&str> = api.errors.iter().map(|e| e.name.as_str()).collect();

    // Collect opaque type names — these are pointer-wrapped handles, not JSON-serializable structs.
    let opaque_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.as_str())
        .collect();

    // Collect all enum type names (both unit and data enums from api.enums).
    // These types do NOT have _from_json/_to_json/_free helpers in the FFI header —
    // only non-opaque api.types have those helpers. Functions that use an enum type
    // as a parameter or return value (via TypeRef::Named) cannot be correctly generated
    // (unless the type also appears as an opaque type in api.types) and are excluded.
    let ffi_enum_names: HashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

    // Generate struct types
    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !visitor_types.contains(typ.name.as_str()))
    {
        if typ.is_opaque {
            // If an error type has the same name as this opaque type, the structured error
            // struct was already emitted by gen_go_error_types. Skip the duplicate struct
            // definition but still emit the Free() method.
            if error_names.contains(typ.name.as_str()) {
                writeln!(out, "{}\n", gen_opaque_type_free_only(typ, ffi_prefix)).ok();
            } else {
                writeln!(out, "{}\n", gen_opaque_type(typ, ffi_prefix)).ok();
            }
        } else {
            writeln!(out, "{}\n", gen_struct_type(typ, &unit_enum_names)).ok();
            // Generate functional options pattern if type has defaults.
            // Skip "Update" types (e.g., ConversionOptionsUpdate) — they are partial update
            // structs that share field names with the primary config type, producing duplicate
            // With* function declarations.
            if typ.has_default && !typ.name.ends_with("Update") {
                writeln!(out, "{}\n", gen_config_options(typ, &unit_enum_names)).ok();
            }
        }
    }

    // Generate free function wrappers.
    // Async functions are included — the underlying FFI uses block_on() for synchronous C calls.
    // Skip functions excluded from FFI generation (their C symbols don't exist in the header)
    // and functions whose parameter or return types are enum types without FFI JSON helpers.
    for func in api.functions.iter().filter(|f| {
        !ffi_exclude_functions.contains(&f.name)
            && !uses_ffi_enum_type(&f.params, &f.return_type, &ffi_enum_names, &opaque_names)
    }) {
        writeln!(
            out,
            "{}\n",
            gen_function_wrapper(func, ffi_prefix, &opaque_names, bridge_param_names, bridge_type_aliases)
        )
        .ok();
    }

    // Generate struct methods.
    // Skip static methods that return Named types (e.g., Default() constructors) —
    // these are redundant with the generated New*() functional options constructors,
    // and the opaque handle conversion pipeline is not yet implemented.
    // Streaming adapter methods use a callback-based C signature that CGO can't call directly —
    // they are skipped here and must be implemented via a separate Go-native streaming API.
    // Also skip methods excluded from FFI or using enum types without FFI JSON helpers.
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        // Types that are both opaque and error types are emitted as Go value
        // structs (Code/Message fields) by `gen_go_error_struct` — they have
        // no `ptr` field to dispatch through. Skip method emission here so we
        // do not generate `h.ptr` references that fail to compile against a
        // value-type struct.
        if typ.is_opaque && error_names.contains(typ.name.as_str()) {
            continue;
        }
        for method in &typ.methods {
            if method.is_static && matches!(method.return_type, TypeRef::Named(_)) {
                continue;
            }
            if streaming_methods.contains(&method.name) {
                continue;
            }
            if ffi_exclude_functions.contains(&method.name) {
                continue;
            }
            if uses_ffi_enum_type(&method.params, &method.return_type, &ffi_enum_names, &opaque_names) {
                continue;
            }
            writeln!(out, "{}\n", gen_method_wrapper(typ, method, ffi_prefix, &opaque_names)).ok();
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::config::NewAlefConfig;

    fn resolved_one(toml: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn make_config() -> ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.ffi]
prefix = "test"
[crates.go]
module = "github.com/test/test-lib"
"#,
        )
    }

    #[test]
    fn test_package_name_extracts_last_segment() {
        assert_eq!(GoBackend::package_name("github.com/org/my-lib"), "mylib");
        assert_eq!(GoBackend::package_name("binding"), "binding");
    }

    #[test]
    fn test_strip_trailing_whitespace_normalizes_lines() {
        let input = "line one   \nline two\n";
        let result = strip_trailing_whitespace(input);
        assert_eq!(result, "line one\nline two\n");
    }

    #[test]
    fn test_is_ffi_enum_type_returns_true_for_known_enum() {
        let mut enum_names = HashSet::new();
        enum_names.insert("Status".to_string());
        assert!(is_ffi_enum_type("Status", &enum_names));
        assert!(!is_ffi_enum_type("Config", &enum_names));
    }

    #[test]
    fn test_generate_bindings_produces_binding_go_file() {
        use alef_core::ir::ApiSurface;
        let config = make_config();
        let api = ApiSurface {
            crate_name: "test-lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        };
        let backend = GoBackend;
        let files = backend.generate_bindings(&api, &config).unwrap();
        assert!(!files.is_empty());
        assert!(files[0].path.to_string_lossy().contains("binding.go"));
    }
}
