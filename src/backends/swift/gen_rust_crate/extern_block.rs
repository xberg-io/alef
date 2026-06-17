//! Emits `extern "Rust"` blocks for the `#[swift_bridge::bridge]` module.
//!
//! Covers type declarations, enum declarations, and top-level function declarations.
//! Trait bridge extern blocks live in `trait_bridge.rs`.

use crate::backends::swift::gen_rust_crate::type_bridge::{
    bridge_type, bridge_type_enum_aware, bridge_type_enum_aware_ref, bridge_type_with_handles, is_vec_of_enum,
    needs_json_bridge,
};
use crate::backends::swift::gen_rust_crate::wrappers::is_unbridgeable_getter;
use crate::core::config::AdapterConfig;
use crate::core::ir::{EnumDef, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::core::keywords::swift_ident;
use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};
use std::collections::{HashMap, HashSet};

/// Returns the subset of `ty.fields` that appear in the swift-bridge constructor extern
/// (filters out fields marked `binding_excluded` and any field key listed in `exclude_fields`).
///
/// Order matches `ty.fields` — the positional argument order swift-bridge uses to emit
/// the generated `convenience init(_ a, _ b, ...)`.
pub(crate) fn constructor_fields<'a>(ty: &'a TypeDef, exclude_fields: &HashSet<String>) -> Vec<&'a FieldDef> {
    ty.fields
        .iter()
        .filter(|f| {
            let field_key = format!("{}.{}", ty.name, f.name.to_snake_case());
            !f.binding_excluded && !exclude_fields.contains(&field_key)
        })
        .collect()
}

