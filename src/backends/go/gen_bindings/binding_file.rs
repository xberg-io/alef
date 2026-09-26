use super::constructors::gen_go_opaque_constructor;
use super::functions::{
    gen_adapter_wrapper, gen_capsule_function_wrapper, gen_convert_with_visitor_wrapper, gen_function_wrapper,
};
use super::methods::{gen_method_wrapper, gen_streaming_method_wrapper};
use super::types::{
    gen_config_options, gen_duration_millis_helper, gen_enum_type, gen_last_error_helper, gen_opaque_type,
    gen_opaque_type_free_only, gen_ptr_helper, gen_struct_type, gen_unmarshal_bytes_helper, go_struct_field_names,
};
use crate::codegen::naming::{field_uses_duration_map_wire, go_type_name, to_go_name};
use crate::core::config::{AdapterPattern, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, TypeRef};
use std::collections::HashSet;

/// Strip trailing whitespace from every line and ensure the file ends with a single newline.
pub(super) fn strip_trailing_whitespace(content: &str) -> String {
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

/// Sort key for a rendered Go import line (`"path"` or `alias "path"`): the text between the
/// first pair of double quotes, matching gofmt's own import-sort rule of ordering by import
/// path and ignoring any alias prefix. Falls back to the whole line for a malformed line (no
/// quotes) so a bad input degrades to a stable, if unhelpful, sort rather than panicking.
pub(super) fn go_import_sort_key(line: &str) -> &str {
    let after_open = line.find('"').map(|i| &line[i + 1..]).unwrap_or(line);
    after_open.find('"').map(|i| &after_open[..i]).unwrap_or(after_open)
}

fn body_uses_qualified_name(body: &str, qualified_name: &str) -> bool {
    body.lines().any(|line| {
        line.split("//")
            .next()
            .is_some_and(|code| code.contains(qualified_name))
    })
}

/// Run `gofmt -s` on generated Go code. Falls back to the original if gofmt is unavailable.
pub(super) fn format_go_code(code: &str) -> String {
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
pub(super) fn is_ffi_enum_type(name: &str, ffi_enum_names: &HashSet<String>) -> bool {
    ffi_enum_names.contains(name)
}

/// Returns true if a function references a DATA enum type (from `api.enums`) as a parameter type
/// or return type, for which the FFI header lacks `_from_json`/`_to_json`/`_free` helpers.
///
/// Unit-variant enums (in `ffi_param_enum_names`) can be marshaled to/from i32 and do NOT cause
/// skipping. Data enums without those helpers cannot be generated correctly and must be skipped.
fn uses_ffi_enum_type(
    func_params: &[crate::core::ir::ParamDef],
    return_type: &TypeRef,
    ffi_enum_names: &HashSet<String>,
    ffi_param_enum_names: &HashSet<String>,
    opaque_names: &std::collections::HashSet<&str>,
) -> bool {
    let named_is_problem =
        |n: &str| is_ffi_enum_type(n, ffi_enum_names) && !ffi_param_enum_names.contains(n) && !opaque_names.contains(n);
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

/// True if any non-opaque type in `api` has a `Duration`-typed struct field that is actually
/// emitted as `DurationMillis`.
///
/// Decides whether the generated file needs the package-level `DurationMillis` wire
/// helper (see [`gen_duration_millis_helper`]) and, in turn, the `encoding/json` import
/// it depends on.
///
/// Consults the same predicate as `type_map::go_field_type`
/// (`crate::codegen::naming::field_uses_duration_map_wire`): a field with a hand-written codec
/// keeps the bare `uint64`, so counting it here would emit a `DurationMillis` no field
/// references — and with it an `encoding/json` import Go rejects as unused. ~keep
fn api_has_duration_field(api: &ApiSurface) -> bool {
    api.types
        .iter()
        .filter(|typ| !typ.is_opaque)
        .any(|typ| crate::codegen::shared::binding_fields(&typ.fields).any(field_uses_duration_map_wire))
}

/// Returns true if a type reference mentions any excluded type.
fn references_excluded_type(ty: &TypeRef, exclude_types: &HashSet<String>) -> bool {
    exclude_types.iter().any(|name| ty.references_named(name))
}

/// Returns true if any parameter or return type mentions an excluded type.
fn signature_references_excluded_type(
    params: &[crate::core::ir::ParamDef],
    return_type: &TypeRef,
    exclude_types: &HashSet<String>,
) -> bool {
    references_excluded_type(return_type, exclude_types)
        || params
            .iter()
            .any(|param| references_excluded_type(&param.ty, exclude_types))
}

pub(super) fn find_options_bridge_function<'a>(
    api: &'a ApiSurface,
    bridge_cfg: &TraitBridgeConfig,
) -> Option<&'a crate::core::ir::FunctionDef> {
    api.functions
        .iter()
        .find(|func| options_bridge_function_matches(func, bridge_cfg))
}

fn options_bridge_function_matches(func: &crate::core::ir::FunctionDef, bridge_cfg: &TraitBridgeConfig) -> bool {
    let Some(options_type) = bridge_cfg.options_type.as_deref() else {
        return false;
    };
    func.params
        .iter()
        .any(|param| type_ref_named_type(&param.ty) == Some(options_type))
}

fn type_ref_named_type(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => type_ref_named_type(inner),
        _ => None,
    }
}

