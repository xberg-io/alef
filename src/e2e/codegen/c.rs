//! C e2e test generator using assert.h and a Makefile.
//!
//! Generates `e2e/c/Makefile`, per-category `test_{category}.c` files,
//! a `main.c` test runner, a `test_runner.h` header, and a
//! `download_ffi.sh` script for downloading prebuilt FFI libraries from
//! GitHub releases.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::escape::{escape_c, sanitize_filename};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;
use heck::{ToPascalCase, ToSnakeCase};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;

/// C e2e code generator.
pub struct CCodegen;

/// Returns true when `t` is a primitive C scalar type (uint64_t, int32_t, double,
/// etc.) that should be emitted as a typed local variable rather than a heap
/// `char*` accessor result.
pub(crate) fn is_primitive_c_type(t: &str) -> bool {
    matches!(
        t,
        "uint8_t"
            | "uint16_t"
            | "uint32_t"
            | "uint64_t"
            | "int8_t"
            | "int16_t"
            | "int32_t"
            | "int64_t"
            | "uintptr_t"
            | "intptr_t"
            | "size_t"
            | "ssize_t"
            | "double"
            | "float"
            | "bool"
            | "int"
    )
}

/// Returns `true` when `fields_c_types["{parent}.{field}"]` is the magic
/// sentinel `"skip"` — the C codegen should omit any assertion that touches
/// this field rather than emitting a call to a non-existent FFI function.
fn is_skipped_c_field(fields_c_types: &HashMap<String, String>, parent_snake: &str, field_snake: &str) -> bool {
    let key = format!("{parent_snake}.{field_snake}");
    fields_c_types.get(&key).is_some_and(|t| t == "skip")
}

/// Field names whose declared `fields_c_types` type is a real IR enum, derived from the
/// IR rather than authored in config. `fields_enum` membership is `try_emit_enum_accessor`'s
/// gate (see below): a field absent from it falls through to `infer_opaque_handle_type`,
/// whose match condition (non-primitive, non-`char*`) is a strict superset of the enum
/// arm's — so a genuinely enum-typed field that a config entry simply forgot to list
/// silently renders as an opaque handle instead, and `render_assertion` then emits
/// `strcmp()` against a `uint64_t`. Returning these field names lets the caller union
/// them into the effective `fields_enum` set so the IR can independently satisfy the
/// gate — an override, not the sole source of truth. ~keep
fn enum_fields_from_ir(
    fields_c_types: &HashMap<String, String>,
    enums: &[crate::core::ir::EnumDef],
) -> HashSet<String> {
    fields_c_types
        .iter()
        .filter(|(_, type_name)| enums.iter().any(|e| &e.name == *type_name))
        .filter_map(|(key, _)| key.rsplit('.').next().map(str::to_string))
        .collect()
}

/// The single seam deciding the C "none" sentinel for an omitted optional argument -- `0`
/// for the scalar `AlefHandle` handle representation, `NULL` for a real pointer. See
/// `c::optional_arg` for the full rationale; re-exported here (rather than imported at every
/// use site) so the submodules keep naming it `super::c_optional_sentinel`. ~keep
use optional_arg::{c_optional_sentinel, ir_declares_a_json_string_param, resolve_optional_sentinel};

/// Infer the opaque-handle PascalCase return type for a bare-field accessor.
///
/// Returns `Some(pascal_type)` when the accessor `{prefix}_{parent}_{field}`
/// returns a pointer to an opaque struct (e.g. `SAMPLELLMUsage*`) rather than
/// a `char*` or primitive scalar.
///
/// Detection strategy:
/// 1. Direct lookup `fields_c_types["{parent}.{field}"]` — if present and
///    NOT a primitive AND NOT `char*`, treat as an opaque handle of that
///    PascalCase type.
/// 2. Inferred lookup — when ANY key in `fields_c_types` starts with
///    `"{field}."` (the snake_case of `field` as a parent type), the field
///    must be a struct whose nested fields are mapped. Default the struct
///    type to `field.to_pascal_case()`. This mirrors the fallback used by
///    `emit_nested_accessor` for intermediate segments.
///
/// Returns `None` when the field looks like a `char*` string accessor.
fn infer_opaque_handle_type(
    fields_c_types: &HashMap<String, String>,
    parent_snake_type: &str,
    field_snake: &str,
) -> Option<String> {
    let lookup_key = format!("{parent_snake_type}.{field_snake}");
    if let Some(t) = fields_c_types.get(&lookup_key) {
        if !is_primitive_c_type(t) && t != "char*" {
            return Some(t.clone());
        }
        // Primitive or explicit char* — caller handles those paths.
        return None;
    }
    // Inferred: nested keys exist with `field_snake` as the parent type prefix.
    let nested_prefix = format!("{field_snake}.");
    if fields_c_types.keys().any(|k| k.starts_with(&nested_prefix)) {
        return Some(field_snake.to_pascal_case());
    }
    None
}

/// Try to emit an enum-aware field accessor: when `raw_field`/`resolved_field`
/// is registered in `fields_enum` AND `fields_c_types[parent.field]` resolves
/// to a non-primitive PascalCase type name, treat the accessor return as an
/// opaque enum pointer and convert it to `char*` via the FFI's
/// `{prefix}_{enum_snake}_to_string` accessor.
///
/// Without this, the C codegen would default-declare the accessor result as
/// `char* status = {prefix}_batch_object_status(result);` and string-compare
/// it — but the FFI returns `SAMPLELLMBatchStatus*` (an opaque enum struct
/// pointer), not a C string. The mismatch causes immediate `Abort trap: 6` /
/// `strcmp(NULL,...)` failures in every assertion that targets an enum field.
///
/// Returns `true` when an accessor was emitted (caller must NOT emit the
/// default `char*` declaration). When emitted, the opaque-enum handle is
/// pushed to `intermediate_handles` so the existing cleanup loop frees it via
/// `{prefix}_{enum_snake}_free(...)` after the test body runs.
#[allow(clippy::too_many_arguments)]
fn try_emit_enum_accessor(
    out: &mut String,
    prefix: &str,
    prefix_upper: &str,
    raw_field: &str,
    resolved_field: &str,
    parent_snake_type: &str,
    accessor_fn: &str,
    parent_handle: &str,
    local_var: &str,
    fields_c_types: &HashMap<String, String>,
    fields_enum: &HashSet<String>,
    intermediate_handles: &mut Vec<(String, String)>,
) -> bool {
    if !(fields_enum.contains(raw_field) || fields_enum.contains(resolved_field)) {
        return false;
    }
    let lookup_key = format!("{parent_snake_type}.{resolved_field}");
    let Some(enum_pascal) = fields_c_types.get(&lookup_key) else {
        return false;
    };
    if is_primitive_c_type(enum_pascal) || enum_pascal == "char*" {
        return false;
    }
    let enum_snake = enum_pascal.to_snake_case();
    let handle_var = format!("{local_var}_handle");
    let _ = writeln!(
        out,
        "    {prefix_upper}AlefHandle {handle_var} = {accessor_fn}({parent_handle});"
    );
    let _ = writeln!(out, "    assert({handle_var} != 0);");
    let _ = writeln!(
        out,
        "    char* {local_var} = {prefix}_{enum_snake}_to_string({handle_var});"
    );
    intermediate_handles.push((handle_var, enum_snake));
    true
}