/// Returns `true` when `emit_extern_block_for_type` will emit a `#[swift_bridge(init)]`
/// constructor extern for `ty`. Mirrors the gating logic inside `emit_extern_block_for_type`
/// so callers (gen_bindings.rs `intoRust()` emission) can detect the presence of a
/// matching bulk constructor without re-running the whole emitter.
pub(crate) fn has_constructor_extern(ty: &TypeDef, exclude_fields: &HashSet<String>) -> bool {
    let fields = constructor_fields(ty, exclude_fields);
    if fields.is_empty() {
        return false;
    }
    // Primitive-only DTOs (every field is a bare primitive — `bool`/`u32`/`usize`/…) can
    // always be positionally constructed via swift-bridge regardless of whether the type
    // implements `Default`. Without this fast path, serde-enabled primitive-only types
    // (e.g. `Point { row: u32, column: u32 }`, `ByteRange { start: usize, end: usize }`)
    // would slip into the JSON-roundtrip path whose matching Rust-side `*_from_json`
    // shim may be filtered out, leaving Swift with a dangling
    // `RustBridge.pointFromJson` reference at link time.
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
    enum_names: &HashSet<String>,
) -> String {
    let mut block = String::new();
    block.push_str("    extern \"Rust\" {\n");
    block.push_str(&crate::backends::swift::template_env::render(
        "extern_type_decl.jinja",
        minijinja::context! {
            name => &ty.name,
        },
    ));

    // Constructor — use bridge_type to avoid nested generics that swift-bridge 0.1.59
    // cannot parse (Vec<Vec<T>>, HashMap<K,V>); those become String (JSON).
    // Excluded fields are omitted from the constructor params.
    //
    // When the wrapper would use mutable-default construction but the type does not
    // implement Default, wrappers.rs omits the impl entirely. We mirror that here by
    // also skipping the extern declaration — swift-bridge must not declare `fn new()`
    // without a corresponding Rust impl or linking will fail with E0599.
    //
    // The gating predicate lives in `has_constructor_extern` so that gen_bindings.rs
    // can match the same emission decision when choosing between a bulk constructor
    // and the JSON roundtrip in `intoRust()`.
    let constructor_fields = constructor_fields(ty, exclude_fields);
    let emit_constructor = has_constructor_extern(ty, exclude_fields);

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
                // Escape Swift keywords so the swift-bridge-generated init param
                // doesn't become invalid Swift (e.g. `_ extension: T` referencing
                // `extension` as expression in the body).
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

    // Getters — skip declaration entirely for fields whose impl cannot be safely
    // bridged. The matching `wrappers.rs` skips the impl for the same fields.
    //
    // Escape Swift keywords so e.g. `fn extension(&self)` becomes `fn extension_(&self)` —
    // matches the impl block in `wrappers.rs`.
    for field in &ty.fields {
        if is_unbridgeable_getter(ty, field, exclude_fields, type_paths, no_serde_names) {
            continue;
        }
        // Use enum-aware bridge type so that enum-typed Named fields are declared as
        // `String` in the extern block rather than as the opaque enum wrapper. This
        // prevents swift-bridge from generating Vec<EnumName> Vectorizable conformance
        // that references C-ABI symbols not generated by the Rust proc macro for enums.
        //
        // For optional Vec<Named(enum)> fields, force JSON-serialization (String) because
        // swift-bridge cannot handle Option<Vec<String>> as a plain getter return type.
        let bridge_ty = bridge_type_enum_aware(&field.ty, enum_names);
        let bridge_ty = if field.optional && !needs_json_bridge(&field.ty) {
            // Option<Vec<String>> is not natively supported by swift-bridge; collapse
            // to plain String (JSON) only when the Vec inner type is an enum.  For
            // Option<Vec<Named(struct)>> the caller returns the opaque wrapper vector
            // directly (swift-bridge supports Vec<OpaqueType>).
            if is_vec_of_enum(&field.ty, &enum_names.iter().map(|s| s.as_str()).collect()) {
                "String".to_string()
            } else {
                format!("Option<{bridge_ty}>")
            }
        } else {
            bridge_ty
        };
        let name = swift_ident(&field.name.to_snake_case());
        // Rust-side getter keeps the snake_case ident; Swift-side gets a camelCase
        // accessor via `swift_name` so consumer code reads `ref.deviceId()` rather
        // than `ref.device_id()`.
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

    // For opaque types with no methods, swift-bridge does not generate a destructor
    // (the `$_free` symbol). The C ABI handle becomes unleak-able, breaking linking.
    // A no-op method (returning unit) makes swift-bridge recognize the type as owned
    // and generate the destructor. Callers never invoke it — it exists only to signal
    // ownership to the swift-bridge codegen.
    if ty.is_opaque && ty.methods.is_empty() {
        block.push_str(&crate::backends::swift::template_env::render(
            "extern_fn_noop.jinja",
            minijinja::context! {
                name => &ty.name,
            },
        ));
    }

    block.push_str("    }\n\n");
    block
}

pub(crate) fn emit_extern_block_for_enum(en: &EnumDef) -> String {
    // Declare the enum as an opaque extern "Rust" type so that it can be used
    // as a parameter in constructor/function signatures (e.g.
    // `fn new(content: UserContent, ...)`).  Without this declaration,
    // swift-bridge rejects any function whose parameter list mentions the enum,
    // producing "Type must be declared with `type UserContent`".
    //
    // NOTE: swift-bridge 0.1.59 also generates `extension T: Vectorizable`
    // (with `__swift_bridge__$Vec_T$*` C-ABI symbols) for every opaque
    // `type T;` declaration.  These symbols compile fine on the Rust side
    // (`cargo build` succeeds) because `Vec<EnumName>` is valid Rust.  The
    // Vectorizable conformance only causes failures when the *Swift* side is
    // compiled (Xcode / full XCFramework build) and the enum is not a Swift
    // class type. A Rust-only cargo build does not surface those symbols.
    //
    // Getters that *return* enum-typed fields use `String` (the serde variant
    // name via `to_string()`) rather than the opaque handle — see
    // `emit_extern_block_for_type` and `wrappers::emit_getters`.  Only the
    // parameter path needs the type declared here.
    let mut block = String::new();
    block.push_str("    extern \"Rust\" {\n");
    block.push_str(&crate::backends::swift::template_env::render(
        "extern_enum_type.jinja",
        minijinja::context! {
            name => &en.name,
        },
    ));
    // Expose a `toString()` method so Swift can obtain the lowercase variant
    // name (e.g. "stop", "length") when the enum IS returned as an opaque
    // handle via a public function.  For struct-field getters the value is
    // already serialised to String before crossing the bridge, so this method
    // is not required there — but it is cheap to include and may be useful for
    // future Swift consumers.
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
    // Static / associated functions (e.g. `T::default()`) can't be bridged via
    // `client: &T` shims — see the matching filter in `wrappers::emit_type_method_shims`.
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

        // Build parameter list: first param is `client: &TypeName` (or `&mut` for
        // RefMut receivers), then method params.
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

        // Emit swift_name attribute when the generated Swift name differs from fn_name.
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
    // Constructor returns Result<TypeName, String> so errors propagate as Swift throws.
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

pub(crate) fn emit_extern_block_for_functions(
    functions: &[FunctionDef],
    handle_returned_types: &HashSet<String>,
    enum_names: &HashSet<String>,
) -> String {
    let mut block = String::new();
    block.push_str("    extern \"Rust\" {\n");

    for f in functions {
        // Escape Swift reserved keywords; swift-bridge emits the bridge fn name
        // verbatim into Swift, so `fn subscript(...)` would become invalid Swift.
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

        // Returns route through the handle-aware bridge mapper so that Named types
        // returned from public functions stay as
        // opaque handles instead of getting JSON-collapsed to `String`.
        let return_ty = if f.error_type.is_some() {
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
        // attribute (the build script's parser rejects it). To bridge async
        // functions, we declare them as plain `fn` in the extern block — the
        // wrapper will block on the future at the bridge boundary.
        //
        // `swift_name` rebinds the Swift-side function name to camelCase so the
        // host wrapper (`Sources/{Module}/SampleCrate.swift`) can call
        // `RustBridge.batchExtractBytes(...)` instead of the snake_case Rust
        // identifier — which is what the wrapper emits for idiomatic Swift.
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
pub(crate) fn emit_extern_block_for_streaming_adapters(adapters: &[AdapterConfig]) -> Option<String> {
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

    // 0. Owner-type forward declarations. swift-bridge requires every type
    //    referenced inside an `extern "Rust"` block to be declared in that
    //    same block. Each `_start(client: &{OwnerType}, …)` references the
    //    owner type, so we must declare it here even though it's also declared
    //    in the main extern block emitted by `emit_extern_block_for_type`.
    //
    //    The `#[swift_bridge(already_declared)]` attribute tells swift-bridge
    //    to treat this as a forward reference and NOT regenerate the
    //    `_free` / Drop trampolines for the type — those live with the
    //    primary declaration. Without the attribute, swift-bridge emits
    //    duplicate `__swift_bridge__{Type}__free` symbols and the binding
    //    crate fails to compile with E0428.
    let mut owner_types: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for adapter in &streaming {
        if let Some(owner) = adapter.owner_type.as_deref() {
            owner_types.insert(owner.to_string());
        }
    }
    for owner in &owner_types {
        block.push_str("        #[swift_bridge(already_declared)]\n");
        block.push_str(&format!("        type {owner};\n"));
    }

    // 1. Opaque handle type declarations. The methods that take `&mut self`
    //    must appear in the same extern block as the type declaration.
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

        // _start params: client receiver + adapter params (by reference because
        // swift-bridge wrapper newtypes are non-Copy). The Rust shim clones the
        // unwrapped inner value when it needs ownership for the async call.
        let mut start_params: Vec<String> = vec![format!("client: &{owner_type}")];
        for p in &adapter.params {
            // Adapter param types are stored as Rust path strings (e.g.
            // `sample_llm::ChatCompletionRequest`). Strip any module prefix —
            // the swift-bridge extern sees only the simple wrapper-newtype name.
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

        // `next` is a method on the handle. swift-bridge places it as a Swift
        // instance method on the generated class. The Rust impl `next(&mut self)`
        // lives in `wrappers.rs::emit_streaming_adapter_shims`.
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
mod streaming_extern_tests {
    //! Regression coverage for the `extern "Rust"` block emitted for streaming
    //! adapters. swift-bridge's `__swift_bridge__{Type}__free` symbol is generated
    //! once per `type {Type};` declaration. When the same opaque handle (e.g.
    //! `CrawlEngineHandle`) is declared both in the main bridge block AND in the
    //! streaming block — without `#[swift_bridge(already_declared)]` on the second
    //! occurrence — the binding crate fails to compile with E0428:
    //!
    //! ```text
    //! error[E0428]: the name `__swift_bridge__CrawlEngineHandle__free`
    //!               is defined multiple times
    //! ```
    //!
    //! The streaming emitter MUST reference the owner type inside its own block
    //! (swift-bridge resolves `client: &Owner` against the enclosing extern block
    //! only), so the right fix is to forward-declare the owner with the
    //! `already_declared` attribute that suppresses the duplicate free trampoline.

    use super::emit_extern_block_for_streaming_adapters;
    use crate::core::config::{AdapterConfig, AdapterParam, AdapterPattern};

    fn streaming_adapter_with_owner(name: &str, owner: &str) -> AdapterConfig {
        AdapterConfig {
            name: name.to_string(),
            pattern: AdapterPattern::Streaming,
            core_path: format!("sample_crate::{name}"),
            params: vec![AdapterParam {
                name: "req".to_string(),
                ty: "sample_crate::StreamRequest".to_string(),
                optional: false,
            }],
            returns: None,
            error_type: Some("String".to_string()),
            owner_type: Some(owner.to_string()),
            item_type: Some("StreamItem".to_string()),
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: Some("sample_crate::StreamRequest".to_string()),
            skip_languages: vec![],
        }
    }

    #[test]
    fn streaming_extern_block_forward_declares_owner_with_already_declared() {
        let adapters = vec![streaming_adapter_with_owner("crawl_stream", "CrawlEngineHandle")];
        let block =
            emit_extern_block_for_streaming_adapters(&adapters).expect("streaming adapter should produce a block");

        assert!(
            block.contains("#[swift_bridge(already_declared)]"),
            "streaming extern block must mark owner forward-decl with already_declared \
             to avoid duplicate __swift_bridge__{{Owner}}__free symbols:\n{block}"
        );

        let attr_idx = block
            .find("#[swift_bridge(already_declared)]")
            .expect("already_declared attribute must be present");
        let owner_decl = "type CrawlEngineHandle;";
        let owner_idx = block
            .find(owner_decl)
            .expect("owner forward declaration must be present");
        assert!(
            attr_idx < owner_idx,
            "already_declared attribute must immediately precede the owner `type` declaration:\n{block}"
        );
    }

    #[test]
    fn streaming_extern_block_emits_owner_only_once_per_unique_owner() {
        // Two streaming adapters that share an owner — the forward declaration
        // must be emitted exactly once, not duplicated across adapters.
        let adapters = vec![
            streaming_adapter_with_owner("crawl_stream", "CrawlEngineHandle"),
            streaming_adapter_with_owner("batch_crawl_stream", "CrawlEngineHandle"),
        ];
        let block =
            emit_extern_block_for_streaming_adapters(&adapters).expect("streaming adapters should produce a block");

        let occurrences = block.matches("type CrawlEngineHandle;").count();
        assert_eq!(
            occurrences, 1,
            "owner handle `CrawlEngineHandle` must appear exactly once in the streaming \
             extern block (regardless of how many adapters share the owner); \
             found {occurrences} occurrences in:\n{block}"
        );
    }
}
