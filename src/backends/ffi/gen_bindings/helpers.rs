use crate::backends::ffi::type_map::is_void_return;
use crate::codegen::c_consumer;
use crate::codegen::doc_emission::emit_c_doxygen;
use crate::core::ir::{ApiSurface, TypeRef};
use ahash::AHashSet;

/// Render a `/** ... */` Doxygen block above a `typedef` line. `doc` is the
/// raw rustdoc lifted from the upstream type's `///` comments; an empty `doc`
/// yields the empty string so the caller can place a bare `typedef` directly.
///
/// The block is built by reusing the shared `emit_c_doxygen` emitter (which
/// produces `///`-prefixed lines) and converting the result into `/** * */`
/// form, because `forward_decls` is C-text passthrough — there is no source
/// line for cbindgen to lift `///` comments from. Indentation is forced to
/// zero so the inserted block aligns with the typedef.
fn render_doxygen_typedef_block(doc: &str) -> String {
    if doc.trim().is_empty() {
        return String::new();
    }
    let mut raw = String::new();
    emit_c_doxygen(&mut raw, doc, "");
    let mut out = String::with_capacity(raw.len() + 16);
    out.push_str("/**\n");
    for line in raw.lines() {
        let body = line.strip_prefix("/// ").unwrap_or(line.trim_start_matches("///"));
        if body.is_empty() {
            out.push_str(" *\n");
        } else {
            out.push_str(" * ");
            out.push_str(body);
            out.push('\n');
        }
    }
    out.push_str(" */\n");
    out
}

/// Render an expression that produces a Copy-typed value, avoiding clippy::clone_on_copy.
///
/// `expr` is either a place expression (e.g., `obj.field`, `(*obj.field)`) or a binding
/// to a reference (e.g., `val`). For places, auto-copy applies. For refs, we deref.
fn copy_expr(expr: &str) -> String {
    if expr.starts_with("obj.") || expr.starts_with("(*") {
        expr.to_string()
    } else {
        format!("*{expr}")
    }
}

/// Generate code to convert a Rust value reference to a C return value.
///
/// `expr` is the Rust expression to read from (a borrowed place or ref binding).
/// `enum_names` is the set of IR enum type names — Copy in our codegen, so we use
/// the copy path instead of `.clone()` (avoids `clippy::clone_on_copy`).
/// `clone_names` is the set of IR named-type names that implement `Clone`.
/// `Named` types outside `clone_names` cannot be returned as owned handles safely.
pub(super) fn gen_value_to_c(
    expr: &str,
    ty: &TypeRef,
    indent: &str,
    enum_names: &AHashSet<String>,
    clone_names: &AHashSet<String>,
) -> String {
    match ty {
        TypeRef::Primitive(p) => {
            let type_class = if matches!(p, crate::core::ir::PrimitiveType::Bool) {
                "primitive_bool"
            } else {
                "primitive_other"
            };
            crate::backends::ffi::template_env::render(
                "value_to_c_conversion.jinja",
                minijinja::context! {
                    type_class => type_class,
                    expr => expr,
                    indent => indent,
                },
            )
        }
        TypeRef::String | TypeRef::Char => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "string",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Path => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "path",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Json => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "json_or_vec_or_map",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Named(name) => {
            if enum_names.contains(name.as_str()) {
                // Copy-typed enums: clippy::clone_on_copy fires on .clone(). Use auto-copy/deref.
                let copy = copy_expr(expr);
                crate::backends::ffi::template_env::render(
                    "value_to_c_conversion.jinja",
                    minijinja::context! {
                        type_class => "named_enum",
                        expr => expr,
                        copy_expr => &copy,
                        indent => indent,
                    },
                )
            } else if clone_names.contains(name.as_str()) {
                crate::backends::ffi::template_env::render(
                    "value_to_c_conversion.jinja",
                    minijinja::context! {
                        type_class => "named_clone",
                        expr => expr,
                        indent => indent,
                    },
                )
            } else {
                crate::backends::ffi::template_env::render(
                    "value_to_c_conversion.jinja",
                    minijinja::context! {
                        type_class => "named_non_clone",
                        expr => expr,
                        indent => indent,
                    },
                )
            }
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "json_or_vec_or_map",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Bytes => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "bytes",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Duration => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "duration",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Unit => String::new(),
        TypeRef::Optional(inner) => {
            let inner_conversion = gen_value_to_c("val", inner, &format!("{indent}        "), enum_names, clone_names);
            let null_value = null_return_value(&TypeRef::Optional(inner.clone()));
            crate::backends::ffi::template_env::render(
                "value_to_c_conversion.jinja",
                minijinja::context! {
                    type_class => "optional",
                    expr => expr,
                    indent => indent,
                    inner_conversion => &inner_conversion,
                    null_value => null_value,
                },
            )
        }
    }
}

/// Generate a type-appropriate unsupported body for FFI.
/// Uses set_last_error + null/zero return instead of panicking.
pub(super) fn gen_ffi_unimplemented_body(return_type: &TypeRef, fn_name: &str, has_error: bool) -> String {
    let err_msg = format!("Not implemented: {fn_name}");
    if has_error && is_void_return(return_type) {
        format!("    set_last_error(99, \"{err_msg}\");\n    -1")
    } else if is_void_return(return_type) {
        format!("    set_last_error(99, \"{err_msg}\");")
    } else {
        let ret = null_return_value(return_type);
        format!("    set_last_error(99, \"{err_msg}\");\n    {ret}")
    }
}

/// Return the null/zero value for a given type in return position.
pub(super) fn null_return_value(ty: &TypeRef) -> &'static str {
    use crate::core::ir::PrimitiveType;
    match ty {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::F32 | PrimitiveType::F64 => "0.0",
            _ => "0",
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "std::ptr::null_mut()",
        TypeRef::Bytes => "std::ptr::null_mut()",
        TypeRef::Named(_) => "0",
        TypeRef::Vec(_) | TypeRef::Map(_, _) => "std::ptr::null_mut()",
        TypeRef::Duration => "0",
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Primitive(p) => match p {
                PrimitiveType::F32 | PrimitiveType::F64 => "0.0",
                _ => "0",
            },
            TypeRef::Optional(inner2) => match inner2.as_ref() {
                TypeRef::Primitive(p) => match p {
                    PrimitiveType::F32 | PrimitiveType::F64 => "0.0",
                    _ => "0",
                },
                TypeRef::Named(_) => "0",
                _ => "std::ptr::null_mut()",
            },
            TypeRef::Duration => "0",
            TypeRef::Named(_) => "0",
            _ => "std::ptr::null_mut()",
        },
        TypeRef::Unit => "()",
    }
}