/// Returns the host capsule config when `func` returns a configured capsule type by value
/// (bare `Named`). Optional capsule returns fall through to the standard opaque path.
fn go_capsule_return_config<'a>(
    func: &crate::core::ir::FunctionDef,
    capsule_types: &'a std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
) -> Option<&'a crate::core::config::HostCapsuleTypeConfig> {
    if let crate::core::ir::TypeRef::Named(name) = &func.return_type {
        capsule_types.get(name.as_str())
    } else {
        None
    }
}

/// Derive the Go package qualifier used in the body from a capsule `host_type`.
///
/// `host_type` looks like `*tree_sitter.Language` or `tree_sitter.Language`; the
/// qualifier is the identifier the generated body references (`tree_sitter`). This is
/// the alias the import must declare explicitly: when the import path's last element
/// (`go-tree-sitter`) differs from the package name (`tree_sitter`), an unaliased
/// import is stripped by `goimports` in cgo files (it cannot resolve the package name
/// from the path), breaking the build. An explicit alias is matched syntactically and
/// survives. Returns `None` when no qualifier can be derived (no alias needed).
fn go_capsule_import_alias(host_type: &str) -> Option<&str> {
    let bare = host_type.trim_start_matches(['*', '&', '[', ']', ' ']);
    bare.split_once('.').map(|(qualifier, _)| qualifier)
}