impl E2eCodegen for CCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
        functions: &[crate::core::ir::FunctionDef],
        errors: &[crate::core::ir::ErrorDef],
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);
        let ir = CallIr { functions, type_defs };

        let mut files = Vec::new();

        // Resolve default call config with overrides.
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let result_var = call.effective_result_var();
        let prefix = effective_c_prefix(call, config);
        let header = overrides
            .and_then(|o| o.header.as_ref())
            .cloned()
            .unwrap_or_else(|| config.ffi_header_name());

        // Resolve package config.
        let c_pkg = e2e_config.resolve_package("c");
        // lib_name is the actual Rust library name (for linking)
        let lib_name = config.ffi_lib_name();

        // ffi_pkg_name is the release artifact package name (for downloads).
        // Derived from lib_name (for example, "sample_ffi" stays "sample_ffi") because
        // the publish workflow stages tarballs as "${lib_name}-v${VERSION}-${TRIPLE}.tar.gz".
        // The explicit e2e package name is a fallback for edge cases where the release
        // artifact name differs from the library name.
        let ffi_pkg_name = c_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| lib_name.clone());

        // Filter active groups (with non-skipped fixtures).
        let active_groups: Vec<(&FixtureGroup, Vec<&Fixture>)> = groups
            .iter()
            .filter_map(|group| {
                let active: Vec<&Fixture> = group
                    .fixtures
                    .iter()
                    .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                    .filter(|f| f.visitor.is_none())
                    .collect();
                if active.is_empty() { None } else { Some((group, active)) }
            })
            .collect();

        // Collect active visitor fixtures (flattened across all groups).
        let visitor_fixtures: Vec<&Fixture> = groups
            .iter()
            .flat_map(|group| group.fixtures.iter())
            .filter(|f| super::should_include_fixture(f, lang, e2e_config))
            .filter(|f| f.visitor.is_some())
            .filter(|f| c_visitor_fixture_has_typed_call(f, e2e_config, ir))
            .collect();

        // Resolve FFI crate path for local repo builds.
        // Default to `../../crates/{name}-ffi` derived from the crate name so that
        // projects with named FFI crates resolve to `../../crates/{name}-ffi/include/`
        // rather than the generic (incorrect) `../../crates/ffi`.
        // When `[crates.output] ffi` is set explicitly, derive the crate path from
        // that value so that renamed FFI crates (e.g. `parser-core-core-ffi`) resolve
        // correctly without any hardcoded special cases.
        let ffi_crate_path = c_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| config.ffi_crate_path_from(&format!("{}/c", e2e_config.effective_output())))
            .map_err(anyhow::Error::msg)?;

        // Generate Makefile.
        let mut category_names: Vec<String> = active_groups
            .iter()
            .map(|(g, _)| sanitize_filename(&g.category))
            .collect();
        if !visitor_fixtures.is_empty() {
            category_names.push("visitor".to_string());
        }
        let needs_mock_server = active_groups
            .iter()
            .flat_map(|(_, fixtures)| fixtures.iter())
            .any(|f| f.needs_mock_server());
        files.push(GeneratedFile {
            path: output_base.join("Makefile"),
            content: render_makefile(&category_names, &header, &ffi_crate_path, &lib_name, needs_mock_server)?,
            generated_header: true,
        });

        // Generate download_ffi.sh for downloading prebuilt FFI from GitHub releases.
        let github_repo = config.github_repo();
        let version = config.resolved_version().unwrap_or_else(|| "0.0.0".to_string());
        files.push(GeneratedFile {
            path: output_base.join("download_ffi.sh"),
            content: render_download_script(&github_repo, &version, &ffi_pkg_name),
            generated_header: true,
        });

        // Generate test_runner.h.
        files.push(GeneratedFile {
            path: output_base.join("test_runner.h"),
            content: render_test_runner_header(&active_groups, &visitor_fixtures),
            generated_header: true,
        });

        // Generate main.c.
        files.push(GeneratedFile {
            path: output_base.join("main.c"),
            content: render_main_c(&active_groups, &visitor_fixtures, &e2e_config.env),
            generated_header: true,
        });

        // Generate .gitignore so locally-built binaries and mock-server pipe
        // artifacts are never accidentally checked in. A committed macOS Mach-O
        // `run_tests` binary will fail Linux CI with `Exec format error`.
        files.push(GeneratedFile {
            path: output_base.join(".gitignore"),
            content: render_gitignore(),
            generated_header: false,
        });

        let field_resolver = FieldResolver::new(
            &e2e_config.fields,
            &e2e_config.fields_optional,
            &e2e_config.result_fields,
            &e2e_config.fields_array,
            &std::collections::HashSet::new(),
        );

        // Generate per-category test files.
        // Each fixture may reference a named call config (fixture.call), so we pass
        // e2e_config to render_test_file so it can resolve per-fixture call settings.
        for (group, active) in &active_groups {
            let filename = format!("test_{}.c", sanitize_filename(&group.category));
            let content = render_test_file(
                &group.category,
                active,
                &header,
                &prefix,
                result_var,
                e2e_config,
                lang,
                &field_resolver,
                config,
                type_defs,
                enums,
                errors,
                ir,
            )?;
            files.push(GeneratedFile {
                path: output_base.join(filename),
                content,
                generated_header: true,
            });
        }

        // Generate test_visitor.c if there are visitor fixtures.
        if !visitor_fixtures.is_empty() {
            files.push(GeneratedFile {
                path: output_base.join("test_visitor.c"),
                content: render_visitor_test_file(&visitor_fixtures, &header, &prefix, e2e_config, config, ir)?,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn render_snippet_body(
        &self,
        fixture: &Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
    ) -> Result<String> {
        render_c_snippet(fixture, e2e_config, config, type_defs, &[])
    }

    fn render_snippet_body_with_functions(
        &self,
        fixture: &Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
        functions: &[crate::core::ir::FunctionDef],
        _errors: &[crate::core::ir::ErrorDef],
    ) -> Result<String> {
        render_c_snippet(fixture, e2e_config, config, type_defs, functions)
    }

    fn language_name(&self) -> &'static str {
        "c"
    }
}

fn render_c_snippet(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    functions: &[crate::core::ir::FunctionDef],
) -> Result<String> {
    let ir = CallIr { functions, type_defs };
    let mut info = resolve_fixture_call_info(fixture, e2e_config, config, "c", ir);
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let prefix = effective_c_prefix(call, config);
    if info.client_factory.is_none()
        && info.c_engine_factory.is_none()
        && !prefix.is_empty()
        && !info.function_name.starts_with(&format!("{prefix}_"))
    {
        info.function_name = crate::codegen::naming::abi_symbol(&prefix, &info.function_name);
    }
    let header = call
        .overrides
        .get("c")
        .and_then(|value| value.header.clone())
        .unwrap_or_else(|| config.ffi_header_name());
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    let resolver = FieldResolver::new(
        e2e_config.effective_fields(call),
        e2e_config.effective_fields_optional(call),
        e2e_config.effective_result_fields(call),
        e2e_config.effective_fields_array(call),
        e2e_config.effective_fields_method_calls(call),
    )
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields);
    test_function::render_snippet_body(test_function::SnippetContext {
        fixture,
        e2e_config,
        header: &header,
        prefix: &prefix,
        info: &info,
        field_resolver: &resolver,
        config,
        type_defs,
        ir,
    })
}

fn effective_c_prefix(call: &CallConfig, config: &ResolvedCrateConfig) -> String {
    call.overrides
        .get("c")
        .and_then(|value| value.prefix.clone())
        .unwrap_or_else(|| config.ffi_prefix())
}

fn effective_c_header(call: &CallConfig, config: &ResolvedCrateConfig) -> String {
    call.overrides
        .get("c")
        .and_then(|value| value.header.clone())
        .unwrap_or_else(|| config.ffi_header_name())
}

/// Resolve per-call-config C-specific settings for a given call config and lang.
struct ResolvedCallInfo {
    function_name: String,
    /// Not a `String`: a call whose result type nothing real names must fail at the point the
    /// emitter needs the name, not silently become a PascalCased call name. Paths that never
    /// name a result type — `returns_void` calls, streaming adapters, the `raw_c_result_type`
    /// scalar path — never call [`ResultTypeName::require`] and are unaffected. ~keep
    result_type_name: ResultTypeName,
    options_type_name: String,
    client_factory: Option<String>,
    args: Vec<crate::e2e::config::ArgMapping>,
    raw_c_result_type: Option<String>,
    c_free_fn: Option<String>,
    c_engine_factory: Option<String>,
    result_is_option: bool,
    returns_void: bool,
    /// When `true`, the FFI signature for this method follows the byte-buffer
    /// out-pointer pattern: `int32_t fn(this, req, uint8_t** out_ptr,
    /// uintptr_t* out_len, uintptr_t* out_cap)`. The C codegen emits out-param
    /// declarations, a status-code check, and `<prefix>_free_bytes` rather
    /// than treating the result as an opaque response handle.
    result_is_bytes: bool,
    streaming: Option<bool>,
    /// Per-language `extra_args` from call overrides — verbatim trailing
    /// arguments appended after the configured `args`. The C codegen passes
    /// `NULL` for absent optional pointers via this mechanism.
    extra_args: Vec<String>,
}

/// The core-IR seam this backend resolves calls through, shared with every other e2e backend.
///
/// These lived here until each backend needed them; the definitions and their rationale are now
/// in [`super::call_ir`]. Re-exported rather than re-imported at every use site so the `c`
/// submodules keep naming them `super::CallIr` / `super::named_type`. ~keep
pub(super) use super::call_ir::{CallIr, TargetParams, named_type};

fn resolve_call_info(
    call: &CallConfig,
    lang: &str,
    ir: CallIr<'_>,
    trait_bridge_registry_identity: Option<&str>,
) -> ResolvedCallInfo {
    let overrides = call.overrides.get(lang);
    let function_name = overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call.function.clone());
    // Fall back to the *base* (non-C-overridden) function name when no explicit
    // result_type is set.  Using the C-overridden name (e.g. "htm_convert") would
    // produce a doubled-prefix type like `HTMHtmConvert*`; the base name
    // ("convert") yields the correct `HTMConvert*` shape.
    let result_type_name = overrides
        .and_then(|o| o.result_type.as_ref())
        .cloned()
        .inspect(|configured| warn_if_result_type_override_disables_verification(configured, call, lang))
        .or_else(|| resolve_ir_result_type(call, lang, ir))
        .map(ResultTypeName::Resolved)
        .unwrap_or_else(|| unresolved_result_type_name(call, lang, ir, trait_bridge_registry_identity));
    let options_type_name = overrides
        .and_then(|o| o.options_type.as_deref())
        .or(call.options_type.as_deref())
        .unwrap_or_default()
        .to_string();
    let client_factory = overrides.and_then(|o| o.client_factory.as_ref()).cloned();
    let raw_c_result_type = overrides
        .and_then(|o| o.raw_c_result_type.clone())
        .or_else(|| return_shape::resolve_raw_c_result_type(call, lang, ir));
    let c_free_fn = overrides.and_then(|o| o.c_free_fn.clone());
    let c_engine_factory = overrides.and_then(|o| o.c_engine_factory.clone());
    let result_is_option = overrides
        .and_then(|o| if o.result_is_option { Some(true) } else { None })
        .unwrap_or(call.result_is_option);
    let returns_void = call.returns_void;
    // result_is_bytes is read from either the call-level config (preferred —
    // the byte-buffer FFI shape is identical across languages that use the
    // same FFI crate) or the per-language override (back-compat with the
    // pattern used by Java / PHP / etc.).
    let result_is_bytes = call.result_is_bytes || overrides.is_some_and(|o| o.result_is_bytes);
    let extra_args = overrides.map(|o| o.extra_args.clone()).unwrap_or_default();
    let mut args = call.args.clone();
    // `ir` is the Rust core's IR, so this lookup wants the Rust identity and must NOT
    // resolve `overrides.c.function` — that names a prefixed C export (`samplellm_chat`), not
    // the Rust function (`chat`). `core_lookup_name` keeps the base name as the key and only
    // supplies a fallback when the base names nothing at all, which stops the key degrading
    // to `""` and silently deriving arg/result types from the empty string. ~keep
    let core_lookup_name = call.core_lookup_name(lang);
    if let Some(signature) = core_lookup_name.as_deref().and_then(|name| ir.signature(name)) {
        for (index, arg) in args.iter_mut().enumerate() {
            if arg.element_type.is_some() || arg.arg_type != "json_object" {
                continue;
            }
            let parameter = signature
                .params
                .iter()
                .find(|parameter| parameter.name == arg.name)
                .or_else(|| signature.params.get(index));
            arg.element_type = parameter
                .and_then(|parameter| named_type(&parameter.ty))
                .map(str::to_string);
        }
    }
    ResolvedCallInfo {
        function_name,
        result_type_name,
        options_type_name,
        client_factory,
        args,
        raw_c_result_type,
        c_free_fn,
        c_engine_factory,
        result_is_option,
        returns_void,
        result_is_bytes,
        streaming: call.streaming_enabled(),
        extra_args,
    }
}

/// Name the type a call's result handle points at, read from the core IR.
///
/// The declared return type is already the `Ok` type: the extractor splits `Result<T, E>`
/// into `return_type = T` plus a separate `error_type`, so a fallible
/// `fn complete(..) -> Result<CompletionResponse, String>` resolves to `CompletionResponse`.
///
/// The named type is reached through [`named_type`], the recursive unwrapper this module
/// already uses for argument element types — a second, one-level-deep match sitting beside it
/// answered `None` for `Result<Vec<Model>, E>` and every other nesting, and every `None` here
/// lands on [`unresolved_result_type_name`].
///
/// The lookup goes through [`CallIr::signature`], so a call naming an inherent or trait method
/// resolves too; `ApiSurface::functions` alone would answer `None` for every one of them.
fn resolve_ir_result_type(call: &CallConfig, lang: &str, ir: CallIr<'_>) -> Option<String> {
    let lookup_name = call.core_lookup_name(lang)?;
    let signature = ir.signature(&lookup_name)?;
    named_type(signature.return_type).map(str::to_string)
}