pub(super) fn ffi_null_return_value<'a>(ty: &TypeRef, ffi_return_type: Option<&'a str>) -> &'a str {
    match ffi_return_type {
        Some("AlefHandle") => "0",
        Some(return_type) if return_type.starts_with("*const ") => "std::ptr::null()",
        Some(return_type) if return_type.starts_with("*mut ") => "std::ptr::null_mut()",
        _ => null_return_value(ty),
    }
}

pub(super) fn gen_owned_value_to_c(expr: &str, ty: &TypeRef, indent: &str, _enum_names: &AHashSet<String>) -> String {
    match ty {
        TypeRef::Primitive(prim) => match prim {
            crate::core::ir::PrimitiveType::Bool => crate::backends::ffi::template_env::render(
                "owned_value_to_c_bool.jinja",
                minijinja::context! {
                    expr => expr,
                    indent => indent,
                },
            ),
            _ => crate::backends::ffi::template_env::render(
                "value_to_c_conversion.jinja",
                minijinja::context! {
                    type_class => "primitive_other",
                    expr => expr,
                    indent => indent,
                },
            ),
        },
        TypeRef::String | TypeRef::Char => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "string",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Json => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "json_or_vec_or_map",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Path => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "path",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Named(_) => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "named_owned",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Vec(_) | TypeRef::Map(_, _) => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "json_or_vec_or_map",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Bytes => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "bytes",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Optional(inner) => {
            // clippy::manual_unwrap_or.  Bool needs `as i32` and stays with the match form.
            if let TypeRef::Primitive(prim) = inner.as_ref()
                && !matches!(prim, crate::core::ir::PrimitiveType::Bool)
            {
                let null_value = null_return_value(&TypeRef::Optional(inner.clone()));
                return format!("{indent}{expr}.unwrap_or({null_value})");
            }
            let inner_conversion = gen_owned_value_to_c("val", inner, &format!("{indent}        "), _enum_names);
            let null_value = null_return_value(&TypeRef::Optional(inner.clone()));
            crate::backends::ffi::template_env::render(
                "value_to_c_conversion.jinja",
                minijinja::context! {
                    type_class => "optional_owned",
                    expr => expr,
                    indent => indent,
                    inner_conversion => &inner_conversion,
                    null_value => null_value,
                },
            )
        }
        TypeRef::Duration => crate::backends::ffi::template_env::render(
            "value_to_c_conversion.jinja",
            minijinja::context! {
                type_class => "duration",
                expr => expr,
                indent => indent,
            },
        ),
        TypeRef::Unit => String::new(),
    }
}

