//! Emits `extern "Rust"` blocks for the `#[swift_bridge::bridge]` module.
//!
//! Covers type declarations, enum declarations, and top-level function declarations.
//! Trait bridge extern blocks live in `trait_bridge.rs`.

use crate::backends::swift::gen_rust_crate::type_bridge::{
    bridge_type, bridge_type_enum_and_serde_struct_aware, bridge_type_enum_aware, bridge_type_enum_aware_ref,
    bridge_type_with_handles, is_vec_of_enum, needs_json_bridge,
};
use crate::backends::swift::gen_rust_crate::wrappers::is_unbridgeable_getter;
use crate::core::config::AdapterConfig;
use crate::core::ir::{EnumDef, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::core::keywords::swift_ident;
use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Returns the subset of `ty.fields` that appear in the swift-bridge constructor extern
/// (filters out fields marked `binding_excluded`, any field key listed in `exclude_fields`,
/// and any field whose `#[cfg(...)]` condition is not satisfied by the configured features).
///
/// Order matches `ty.fields` — the positional argument order swift-bridge uses to emit
/// the generated `convenience init(_ a, _ b, ...)`.
pub(crate) fn constructor_fields<'a>(
    ty: &'a TypeDef,
    exclude_fields: &HashSet<String>,
    configured_features: &std::collections::HashSet<&str>,
) -> Vec<&'a FieldDef> {
    ty.fields
        .iter()
        .filter(|f| {
            let field_key = format!("{}.{}", ty.name, f.name.to_snake_case());
            !f.binding_excluded
                && !exclude_fields.contains(&field_key)
                && super::feature_gate::cfg_satisfied(f.cfg.as_deref(), configured_features)
        })
        .collect()
}

/// Returns `true` when `emit_extern_block_for_type` will emit a `#[swift_bridge(init)]`
/// constructor extern for `ty`. Mirrors the gating logic inside `emit_extern_block_for_type`
/// so callers (gen_bindings.rs `intoRust()` emission) can detect the presence of a
/// matching bulk constructor without re-running the whole emitter.
pub(crate) fn has_constructor_extern(
    ty: &TypeDef,
    exclude_fields: &HashSet<String>,
    configured_features: &std::collections::HashSet<&str>,
) -> bool {
    let fields = constructor_fields(ty, exclude_fields, configured_features);
    if fields.is_empty() {
        return false;
    }
    let all_primitive_fields = fields.iter().all(|f| matches!(f.ty, TypeRef::Primitive(_)));
    if all_primitive_fields {
        return true;
    }
    let has_vec_non_primitive = fields.iter().any(
        |f| matches!(&f.ty, TypeRef::Vec(inner) if !matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::Bytes)),
    );
    let has_non_serde_string_field = !ty.has_serde
        && fields
            .iter()
            .any(|f| matches!(f.ty, TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Char));
    let needs_default_construction = ty.has_serde
        || has_vec_non_primitive
        || has_non_serde_string_field
        || ty.has_stripped_cfg_fields
        || fields
            .iter()
            .any(|f| needs_json_bridge(&f.ty) || matches!(f.ty, TypeRef::Named(_)));
    !needs_default_construction || ty.has_default
}