/// Generate the complete Go binding file wrapping the C FFI layer.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_go_file(
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
    streaming_methods: &std::collections::HashMap<(String, String), String>,
    ffi_exclude_functions: &HashSet<String>,
    exclude_types: &HashSet<String>,
    value_only_types: &HashSet<String>,
    visitor_bridge_cfg: Option<&TraitBridgeConfig>,
    feature_cflags: &str,
) -> anyhow::Result<String> {
    let mut header = String::with_capacity(2048);

    header.push_str(&hash::header(CommentStyle::DoubleSlash));
    header.push('\n');

    let depth = go_output_dir.trim_end_matches('/').matches('/').count() + 1;
    let to_root = "../".repeat(depth);

    header.push_str(&crate::backends::go::template_env::render(
        "package_doc_and_declaration.jinja",
        minijinja::context! {
            pkg_name => pkg_name,
            crate_name => &config.name,
        },
    ));
    header.push_str(&crate::backends::go::template_env::render(
        "cgo_preamble_binding.jinja",
        minijinja::context! {
            to_root => &to_root,
            ffi_crate_dir => ffi_crate_dir,
            ffi_lib_name => ffi_lib_name,
            ffi_header => ffi_header,
            feature_cflags => feature_cflags,
        },
    ));
    header.push('\n');

    let mut body = String::with_capacity(8192);

    body.push_str(&gen_last_error_helper(api, ffi_prefix));
    body.push_str("\n\n");

    body.push_str(&gen_unmarshal_bytes_helper());
    body.push_str("\n\n");

    body.push_str(&gen_ptr_helper());
    body.push_str("\n\n");

    let needs_duration_helper = api_has_duration_field(api);
    if needs_duration_helper {
        body.push_str(&gen_duration_millis_helper());
        body.push_str("\n\n");
    }

    if !api.errors.is_empty() {
        body.push_str(&crate::codegen::error_gen::gen_go_sentinel_errors(&api.errors));
        body.push_str("\n\n");
        for error in &api.errors {
            body.push_str(&crate::codegen::error_gen::gen_go_error_struct(error, pkg_name));
            body.push_str("\n\n");
        }
    }

    let visitor_types = if visitor_bridge_cfg.is_some() || !bridge_param_names.is_empty() {
        config.bridge_associated_types()
    } else {
        std::collections::HashSet::new()
    };
    let emission = crate::backends::go::emission_facts::GoEmissionFacts::new(
        &api.types,
        &api.enums,
        exclude_types.clone(),
        visitor_types,
    );

    // Go type identifiers the loop below emits; disambiguates collisions via `go_free_function_name`. ~keep
    let reserved_type_names: HashSet<String> = emission
        .structs
        .iter()
        .chain(emission.opaque.iter())
        .chain(emission.enums.iter())
        .map(|name| go_type_name(name))
        .collect();

    let unit_enum_names = &emission.unit_enums;
    let passthrough_enum_names = &emission.passthrough_enums;
    let text_types = &config.untagged_union_text_types;
    for enum_def in api
        .enums
        .iter()
        .filter(|definition| emission.enums.contains(definition.name.as_str()))
    {
        body.push_str(&gen_enum_type(enum_def, text_types));
        body.push_str("\n\n");
    }

    let error_names: std::collections::HashSet<&str> = api.errors.iter().map(|e| e.name.as_str()).collect();

    let opaque_names = &emission.opaque;

    let opaque_names_ahash: ahash::AHashSet<String> = opaque_names.iter().map(|s| s.to_string()).collect();

    let ffi_enum_names: HashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

    let ffi_param_enum_names: HashSet<String> = api
        .enums
        .iter()
        .filter(|e| e.variants.iter().all(|v| v.fields.is_empty() && !v.is_tuple))
        .map(|e| e.name.clone())
        .collect();

    let data_enum_names = &emission.data_enums;
    let struct_names = &emission.structs;

    let mut emitted_struct_fields: std::collections::HashMap<&str, HashSet<String>> = std::collections::HashMap::new();

    for typ in api
        .types
        .iter()
        .filter(|definition| emission.emits_type(&definition.name))
    {
        if typ.is_opaque {
            if error_names.contains(typ.name.as_str()) {
                body.push_str(&gen_opaque_type_free_only(typ, ffi_prefix));
                body.push_str("\n\n");
            } else {
                body.push_str(&gen_opaque_type(typ, ffi_prefix));
                body.push_str("\n\n");
            }
            if let Some(ctor) = config.client_constructors.get(&typ.name) {
                body.push_str(&gen_go_opaque_constructor(typ, ffi_prefix, ctor));
                body.push_str("\n\n");
            }
        } else {
            emitted_struct_fields.insert(typ.name.as_str(), go_struct_field_names(typ));
            body.push_str(&gen_struct_type(
                typ,
                unit_enum_names,
                passthrough_enum_names,
                data_enum_names,
                struct_names,
                &config.trait_bridges,
            ));
            body.push_str("\n\n");
            let empty_functional_options = vec![];
            let functional_options = config
                .go
                .as_ref()
                .map(|g| &g.functional_options)
                .unwrap_or(&empty_functional_options);
            if !typ.name.ends_with("Update") && functional_options.contains(&typ.name) {
                body.push_str(&gen_config_options(
                    typ,
                    unit_enum_names,
                    passthrough_enum_names,
                    data_enum_names,
                    struct_names,
                    &config.trait_bridges,
                ));
                body.push_str("\n\n");
            }
        }
    }

    let go_capsule_types: std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig> =
        config.go.as_ref().map(|c| c.capsule_types.clone()).unwrap_or_default();

    for func in api.functions.iter().filter(|f| {
        !ffi_exclude_functions.contains(&f.name)
            && !signature_references_excluded_type(&f.params, &f.return_type, exclude_types)
            && !uses_ffi_enum_type(
                &f.params,
                &f.return_type,
                &ffi_enum_names,
                &ffi_param_enum_names,
                opaque_names,
            )
            && !crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&f.name, &config.trait_bridges)
    }) {
        crate::codegen::mut_writeback::reject_unsupported_writeback(
            &func.name,
            &func.params,
            &func.return_type,
            &opaque_names_ahash,
        )?;
        if let Some(capsule_cfg) = go_capsule_return_config(func, &go_capsule_types) {
            body.push_str(&gen_capsule_function_wrapper(
                func,
                ffi_prefix,
                opaque_names,
                &ffi_enum_names,
                &ffi_param_enum_names,
                capsule_cfg,
                &reserved_type_names,
            ));
            body.push_str("\n\n");
            continue;
        }
        if visitor_bridge_cfg.is_some_and(|bridge_cfg| options_bridge_function_matches(func, bridge_cfg)) {
            body.push_str(&gen_convert_with_visitor_wrapper(
                func,
                ffi_prefix,
                opaque_names,
                value_only_types,
                visitor_bridge_cfg.expect("checked above"),
                &reserved_type_names,
            ));
            body.push_str("\n\n");
        } else {
            body.push_str(&gen_function_wrapper(
                func,
                ffi_prefix,
                opaque_names,
                bridge_param_names,
                bridge_type_aliases,
                value_only_types,
                &ffi_enum_names,
                &ffi_param_enum_names,
                &reserved_type_names,
            ));
            body.push_str("\n\n");
        }
    }

    for adapter in &config.adapters {
        if !matches!(adapter.pattern, AdapterPattern::Streaming) {
            continue;
        }
        if adapter.owner_type.is_none() || adapter.item_type.is_none() {
            continue;
        }
        body.push_str(&gen_adapter_wrapper(adapter, pkg_name, &api.types));
        body.push_str("\n\n");
    }

    for typ in api
        .types
        .iter()
        .filter(|definition| emission.emits_type(&definition.name))
    {
        if typ.is_opaque && error_names.contains(typ.name.as_str()) {
            continue;
        }
        if !typ.is_opaque && !typ.has_serde {
            continue;
        }
        for method in &typ.methods {
            if method.name == "default" {
                continue;
            }
            // A field and an inherent method sharing a name both map to the same exported Go
            // identifier, and Go rejects `field and method with the same name` at compile time.
            // The struct field is emitted first and wins; the instance-method wrapper is dropped.
            // Static methods mint a free `func TypeMethod(...)`, so they never collide. ~keep
            if !method.is_static
                && emitted_struct_fields
                    .get(typ.name.as_str())
                    .is_some_and(|fields| fields.contains(&to_go_name(&method.name)))
            {
                continue;
            }
            if typ.is_opaque
                && method.is_static
                && method.name != "new"
                && matches!(method.return_type, TypeRef::Named(_))
            {
                continue;
            }
            if let Some(item_type) = streaming_methods.get(&(typ.name.clone(), method.name.clone())) {
                body.push_str(&gen_streaming_method_wrapper(
                    typ,
                    method,
                    ffi_prefix,
                    item_type,
                    data_enum_names,
                    opaque_names,
                    value_only_types,
                    &ffi_enum_names,
                    &ffi_param_enum_names,
                ));
                body.push_str("\n\n");
                continue;
            }
            if ffi_exclude_functions.contains(&method.name) {
                continue;
            }
            if signature_references_excluded_type(&method.params, &method.return_type, exclude_types) {
                continue;
            }
            if uses_ffi_enum_type(
                &method.params,
                &method.return_type,
                &ffi_enum_names,
                &ffi_param_enum_names,
                opaque_names,
            ) {
                continue;
            }
            body.push_str(&gen_method_wrapper(
                typ,
                method,
                ffi_prefix,
                opaque_names,
                value_only_types,
                &ffi_enum_names,
                &ffi_param_enum_names,
            ));
            body.push_str("\n\n");
        }
    }

    let has_opaque_types = !emission.opaque.is_empty();
    // `gen_duration_millis_helper` (pushed into `body` above, when `needs_duration_helper`) emits
    // its `MarshalJSON`/`UnmarshalJSON` methods with literal `json.Marshal`/`json.Unmarshal` calls,
    // so `body_uses_qualified_name` already observes that usage; a separate disjunct here would
    // just restate it. Testing `body` directly instead of proxying through "does this package have
    // any sync function / non-static method" avoids importing `encoding/json` into a package whose
    // sync functions and methods are all primitive-only and never emit a `json.` call. ~keep
    let needs_json = body_uses_qualified_name(&body, "json.");

    let mut imports = vec!["fmt"];
    if needs_json {
        imports.push("encoding/json");
    }
    if body_uses_qualified_name(&body, "runtime.") {
        imports.push("runtime");
    }
    if needs_json || has_opaque_types || body_uses_qualified_name(&body, "unsafe.") {
        imports.push("unsafe");
    }
    if !api.errors.is_empty() {
        imports.push("errors");
    }
    let capsule_emitted = go_capsule_types.values().any(|c| !c.package.is_empty())
        && api
            .functions
            .iter()
            .any(|f| go_capsule_return_config(f, &go_capsule_types).is_some());
    let mut capsule_imports: Vec<(Option<&str>, &str)> = Vec::new();
    if capsule_emitted {
        if !imports.contains(&"unsafe") {
            imports.push("unsafe");
        }
        for cfg in go_capsule_types.values().filter(|c| !c.package.is_empty()) {
            let path = cfg.package.as_str();
            if capsule_imports.iter().any(|(_, p)| *p == path) {
                continue;
            }
            capsule_imports.push((go_capsule_import_alias(&cfg.host_type), path));
        }
    }
    let mut import_lines: Vec<String> = imports.iter().map(|p| format!("\"{p}\"")).collect();
    for (alias, path) in &capsule_imports {
        let line = match alias {
            Some(alias) => format!("{alias} \"{path}\""),
            None => format!("\"{path}\""),
        };
        if !import_lines.contains(&line) {
            import_lines.push(line);
        }
    }
    // gofmt sorts a single (non-blank-line-separated) import block alphabetically by
    // import PATH, ignoring any alias prefix a line carries. Building `import_lines` by
    // successive conditional pushes does not guarantee that order (e.g. `errors` landing
    // after `fmt` when `encoding/json` is absent), so sort explicitly rather than relying
    // on push order to already be canonical. ~keep
    import_lines.sort_by(|a, b| go_import_sort_key(a).cmp(go_import_sort_key(b)));
    let imports_str = crate::backends::go::template_env::render(
        "imports_basic.jinja",
        minijinja::context! {
            imports => import_lines,
        },
    );

    let mut out = String::with_capacity(header.len() + imports_str.len() + body.len());
    out.push_str(&header);
    out.push_str(&imports_str);
    out.push_str(&body);

    Ok(out)
}