pub(super) fn cbindgen_exclude_type_names(
    api: &crate::core::ir::ApiSurface,
    config: &crate::core::config::ResolvedCrateConfig,
) -> std::collections::BTreeSet<String> {
    let mut exclude_types: std::collections::BTreeSet<String> = config
        .ffi
        .as_ref()
        .map(|c| {
            c.exclude_types
                .iter()
                .filter_map(|name| bare_rust_type_name(name))
                .collect()
        })
        .unwrap_or_default();
    exclude_types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
    exclude_types.extend(api.enums.iter().filter(|e| e.binding_excluded).map(|e| e.name.clone()));
    exclude_types.extend(api.errors.iter().filter(|e| e.binding_excluded).map(|e| e.name.clone()));
    let live_binding_type_names: std::collections::BTreeSet<&str> = api
        .types
        .iter()
        .filter(|t| !t.binding_excluded)
        .map(|t| t.name.as_str())
        .chain(
            api.enums
                .iter()
                .filter(|e| !e.binding_excluded)
                .map(|e| e.name.as_str()),
        )
        .chain(
            api.errors
                .iter()
                .filter(|e| !e.binding_excluded)
                .map(|e| e.name.as_str()),
        )
        .collect();
    exclude_types.extend(
        api.excluded_type_paths
            .keys()
            .filter_map(|name| bare_rust_type_name(name))
            .filter(|name| !live_binding_type_names.contains(name.as_str())),
    );
    exclude_types
}