pub(crate) fn emit_extern_block_for_type(
    ty: &TypeDef,
    exclude_fields: &HashSet<String>,
    type_paths: &HashMap<String, String>,
    no_serde_names: &HashSet<&str>,
    first_class_names: &HashSet<&str>,
    enum_names: &HashSet<String>,
    configured_features: &std::collections::HashSet<&str>,
) -> String {
    let parent_first_class = first_class_names.contains(ty.name.as_str());
    let mut block = String::new();
    block.push_str("    extern \"Rust\" {\n");
    block.push_str(&crate::backends::swift::template_env::render(
        "extern_type_decl.jinja",
        minijinja::context! {
            name => &ty.name,
        },
    ));

    let constructor_fields = constructor_fields(ty, exclude_fields, configured_features);
    let emit_constructor = has_constructor_extern(ty, exclude_fields, configured_features);

    if emit_constructor {
        let params: Vec<String> = constructor_fields
            .iter()
            .map(|f| {
                let bridge_ty = bridge_type(&f.ty);
                let bridge_ty = if f.optional && !needs_json_bridge(&f.ty) {
                    format!("Option<{bridge_ty}>")
                } else {
                    bridge_ty
                };
                let name = swift_ident(&f.name.to_snake_case());
                format!("{name}: {bridge_ty}")
            })
            .collect();
        block.push_str(&crate::backends::swift::template_env::render(
            "extern_init_attr.jinja",
            minijinja::context! {},
        ));
        block.push_str(&crate::backends::swift::template_env::render(
            "extern_fn_new.jinja",
            minijinja::context! {
                params => params.join(", "),
                return_type => &ty.name,
            },
        ));
    }

    for field in &ty.fields {
        if is_unbridgeable_getter(
            ty,
            field,
            exclude_fields,
            type_paths,
            no_serde_names,
            configured_features,
        ) {
            continue;
        }
        let enum_set: HashSet<&str> = enum_names.iter().map(|s| s.as_str()).collect();
        let bridge_ty =
            bridge_type_enum_and_serde_struct_aware(&field.ty, &enum_set, no_serde_names, parent_first_class);
        let bridge_ty = if field.optional && !needs_json_bridge(&field.ty) {
            if is_vec_of_enum(&field.ty, &enum_set)
                || (matches!(&field.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if !no_serde_names.contains(n.as_str())))
                    && !matches!(&field.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if enum_set.contains(n.as_str()))))
            {
                "String".to_string()
            } else {
                format!("Option<{bridge_ty}>")
            }
        } else {
            bridge_ty
        };
        let name = swift_ident(&field.name.to_snake_case());
        let swift_name = swift_ident(&field.name.to_lower_camel_case());
        if swift_name != name {
            block.push_str(&crate::backends::swift::template_env::render(
                "extern_swift_name_attr.jinja",
                minijinja::context! {
                    swift_name => &swift_name,
                },
            ));
        }
        block.push_str(&crate::backends::swift::template_env::render(
            "extern_fn_getter.jinja",
            minijinja::context! {
                name => &name,
                return_type => &bridge_ty,
            },
        ));
    }

    if type_needs_own_block_noop(ty) {
        let type_snake = ty.name.to_snake_case();
        let noop_fn_name = format!("{type_snake}_noop");
        block.push_str(&crate::backends::swift::template_env::render(
            "extern_fn_noop.jinja",
            minijinja::context! {
                fn_name => &noop_fn_name,
                type_name => &ty.name,
            },
        ));
    }

    block.push_str("    }\n\n");
    block
}

/// Whether an opaque type emitted in its OWN extern block needs a no-op shim method to
/// make swift-bridge synthesize its `$_free` destructor.
///
/// True for opaque types with no visible (non-excluded, non-sanitized, non-static) methods.
/// When true, the extern block declares `fn <type>_noop(client: &Type);` and the matching
/// `pub fn <type>_noop` definition MUST be emitted in `super` (see `deferred_noop::emit_shims`),
/// otherwise the bridge declaration fails to resolve with E0425.
pub(super) fn type_needs_own_block_noop(ty: &TypeDef) -> bool {
    let has_visible_methods = ty
        .methods
        .iter()
        .any(|m| !m.binding_excluded && !m.sanitized && !m.is_static);
    ty.is_opaque && !has_visible_methods
}

pub(crate) fn emit_extern_block_for_enum(en: &EnumDef) -> String {
    // NOTE: swift-bridge 0.1.59 also generates `extension T: Vectorizable`
    let mut block = String::new();
    block.push_str("    extern \"Rust\" {\n");
    block.push_str(&crate::backends::swift::template_env::render(
        "extern_enum_type.jinja",
        minijinja::context! {
            name => &en.name,
        },
    ));
    block.push_str("        fn to_string(&self) -> String;\n");
    block.push_str("    }\n\n");
    block
}