/// Warn when a per-language `result_type` override names a primitive/pointer C spelling
/// (`char*`, `int32_t`, `uintptr_t`, ...) rather than the PascalCase IR struct name the field
/// doc describes.
///
/// `overrides.result_type` short-circuits [`resolve_call_info`]'s `.or_else()` chain before
/// both `resolve_ir_result_type` and [`unresolved_result_type_name`] ever run — so unlike the
/// unresolvable-call case those two cover (which now fails generation, per the `~keep` above), a
/// primitive spelling typed into `result_type` reaches no diagnostic at all. It still becomes
/// `result_type_name`, which is both the accessor prefix and the `parent_is_ir_type` flag
/// `ensure_leaf_field_exists` reads — no IR type is ever named `"char*"`, so nested-field
/// verification silently turns off for the call, exactly as it would via the fallback path,
/// but invisibly. A call whose result genuinely carries no named fields has a documented way
/// to say so (`result_is_bytes` / `result_is_simple` / the Zig-only `result_is_json_struct`,
/// all checked at [`unresolved_result_type_name`]) — `result_type` is not it. ~keep
fn warn_if_result_type_override_disables_verification(configured: &str, call: &CallConfig, lang: &str) {
    if is_primitive_c_type(configured) || configured == "char*" || configured.ends_with('*') {
        tracing::warn!(
            call = %call.function,
            language = %lang,
            result_type = %configured,
            "call/override declares `result_type` as a primitive/pointer C spelling rather than \
             a PascalCase IR type name, which disables nested-field verification for this call \
             because no IR type will ever match this name — if the result genuinely carries no \
             named fields to verify, declare that with `result_is_bytes` / `result_is_simple` \
             instead"
        );
    }
}

/// Stands in for a call whose name is empty in both the base config and the per-language
/// override, so a diagnostic never interpolates to nothing. ~keep
const UNNAMED_CALL_DIAGNOSTIC: &str = "<call with no configured name>";

/// The result type a C call will be emitted against, together with what backs the name.
///
/// The emitter builds three different things out of this one name — the accessor prefix
/// (`{prefix}_{result_snake}_{leaf}`), the cleanup call (`{prefix}_{result_snake}_free`), and
/// the `parent_is_ir_type` flag `ensure_leaf_field_exists` reads. Handing all three a
/// PascalCased *call* name, as this module did before, was self-concealing: the fabricated
/// type matched no IR type, so `ensure_leaf_field_exists` default-allowed every leaf under it
/// and the very check that would have caught the fabrication was switched off by the
/// fabrication. Carrying the outcome rather than a bare `String` forces the emitter to ask for
/// the name through [`ResultTypeName::require`], and asking is where an unresolvable one turns
/// into a generation error instead of a guess. ~keep
pub(super) enum ResultTypeName {
    /// Backed by something real: an explicit `result_type` call override, or the declared
    /// return type the core IR gives for this call.
    Resolved(String),
    /// Derived from the call name in a case where nothing downstream reads it as a claim that
    /// a type of that name exists:
    ///
    /// - No IR was supplied at all (unit tests and the visitor call sites construct a
    ///   [`CallIr`] from empty slices deliberately). `type_defs` is then empty, so every
    ///   IR-keyed check has no data either way and none is lost by the derived name.
    /// - The call/override already declares the result carries no named fields
    ///   (`result_is_bytes` / `result_is_simple` / the Zig-only `result_is_json_struct`), which
    ///   is the config's own statement that there is no named type and no nested field.
    /// - The call resolves to a trait-bridge registry function (`register_fn` / `unregister_fn`
    ///   / `clear_fn` on `[[crates.trait_bridges]]`). Those are FFI exports the backend
    ///   generates itself, not core IR functions, so they never resolve against `ir` — and a
    ///   registry register/unregister/clear operation returns a status code, not a named
    ///   response type, so there is no result to verify in the first place.
    ///
    /// The three cases are NOT interchangeable at the point the emitter decides what the call
    /// returns, so the basis travels with the name -- see [`UnverifiedBasis`]. ~keep
    Unverified { name: String, basis: UnverifiedBasis },
    /// The IR was available, the call resolves to nothing in it (absent, or ambiguous per
    /// [`CallIr::signature`]), and no config declaration says the result has no named type.
    /// There is nothing real to name here, so emitting fails rather than inventing one.
    Unresolvable { call: String, language: String },
}

/// Why a [`ResultTypeName::Unverified`] name is not backed by a real type.
///
/// The three cases answer "what does this call return" differently, and collapsing them into a
/// bare name is what let a failed type lookup masquerade as a positive statement that the call
/// returns an owned opaque handle. That inference is unsound in both directions: it emitted
/// `{PREFIX}AlefHandle result = f(...)` for an `i32` status and then passed the status to
/// `{prefix}_..._free`, which frees an alef `Box` -- heap corruption in the emitted C, reached
/// by every call whose result type failed to resolve, not only by trait bridges. Mirrors
/// `assertions::TargetParams`, which splits the same ambiguity on the argument axis. ~keep
pub(super) enum UnverifiedBasis {
    /// No IR was supplied at all (unit tests and the visitor call sites construct a [`CallIr`]
    /// from empty slices deliberately), so nothing was consulted and nothing was learned.
    ///
    /// Nothing contradicts the pre-existing opaque-handle derivation either, and refusing here
    /// would fail every IR-less caller -- a far larger blast radius than the defect being
    /// fixed. Same trade, and same reasoning, as `assertions::TargetParams::IrAbsent`; the
    /// two halves of one rule must agree on what an absent IR licenses. ~keep
    IrAbsent,
    /// The call/override declares the result carries no named fields (`result_is_bytes` /
    /// `result_is_simple` / the Zig-only `result_is_json_struct`).
    ///
    /// This is the config's own statement that the result is NOT a named struct, so it is
    /// positive evidence against the opaque-handle shape rather than mere silence. Paths that
    /// only need something to call the result by (the byte-buffer out-pointer shape) keep
    /// working through [`ResultTypeName::require`]; paths that would bind the result to a
    /// handle and free it are refused by [`ResultTypeName::require_owned_handle`]. ~keep
    DeclaredNonStruct,
    /// The call resolves to a trait-bridge registry function (`register_fn` / `unregister_fn` /
    /// `clear_fn` on `[[crates.trait_bridges]]`).
    ///
    /// Alef generates these exports itself, so their C return shape is known rather than
    /// guessed: `register_fn_header.jinja`, `unregister_fn.jinja` and `clear_fn.jinja` all
    /// declare `-> i32`, with `0` for success and `1` for failure. There is no result handle at
    /// all, which is why they never resolve against the core IR -- and why binding one to
    /// `{PREFIX}AlefHandle` and freeing it was never a naming slip. ~keep
    TraitBridgeRegistry,
}

impl ResultTypeName {
    /// The name to emit, or the generation error that replaces the name this used to invent.
    ///
    /// This resolves against the core IR, not against the header the run is about to emit,
    /// because the emitted symbol set is not reachable from here: neither
    /// [`E2eCodegen::generate`] nor either snippet entry point receives it — they receive IR
    /// slices (`type_defs`, `enums`, `functions`) and nothing else. Checking a result type
    /// against the symbols that will actually exist would mean threading
    /// `cli::pipeline::generate::header_freshness::scan_generated_ffi_source`'s
    /// `BTreeMap<symbol, Option<cfg>>` down into the generator. Until that is threaded, the IR
    /// is the only real thing available to resolve against, and failing loudly beats a
    /// plausible wrong name. ~keep
    ///
    /// Naming the result after the call is what this replaced: it emitted
    /// `{prefix}_{result}_{field}` accessors and a `{prefix}_{result}_free` cleanup for a type
    /// the generated header never declares, and it silently switched nested-field verification
    /// off for the fixture, because no IR type matches an invented name. ~keep
    pub(super) fn require(&self) -> Result<&str> {
        match self {
            Self::Resolved(name) | Self::Unverified { name, .. } => Ok(name),
            Self::Unresolvable { call, language } => anyhow::bail!(
                "C e2e codegen cannot name the result type of call `{call}` for language \
                 `{language}`: it resolves to no core IR function or method with a named return \
                 type. Set `result_type` on the call's `{language}` override, or declare that the \
                 result carries no named fields (`result_is_bytes` / `result_is_simple`)."
            ),
        }
    }

    /// The name to emit on a path that binds the call's result to `{PREFIX}AlefHandle` and
    /// hands it to `{prefix}_{result_snake}_free`.
    ///
    /// Deliberately stricter than [`require`](Self::require). `require` answers "what is this
    /// result called", which the byte-buffer and streaming shapes ask without ever taking
    /// ownership of a handle. This answers a different question -- "may this result be owned
    /// and freed as an alef `Box`" -- and the failure to resolve a type name is never an
    /// affirmative answer to it. Passing a non-handle to a generated `_free` corrupts the heap
    /// in the emitted C, so the two `Unverified` bases that positively contradict the handle
    /// shape refuse here even though they still have a usable name. ~keep
    pub(super) fn require_owned_handle(&self) -> Result<&str> {
        let name = self.require()?;
        match self {
            Self::Unverified {
                basis: UnverifiedBasis::DeclaredNonStruct,
                ..
            } => anyhow::bail!(
                "C e2e codegen cannot bind the result of `{name}` to an opaque handle: the call \
                 declares its result carries no named fields (`result_is_bytes` / \
                 `result_is_simple` / `result_is_json_struct`). Set `raw_c_result_type` on the \
                 call's `c` override to the C spelling the export returns (`char*`, `int32_t`, \
                 `uintptr_t`, ...), or set `result_type` to the real handle type."
            ),
            // Reaching here is an alef-side routing bug, not a config error: the status-code
            // emission in `test_function.rs` should have claimed this call before any handle path
            // did. The `-> i32` return is declared by `src/backends/ffi/templates/`. ~keep
            Self::Unverified {
                basis: UnverifiedBasis::TraitBridgeRegistry,
                ..
            } => anyhow::bail!(
                "C e2e codegen cannot bind the result of trait-bridge registry export `{name}` to \
                 an opaque handle: `register_fn` / `unregister_fn` / `clear_fn` exports return an \
                 `i32` status code, not a handle."
            ),
            Self::Resolved(_) | Self::Unverified { .. } | Self::Unresolvable { .. } => Ok(name),
        }
    }

    /// True when the C export this call names returns an `i32` status code rather than a
    /// result the emitted test could own, assert on, or free.
    ///
    /// Positive knowledge, not a fallback: the only calls that answer `true` are trait-bridge
    /// registry exports, which alef generates from its own templates and which all declare
    /// `-> i32`. Every branch that presupposes a different return shape -- a client method, an
    /// engine factory, an opaque handle -- must consult this first, because a wrong shape here
    /// is not a cosmetic mismatch: it emits a free for a value that is not a heap allocation. ~keep
    pub(super) fn returns_status_code(&self) -> bool {
        matches!(
            self,
            Self::Unverified {
                basis: UnverifiedBasis::TraitBridgeRegistry,
                ..
            }
        )
    }
}