#[cfg(test)]
mod import_order_tests {
    use super::go_import_sort_key;

    #[test]
    fn sort_key_extracts_path_ignoring_alias() {
        assert_eq!(go_import_sort_key("\"errors\""), "errors");
        assert_eq!(go_import_sort_key("alias \"zzz/aaa\""), "zzz/aaa");
    }

    /// Reproduces the concrete drift this fix closes: a crate with declared errors but no
    /// sync functions or non-static methods (`needs_json == false`) used to render
    /// `["fmt", "errors"]` verbatim (`imports.insert(1.min(imports.len()), "errors")` pushed
    /// `errors` onto the end of a single-element list). `gofmt` reorders that to
    /// `["errors", "fmt"]`, which is exactly the byte-level drift a consumer's own `gofmt -w`
    /// hook would introduce after generation, permanently stranding the file. Confirmed
    /// against a real `gofmt` invocation in the scratch verification for this change; this
    /// test pins the sort key alef now applies at generation time so no formatter has to run
    /// after the fact for this class of drift. ~keep
    #[test]
    fn import_lines_sort_matches_gofmt_path_order() {
        let mut import_lines: Vec<String> = vec!["\"fmt\"".to_string(), "\"errors\"".to_string()];
        import_lines.sort_by(|a, b| go_import_sort_key(a).cmp(go_import_sort_key(b)));
        assert_eq!(import_lines, vec!["\"errors\"".to_string(), "\"fmt\"".to_string()]);
    }