/// Emit a separate `extern "Rust"` block with free functions bridging each method of `ty`.
///
/// Each method `fn method_name(self, param: T) -> R` on type `TypeName` becomes a
/// free function `fn type_name_method_name(client: &TypeName, param: T) -> R` in the bridge.
/// The Swift-side name is camelCased: `typeNameMethodName`.
///
/// Skips sanitized methods (their signatures contain types that cannot be bridged).
pub(crate) fn emit_extern_block_for_type_methods(
    ty: &TypeDef,
    handle_returned_types: &std::collections::HashSet<String>,
    enum_names: &std::collections::HashSet<&str>,
) -> Option<String> {
    let bridgeable: Vec<_> = ty.methods.iter().filter(|m| !m.sanitized && !m.is_static).collect();
    if bridgeable.is_empty() {
        return None;
    }

    let mut block = String::new();
    block.push_str("    extern \"Rust\" {\n");

    for method in &bridgeable {
        let type_snake = ty.name.to_snake_case();
        let method_snake = method.name.to_snake_case();
        let fn_name = format!("{type_snake}_{method_snake}");
        let swift_name = swift_ident(&fn_name.to_lower_camel_case());

        let client_receiver = if matches!(method.receiver, Some(crate::core::ir::ReceiverKind::RefMut)) {
            format!("client: &mut {}", ty.name)
        } else {
            format!("client: &{}", ty.name)
        };
        let mut params: Vec<String> = vec![client_receiver];
        for p in &method.params {
            let bridge_ty = bridge_type_enum_aware_ref(&p.ty, enum_names);
            let bridge_ty = if p.optional && !needs_json_bridge(&p.ty) {
                format!("Option<{bridge_ty}>")
            } else {
                bridge_ty
            };
            let name = swift_ident(&p.name.to_snake_case());
            params.push(format!("{name}: {bridge_ty}"));
        }
        let params_str = params.join(", ");

        let return_ty = if method.error_type.is_some() {
            let ok_ty = bridge_type_with_handles(&method.return_type, handle_returned_types);
            if matches!(method.return_type, TypeRef::Unit) {
                "Result<(), String>".to_string()
            } else {
                format!("Result<{ok_ty}, String>")
            }
        } else {
            bridge_type_with_handles(&method.return_type, handle_returned_types)
        };

        if swift_name != fn_name {
            block.push_str(&crate::backends::swift::template_env::render(
                "extern_swift_name_attr.jinja",
                minijinja::context! {
                    swift_name => &swift_name,
                },
            ));
        }
        block.push_str(&crate::backends::swift::template_env::render(
            "extern_fn_decl.jinja",
            minijinja::context! {
                fn_name => &fn_name,
                params => &params_str,
                return_type => &return_ty,
            },
        ));
    }

    block.push_str("    }\n\n");
    Some(block)
}

/// Also emit a `createDefaultClient`-style constructor extern for types with methods,
/// so Swift can instantiate them via `RustBridge.create<TypeName>(apiKey:baseUrl:)`.
pub(crate) fn emit_extern_block_for_type_constructor(ty: &TypeDef) -> Option<String> {
    if ty.methods.iter().all(|m| m.sanitized) {
        return None;
    }
    let type_snake = ty.name.to_snake_case();
    let fn_name = format!("create_{type_snake}");
    let swift_name = swift_ident(&fn_name.to_lower_camel_case());

    let mut block = String::new();
    block.push_str("    extern \"Rust\" {\n");

    if swift_name != fn_name {
        block.push_str(&crate::backends::swift::template_env::render(
            "extern_swift_name_attr.jinja",
            minijinja::context! {
                swift_name => &swift_name,
            },
        ));
    }
    block.push_str(&crate::backends::swift::template_env::render(
        "extern_fn_decl.jinja",
        minijinja::context! {
            fn_name => &fn_name,
            params => "api_key: String, base_url: Option<String>",
            return_type => format!("Result<{}, String>", ty.name),
        },
    ));

    block.push_str("    }\n\n");
    Some(block)
}