/// Classify a call whose result type neither config nor the IR named.
///
/// `trait_bridge_registry_identity` is the derived C identity
/// ([`crate::e2e::codegen::recipe::trait_bridge_derived_c_identity`]'s second tuple element,
/// e.g. `"clear_validator"`) when the caller has already matched this call against a
/// `[[crates.trait_bridges]]` `register_fn` / `unregister_fn` / `clear_fn`, or `None` for an
/// ordinary call. It is threaded in rather than recomputed here because the match needs
/// `ResolvedCrateConfig` and the fixture, neither of which this function (or [`resolve_call_info`],
/// its only caller) otherwise takes — widening this signature to the whole config just to run a
/// lookup already available at the call site would be a worse trade than one extra parameter.
///
/// Three of the four arms still derive a name from the call, and all three are cases where the
/// derived name is provably not read as a type claim — see [`ResultTypeName::Unverified`]. The
/// fourth is the authoring gap: the IR was there to consult, the call is not in it, nothing
/// declares that it has no named result, and it does not name a trait-bridge registry function
/// either. That one used to warn and hand back the invented name anyway, which is how a suite
/// could be generated with field verification off for a fixture and nothing but a log line said
/// so. It is now an error, raised where the name would have been emitted. ~keep
fn unresolved_result_type_name(
    call: &CallConfig,
    lang: &str,
    ir: CallIr<'_>,
    trait_bridge_registry_identity: Option<&str>,
) -> ResultTypeName {
    let result_type = call.function.to_pascal_case();
    // Checked BEFORE `ir.is_absent()`, unlike the other two arms: a registry export is matched
    // against `[[crates.trait_bridges]]` config, which is available whether or not any IR is,
    // so an absent IR tells us strictly less about this call than the config already does.
    // Ordering it second made an IR-less run classify a bridge call as `IrAbsent` -- "nothing
    // is known" -- when its return shape is in fact fully known, and `IrAbsent` is the one
    // basis that still licenses the opaque-handle path. That is how a status code reached
    // `{prefix}_..._free`. ~keep
    //
    // A registry register/unregister/clear export is generated by the FFI backend itself
    // (`src/backends/ffi/trait_bridge/registration.rs`), never appears in the core IR, and
    // returns an `i32` status code -- there is no named response type it could ever resolve
    // to, so this is not the authoring gap the `Unresolvable` arm below exists to catch. The
    // derived identity (not `call.function`, which is legitimately blank when the call names
    // itself only per language -- exactly the shape a bridge call takes) keeps the name real
    // and non-empty rather than collapsing to the degenerate `{prefix}__free` a blank
    // PascalCase produced before `fallback_result_type_name` was removed. ~keep
    if let Some(identity) = trait_bridge_registry_identity {
        let result_type = identity.to_pascal_case();
        tracing::debug!(
            call = %call.function,
            language = %lang,
            %result_type,
            "call resolves to a trait-bridge registry function (register_fn / unregister_fn / \
             clear_fn), which is a generated FFI export with no core IR counterpart and no named \
             result to verify"
        );
        return ResultTypeName::Unverified {
            name: result_type,
            basis: UnverifiedBasis::TraitBridgeRegistry,
        };
    }
    if ir.is_absent() {
        tracing::debug!(
            call = %call.function,
            language = %lang,
            %result_type,
            "no core IR available to this generator; result type derived from the call name"
        );
        return ResultTypeName::Unverified {
            name: result_type,
            basis: UnverifiedBasis::IrAbsent,
        };
    }
    if call_declares_non_struct_result(call, lang) {
        tracing::debug!(
            call = %call.function,
            language = %lang,
            %result_type,
            "call did not resolve to a core IR function or method with a named return type, but \
             the call/override already declares the result carries no named fields \
             (result_is_bytes / result_is_simple / result_is_json_struct) — there is no named \
             type to set and no nested field for the derived type to hide"
        );
        return ResultTypeName::Unverified {
            name: result_type,
            basis: UnverifiedBasis::DeclaredNonStruct,
        };
    }
    // WARN, not ERROR: whether this is fatal depends on which emission path the call takes. A
    // `raw_c_result_type` call — `char*` derived from a `Vec<String>` return, say — renders
    // correctly without ever naming a result type, so classifying here is "degraded but
    // continuing". The unrecoverable case is reported by [`ResultTypeName::require`] at the point
    // of use, where there is enough context to say what would otherwise have been emitted. ~keep
    // Name the call by the symbol this language actually emits, not by the raw base `function`.
    // The base is legitimately empty when a call names itself only per language, and a diagnostic
    // whose whole job is to tell an author which call to fix is worse than useless when it
    // interpolates to the empty string. ~keep
    let call_name = call.effective_function(lang).unwrap_or(UNNAMED_CALL_DIAGNOSTIC);
    tracing::warn!(
        call = %call_name,
        language = %lang,
        "call did not resolve to a core IR function or method with a named return type and \
         declares no non-struct result; there is no real type to name, so any emission path that \
         needs one now fails rather than inventing it — set `result_type` on the call override"
    );
    ResultTypeName::Unresolvable {
        call: call_name.to_string(),
        language: lang.to_string(),
    }
}

/// True when the call/override already declares that the result carries no named fields to
/// verify: `result_is_bytes` (raw byte buffer), `result_is_simple` (a bare scalar), or the
/// Zig-only `result_is_json_struct` escape hatch (an opaque JSON blob the Zig generator parses
/// and verifies structurally, not through named-field lookup). [`unresolved_result_type_name`]'s
/// error arm exists to catch a genuine authoring gap — a call that SHOULD have resolved to a named
/// IR type but didn't, so no real type can be named — and none of these three flags describe that
/// gap: they are the config's own declaration that there is no named type and no nested field to
/// check, which is what makes the derived PascalCase name provably unread as a type claim there.
/// Checking only `result_is_bytes` would fail generation on every declared-simple or
/// declared-json-struct call, which is the same false alarm with a much larger blast radius. ~keep
fn call_declares_non_struct_result(call: &CallConfig, lang: &str) -> bool {
    if call.result_is_simple || call.result_is_bytes {
        return true;
    }
    call.overrides
        .get(lang)
        .is_some_and(|o| o.result_is_simple || o.result_is_bytes || o.result_is_json_struct)
}

/// Resolve call info for a fixture, with fallback to default call's client_factory.
///
/// Named call configs (e.g. `[e2e.calls.embed]`) may not repeat the `client_factory`
/// setting. We fall back to the default `[e2e.call]` override's client_factory so that
/// all methods on the same client use the same pattern.
fn resolve_fixture_call_info(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    lang: &str,
    ir: CallIr<'_>,
) -> ResolvedCallInfo {
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );

    // `trait_bridge_derived_c_identity` derives the C ABI symbol the FFI backend
    // actually generates for a trait-bridge registry operation, rather than trusting
    // the raw `fixture.call` config text (`register_fn`/`unregister_fn`/`clear_fn`),
    // which can diverge from it for `unregister`/`clear` (see that function's doc
    // comment for the exact derivation rule).
    //
    // Gated on `call_skip_reason`, NOT `fixture.skip` -- those are different questions.
    // `fixture.skip.languages` opts a fixture out of the *executable test harness*
    // only (`documentation_rendering_is_independent_of_test_harness_skips`); the
    // docs-snippet generator renders a fixture-skipped-for-`lang` fixture's
    // documentation regardless, so blocking this derivation for that case does not
    // stop the snippet from rendering -- it only starves it of the correct symbol,
    // leaving the naive `call.function` config text (already resolved into
    // `info.function_name` by the time `resolve_call_info` returns) uncorrected. That
    // was the actual defect behind 13 C `plugin_api` snippets calling a plural
    // `..._clear_reranker_backends`-shaped symbol the header never declares: every one
    // of those fixtures sets a well-formed base `function` *and* `skip.languages =
    // ["c"]` (originally true only of the register-shaped call sharing the same
    // trait, which genuinely cannot cross a callback-free C ABI), so gating on
    // `fixture.skip` left the wrong, already-populated name in place instead of the
    // real one this derivation can supply for the register-independent clear/unregister
    // exports. `call_skip_reason` (call-level `skip_languages`) is the authority for
    // "this language cannot represent this call at all" that both the executable
    // harness (`fixture_inclusion`) and the docs generator's own inclusion filter
    // (`generate_snippet_report_with_extensions`) already gate on; matching it here
    // keeps this function's "own terms" protection (see below) for a call that
    // genuinely has no C shape, without also punishing a call that does. ~keep
    //
    // This must not depend on the docs generator's upstream filtering having
    // happened -- a caller that reaches this function directly (as this module's own
    // unit tests, and the compiled e2e test-file path via `render_test_file`, both do)
    // must get the same protection on its own terms.
    //
    // Computed once, up front: both the function-name fallback below and the
    // result-type classification inside `resolve_call_info` need the same match, and a
    // registry function's `function_name` may already be non-empty (an explicit
    // per-language override, as a well-formed config sets) while its result type is
    // still unresolvable against the core IR -- the two fallbacks are independent, so
    // neither can be conditioned on the other having fired. ~keep
    let call_skipped_for_lang = crate::e2e::codegen::call_skip_reason(fixture, lang, e2e_config).is_some();
    let trait_bridge_identity = (!call_skipped_for_lang)
        .then(|| crate::e2e::codegen::recipe::trait_bridge_derived_c_identity(config, fixture))
        .flatten();

    let mut info = resolve_call_info(
        call,
        lang,
        ir,
        trait_bridge_identity.as_ref().map(|(_, name)| name.as_str()),
    );

    trait_bridge_registry_identity::apply(&mut info, call, lang, trait_bridge_identity);

    let default_overrides = e2e_config.call.overrides.get(lang);

    // Neither factory fallback may reach a status-code export. Both describe how to obtain a
    // receiver for a *method* call, and a trait-bridge registry export is a free function on
    // the registry with no receiver at all -- inheriting a default `client_factory` makes the
    // emitter call `{prefix}_default_client_clear_{trait}(client, ...)`, a symbol the header
    // never declares, and (in a docs snippet) prefaces it with an `API key must be set` guard
    // for a purely local registry operation. The inheritance is a convenience for suites where
    // every call really is a method on one client; it is not evidence about a call whose shape
    // is already known. ~keep
    let returns_status_code = info.result_type_name.returns_status_code();

    // Fallback: if the named call has no client_factory override, inherit from the
    // default call config so all calls use the same client pattern.
    if info.client_factory.is_none()
        && !returns_status_code
        && let Some(factory) = default_overrides.and_then(|o| o.client_factory.as_ref())
    {
        info.client_factory = Some(factory.clone());
    }

    // Fallback: if the named call has no c_engine_factory override, inherit from the
    // default call config so all calls use the same engine pattern.
    if info.c_engine_factory.is_none()
        && !returns_status_code
        && let Some(factory) = default_overrides.and_then(|o| o.c_engine_factory.as_ref())
    {
        info.c_engine_factory = Some(factory.clone());
    }

    info
}