    /// Negative control: an already-canonical import list must not be reordered.
    #[test]
    fn import_lines_sort_is_a_no_op_when_already_canonical() {
        let mut import_lines: Vec<String> = vec![
            "\"encoding/json\"".to_string(),
            "\"errors\"".to_string(),
            "\"fmt\"".to_string(),
        ];
        let before = import_lines.clone();
        import_lines.sort_by(|a, b| go_import_sort_key(a).cmp(go_import_sort_key(b)));
        assert_eq!(import_lines, before);
    }

    /// Renders the actual `imports_basic.jinja` template for the fixed drift case and, when
    /// `gofmt` is on `PATH`, asserts the rendered file is byte-identical to `gofmt`'s own
    /// output — the same self-skipping shape as `e2e::codegen::go::snippet`'s
    /// `snippet_matches_gofmt_when_available`. Skips (rather than failing) when `gofmt` is
    /// unavailable, so this test still runs the structural sort-key assertions above on every
    /// machine and only exercises the real formatter where it can. ~keep
    #[test]
    fn rendered_import_block_matches_gofmt_when_available() {
        use std::io::Write as _;

        let import_lines = vec!["\"errors\"".to_string(), "\"fmt\"".to_string()];
        let rendered = crate::backends::go::template_env::render(
            "imports_basic.jinja",
            minijinja::context! { imports => import_lines },
        );
        let code = format!("package pkg\n\n{rendered}\nvar _ = fmt.Sprintf\nvar _ = errors.New\n");

        let Ok(mut child) = std::process::Command::new("gofmt")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
        else {
            return;
        };
        child
            .stdin
            .take()
            .expect("gofmt stdin")
            .write_all(code.as_bytes())
            .expect("write Go source");
        let output = child.wait_with_output().expect("wait for gofmt");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(String::from_utf8(output.stdout).expect("gofmt output is UTF-8"), code);
    }
}