/// Emit wrapper externs for instance methods on first-class (non-opaque) DTOs.
///
/// First-class DTOs in Swift are value types (Codable structs) that swift-bridge
/// cannot bridge directly as opaque types. Instead, we emit helper externs that
/// marshal self through JSON:
/// - Input: JSON string of self + method parameters
/// - Output: JSON string of return value
///
/// The Rust implementation (in `wrappers/methods.rs`) handles deserialization,
/// calling the actual method, and serializing the result back.
pub(crate) fn emit_extern_block_for_first_class_dto_methods(
    ty: &TypeDef,
    _handle_returned_types: &std::collections::HashSet<String>,
    enum_names: &std::collections::HashSet<&str>,
) -> Option<String> {
    if ty.is_opaque {
        return None;
    }

    let instance_methods: Vec<_> = ty.methods.iter().filter(|m| !m.sanitized && !m.is_static).collect();
    if instance_methods.is_empty() {
        return None;
    }

    let mut block = String::new();
    block.push_str("    extern \"Rust\" {\n");

    for method in instance_methods {
        let type_snake = ty.name.to_snake_case();
        let method_snake = method.name.to_snake_case();
        let fn_name = format!("{type_snake}_{method_snake}_from_json");
        let swift_name = swift_ident(&fn_name.to_lower_camel_case());

        let mut params: Vec<String> = vec!["json: String".to_string()];
        for p in &method.params {
            let bridge_ty = bridge_type_enum_aware_ref(&p.ty, enum_names);
            let bridge_ty = if p.optional && !needs_json_bridge(&p.ty) {
                format!("Option<{bridge_ty}>")
            } else {
                bridge_ty
            };
            let name = swift_ident(&p.name.to_snake_case());
            params.push(format!("{name}: {bridge_ty}"));
        }
        let params_str = params.join(", ");

        let return_ty = "Result<String, String>";

        if swift_name != fn_name {
            block.push_str(&crate::backends::swift::template_env::render(
                "extern_swift_name_attr.jinja",
                minijinja::context! {
                    swift_name => &swift_name,
                },
            ));
        }
        block.push_str(&crate::backends::swift::template_env::render(
            "extern_fn_decl.jinja",
            minijinja::context! {
                fn_name => &fn_name,
                params => &params_str,
                return_type => return_ty,
            },
        ));
    }

    block.push_str("    }\n\n");
    Some(block)
}

pub(crate) fn emit_extern_block_for_functions(
    functions: &[FunctionDef],
    handle_returned_types: &HashSet<String>,
    enum_names: &HashSet<String>,
    deferred_empty_handle_types: &HashSet<String>,
    capsule_types: &std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
) -> String {
    let mut block = String::new();
    block.push_str("    extern \"Rust\" {\n");

    for ty_name in deferred_empty_handle_types {
        block.push_str(&crate::backends::swift::template_env::render(
            "extern_type_decl.jinja",
            minijinja::context! {
                name => ty_name,
            },
        ));
        let type_snake = ty_name.to_snake_case();
        let noop_fn_name = format!("{type_snake}_noop");
        block.push_str(&crate::backends::swift::template_env::render(
            "extern_fn_noop.jinja",
            minijinja::context! {
                fn_name => &noop_fn_name,
                type_name => ty_name,
            },
        ));
    }
    if !deferred_empty_handle_types.is_empty() {
        block.push('\n');
    }

    for f in functions {
        let fn_name = swift_ident(&f.name.to_snake_case());
        let params: Vec<String> = f
            .params
            .iter()
            .map(|p| {
                let bridge_ty = bridge_type_enum_aware(&p.ty, enum_names);
                let bridge_ty = if p.optional {
                    format!("Option<{bridge_ty}>")
                } else {
                    bridge_ty
                };
                let name = swift_ident(&p.name.to_snake_case());
                format!("{name}: {bridge_ty}")
            })
            .collect();
        let params_str = params.join(", ");

        let is_capsule_return = matches!(&f.return_type, TypeRef::Named(n) if capsule_types.contains_key(n.as_str()));

        let return_ty = if is_capsule_return {
            "usize".to_string()
        } else if f.error_type.is_some() {
            let ok_ty = bridge_type_with_handles(&f.return_type, handle_returned_types);
            if matches!(f.return_type, TypeRef::Unit) {
                "Result<(), String>".to_string()
            } else {
                format!("Result<{ok_ty}, String>")
            }
        } else {
            bridge_type_with_handles(&f.return_type, handle_returned_types)
        };

        // swift-bridge 0.1.59 does not support the `#[swift_bridge(async)]`
        let swift_name = swift_ident(&f.name.to_lower_camel_case());
        if swift_name != fn_name {
            block.push_str(&crate::backends::swift::template_env::render(
                "extern_swift_name_attr.jinja",
                minijinja::context! {
                    swift_name => &swift_name,
                },
            ));
        }
        block.push_str(&crate::backends::swift::template_env::render(
            "extern_fn_decl.jinja",
            minijinja::context! {
                fn_name => &fn_name,
                params => &params_str,
                return_type => &return_ty,
            },
        ));
    }

    block.push_str("    }\n\n");
    block
}