fn c_visitor_fixture_has_typed_call(fixture: &Fixture, e2e_config: &E2eConfig, ir: CallIr<'_>) -> bool {
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // `None`: this predicate only reads `info.options_type_name`, never `result_type_name`,
    // so a trait-bridge identity would be inert here even if computed. ~keep
    let info = resolve_call_info(call, "c", ir, None);
    let has_function = call
        .overrides
        .get("c")
        .and_then(|override_config| override_config.function.as_deref())
        .is_some_and(|function| !function.is_empty());
    has_function && !info.options_type_name.is_empty()
}

mod assertions;
mod call_patterns;
#[cfg(test)]
mod client_factory_optional_arg_tests;
#[cfg(test)]
mod collection_empty_assertion_tests;
mod collection_wildcard;
mod docs_input;
mod enum_field_inference;
mod ffi_constructors;
mod optional_arg;
mod primitive_field_inference;
mod project;
mod return_shape;
mod runner;
#[cfg(test)]
mod snippet_regressions;
#[cfg(test)]
mod std_arg_tests;
mod streaming;
mod test_function;
mod trait_bridge_registry_identity;
#[cfg(test)]
mod trait_bridge_registry_symbol_tests;
mod trait_bridge_snippet;
mod visitor;
mod void_call_status;
#[cfg(test)]
mod wildcard_collection_regression_tests;

use assertions::{
    FieldConfigSources, LeafFieldCheck, build_args_string_c, emit_nested_accessor, ensure_leaf_field_exists,
    render_assertion,
};
use collection_wildcard::{NestedLeafOutcome, classify_nested_leaf, render_wildcard_assertion};
// Test-only: the tests here and in `snippet_regressions` (which pulls this scope in via
// `use super::*`) construct sources explicitly to pin which config key a diagnostic names, while
// non-test code only ever goes through `FieldConfigSources`. Importing it unconditionally would be
// an unused import, which this repo's clippy config escalates to a build failure. ~keep
#[cfg(test)]
use assertions::EffectiveConfigSource;
use call_patterns::{render_bytes_test_function, render_engine_factory_test_function};
use enum_field_inference::enum_fields_c_types_from_ir;
use primitive_field_inference::primitive_fields_c_types_from_ir;
use project::{render_download_script, render_gitignore, render_makefile};
use runner::{render_main_c, render_test_runner_header};
use streaming::{
    render_c_diagnostic_skip, render_streaming_test_function, resolve_c_client_owner_type, resolve_c_streaming_adapter,
    validate_c_snippet_metadata,
};
use test_function::render_test_function_impl;
use visitor::render_visitor_test_file;

#[allow(clippy::too_many_arguments)]
fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    header: &str,
    _prefix: &str,
    result_var: &str,
    e2e_config: &E2eConfig,
    lang: &str,
    field_resolver: &FieldResolver,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    errors: &[crate::core::ir::ErrorDef],
    ir: CallIr<'_>,
) -> anyhow::Result<String> {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Block));
    let _ = writeln!(out, "/* E2e tests for category: {category} */");
    let _ = writeln!(out);
    let _ = writeln!(out, "#include <assert.h>");
    let _ = writeln!(out, "#include <stdint.h>");
    let _ = writeln!(out, "#include <string.h>");
    let _ = writeln!(out, "#include <stdio.h>");
    let _ = writeln!(out, "#include <stdlib.h>");
    let mut headers = std::collections::BTreeSet::from([header.to_string()]);
    for fixture in fixtures {
        let call = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        headers.insert(effective_c_header(call, config));
    }
    for header in headers {
        let _ = writeln!(out, "#include \"{header}\"");
    }
    let _ = writeln!(out, "#include \"test_runner.h\"");
    let _ = writeln!(out);

    // Extend the operator-declared `fields_c_types` with entries the IR itself proves are
    // enum-typed, before any per-fixture derivation runs. Neither input varies per fixture
    // (`type_defs`/`enums` are the whole crate's IR), so this is computed once. Config always
    // wins: `or_insert` never overwrites an operator's own declaration, including one that
    // deliberately names a different accessor shape (e.g. `"skip"` or `"char*"`). ~keep
    let mut effective_fields_c_types = e2e_config.fields_c_types.clone();
    for (key, type_name) in enum_fields_c_types_from_ir(type_defs, enums) {
        effective_fields_c_types.entry(key).or_insert(type_name);
    }
    // Same precedence for plain scalar leaf fields (`bool`/`u32`/`f64`/...) that never got a
    // `fields_c_types` entry either — see `primitive_field_inference` module docs. ~keep
    for (key, type_name) in primitive_fields_c_types_from_ir(type_defs) {
        effective_fields_c_types.entry(key).or_insert(type_name);
    }

    for (i, fixture) in fixtures.iter().enumerate() {
        // Visitor fixtures are filtered out before render_test_file is called.
        // This guard is a safety net in case a fixture reaches here unexpectedly.
        if fixture.visitor.is_some() {
            panic!(
                "C e2e generator: visitor pattern not supported for fixture: {}",
                fixture.id
            );
        }

        // `ir`, not an empty slice: `resolve_call_info` derives `result_type_name` from the
        // declared return type here, and `result_type_name` is what `parent_is_ir_type` — and
        // through it `ensure_leaf_field_exists` — reads. Passing an empty slice would make
        // every call unresolvable-but-excused (`CallIr::is_absent`), which is how a suite used
        // to be generated with field verification off and nothing but a log line saying so. ~keep
        let call_info = resolve_fixture_call_info(fixture, e2e_config, config, lang, ir);

        // Effective enum fields for this fixture: merge global e2e_config.fields_enum
        // (HashSet) with the per-call C override's enum_fields (HashMap keys). This
        // mirrors Ruby/Java's pattern: global = always-enum-typed paths; per-call =
        // context-dependent paths (BatchObject.status is BatchStatus, but
        // ResponseObject.status is plain String).
        let mut effective_fields_enum = e2e_config.fields_enum.clone();
        let fixture_call = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        let prefix = effective_c_prefix(fixture_call, config);
        if let Some(co) = fixture_call.overrides.get(lang) {
            for k in co.enum_fields.keys() {
                effective_fields_enum.insert(k.clone());
            }
        }
        // `fields_enum` above is config-declared and can miss a field the IR itself
        // already proves is enum-shaped — union in every field whose `fields_c_types`
        // entry names a real IR enum so a missing declaration falls back to IR truth
        // instead of silently falling through to the opaque-handle arm (which emits
        // `strcmp()` against a `uint64_t` handle). This only ever ADDS field names: an
        // explicit config entry the IR check doesn't independently confirm (e.g. a
        // synthetic field with no `fields_c_types` entry) still passes through untouched.
        // Reads `effective_fields_c_types`, not the raw config map, so a field the IR-derived
        // pass above declared (no config entry at all) is also recognized as enum-shaped here. ~keep
        effective_fields_enum.extend(enum_fields_from_ir(&effective_fields_c_types, enums));

        // Per-call field resolver: overrides the top-level resolver when this call
        // declares its own result_fields / fields / fields_optional / fields_array.
        // Without this, `pages.length` on a `crawl` call would skip because the
        // default `result_fields` (configured for the top-level `scrape` call)
        // does not contain `pages`.
        let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) =
            FieldResolver::ir_field_sets(type_defs);
        let per_call_field_resolver = FieldResolver::new(
            e2e_config.effective_fields(fixture_call),
            e2e_config.effective_fields_optional(fixture_call),
            e2e_config.effective_result_fields(fixture_call),
            e2e_config.effective_fields_array(fixture_call),
            &std::collections::HashSet::new(),
        )
        .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields);
        let _ = field_resolver; // top-level resolver retained for compat; per-call wins
        let field_resolver = &per_call_field_resolver;

        // Which `result_fields`/`fields` collections govern THIS fixture's call, by the
        // identical shadowing rule `effective_result_fields`/`effective_fields` just
        // applied above — a nested-field diagnostic must name the same key that actually
        // shaped `field_resolver`, or it sends an operator's edit to a config key their
        // call ignores. ~keep
        let config_sources = FieldConfigSources::resolve(e2e_config, fixture_call);

        // `out` accumulates every fixture's rendered function in this file, so the
        // strict-availability scan below must only look at the text THIS fixture's
        // own render appended — scanning the whole buffer would misattribute an
        // earlier fixture's skip comment to this fixture's id.
        let fixture_start = out.len();
        // What the core IR says about this fixture's target parameters -- the identical
        // resolution `render_snippet_body` performs, so the doc-snippet path and the real
        // e2e-test-file emitter (which also drives `test_apps/`) agree on one call's declared
        // signature instead of this path always rendering `IrAbsent`. See `c::optional_arg`. ~keep
        let target_params = if crate::e2e::codegen::recipe::trait_bridge_derived_c_identity(config, fixture).is_some() {
            TargetParams::Known(&[])
        } else {
            TargetParams::resolve(fixture_call, lang, ir)
        };
        render_test_function_impl(
            &mut out,
            fixture,
            &prefix,
            &call_info.function_name,
            result_var,
            &call_info.args,
            field_resolver,
            &effective_fields_c_types,
            &effective_fields_enum,
            &call_info.result_type_name,
            &call_info.options_type_name,
            call_info.client_factory.as_deref(),
            call_info.raw_c_result_type.as_deref(),
            call_info.c_free_fn.as_deref(),
            call_info.c_engine_factory.as_deref(),
            call_info.result_is_option,
            call_info.result_is_bytes,
            call_info.streaming,
            &call_info.extra_args,
            config,
            type_defs,
            errors,
            false,
            &config_sources,
            target_params,
        )?;
        crate::e2e::codegen::fail_on_unavailable_field_markers(
            &out[fixture_start..],
            "c",
            &fixture.id,
            &fixture.assertions,
        );
        crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&out[fixture_start..], "c", &fixture.id);
        if i + 1 < fixtures.len() {
            let _ = writeln!(out);
        }
    }

    Ok(out)
}

#[allow(clippy::too_many_arguments)]
/// Convert a `serde_json::Value` to a C literal string.
fn json_to_c(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_c(s)),
        serde_json::Value::Bool(true) => "1".to_string(),
        serde_json::Value::Bool(false) => "0".to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "NULL".to_string(),
        other => format!("\"{}\"", escape_c(&other.to_string())),
    }
}

/// Emit a test backend stub.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> super::TestBackendEmission {
    trait_bridge_snippet::emit_test_backend(trait_bridge, methods, fixture)
}

#[cfg(test)]
mod snippet_tests {
    use super::*;