pub(super) fn gen_cbindgen_toml(
    prefix: &str,
    api: &crate::core::ir::ApiSurface,
    capsule_types: &std::collections::HashMap<String, crate::core::config::FfiCapsuleTypeConfig>,
    exclude_types: &std::collections::BTreeSet<String>,
) -> String {
    // The literal string written here becomes cbindgen's `[export] prefix`, which cbindgen then
    // prepends verbatim to every exported type name. Shouty-snake is the idiomatic C symbol
    // prefix (and is what `docs/naming.rs`'s Ffi/C arm independently predicts for the header),
    // so this derivation -- not a bare uppercase -- is what actually belongs in cbindgen.toml.
    // It lives in `c_consumer::export_type_prefix` so the consumers that must *name* header
    // types (docs snippets, e2e suites) read the same formula instead of re-deriving it.
    // See the note on `type_name`'s Ffi/C arm in docs/naming.rs for the full history. ~keep
    let prefix_upper = c_consumer::export_type_prefix(prefix);
    let feature_defines = cbindgen_feature_defines(api, &prefix_upper);

    let capsule_used_as_opaque: std::collections::HashSet<&str> = api
        .types
        .iter()
        .flat_map(|t| t.methods.iter())
        .filter_map(|m| match &m.return_type {
            crate::core::ir::TypeRef::Named(name) if capsule_types.contains_key(name) => Some(name.as_str()),
            _ => None,
        })
        .collect();
    let mut entries: Vec<(String, String, bool)> = api
        .types
        .iter()
        .filter(|t| !exclude_types.contains(&t.name))
        .filter(|t| !capsule_types.contains_key(t.name.as_str()) || capsule_used_as_opaque.contains(t.name.as_str()))
        .map(|t| (format!("{prefix_upper}{}", t.name), t.doc.clone(), t.is_opaque))
        .collect();

    {
        let c_names = super::capsule::capsule_forward_declared_c_types(api, capsule_types, &capsule_used_as_opaque);
        for c_name in c_names {
            if !entries.iter().any(|(name, _, _)| name == c_name) {
                entries.push((c_name.to_string(), String::new(), false));
            }
        }
    }

    for e in api.enums.iter().filter(|e| !exclude_types.contains(&e.name)) {
        let c_name = format!("{prefix_upper}{}", e.name);
        if !entries.iter().any(|(name, _, _)| name == &c_name) {
            entries.push((c_name, e.doc.clone(), false));
        }
    }

    for err in api.errors.iter().filter(|err| !exclude_types.contains(&err.name)) {
        if !err.methods.is_empty() {
            let c_name = format!("{prefix_upper}{}", err.name);
            if !entries.iter().any(|(name, _, _)| name == &c_name) {
                entries.push((c_name, err.doc.clone(), false));
            }
        }
    }

    for svc in api.services.iter() {
        let c_name = format!("{prefix_upper}{}", svc.name);
        if !entries.iter().any(|(name, _, _)| name == &c_name) {
            entries.push((c_name, svc.doc.clone(), false));
        }
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let forward_decls: String = entries
        .iter()
        .map(|(name, doc, is_handle)| {
            let doc_block = render_doxygen_typedef_block(doc);
            if *is_handle {
                format!("{doc_block}typedef uint64_t {name};")
            } else if doc_block.is_empty() {
                format!("typedef struct {name} {name};")
            } else {
                format!("{doc_block}typedef struct {name} {name};")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let after_includes = if forward_decls.is_empty() {
        String::new()
    } else {
        toml_multiline_basic_string(&format!("/* Opaque type forward declarations */\n{forward_decls}\n"))
    };

    crate::backends::ffi::template_env::render(
        "cbindgen_toml.jinja",
        minijinja::context! {
            prefix_upper => &prefix_upper,
            after_includes => &after_includes,
            feature_defines => feature_defines,
            export_exclude => exclude_types.iter().cloned().collect::<Vec<_>>(),
        },
    )
}

/// The `[defines]` table cbindgen uses to turn a source `#[cfg(feature = "x")]` into a header
/// `#if defined(...)` guard.
///
/// This is the second of two independent walks over the same IR that decide which features exist;
/// the other is `codegen::cfg::collect_cfg_features`, which feeds every binding crate's
/// `[features]` table and, through `backends::go::cgo_features`, the cgo preamble's `-D` list.
/// The two are deliberately not unified: this walk has no `is_host` rust_path filter, because a
/// type merged from a foreign `[[crates.source_crates]]` crate still needs its header declaration
/// guarded even though forwarding its feature to the core dependency would break cargo
/// resolution. Their per-position coverage is pinned by `super::tests::feature_defines`; edit one
/// walk and that test tells you whether the other has to move too. ~keep
pub(super) fn cbindgen_feature_defines(api: &crate::core::ir::ApiSurface, prefix_upper: &str) -> Vec<(String, String)> {
    // Method gates count as much as item gates: `gen_method_wrapper` emits `#[cfg(...)]` on the
    // exported wrapper, and a feature missing from `[defines]` makes cbindgen emit that
    // declaration *unguarded* rather than dropping or guarding it — see the note below. ~keep
    let method_cfgs = api
        .types
        .iter()
        .flat_map(|item| item.methods.iter())
        .chain(api.enums.iter().flat_map(|item| item.methods.iter()))
        // The error arm is the one position this walk reads and `collect_cfg_features` does not,
        // and it is currently inert on both sides: `codegen::error_gen::gen_ffi_error_methods`
        // emits the introspection wrappers with no `#[cfg]`, so cbindgen never guards them and
        // this entry never matches. It is kept, not deleted, because the gate belongs on those
        // wrappers — the day one of them re-emits `MethodDef::rust_cfg_attribute`, the header
        // needs the define and the binding crates need the feature declared. ~keep
        .chain(api.errors.iter().flat_map(|item| item.methods.iter()))
        .filter_map(|method| method.cfg.as_deref());
    let mut cfgs = api
        .types
        .iter()
        .filter_map(|item| item.cfg.as_deref())
        .chain(api.enums.iter().filter_map(|item| item.cfg.as_deref()))
        .chain(api.functions.iter().filter_map(|item| item.cfg.as_deref()))
        .chain(api.services.iter().filter_map(|item| item.cfg.as_deref()))
        .chain(method_cfgs);
    let mut features = std::collections::BTreeSet::new();
    for cfg in &mut cfgs {
        collect_cfg_feature_names(cfg, &mut features);
    }
    features
        .into_iter()
        .map(|feature| {
            let macro_name = feature
                .chars()
                .map(|character| {
                    if character.is_ascii_alphanumeric() {
                        character.to_ascii_uppercase()
                    } else {
                        '_'
                    }
                })
                .collect::<String>();
            // cbindgen's `[defines]` key matcher (`DefineKey::load` in
            // cbindgen's `ir::cfg`) splits the key on `=` and trims
            // whitespace only — it does not strip quotes. The value side
            // must therefore be the bare feature name (`feature = tokenizer`,
            // matching cbindgen's own docs), not a quoted string
            // (`feature = "tokenizer"`); the latter never equals the
            // unquoted `cfg_value` cbindgen extracts from a parsed
            // `#[cfg(feature = "tokenizer")]` attribute via `LitStr::value()`,
            // so the define silently fails to match and the item is emitted
            // unguarded. ~keep
            (
                format!("feature = {feature}"),
                format!("{prefix_upper}_FEATURE_{macro_name}"),
            )
        })
        .collect()
}

fn collect_cfg_feature_names<'a>(cfg: &'a str, features: &mut std::collections::BTreeSet<&'a str>) {
    const FEATURE_PREFIX: &str = "feature = \"";
    let mut remainder = cfg;
    while let Some(start) = remainder.find(FEATURE_PREFIX) {
        let value = &remainder[start + FEATURE_PREFIX.len()..];
        let Some(end) = value.find('"') else {
            return;
        };
        if end > 0 {
            features.insert(&value[..end]);
        }
        remainder = &value[end + 1..];
    }
}

fn bare_rust_type_name(name: &str) -> Option<String> {
    let bare = name.rsplit("::").next()?.trim();
    if bare.is_empty() { None } else { Some(bare.to_string()) }
}

fn toml_multiline_basic_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace("\"\"\"", "\\\"\\\"\\\"")
        .replace('\u{8}', "\\b")
        .replace('\u{c}', "\\f");
    format!("\"\"\"\n{escaped}\"\"\"")
}

pub(super) fn gen_build_rs(
    header_name: &str,
    lib_name: &str,
    ffi_crate_root: &str,
    go_output_dir: Option<&str>,
    prefix: &str,
    capsule_types: &std::collections::HashMap<String, crate::core::config::FfiCapsuleTypeConfig>,
) -> anyhow::Result<String> {
    // Must match `gen_cbindgen_toml`'s `prefix_upper` above. The capsule fixup rewrites capsule
    // pointee type names as they literally appear in the generated header text, and the feature
    // stamp has to spell both the include guard (`{prefix_upper}_H`) and the guard macros
    // (`{prefix_upper}_FEATURE_X`) exactly as cbindgen wrote them -- a different derivation here
    // would silently stop matching for a prefix with an internal capital. ~keep
    let prefix_upper = c_consumer::export_type_prefix(prefix);
    let escaped_header_name = super::rust_literal::escape_rust_str_literal(header_name);
    let capsule_header_fixup = super::rust_literal::capsule_header_fixup(capsule_types, &prefix_upper);
    // The Go include dir is a SECOND publish destination, not a fan-out list that happens to
    // have one entry. Go's `binding.go` carries `#include "<header>"` under cgo, so the header
    // must be vendored next to the Go sources for the package to compile at all. Every other
    // binding (Panama/JNI, P/Invoke, PyO3, NAPI, Magnus, ...) resolves the cdylib's symbols at
    // run time and never sees a header; Zig and generated C read the canonical staged header
    // directly. Do NOT add package directories here just because they ship the cdylib -- a
    // vendored copy of a generated header that nothing compiles is a drift generator that rots
    // into a false record of the ABI. ~keep
    let go_header_destination = match go_output_dir {
        Some(go_dir) => {
            let go_dir = go_dir.trim_end_matches('/');
            let target = format!("{go_dir}/include/{header_name}");
            let relative = crate::core::config::abi_grammar::relative_repo_path(ffi_crate_root, &target)
                .map_err(anyhow::Error::msg)?;
            let destination = super::rust_literal::escape_rust_str_literal(&relative);
            format!("        Path::new(\"{destination}\"),\n")
        }
        None => String::new(),
    };
    Ok(crate::backends::ffi::template_env::render(
        "build_rs.jinja",
        minijinja::context! {
            header_name => &escaped_header_name,
            lib_name => lib_name,
            prefix_upper => &prefix_upper,
            go_header_destination => go_header_destination,
            capsule_header_fixup => capsule_header_fixup,
        },
    ))
}

#[derive(serde::Serialize)]
struct FfiErrorVariantCode {
    pattern: String,
    code_expression: String,
}

#[derive(serde::Serialize)]
struct FfiErrorCodeImpl {
    error_path: String,
    variants: Vec<FfiErrorVariantCode>,
}

pub(super) fn gen_last_error(api: &ApiSurface, prefix: &str, core_import: &str) -> String {
    let taxonomy = api.error_taxonomy();
    let error_code_impls: Vec<_> = api
        .errors
        .iter()
        .map(|error| {
            let error_path = if error.rust_path.contains("::") {
                error.rust_path.replace('-', "_")
            } else {
                format!("{core_import}::{}", error.name)
            };
            let variants = error
                .variants
                .iter()
                .map(|variant| {
                    let suffix = if variant.is_unit {
                        String::new()
                    } else if variant.is_tuple {
                        "(..)".to_string()
                    } else {
                        " { .. }".to_string()
                    };
                    FfiErrorVariantCode {
                        pattern: format!("{error_path}::{}{suffix}", variant.name),
                        code_expression: variant.error_code.map_or_else(
                            || "ALEF_FFI_UNKNOWN_ERROR".to_string(),
                            |_| {
                                let variant_name = crate::codegen::naming::ffi_error_code_variant_name(
                                    &error.rust_path,
                                    &variant.name,
                                );
                                format!("AlefFfiErrorCode::{variant_name} as i32")
                            },
                        ),
                    }
                })
                .collect();
            FfiErrorCodeImpl { error_path, variants }
        })
        .collect();
    let has_error_code_impls = !error_code_impls.is_empty();
    crate::backends::ffi::template_env::render(
        "last_error.jinja",
        minijinja::context! {
            prefix => prefix,
            builtin_prefix => crate::codegen::naming::ffi_builtin_error_code_prefix(prefix),
            error_code_impls => error_code_impls,
            has_error_code_impls => has_error_code_impls,
            taxonomy => taxonomy.iter().map(|entry| minijinja::context! {
                code => entry.code,
                enum_variant => crate::codegen::naming::ffi_error_code_variant_name(&entry.error_type, &entry.variant),
            }).collect::<Vec<_>>(),
            no_error_code => ApiSurface::FFI_ERROR_CODE_NONE,
            conversion_error_code => ApiSurface::FFI_ERROR_CODE_CONVERSION,
            unknown_error_code => ApiSurface::FFI_ERROR_CODE_UNKNOWN,
            panic_error_code => ApiSurface::FFI_ERROR_CODE_PANIC,
            invalid_handle_error_code => ApiSurface::FFI_ERROR_CODE_INVALID_HANDLE,
        },
    )
}

pub(super) fn gen_free_string(prefix: &str) -> String {
    crate::backends::ffi::template_env::render(
        "free_string.jinja",
        minijinja::context! {
            prefix => prefix,
        },
    )
}

pub(super) fn gen_version(prefix: &str) -> String {
    crate::backends::ffi::template_env::render(
        "version_fn.jinja",
        minijinja::context! {
            prefix => prefix,
        },
    )
}

pub(super) fn gen_free_bytes(prefix: &str) -> String {
    crate::backends::ffi::template_env::render("free_bytes.jinja", minijinja::context! { prefix => prefix })
}

/// Generate a lazily-initialized tokio runtime helper for blocking on async
/// functions from synchronous FFI entry points.
pub(super) fn gen_ffi_tokio_runtime() -> String {
    crate::backends::ffi::template_env::render("ffi_tokio_runtime.jinja", minijinja::context! {})
}

/// Generate the three iterator-handle functions for a streaming adapter:
///
/// - `{prefix}_{type_snake}_{name}_start` — create handle from client + request
/// - `{prefix}_{type_snake}_{name}_next`  — advance stream, return boxed chunk or null
/// - `{prefix}_{type_snake}_{name}_free`  — drop handle
///
/// Also emits the opaque handle struct that owns the tokio runtime + BoxStream.
///
/// The handle name is derived as `{PascalPrefix}{PascalOwnerType}{PascalName}StreamHandle`.
/// The function prefix is `{prefix}_{owner_type_snake}_{adapter_name}`.
///
/// Error protocol: `_next` returns null on both clean end-of-stream AND error.
/// After null, caller checks `{prefix}_last_error_code()` — 0 is clean end, non-zero is error.
pub(super) fn gen_stream_handle_functions(
    prefix: &str,
    owner_type: &str,
    adapter_name: &str,
    core_path: &str,
    item_type: &str,
    request_type: &str,
    core_import: &str,
) -> String {
    use heck::{ToPascalCase, ToSnakeCase};

    let pascal_prefix = prefix.to_pascal_case();
    let pascal_owner = owner_type.to_pascal_case();
    let pascal_name = adapter_name.to_pascal_case();
    let owner_snake = owner_type.to_snake_case();

    let handle_name = format!("{pascal_prefix}{pascal_owner}{pascal_name}StreamHandle");
    let fn_start = c_consumer::stream_adapter_symbol(prefix, owner_type, adapter_name, "start");
    let fn_next = c_consumer::stream_adapter_symbol(prefix, owner_type, adapter_name, "next");
    let fn_free = c_consumer::stream_adapter_symbol(prefix, owner_type, adapter_name, "free");

    let core_item = format!("{core_import}::{item_type}");
    let boxed_err = "Box<dyn std::error::Error + Send + Sync + 'static>";
    let stream_ty = format!("futures_util::stream::BoxStream<'static, Result<{core_item}, {boxed_err}>>");
    let owner_ty = format!("{core_import}::{owner_type}");

    format!(
        r#"/// Opaque handle owning a tokio runtime and a boxed chat-stream for iterator-style consumption.
///
/// Created by `{fn_start}`, advanced by `{fn_next}`, destroyed by `{fn_free}`.
/// The handle is NOT thread-safe — callers must ensure only one thread calls `_next` at a time.
pub struct {handle_name} {{
    rt: tokio::runtime::Runtime,
    stream: std::sync::Mutex<Option<{stream_ty}>>,
}}

/// Start a streaming chat completion and return an opaque iterator handle.
///
/// Returns null and sets `{prefix}_last_error_code` on failure (null pointers or stream-open error).
/// On success the caller owns the returned pointer and MUST call `{fn_free}` when done.
///
/// # Safety
/// `client` must be a non-null valid pointer to a live `{owner_ty}` produced by this library.
/// `req` must be a non-null valid pointer to a live `{request_type}` produced by this library.
/// Both pointers must remain valid until this function returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {fn_start}(
    client: AlefHandle,
    req: AlefHandle,
) -> AlefHandle {{
    catch_ffi_panic(0, || {{
    clear_last_error();

    let requests = [
        HandleRequest {{ handle: client, expected_type: std::any::TypeId::of::<{owner_ty}>() }},
        HandleRequest {{ handle: req, expected_type: std::any::TypeId::of::<{request_type}>() }},
    ];
    let values = match acquire_handles(&requests) {{
        Ok(values) => values,
        Err(error) => {{ set_handle_error(&error); return 0; }}
    }};
    let mut guards = Vec::with_capacity(values.len());
    for (token, value) in &values {{
        match value.lock() {{
            Ok(guard) => guards.push((*token, guard)),
            Err(_) => {{ set_handle_error(&HandleError::RegistryPoisoned); return 0; }}
        }}
    }}
    let client_ptr = match locked_handle_ptr::<{owner_ty}>(&mut guards, client) {{
        Ok(value) => value,
        Err(error) => {{ set_handle_error(&error); return 0; }}
    }};
    let request_ptr = match locked_handle_ptr::<{request_type}>(&mut guards, req) {{
        Ok(value) => value,
        Err(error) => {{ set_handle_error(&error); return 0; }}
    }};
    // SAFETY: both registry entry guards remain held for the duration of this call.
    let client_ref = unsafe {{ &*client_ptr }};
    // SAFETY: both registry entry guards remain held for the duration of this call.
    let req_owned = unsafe {{ &*request_ptr }}.clone();

    // 16 MiB: tokio's ~2 MB default worker stack can overflow on a deep extraction
    // future (a nested archive member, a multi-stage OCR pipeline), and a stack overflow
    // aborts the process with SIGBUS instead of raising a catchable panic.
    const STREAM_HANDLE_RUNTIME_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(STREAM_HANDLE_RUNTIME_STACK_SIZE_BYTES)
        .build()
    {{
        Ok(r) => r,
        Err(e) => {{
            set_last_error(99, &format!("{fn_start}: failed to create tokio runtime: {{e}}"));
            return 0;
        }}
    }};

    let stream_result = rt.block_on(async {{ client_ref.{core_path}(req_owned).await }});

    let raw_stream = match stream_result {{
        Ok(s) => s,
        Err(e) => {{
            set_last_error(99, &format!("{fn_start}: failed to open stream: {{e}}"));
            return 0;
        }}
    }};

    // Map the stream's concrete error type to Box<dyn Error> to erase it from the handle type.
    let mapped: {stream_ty} = {{
        use futures_util::StreamExt;
        Box::pin(raw_stream.map(|r| r.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>)))
    }};

    let handle = {handle_name} {{
        rt,
        stream: std::sync::Mutex::new(Some(mapped)),
    }};
    match insert_handle(handle) {{
        Ok(handle) => handle,
        Err(error) => {{ set_handle_error(&error); 0 }}
    }}
    }})
}}

/// Advance the stream and return a heap-allocated chunk, or null.
///
/// Returns null in two cases:
/// - Clean end-of-stream: `{prefix}_last_error_code()` returns 0.
/// - Stream error: `{prefix}_last_error_code()` returns non-zero.
///
/// The returned pointer is heap-allocated and the caller MUST free it by calling
/// `{prefix}_{owner_snake}_{item_type}_free` (or the appropriate type-free function).
///
/// # Safety
/// `handle` must be a non-null valid pointer previously returned by `{fn_start}` and not yet
/// freed. Calling `_next` after `_free` is undefined behaviour. The handle must not be shared
/// across threads without external synchronisation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {fn_next}(
    handle: AlefHandle,
) -> AlefHandle {{
    catch_ffi_panic(0, || {{
    clear_last_error();

    let value = match acquire_handles(&[HandleRequest {{
        handle,
        expected_type: std::any::TypeId::of::<{handle_name}>(),
    }}]) {{
        Ok(mut values) => match values.pop() {{
            Some(value) => value,
            None => {{ set_handle_error(&HandleError::UnknownSlot); return 0; }}
        }},
        Err(error) => {{ set_handle_error(&error); return 0; }}
    }};
    let h_guard = match value.1.lock() {{
        Ok(guard) => guard,
        Err(_) => {{ set_handle_error(&HandleError::RegistryPoisoned); return 0; }}
    }};
    let h = match h_guard.downcast_ref::<{handle_name}>() {{
        Some(value) => value,
        None => {{ set_handle_error(&HandleError::WrongType); return 0; }}
    }};

    let mut guard = match h.stream.lock() {{
        Ok(g) => g,
        Err(_) => {{
            set_last_error(99, "{fn_next}: stream mutex is poisoned");
            return 0;
        }}
    }};

    let stream = match guard.as_mut() {{
        Some(s) => s,
        None => {{
            // Stream already exhausted or taken.
            return 0;
        }}
    }};

    use futures_util::StreamExt;
    match h.rt.block_on(stream.next()) {{
        Some(Ok(chunk)) => {{
            // SAFETY: We box the chunk and transfer ownership to the caller via raw pointer.
            // The caller must free it via the appropriate type-free function.
            match insert_handle(chunk) {{
                Ok(handle) => handle,
                Err(error) => {{ set_handle_error(&error); 0 }}
            }}
        }}
        Some(Err(e)) => {{
            set_last_error(99, &format!("{fn_next}: stream error: {{e}}"));
            0
        }}
        None => {{
            // Clean end-of-stream — error code remains 0 (cleared at top of function).
            *guard = None;
            0
        }}
    }}
    }})
}}

/// Free a stream handle created by `{fn_start}`.
///
/// Safe to call with a null pointer (no-op). After this call the handle pointer is invalid.
///
/// # Safety
/// `handle` must either be null or a valid pointer previously returned by `{fn_start}` and
/// not yet freed. Double-free is undefined behaviour.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {fn_free}(handle: AlefHandle) {{
    catch_ffi_panic((), || {{
    if handle != 0
        && let Err(error) = remove_handle::<{handle_name}>(handle)
    {{
        set_handle_error(&error);
    }}
    }})
}}"#,
        handle_name = handle_name,
        fn_start = fn_start,
        fn_next = fn_next,
        fn_free = fn_free,
        prefix = prefix,
        owner_ty = owner_ty,
        request_type = request_type,
        core_path = core_path,
        stream_ty = stream_ty,
        owner_snake = owner_snake,
        item_type = item_type,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The feature stamp `gen_build_rs` writes into the header must test the *same* macro names
    /// `gen_cbindgen_toml` puts in cbindgen's `[defines]`, and must anchor on the *same* include
    /// guard cbindgen opens the file with. Both are derived here from the two emitters rather
    /// than pinned, because a stamp spelled any other way defines macros no `#if` in the header
    /// ever tests -- it fails silently, and only in a consumer's compiler.
    ///
    /// `SampleCore` is the discriminating prefix: shouty-snake (`SAMPLE_CORE`) and bare uppercase
    /// (`SAMPLECORE`) diverge on it, so a re-derivation that drifts is visible here and invisible
    /// for an already-underscored prefix like `ts_pack`. ~keep
    #[test]
    fn build_rs_feature_stamp_matches_the_cbindgen_defines_and_include_guard() {
        let api = crate::core::ir::ApiSurface {
            crate_name: "sample_core".to_string(),
            version: "0.1.0".to_string(),
            functions: vec![crate::core::ir::FunctionDef {
                name: "download".to_string(),
                rust_path: "sample_core::download".to_string(),
                cfg: Some(r#"feature = "download""#.to_string()),
                ..crate::core::ir::FunctionDef::default()
            }],
            ..crate::core::ir::ApiSurface::default()
        };
        let capsule_types = std::collections::HashMap::new();
        let cbindgen = gen_cbindgen_toml("SampleCore", &api, &capsule_types, &std::collections::BTreeSet::new());
        let build = gen_build_rs(
            "sample.h",
            "libsample_ffi",
            "crates/sample-ffi",
            None,
            "SampleCore",
            &capsule_types,
        )
        .expect("valid build.rs paths");

        let guard_macro = cbindgen
            .lines()
            .find_map(|line| line.trim().strip_prefix(r#""feature = download" = ""#))
            .map(|rest| rest.trim_end_matches('"'))
            .expect("control: cbindgen.toml must map the gated feature to a guard macro");
        assert_eq!(guard_macro, "SAMPLE_CORE_FEATURE_DOWNLOAD");
        let include_guard = cbindgen
            .lines()
            .find_map(|line| line.trim().strip_prefix(r#"include_guard = ""#))
            .map(|rest| rest.trim_end_matches('"'))
            .expect("control: cbindgen.toml must declare the header's include guard");
        assert_eq!(include_guard, "SAMPLE_CORE_H");

        let macro_stem = guard_macro
            .strip_suffix("DOWNLOAD")
            .expect("guard macro ends with the feature name");
        assert!(
            build.contains(macro_stem),
            "build.rs must probe and define `{macro_stem}*`, the macros cbindgen guards with:\n{build}"
        );
        assert!(
            build.contains(&format!(r##""#define {include_guard}\n""##)),
            "build.rs must anchor the stamp on cbindgen's own include guard `{include_guard}`:\n{build}"
        );
        assert!(
            !build.contains("SAMPLECORE"),
            "build.rs must not fall back to a bare-uppercase prefix for the feature stamp:\n{build}"
        );
    }
}