/// Emit phantom extern "Rust" declarations for Vec<T> for all opaque types so that
/// swift-bridge-build emits the full suite of C ABI symbols for Vec operations.
///
/// Returns empty string if there are no types to register.
pub(crate) fn emit_extern_block_for_vec_accessors(visible_types: &[&TypeDef], visible_enums: &[&EnumDef]) -> String {
    if visible_types.is_empty() && visible_enums.is_empty() {
        return String::new();
    }

    let mut block = String::new();
    block.push_str("    extern \"Rust\" {\n");
    block.push_str("        // Phantom Vec<T> functions: swift-bridge-build must emit the full Vec support\n");
    block.push_str(
        "        // C ABI symbols (__swift_bridge__$Vec_T$new, drop, push, pop, get, get_mut, as_ptr, len)\n",
    );
    block.push_str("        // which the auto-generated Swift Vec<T> conformances reference.\n");
    block.push_str("        //\n");
    block.push_str("        // swift-bridge 0.1.59 only emits these when Vec<T> appears as a return type\n");
    block.push_str("        // in an extern block. Without these phantom functions, Swift linker fails when\n");
    block.push_str("        // trying to construct or manipulate Vec<T> of opaque types.\n");
    block.push_str("        //\n");
    block.push_str("        // These declarations are paired with phantom_impl functions below the bridge module.\n");

    for ty in visible_types {
        let type_snake = ty.name.to_snake_case();
        block.push_str(&crate::backends::swift::template_env::render(
            "rust_phantom_vec_decl.rs.jinja",
            minijinja::context! {
                type_snake => &type_snake,
                type_name => &ty.name,
            },
        ));
    }
    for en in visible_enums {
        let enum_snake = en.name.to_snake_case();
        block.push_str(&crate::backends::swift::template_env::render(
            "rust_phantom_vec_decl.rs.jinja",
            minijinja::context! {
                type_snake => &enum_snake,
                type_name => &en.name,
            },
        ));
    }

    block.push_str("    }\n\n");
    block
}

/// Emit the phantom Vec accessor implementations OUTSIDE the swift-bridge module.
///
/// These paired with the extern declarations emitted by `emit_extern_block_for_vec_accessors`.
/// swift-bridge-build sees the extern declarations and generates the C ABI symbols,
/// and these implementations satisfy the linker.
pub(crate) fn emit_phantom_vec_impl(visible_types: &[&TypeDef], visible_enums: &[&EnumDef]) -> String {
    if visible_types.is_empty() && visible_enums.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    for ty in visible_types {
        if let Some(cfg) = ty.cfg.as_deref() {
            out.push_str(&format!("#[cfg({cfg})]\n"));
        }
        let type_snake = ty.name.to_snake_case();
        out.push_str(&crate::backends::swift::template_env::render(
            "rust_phantom_vec_impl.rs.jinja",
            minijinja::context! {
                type_snake => &type_snake,
                type_name => &ty.name,
            },
        ));
    }
    for en in visible_enums {
        if let Some(cfg) = en.cfg.as_deref() {
            out.push_str(&format!("#[cfg({cfg})]\n"));
        }
        let enum_snake = en.name.to_snake_case();
        out.push_str(&crate::backends::swift::template_env::render(
            "rust_phantom_vec_impl.rs.jinja",
            minijinja::context! {
                type_snake => &enum_snake,
                type_name => &en.name,
            },
        ));
    }
    out
}