    #[test]
    fn full_generation_uses_the_derived_ffi_prefix() {
        let group = FixtureGroup {
            category: "basic".into(),
            fixtures: vec![Fixture {
                id: "clear".into(),
                description: "Clear".into(),
                ..Fixture::default()
            }],
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "clear".into();
        e2e.call.returns_void = true;
        let config = ResolvedCrateConfig {
            name: "sample-core".into(),
            ..ResolvedCrateConfig::default()
        };

        let generated = CCodegen
            .generate(&[group], &e2e, &config, &[], &[], &[], &[])
            .expect("C harness renders");
        let test = generated
            .iter()
            .find(|file| file.path.ends_with("test_basic.c"))
            .expect("category test is emitted");
        assert!(test.content.contains("SAMPLE_COREAlefHandle"), "{}", test.content);
        assert!(test.content.contains("sample_core_clear_free("), "{}", test.content);
        assert!(!test.content.contains(" _clear_free("), "{}", test.content);
    }

    #[test]
    fn full_generation_uses_named_call_prefix_and_header() {
        let group = FixtureGroup {
            category: "basic".into(),
            fixtures: vec![Fixture {
                id: "secondary".into(),
                call: Some("secondary".into()),
                ..Fixture::default()
            }],
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "default_call".into();
        e2e.calls.insert(
            "secondary".into(),
            CallConfig {
                function: "secondary_call".into(),
                returns_void: true,
                overrides: HashMap::from([(
                    "c".into(),
                    crate::e2e::config::CallOverride {
                        prefix: Some("secondary".into()),
                        header: Some("secondary.h".into()),
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample-core".into(),
            ..Default::default()
        };
        let files = CCodegen.generate(&[group], &e2e, &config, &[], &[], &[], &[]).unwrap();
        let test = files.iter().find(|file| file.path.ends_with("test_basic.c")).unwrap();
        assert!(test.content.contains("#include \"secondary.h\""), "{}", test.content);
        assert!(test.content.contains("SECONDARYAlefHandle"), "{}", test.content);
        assert!(
            test.content.contains("secondary_secondary_call_free("),
            "{}",
            test.content
        );
    }

    #[test]
    fn snippet_keeps_header_and_call_without_test_harness() {
        let fixture = Fixture {
            id: "count".into(),
            description: "Count".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "sample_count".into();
        e2e.call.result_var = "result".into();
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let rendered = CCodegen
            .render_snippet_body(&fixture, &e2e, &config, &[], &[])
            .expect("snippet renders");
        assert!(rendered.contains("#include \""));
        assert!(rendered.contains("sample_count("));
        assert!(rendered.contains("int main(void)"));
        assert!(!rendered.contains("void test_"));
        assert!(!rendered.contains("assert("));
        assert!(rendered.contains("_free(result)"), "{rendered}");
    }

    /// A crate IR that names one function, so `CallIr::is_absent()` is false and the generator
    /// genuinely had something to resolve against. The fixture's call is not that function.
    fn unrelated_ir() -> [crate::core::ir::FunctionDef; 1] {
        [crate::core::ir::FunctionDef {
            name: "unrelated".into(),
            return_type: crate::core::ir::TypeRef::Named("Unrelated".into()),
            ..crate::core::ir::FunctionDef::default()
        }]
    }

    /// The defect this pair pins: `list_ocr_backends` was PascalCased into `ListOcrBackends`,
    /// a type the generated header never declares, and the snippet then spelled it into a
    /// `{prefix}_{result}_free` call for a family that has no `_free` member — while the
    /// invented name simultaneously switched `ensure_leaf_field_exists` off, because
    /// `parent_is_ir_type` can only be true for a name the IR actually declares. An
    /// unresolvable result type must therefore produce an ERROR here, not a snippet: the
    /// emitted symbol set is not reachable from this generator (see `ResultTypeName::require`),
    /// so there is nothing better than the IR to resolve against and nothing at all to guess
    /// from. The positive control below shares this shape exactly apart from the IR entry, so
    /// this test cannot pass by making every render fail. ~keep
    #[test]
    fn should_refuse_to_emit_a_snippet_whose_result_type_resolves_to_nothing_real() {
        let fixture = Fixture {
            id: "list_backends".into(),
            description: "List backends".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "sample_list_backends".into();
        e2e.call.result_var = "result".into();
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let error = render_c_snippet(&fixture, &e2e, &config, &[], &unrelated_ir())
            .expect_err("a result type nothing real names must fail generation, not emit a snippet");

        let message = format!("{error:#}");
        assert!(
            message.contains("sample_list_backends"),
            "the failure must name the call an operator has to fix: {message}"
        );
        assert!(
            message.contains("result_type"),
            "the failure must name the config key that fixes it: {message}"
        );
        assert!(
            !message.contains("SampleListBackends"),
            "the failure must not hand back the PascalCased call name as if it were a type: {message}"
        );
    }

    /// Positive control for the test above, identical apart from the IR declaring the call.
    /// A resolvable result type must still render, and must render *through the same emission
    /// path* — the opaque-handle path whose `{prefix}_{result_snake}_free` is exactly the symbol
    /// the fabricated name used to corrupt. Without this, the failure test above would be
    /// satisfied by an emitter that refused everything. ~keep
    #[test]
    fn should_still_emit_a_snippet_when_the_ir_names_the_result_type() {
        let fixture = Fixture {
            id: "list_backends".into(),
            description: "List backends".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "sample_list_backends".into();
        e2e.call.result_var = "result".into();
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let functions = [crate::core::ir::FunctionDef {
            name: "sample_list_backends".into(),
            return_type: crate::core::ir::TypeRef::Named("BackendList".into()),
            ..crate::core::ir::FunctionDef::default()
        }];

        let rendered =
            render_c_snippet(&fixture, &e2e, &config, &[], &functions).expect("a call the IR names must still render");

        assert!(rendered.contains("sample_list_backends("), "{rendered}");
        assert!(
            rendered.contains("sample_backend_list_free(result)"),
            "cleanup must be derived from the IR-declared type, not the call name: {rendered}"
        );
        assert!(
            !rendered.contains("sample_list_backends_free"),
            "the call-name-derived cleanup symbol must never appear: {rendered}"
        );
    }

    /// `clear_fn = "clear_sample_backends"` (plural, human-written config text) on a
    /// trait named `SampleBackend` (singular). `registration.rs` derives the exported
    /// symbol from the trait name's snake_case form, discarding the config text's
    /// spelling, so the real ABI symbol is `sample_clear_sample_backend` (singular) --
    /// and it takes a trailing `out_error` out-param (`clear_fn.jinja`), so the call
    /// site must pass `NULL`. This fails against the pre-fix code, which trusted
    /// `fixture.call`'s raw text verbatim and emitted the argument-less, plural,
    /// nonexistent `sample_clear_sample_backends()`.
    #[test]
    fn trait_bridge_operation_uses_declared_abi_identity() {
        let fixture = Fixture {
            id: "clear_sample_backends".into(),
            description: "Clear registered sample backends".into(),
            call: Some("clear_sample_backends".into()),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.calls.insert(
            "clear_sample_backends".into(),
            CallConfig {
                returns_result: false,
                returns_void: true,
                ..CallConfig::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "SampleBackend".into(),
                clear_fn: Some("clear_sample_backends".into()),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        };

        let rendered = render_c_snippet(&fixture, &e2e, &config, &[], &[]).expect("C snippet renders");

        assert!(rendered.contains("sample_clear_sample_backend(NULL)"), "{rendered}");
        assert!(!rendered.contains("sample_clear_sample_backends("), "{rendered}");
        assert!(!rendered.contains("has no function identity"), "{rendered}");
    }

    /// `unregister_fn`'s C export always takes a trailing `out_error` out-param
    /// (`unregister_fn.jinja`) in addition to the configured `name` argument, but the
    /// shared, language-agnostic call args config (`args = [{ name, field, type }]`)
    /// has no way to express a C-only out-param. This fails against the pre-fix code:
    /// the void-call branch built its argument list purely from `info.args` and never
    /// consulted `info.extra_args`, so it emitted `sample_unregister_sample_backend(name)`
    /// -- one argument short of the real two-argument ABI signature.
    #[test]
    fn trait_bridge_unregister_appends_out_error_out_param() {
        let fixture = Fixture {
            id: "unregister_sample_backend".into(),
            description: "Unregister a sample backend".into(),
            call: Some("unregister_sample_backend".into()),
            input: serde_json::json!({ "name": "nonexistent-backend" }),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.calls.insert(
            "unregister_sample_backend".into(),
            CallConfig {
                returns_result: false,
                returns_void: true,
                args: vec![crate::core::config::e2e::ArgMapping {
                    name: "name".into(),
                    field: "input.name".into(),
                    arg_type: "string".into(),
                    optional: false,
                    owned: false,
                    element_type: None,
                    go_type: None,
                    vec_inner_is_ref: false,
                    trait_name: None,
                }],
                ..CallConfig::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "SampleBackend".into(),
                unregister_fn: Some("unregister_sample_backend".into()),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        };

        let rendered = render_c_snippet(&fixture, &e2e, &config, &[], &[]).expect("C snippet renders");

        assert!(
            rendered.contains("sample_unregister_sample_backend(\"nonexistent-backend\", NULL)"),
            "{rendered}"
        );
    }

    // The fixture-level-skip-vs-derivation coverage (resolver-level and end-to-end) lives in
    // `trait_bridge_registry_symbol_tests.rs`, alongside the sibling "configured base function
    // shadows the derivation" cases this same authority resolves. ~keep
    #[test]
    fn expected_error_snippet_checks_the_native_null_result() {
        let mut fixture = Fixture {
            id: "invalid".into(),
            description: "Invalid".into(),
            ..Fixture::default()
        };
        fixture.assertions.push(crate::e2e::fixture::Assertion {
            assertion_type: "error".into(),
            ..Default::default()
        });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "sample_parse".into();
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let rendered = CCodegen
            .render_snippet_body(&fixture, &e2e, &config, &[], &[])
            .expect("snippet renders");
        assert!(rendered.contains("!= 0) { return EXIT_FAILURE; }"), "{rendered}");
        assert!(!rendered.contains("assert("));
    }

    #[test]
    fn engine_factory_snippet_reuses_native_call_preparation() {
        let fixture = Fixture {
            id: "engine_call".into(),
            description: "Engine call".into(),
            input: serde_json::json!({ "url": "https://example.test" }),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "sample_scrape".into();
        e2e.call.result_var = "result".into();
        e2e.call.overrides.insert(
            "c".into(),
            crate::core::config::e2e::CallOverride {
                c_engine_factory: Some("EngineConfig".into()),
                ..Default::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let rendered = CCodegen
            .render_snippet_body(&fixture, &e2e, &config, &[], &[])
            .expect("engine-factory snippet renders");

        assert!(rendered.contains("create_engine"), "{rendered}");
        assert!(rendered.contains("sample_scrape(engine"), "{rendered}");
        assert!(rendered.contains("crawl_engine_handle_free(engine)"), "{rendered}");
    }

    #[test]
    fn simple_result_snippet_uses_prefixed_string_api() {
        let fixture = Fixture {
            id: "list_formats".into(),
            description: "List formats".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "list_formats".into();
        e2e.call.result_var = "result".into();
        e2e.call.result_is_simple = true;
        e2e.call.overrides.insert(
            "c".into(),
            crate::core::config::e2e::CallOverride {
                raw_c_result_type: Some("char*".into()),
                ..Default::default()
            },
        );
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let rendered = CCodegen
            .render_snippet_body(&fixture, &e2e, &config, &[], &[])
            .expect("simple-result snippet renders");

        assert!(rendered.contains("char* result = sample_list_formats();"), "{rendered}");
        assert!(rendered.contains("sample_free_string(result);"), "{rendered}");
        assert!(!rendered.contains("SAMPLEListFormats"), "{rendered}");
    }

    #[test]
    fn scalar_result_snippets_preserve_numeric_types_without_string_cleanup() {
        for raw_type in ["int32_t", "bool"] {
            let fixture = Fixture {
                id: "count_formats".into(),
                description: "Count formats".into(),
                ..Fixture::default()
            };
            let mut e2e = E2eConfig::default();
            e2e.call.function = "count_formats".into();
            e2e.call.result_var = "result".into();
            e2e.call.result_is_simple = true;
            e2e.call.overrides.insert(
                "c".into(),
                crate::core::config::e2e::CallOverride {
                    raw_c_result_type: Some(raw_type.into()),
                    ..Default::default()
                },
            );
            let config = ResolvedCrateConfig {
                name: "sample".into(),
                ..ResolvedCrateConfig::default()
            };

            let rendered = CCodegen
                .render_snippet_body(&fixture, &e2e, &config, &[], &[])
                .expect("numeric-result snippet renders");

            assert!(
                rendered.contains(&format!("{raw_type} result = sample_count_formats();")),
                "{rendered}"
            );
            assert!(!rendered.contains("free_string"), "{rendered}");
        }
    }

    #[test]
    fn raw_result_error_snippet_fails_on_unexpected_success() {
        for (raw_type, expected_failure_check) in [
            ("char*", "if (result != 0) { return EXIT_FAILURE; }"),
            ("int32_t", "if (result != 0) { return EXIT_FAILURE; }"),
            ("uintptr_t", "assert(sample_last_error_code() != 0"),
        ] {
            let mut fixture = Fixture {
                id: "invalid_input".into(),
                description: "Invalid input".into(),
                ..Fixture::default()
            };
            fixture.assertions.push(crate::e2e::fixture::Assertion {
                assertion_type: "error".into(),
                ..Default::default()
            });
            let mut e2e = E2eConfig::default();
            e2e.call.function = "parse_input".into();
            e2e.call.result_var = "result".into();
            e2e.call.result_is_simple = true;
            e2e.call.overrides.insert(
                "c".into(),
                crate::core::config::e2e::CallOverride {
                    raw_c_result_type: Some(raw_type.into()),
                    ..Default::default()
                },
            );
            let config = ResolvedCrateConfig {
                name: "sample".into(),
                ..ResolvedCrateConfig::default()
            };

            let rendered = CCodegen
                .render_snippet_body(&fixture, &e2e, &config, &[], &[])
                .expect("raw-result error snippet renders");

            assert!(
                rendered.contains(expected_failure_check),
                "raw_type={raw_type}: {rendered}"
            );
        }
    }

    /// Identifiers a snippet guard may name without a preceding local declaration. ~keep
    const GUARD_FREE_IDENTIFIERS: &[&str] = &["NULL", "EXIT_FAILURE", "EXIT_SUCCESS", "true", "false", "sizeof"];

    /// The condition of an `if (...)` guard, paren-balanced so a call inside it does not
    /// terminate the scan early.
    fn guard_condition(line: &str) -> Option<&str> {
        let rest = line.trim().strip_prefix("if (")?;
        let mut depth = 1usize;
        for (index, character) in rest.char_indices() {
            match character {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&rest[..index]);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Identifiers a condition reads as values: call names, string-literal contents and
    /// numeric literals are excluded.
    fn condition_identifiers(condition: &str) -> Vec<String> {
        let characters: Vec<char> = condition.chars().collect();
        let mut identifiers = Vec::new();
        let mut index = 0;
        let mut in_string = false;
        while index < characters.len() {
            let character = characters[index];
            if in_string {
                index += if character == '\\' { 2 } else { 1 };
                if character == '"' {
                    in_string = false;
                }
                continue;
            }
            if character == '"' {
                in_string = true;
                index += 1;
                continue;
            }
            if character.is_alphabetic() || character == '_' {
                let start = index;
                while index < characters.len() && (characters[index].is_alphanumeric() || characters[index] == '_') {
                    index += 1;
                }
                if characters.get(index) != Some(&'(') {
                    identifiers.push(characters[start..index].iter().collect());
                }
                continue;
            }
            index += 1;
        }
        identifiers
    }

    /// The variable a statement declares. Deliberately reimplemented here rather than shared
    /// with `test_function::declared_variable`: a checker that reuses the emitter's own
    /// heuristic cannot fail when that heuristic is what is wrong. ~keep
    fn declared_name(line: &str) -> Option<String> {
        let statement = line.trim().trim_end_matches(';');
        let declarator = statement.split('=').next()?.trim();
        if declarator.contains(['(', ')', '{', '}', '!', '<', '>', ',', '#'])
            || declarator.split_whitespace().count() < 2
        {
            return None;
        }
        let last = declarator.split_whitespace().next_back()?;
        let name = last.trim_start_matches('*').split('[').next()?;
        (!name.is_empty()).then(|| name.to_string())
    }

    fn guard_uses_before_declaration(snippet: &str) -> Vec<String> {
        let mut declared: HashSet<String> = HashSet::new();
        let mut violations = Vec::new();
        for line in snippet.lines() {
            if let Some(condition) = guard_condition(line) {
                for identifier in condition_identifiers(condition) {
                    if !GUARD_FREE_IDENTIFIERS.contains(&identifier.as_str()) && !declared.contains(&identifier) {
                        violations.push(format!("`{identifier}` read by guard `{}`", line.trim()));
                    }
                }
            }
            if let Some(name) = declared_name(line) {
                declared.insert(name);
            }
        }
        violations
    }

    fn error_fixture(id: &str) -> Fixture {
        let mut fixture = Fixture {
            id: id.into(),
            description: "Expected to fail".into(),
            ..Fixture::default()
        };
        fixture.assertions.push(crate::e2e::fixture::Assertion {
            assertion_type: "error".into(),
            ..Default::default()
        });
        fixture
    }

    /// Property, not string: whatever a generated C snippet's `if (...)` guards read must
    /// already be declared above them, because a snippet is a standalone translation unit and
    /// a use-before-declaration is a hard compile error, not a failing assertion.
    ///
    /// The checker is a whole-snippet scan, so it also covers the `free`-guards the
    /// engine-factory and client paths emit — not just the error-path failure guard. ~keep
    #[test]
    fn every_guard_identifier_in_a_generated_snippet_is_declared_before_it_is_read() {
        let sample = || ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let mut client_e2e = E2eConfig::default();
        client_e2e.call.function = "chat".into();
        client_e2e.call.overrides.insert(
            "c".into(),
            crate::core::config::e2e::CallOverride {
                client_factory: Some("create_client".into()),
                ..Default::default()
            },
        );
        let client_config = ResolvedCrateConfig {
            adapters: vec![
                serde_json::from_value(serde_json::json!({
                    "name": "chat",
                    "pattern": "async_method",
                    "core_path": "sample::chat",
                    "owner_type": "DefaultClient"
                }))
                .expect("client adapter config"),
            ],
            ..sample()
        };

        let mut raw_e2e = E2eConfig::default();
        raw_e2e.call.function = "parse_input".into();
        raw_e2e.call.result_var = "result".into();
        raw_e2e.call.result_is_simple = true;
        raw_e2e.call.overrides.insert(
            "c".into(),
            crate::core::config::e2e::CallOverride {
                raw_c_result_type: Some("char*".into()),
                ..Default::default()
            },
        );

        let mut handle_e2e = E2eConfig::default();
        handle_e2e.call.function = "sample_parse".into();

        let cases: Vec<(&str, Fixture, E2eConfig, ResolvedCrateConfig)> = vec![
            (
                "client-factory error",
                error_fixture("chat_auth_401"),
                client_e2e,
                client_config,
            ),
            ("raw-result error", error_fixture("parse_invalid"), raw_e2e, sample()),
            (
                "opaque-handle error",
                error_fixture("parse_failed"),
                handle_e2e,
                sample(),
            ),
        ];

        for (label, fixture, e2e, config) in cases {
            let rendered = render_c_snippet(&fixture, &e2e, &config, &[], &[]).expect("snippet renders");
            let violations = guard_uses_before_declaration(&rendered);
            assert!(
                violations.is_empty(),
                "{label}: guard reads an undeclared identifier: {violations:?}\n{rendered}"
            );
        }
    }

    /// Negative control for the checker above. This is the exact shape alef 0.60.0 published for
    /// every error fixture with a client factory: the client-construction assertion was rewritten
    /// into the result guard, so the guard named a variable declared on the next line. A checker
    /// that cannot see this defect proves nothing about the snippets that pass it. ~keep
    #[test]
    fn guard_checker_rejects_the_historic_use_before_declaration_snippet() {
        let historic = concat!(
            "int main(void) {\n",
            "    SAMPLEDefaultClient* client = sample_create_client(\"test-key\", NULL);\n",
            "    if (result != NULL) { return EXIT_FAILURE; }\n",
            "    SAMPLEBatchObject* result = sample_default_client_cancel_batch(client, \"batch-1\");\n",
            "    sample_default_client_free(client);\n",
            "    if (result != NULL) { return EXIT_FAILURE; }\n",
            "    return EXIT_SUCCESS;\n",
            "}\n",
        );

        let violations = guard_uses_before_declaration(historic);

        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(violations[0].contains("`result`"), "{violations:?}");
    }

    #[test]
    fn raw_result_test_function_asserts_failure_per_result_type() {
        // Direct test of the real e2e-test-file emitter (render_test_function_impl),
        // which is where the defect lived: for raw_c_result_type functions
        // (char*/int32_t/uintptr_t), an "error"-only fixture previously emitted
        // no assertion at all, so a call that unexpectedly SUCCEEDED still made
        // the generated test pass. Assert the exact failing construct per type.
        let cases: &[(&str, &str)] = &[
            ("char*", "assert(result == NULL && \"expected call to fail\");"),
            ("int32_t", "assert(result < 0 && \"expected call to fail\");"),
            (
                "uintptr_t",
                "assert(sample_last_error_code() != 0 && \"expected call to fail\");",
            ),
        ];
        for (raw_type, expected_assert) in cases {
            let mut fixture = Fixture {
                id: "invalid_input".into(),
                description: "Invalid input".into(),
                ..Fixture::default()
            };
            fixture.assertions.push(crate::e2e::fixture::Assertion {
                assertion_type: "error".into(),
                ..Default::default()
            });
            let config = ResolvedCrateConfig {
                name: "sample".into(),
                ..ResolvedCrateConfig::default()
            };
            let field_resolver = FieldResolver::new(
                &HashMap::new(),
                &HashSet::new(),
                &HashSet::new(),
                &HashSet::new(),
                &HashSet::new(),
            );

            let mut out = String::new();
            render_test_function_impl(
                &mut out,
                &fixture,
                "sample",
                "sample_parse_input",
                "result",
                &[],
                &field_resolver,
                &HashMap::new(),
                &HashSet::new(),
                &ResultTypeName::Resolved("Result".into()),
                "",
                None,
                Some(raw_type),
                None,
                None,
                false,
                false,
                None,
                &[],
                &config,
                &[],
                &[],
                false,
                &FieldConfigSources {
                    result_fields: EffectiveConfigSource::Global,
                    fields: EffectiveConfigSource::Global,
                },
                TargetParams::IrAbsent,
            )
            .expect("test fixture renders");

            assert!(
                out.contains(expected_assert),
                "raw_type={raw_type}: expected `{expected_assert}` in:\n{out}"
            );
            assert!(
                !out.contains("expected call to succeed"),
                "raw_type={raw_type}: unexpected success-path assertion in:\n{out}"
            );
        }
    }

    #[test]
    fn raw_result_test_function_falls_back_to_last_error_code_for_unmodeled_raw_types() {
        // raw_c_result_type is a free-form config string (bool, uint64_t, size_t, ...),
        // not a closed char*/int32_t/uintptr_t set. A fixture using any type outside
        // that trio must still emit a failing check via the always-present
        // last_error_code FFI symbol — not silently emit nothing.
        for raw_type in ["bool", "uint64_t", "size_t"] {
            let mut fixture = Fixture {
                id: "invalid_input".into(),
                description: "Invalid input".into(),
                ..Fixture::default()
            };
            fixture.assertions.push(crate::e2e::fixture::Assertion {
                assertion_type: "error".into(),
                ..Default::default()
            });
            let config = ResolvedCrateConfig {
                name: "sample".into(),
                ..ResolvedCrateConfig::default()
            };
            let field_resolver = FieldResolver::new(
                &HashMap::new(),
                &HashSet::new(),
                &HashSet::new(),
                &HashSet::new(),
                &HashSet::new(),
            );

            let mut out = String::new();
            render_test_function_impl(
                &mut out,
                &fixture,
                "sample",
                "sample_parse_input",
                "result",
                &[],
                &field_resolver,
                &HashMap::new(),
                &HashSet::new(),
                &ResultTypeName::Resolved("Result".into()),
                "",
                None,
                Some(raw_type),
                None,
                None,
                false,
                false,
                None,
                &[],
                &config,
                &[],
                &[],
                false,
                &FieldConfigSources {
                    result_fields: EffectiveConfigSource::Global,
                    fields: EffectiveConfigSource::Global,
                },
                TargetParams::IrAbsent,
            )
            .expect("test fixture renders");

            assert!(
                out.contains("assert(sample_last_error_code() != 0 && \"expected call to fail\");"),
                "raw_type={raw_type}: expected last_error_code fallback assert in:\n{out}"
            );
        }
    }

    /// Builds an error fixture with `raw_c_result_type = "char*"` plus the extra assertions the
    /// error path has to account for.
    fn render_c_error_fixture(extra: Vec<crate::e2e::fixture::Assertion>, declared: Option<&str>) -> String {
        let mut fixture = Fixture {
            id: "rate_limited".into(),
            description: "Rejects the request".into(),
            ..Fixture::default()
        };
        fixture.assertions.push(crate::e2e::fixture::Assertion {
            assertion_type: "error".into(),
            value: declared.map(|v| serde_json::Value::String(v.to_string())),
            ..Default::default()
        });
        fixture.assertions.extend(extra);
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };
        let field_resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let mut out = String::new();
        let _ = crate::e2e::codegen::take_skip_records();
        render_test_function_impl(
            &mut out,
            &fixture,
            "sample",
            "sample_parse_input",
            "result",
            &[],
            &field_resolver,
            &HashMap::new(),
            &HashSet::new(),
            &ResultTypeName::Resolved("Result".into()),
            "",
            None,
            Some("char*"),
            None,
            None,
            false,
            false,
            None,
            &[],
            &config,
            &[],
            &[],
            false,
            &FieldConfigSources {
                result_fields: EffectiveConfigSource::Global,
                fields: EffectiveConfigSource::Global,
            },
            TargetParams::IrAbsent,
        )
        .expect("test fixture renders");
        out
    }

    /// The defect: a declared `error` value was discarded outright, so `assert(result == NULL)`
    /// was the whole test — it could not tell the expected failure from any other. The C ABI's
    /// `last_error_context()` is the only textual evidence available, and it must be compared.
    #[test]
    fn a_declared_error_value_is_compared_against_the_ffi_error_message() {
        let out = render_c_error_fixture(Vec::new(), Some("rate limit"));

        assert!(
            out.contains("assert(result == NULL && \"expected call to fail\");"),
            "the failure check must still render: {out}"
        );
        assert!(
            out.contains("const char* _err_message = sample_last_error_context();"),
            "the FFI message must be bound: {out}"
        );
        assert!(
            out.contains("assert(strstr(_err_message, \"rate limit\") != NULL && \"error message mismatch\");"),
            "the declared value must be compared: {out}"
        );
    }

    /// Negative control: with no declared value the emitter must not invent a message check.
    #[test]
    fn an_error_assertion_without_a_value_emits_no_message_check() {
        let out = render_c_error_fixture(Vec::new(), None);

        assert!(
            out.contains("assert(result == NULL && \"expected call to fail\");"),
            "the failure check must still render: {out}"
        );
        assert!(!out.contains("last_error_context"), "{out}");
    }

    #[test]
    fn an_equals_on_an_error_field_is_named_instead_of_dropped() {
        let out = render_c_error_fixture(
            vec![crate::e2e::fixture::Assertion {
                assertion_type: "equals".into(),
                field: Some("error.status_code".into()),
                ..Default::default()
            }],
            Some("rate limit"),
        );

        assert!(
            out.contains("assert(result == NULL && \"expected call to fail\");"),
            "the error block must render before we assert anything about the second assertion: {out}"
        );
        assert!(
            out.contains(
                "// skipped: assertion type 'equals' has no accessor for error field error.status_code in this backend"
            ),
            "{out}"
        );

        let records = crate::e2e::codegen::take_skip_records();
        assert_eq!(records.len(), 1, "got: {records:?}");
        assert_eq!(records[0].language, "c");
        assert_eq!(records[0].field, "equals");
    }

    #[test]
    fn void_result_snippet_calls_api_without_placeholder_result() {
        let fixture = Fixture {
            id: "clear_formats".into(),
            description: "Clear formats".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "clear_formats".into();
        e2e.call.returns_void = true;
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let rendered = CCodegen
            .render_snippet_body(&fixture, &e2e, &config, &[], &[])
            .expect("void-result snippet renders");

        assert!(rendered.contains("sample_clear_formats();"), "{rendered}");
        assert!(!rendered.contains("result ="), "{rendered}");
        assert!(!rendered.contains("_free("), "{rendered}");
    }

    /// `enum_fields_from_ir` must recover exactly the field a config author forgot to
    /// list in `fields_enum` -- this is the reported mechanism behind the `strcmp()`-on-
    /// `uint64_t` defect: `BatchObject.status` maps to the real IR enum `BatchStatus` in
    /// `fields_c_types`, but nothing in this config declares `status` an enum field.
    #[test]
    fn enum_fields_from_ir_recovers_field_missing_from_declared_fields_enum() {
        let fields_c_types = HashMap::from([("batch_object.status".to_string(), "BatchStatus".to_string())]);
        let enums = vec![crate::core::ir::EnumDef {
            name: "BatchStatus".into(),
            ..crate::core::ir::EnumDef::default()
        }];

        let derived = enum_fields_from_ir(&fields_c_types, &enums);

        assert_eq!(derived, HashSet::from(["status".to_string()]));
    }

    /// A field whose `fields_c_types` type does NOT name a real IR enum must not be
    /// swept in by the override — otherwise a genuine opaque-struct field would be
    /// misrouted through the enum accessor and the codegen would call a
    /// `_to_string` function cbindgen never generated for it.
    #[test]
    fn enum_fields_from_ir_ignores_a_field_whose_type_is_not_a_registered_enum() {
        let fields_c_types = HashMap::from([("batch_object.usage".to_string(), "BatchUsage".to_string())]);
        let enums = vec![crate::core::ir::EnumDef {
            name: "BatchStatus".into(),
            ..crate::core::ir::EnumDef::default()
        }];

        let derived = enum_fields_from_ir(&fields_c_types, &enums);

        assert!(derived.is_empty(), "got: {derived:?}");
    }

    /// End-to-end proof that the override reaches `try_emit_enum_accessor`: with
    /// `fields_enum` empty (the reported gap) but the IR-derived override unioned in — the
    /// same composition `render_test_file` performs — the enum arm must fire and convert
    /// via `_to_string`, not leave a bare `AlefHandle` for the caller to `strcmp` against.
    #[test]
    fn try_emit_enum_accessor_fires_for_a_field_ir_proves_is_an_enum_even_when_fields_enum_omits_it() {
        let fields_c_types = HashMap::from([("batch_object.status".to_string(), "BatchStatus".to_string())]);
        let enums = vec![crate::core::ir::EnumDef {
            name: "BatchStatus".into(),
            ..crate::core::ir::EnumDef::default()
        }];
        let mut fields_enum: HashSet<String> = HashSet::new();
        fields_enum.extend(enum_fields_from_ir(&fields_c_types, &enums));

        let mut out = String::new();
        let mut handles = Vec::new();
        let fired = try_emit_enum_accessor(
            &mut out,
            "sample",
            "SAMPLE",
            "status",
            "status",
            "batch_object",
            "sample_batch_object_status",
            "result",
            "status",
            &fields_c_types,
            &fields_enum,
            &mut handles,
        );

        assert!(
            fired,
            "enum accessor must fire once the IR-derived override is unioned in"
        );
        assert!(
            out.contains("sample_batch_status_to_string("),
            "must convert via _to_string, not leave a bare handle for strcmp: {out}"
        );
        assert!(!out.contains("strcmp"), "{out}");
    }
}

#[cfg(test)]
mod result_type_resolution_tests;