/// Emit a single `extern "Rust"` block declaring all streaming-adapter
/// `StreamHandle` opaque types and their `_start` + `next` bridge functions.
///
/// Each streaming adapter with an `owner_type` produces:
///
/// 1. An opaque `{Owner}{Adapter}StreamHandle` type declaration. swift-bridge
///    auto-generates a Swift `class` shadow with `deinit { *_free(ptr) }` so
///    Rust's `Drop` runs when the Swift handle goes out of scope — no manual
///    `_free` function is required.
/// 2. A free function `{owner_snake}_{adapter}_start(client, params...) ->
///    Result<{HandleName}, String>` that opens the stream. HTTP-level errors
///    (e.g. 401) surface here before any chunks arrive.
/// 3. A method `next(&mut self) -> Result<String, String>` on the handle.
///    Returns the JSON-encoded chunk or `""` on clean EOF; `Err(message)` on a
///    stream-level error.
///
/// Returns `None` when `adapters` contains no streaming entries.
pub(crate) fn emit_extern_block_for_streaming_adapters(
    adapters: &[AdapterConfig],
    declared_owner_types: &std::collections::HashSet<String>,
) -> Option<String> {
    use crate::core::config::AdapterPattern;

    let streaming: Vec<&AdapterConfig> = adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .filter(|a| a.owner_type.is_some())
        .collect();

    if streaming.is_empty() {
        return None;
    }

    let mut block = String::new();
    block.push_str("    extern \"Rust\" {\n");

    let owner_types: BTreeSet<&str> = streaming
        .iter()
        .filter_map(|adapter| adapter.owner_type.as_deref())
        .collect();

    for owner_type in owner_types {
        if !declared_owner_types.contains(owner_type) {
            block.push_str(&crate::backends::swift::template_env::render(
                "extern_type_decl.jinja",
                minijinja::context! {
                    name => owner_type,
                },
            ));
        }
    }

    for adapter in &streaming {
        let owner_type = adapter.owner_type.as_deref().unwrap_or("");
        let owner_pascal = owner_type.to_pascal_case();
        let adapter_pascal = adapter.name.to_pascal_case();
        let handle_name = format!("{owner_pascal}{adapter_pascal}StreamHandle");
        block.push_str(&crate::backends::swift::template_env::render(
            "extern_type_decl.jinja",
            minijinja::context! {
                name => &handle_name,
            },
        ));
    }
    block.push('\n');

    for adapter in &streaming {
        let owner_type = adapter.owner_type.as_deref().unwrap_or("");
        let owner_snake = owner_type.to_snake_case();
        let owner_pascal = owner_type.to_pascal_case();
        let adapter_pascal = adapter.name.to_pascal_case();
        let handle_name = format!("{owner_pascal}{adapter_pascal}StreamHandle");

        let fn_start = format!("{owner_snake}_{}_start", adapter.name);
        let swift_start = swift_ident(&fn_start.to_lower_camel_case());

        let mut start_params: Vec<String> = vec![format!("client: &{owner_type}")];
        for p in &adapter.params {
            let simple_ty = p.ty.rsplit("::").next().unwrap_or(&p.ty);
            let param_name = swift_ident(&p.name.to_snake_case());
            start_params.push(format!("{param_name}: &{simple_ty}"));
        }
        let start_params_str = start_params.join(", ");

        if swift_start != fn_start {
            block.push_str(&crate::backends::swift::template_env::render(
                "extern_swift_name_attr.jinja",
                minijinja::context! { swift_name => &swift_start },
            ));
        }
        block.push_str(&crate::backends::swift::template_env::render(
            "extern_fn_decl.jinja",
            minijinja::context! {
                fn_name => &fn_start,
                params => &start_params_str,
                return_type => format!("Result<{handle_name}, String>"),
            },
        ));

        block.push_str(&crate::backends::swift::template_env::render(
            "extern_fn_decl.jinja",
            minijinja::context! {
                fn_name => "next",
                params => format!("self: &mut {handle_name}"),
                return_type => "Result<String, String>",
            },
        ));
    }

    block.push_str("    }\n\n");
    Some(block)
}

#[cfg(test)]
mod cfg_filtered_fields_tests;

#[cfg(test)]
mod streaming_extern_tests;

#[cfg(test)]
mod capsule_function_tests;
