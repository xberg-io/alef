//! Java (Panama FFM) trait bridge code generation for plugin systems.
//!
//! Emits four files per `[[trait_bridges]]` entry (Path A - hand-authored interface approach):
//!
//! 1. `I{TraitName}.java` — managed interface users implement
//! 2. `{TraitName}Bridge.java` — Panama upcall-stub bridge class with nested
//!    static fields for the live-bridge registry plus
//!    `register{TraitName}` / `unregister{TraitName}` static helpers
//! 3. `{TraitName}Adapter.java` — wrapper implementing the interface, delegating to user impl
//!
//! The Adapter conforms to the hand-authored sealed interface, wrapping the user's
//! implementation and passing it through to native side during registration.
//!
//! All complex parameter and return marshalling goes through Jackson JSON, matching
//! how the FFI vtable receives values from native callers.

use crate::core::ir::{MethodDef, PrimitiveType, TypeDef, TypeRef};
use heck::{ToPascalCase, ToSnakeCase};
use minijinja::Value;
use std::collections::HashSet;

use crate::backends::java::template_env;
use crate::backends::java::type_map::{java_ffi_type, java_type};

/// Map a `TypeRef` to its Java representation, substituting `String` for any
/// `Named` type that is not in the set of visible (i.e. generated) types,
/// OR that is in the excluded types set.
///
/// This prevents internal/excluded types like `InternalDocument` from leaking
/// into public trait interface signatures and Panama upcall bridge methods.
/// Excluded types are JSON-serialised over the FFI boundary as `String`.
fn java_type_visible(ty: &TypeRef, visible_type_names: &HashSet<&str>, excluded_types: &HashSet<String>) -> String {
    match ty {
        TypeRef::Named(name) => {
            // Substitute with String if: (1) not in visible set, OR (2) explicitly excluded
            if !visible_type_names.contains(name.as_str()) || excluded_types.contains(name) {
                "String".to_string()
            } else {
                java_type(ty).into_owned()
            }
        }
        TypeRef::Optional(inner) => java_type_visible(inner, visible_type_names, excluded_types),
        TypeRef::Vec(inner) => format!(
            "List<{}>",
            java_type_visible_boxed(inner, visible_type_names, excluded_types)
        ),
        TypeRef::Map(k, v) => format!(
            "Map<{}, {}>",
            java_type_visible_boxed(k, visible_type_names, excluded_types),
            java_type_visible_boxed(v, visible_type_names, excluded_types)
        ),
        _ => java_type(ty).into_owned(),
    }
}

/// Boxed variant of `java_type_visible` for use inside `List<...>` / `Map<...>`
/// generics. Substitutes excluded Named types with `String` (already boxed).
fn java_type_visible_boxed(
    ty: &TypeRef,
    visible_type_names: &HashSet<&str>,
    excluded_types: &HashSet<String>,
) -> String {
    match ty {
        TypeRef::Named(name) => {
            // Substitute with String if: (1) not in visible set, OR (2) explicitly excluded
            if !visible_type_names.contains(name.as_str()) || excluded_types.contains(name) {
                "String".to_string()
            } else {
                crate::backends::java::type_map::java_boxed_type(ty).into_owned()
            }
        }
        TypeRef::Optional(inner) => java_type_visible_boxed(inner, visible_type_names, excluded_types),
        TypeRef::Vec(inner) => format!(
            "List<{}>",
            java_type_visible_boxed(inner, visible_type_names, excluded_types)
        ),
        TypeRef::Map(k, v) => format!(
            "Map<{}, {}>",
            java_type_visible_boxed(k, visible_type_names, excluded_types),
            java_type_visible_boxed(v, visible_type_names, excluded_types)
        ),
        _ => crate::backends::java::type_map::java_boxed_type(ty).into_owned(),
    }
}

/// The two generated Java files for one trait bridge.
pub struct BridgeFiles {
    pub interface_content: String,
    pub bridge_content: String,
}

/// Generate both the managed interface file and the bridge class file for one trait.
///
/// `unregister_fn` is the configured name of the host-crate unregister function (e.g.
/// `"unregister_text_backend"`). When `Some`, a `public static void unregister{Trait}(String
/// name)` helper is emitted in the bridge class. When `None`, the method is omitted.
///
/// `clear_fn` is the configured name of the host-crate clear-all function (e.g.
/// `"clear_text_backends"`). When `Some`, a `public static void clearAll{Trait}()` helper is
/// emitted. When `None`, the method is omitted.
#[allow(clippy::too_many_arguments)]
pub fn gen_trait_bridge_files(
    trait_def: &TypeDef,
    prefix: &str,
    package: &str,
    has_super_trait: bool,
    unregister_fn: Option<&str>,
    clear_fn: Option<&str>,
    visible_type_names: &HashSet<&str>,
    excluded_types: &HashSet<String>,
    ffi_skip_methods: &[String],
) -> BridgeFiles {
    BridgeFiles {
        interface_content: gen_interface_file(
            trait_def,
            package,
            has_super_trait,
            visible_type_names,
            excluded_types,
            ffi_skip_methods,
        ),
        bridge_content: gen_bridge_file(
            trait_def,
            prefix,
            package,
            has_super_trait,
            unregister_fn,
            clear_fn,
            visible_type_names,
            excluded_types,
            ffi_skip_methods,
        ),
    }
}

/// Generate a wrapper class file that implements the hand-authored sealed interface.
///
/// This file contains a Bridge wrapper class that:
/// 1. Implements the hand-authored interface (e.g., IDocumentExtractor)
/// 2. Takes the user impl in constructor
/// 3. Delegates all method calls to the user impl
///
/// The Bridge class is what the registration function passes to native side.
///
/// Note: The Adapter does not need to import List or Map since it only delegates to impl.
/// The interface itself declares the required imports for those types. This prevents
/// checkstyle unused-import warnings when methods don't actually use generic containers.
pub fn gen_trait_adapter_bridge_file(
    trait_def: &TypeDef,
    package: &str,
    visible_type_names: &HashSet<&str>,
    excluded_types: &HashSet<String>,
    ffi_skip_methods: &[String],
) -> String {
    let trait_pascal = trait_def.name.to_pascal_case();
    let bridge_class = format!("{trait_pascal}Adapter");
    let interface_name = format!("I{trait_pascal}");

    // Collect non-skipped methods
    let skipped: HashSet<&str> = ffi_skip_methods.iter().map(|s| s.as_str()).collect();
    let bridge_methods: Vec<&MethodDef> = trait_def
        .methods
        .iter()
        .filter(|m| m.trait_source.is_none() && !skipped.contains(m.name.as_str()))
        .collect();

    // Build delegating method bodies
    let mut methods_code = String::new();

    // Trait bridge interfaces always have lifecycle methods (name, version, initialize, shutdown)
    // These are defined as requirements of the sealed interface.
    methods_code.push_str("    @Override\n");
    methods_code.push_str("    public String name() {\n");
    methods_code.push_str("        return impl.name();\n");
    methods_code.push_str("    }\n\n");

    methods_code.push_str("    @Override\n");
    methods_code.push_str("    public String version() {\n");
    methods_code.push_str("        return impl.version();\n");
    methods_code.push_str("    }\n\n");

    methods_code.push_str("    @Override\n");
    methods_code.push_str("    public void initialize() throws Exception {\n");
    methods_code.push_str("        impl.initialize();\n");
    methods_code.push_str("    }\n\n");

    methods_code.push_str("    @Override\n");
    methods_code.push_str("    public void shutdown() throws Exception {\n");
    methods_code.push_str("        impl.shutdown();\n");
    methods_code.push_str("    }\n\n");

    // Add trait-specific methods
    for method in &bridge_methods {
        // Use the method name as-is from the interface (snake_case), not camelCase
        let method_name = &method.name;
        let return_type_str = java_type_visible(&method.return_type, visible_type_names, excluded_types);

        let params_str = method
            .params
            .iter()
            .map(|p| {
                format!(
                    "{} {}",
                    java_type_visible(&p.ty, visible_type_names, excluded_types),
                    java_param_name(&p.name)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");

        let call_args: Vec<String> = method.params.iter().map(|p| java_param_name(&p.name)).collect();
        let call_args_str = call_args.join(", ");

        methods_code.push_str("    @Override\n");
        methods_code.push_str(&format!(
            "    public {} {}({}) throws Exception {{\n",
            return_type_str, method_name, params_str
        ));

        if matches!(method.return_type, TypeRef::Unit) {
            methods_code.push_str(&format!("        impl.{}({});\n", method_name, call_args_str));
        } else {
            methods_code.push_str(&format!("        return impl.{}({});\n", method_name, call_args_str));
        }

        methods_code.push_str("    }\n\n");
    }

    // Collect which imports are actually needed based on method return types and parameters.
    // The Adapter signature must match the interface, so we need imports for List/Map when used.
    let mut needs_list = false;
    let mut needs_map = false;

    fn check_type_needs_list_or_map(ty: &TypeRef) -> (bool, bool) {
        match ty {
            TypeRef::Vec(_) => (true, false),
            TypeRef::Map(_, _) => (false, true),
            TypeRef::Optional(inner) => check_type_needs_list_or_map(inner),
            _ => (false, false),
        }
    }

    for method in &bridge_methods {
        let (ret_list, ret_map) = check_type_needs_list_or_map(&method.return_type);
        needs_list = needs_list || ret_list;
        needs_map = needs_map || ret_map;

        for param in &method.params {
            let (param_list, param_map) = check_type_needs_list_or_map(&param.ty);
            needs_list = needs_list || param_list;
            needs_map = needs_map || param_map;
        }
    }

    let mut imports: Vec<&str> = vec![];
    if needs_list {
        imports.push("java.util.List");
    }
    if needs_map {
        imports.push("java.util.Map");
    }

    let ctx = minijinja::context! {
        package => package,
        imports => imports,
        bridge_class => &bridge_class,
        interface_name => &interface_name,
        methods_code => &methods_code,
    };

    template_env::render("trait_adapter_bridge.jinja", ctx)
}

/// Generate the Java `unregister{Trait}` static helper body.
///
/// Returns an empty string when `unregister_fn` is `None` (opt-in; not all bridges need it).
/// The emitted method calls `NativeLib.{PREFIX}_UNREGISTER_{TRAIT}` via Panama FFM, mirrors
/// the local registry removal, and closes the bridge's arena.
pub fn gen_unregistration_fn(
    trait_pascal: &str,
    trait_snake_upper: &str,
    prefix_upper: &str,
    bridge_class: &str,
    registry_field: &str,
    unregister_fn: Option<&str>,
) -> String {
    if unregister_fn.is_none() {
        return String::new();
    }
    template_env::render(
        "bridge_unregister_method.jinja",
        minijinja::context! {
            trait_pascal => trait_pascal,
            trait_snake_upper => trait_snake_upper,
            prefix_upper => prefix_upper,
            bridge_class => bridge_class,
            registry_field => registry_field,
        },
    )
}

/// Generate the Java `clear{Plural}` static helper body.
///
/// Returns an empty string when `clear_fn` is `None` (opt-in). The emitted method calls
/// `NativeLib.{PREFIX}_CLEAR_{TRAIT}` via Panama FFM (no arguments other than the out-error
/// pointer), then closes and removes every live bridge from the local registry.
///
/// The method name is derived from `clear_fn` (e.g., `clear_renderers` → `clearRenderers`)
/// to match the Rust function name convention where the plural is explicit.
pub fn gen_clear_fn(
    trait_pascal: &str,
    trait_snake_upper: &str,
    prefix_upper: &str,
    bridge_class: &str,
    registry_field: &str,
    clear_fn: Option<&str>,
) -> String {
    if clear_fn.is_none() {
        return String::new();
    }
    // Convert clear_fn from snake_case (e.g., "clear_renderers") to method name
    // (e.g., "clearRenderers") by removing "clear_" prefix and PascalCasing the rest.
    let method_name = if let Some(fn_name) = clear_fn {
        let without_prefix = fn_name.strip_prefix("clear_").unwrap_or(fn_name);
        let words: Vec<&str> = without_prefix.split('_').collect();
        let mut camel = String::from("clear");
        for word in words {
            if !word.is_empty() {
                let mut chars = word.chars();
                if let Some(first) = chars.next() {
                    camel.push(first.to_uppercase().next().unwrap());
                    camel.push_str(chars.as_str());
                }
            }
        }
        camel
    } else {
        format!("clearAll{trait_pascal}")
    };

    template_env::render(
        "bridge_clear_method.jinja",
        minijinja::context! {
            trait_pascal => trait_pascal,
            trait_snake_upper => trait_snake_upper,
            prefix_upper => prefix_upper,
            bridge_class => bridge_class,
            registry_field => registry_field,
            method_name => &method_name,
        },
    )
}

/// Generate the standalone managed `I{Trait}` interface compilation unit.
fn gen_interface_file(
    trait_def: &TypeDef,
    package: &str,
    has_super_trait: bool,
    visible_type_names: &HashSet<&str>,
    excluded_types: &HashSet<String>,
    ffi_skip_methods: &[String],
) -> String {
    let trait_pascal = trait_def.name.to_pascal_case();

    // Methods listed in `ffi_skip_methods` cannot cross the C FFI vtable (e.g.
    // `as_sync_extractor` returns `Option<&dyn SyncExtractor>` which has no FFI
    // representation). They are skipped in the bridge AND in the interface so
    // generated test stubs do not have to implement them.
    let skipped: HashSet<&str> = ffi_skip_methods.iter().map(|s| s.as_str()).collect();

    // Build method list for template. Each method's `signature` is the rendered Java
    // signature (return type + params), which is what the imports scan inspects.
    let methods: Vec<Value> = trait_def
        .methods
        .iter()
        .filter(|m| !skipped.contains(m.name.as_str()))
        .map(|m| {
            let return_type_str = java_type_visible(&m.return_type, visible_type_names, excluded_types);
            let params_str = m
                .params
                .iter()
                .map(|p| {
                    format!(
                        "{} {}",
                        java_type_visible(&p.ty, visible_type_names, excluded_types),
                        java_param_name(&p.name)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            minijinja::context! {
                javadoc => format!("/** {}. */", m.name),
                signature => format!("{} {}({}) throws Exception", return_type_str, m.name, params_str),
            }
        })
        .collect();

    // Render the method signatures (interface body) first, then scan for the
    // generic-container types that drive conditional imports. This mirrors the
    // render-body-then-scan-imports pattern in `ffi_class.rs::gen_ffi_class`.
    // Scanning the rendered signature strings prevents false positives the old
    // `format!("{ret}({params})")` synthesis produced (it allowed `List<...>` to
    // appear in concatenated bytes when no method actually used it).
    let methods_body: String = methods
        .iter()
        .filter_map(|m| m.get_attr("signature").ok())
        .map(|sig| sig.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let needs_list = methods_body.contains("List<");
    let needs_map = methods_body.contains("Map<");

    let ctx = minijinja::context! {
        package => package,
        needs_list => needs_list,
        needs_map => needs_map,
        trait_pascal => &trait_pascal,
        has_super_trait => has_super_trait,
        methods => methods,
    };

    template_env::render("trait_interface.jinja", ctx)
}

/// Generate the bridge class compilation unit with upcall stubs, registry, and
/// register/unregister/clear helpers all nested inside the public top-level class.
#[allow(clippy::too_many_arguments)]
fn gen_bridge_file(
    trait_def: &TypeDef,
    prefix: &str,
    package: &str,
    has_super_trait: bool,
    unregister_fn: Option<&str>,
    clear_fn: Option<&str>,
    visible_type_names: &HashSet<&str>,
    excluded_types: &HashSet<String>,
    ffi_skip_methods: &[String],
) -> String {
    let trait_pascal = trait_def.name.to_pascal_case();
    let trait_snake = trait_def.name.to_snake_case();
    let prefix_upper = prefix.to_uppercase();
    let registry_field = format!("{}_BRIDGES", trait_snake.to_uppercase());
    let bridge_class = format!("{trait_pascal}Bridge");

    // Methods listed in `ffi_skip_methods` cannot be expressed on the C ABI
    // (e.g., trait-object references), so they are absent from the interface
    // and the bridge must not emit upcall stubs or handlers for them.
    let skipped: HashSet<&str> = ffi_skip_methods.iter().map(|s| s.as_str()).collect();
    let bridge_methods: Vec<&MethodDef> = trait_def
        .methods
        .iter()
        .filter(|m| m.trait_source.is_none() && !skipped.contains(m.name.as_str()))
        .collect();

    // Build lifecycle methods
    let lifecycle_methods: Vec<Value> = if has_super_trait {
        vec![
            minijinja::context! {
                signature => "int handleName(MemorySegment userData, MemorySegment outName, MemorySegment outError)",
                body => "outName.set(ValueLayout.ADDRESS, 0, arena.allocateFrom(impl.name()))",
                error_return => "1",
                void_call => true,
                success_return => "0",
            },
            minijinja::context! {
                signature => "int handleVersion(MemorySegment userData, MemorySegment outVersion, MemorySegment outError)",
                body => "outVersion.set(ValueLayout.ADDRESS, 0, arena.allocateFrom(impl.version()))",
                error_return => "1",
                void_call => true,
                success_return => "0",
            },
            minijinja::context! {
                signature => "int handleInitialize(MemorySegment userData, MemorySegment outError)",
                body => "impl.initialize()",
                error_return => "1",
                void_call => true,
                success_return => "0",
            },
            minijinja::context! {
                signature => "int handleShutdown(MemorySegment userData, MemorySegment outError)",
                body => "impl.shutdown()",
                error_return => "1",
                void_call => true,
                success_return => "0",
            },
        ]
    } else {
        vec![]
    };

    // Build stub allocations for lifecycle methods
    let mut stubs: Vec<Value> = vec![];
    if has_super_trait {
        // Lifecycle stubs return i32 status codes, so the FunctionDescriptor return layout
        // must be JAVA_INT (matching the `int` upcall return); JAVA_LONG mis-sizes the value.
        let lifecycle_stubs = vec![
            (
                "Name",
                "int.class",
                "ValueLayout.JAVA_INT",
                ", MemorySegment.class, MemorySegment.class",
            ),
            (
                "Version",
                "int.class",
                "ValueLayout.JAVA_INT",
                ", MemorySegment.class, MemorySegment.class",
            ),
            (
                "Initialize",
                "int.class",
                "ValueLayout.JAVA_INT",
                ", MemorySegment.class",
            ),
            (
                "Shutdown",
                "int.class",
                "ValueLayout.JAVA_INT",
                ", MemorySegment.class",
            ),
        ];
        for (pascal, return_type, descriptor_return, extra_param) in lifecycle_stubs {
            let handle = format!("handle{pascal}");
            let var_name = format!("stub{pascal}");
            let extra_descriptor = if pascal == "Name" || pascal == "Version" {
                ", ValueLayout.ADDRESS, ValueLayout.ADDRESS"
            } else if pascal == "Initialize" || pascal == "Shutdown" {
                ", ValueLayout.ADDRESS"
            } else {
                ""
            };
            stubs.push(minijinja::context! {
                var_name => &var_name,
                pascal_name => pascal,
                handle_name => &handle,
                return_type => return_type,
                method_type_params => format!("MemorySegment.class{extra_param}"),
                descriptor_return => descriptor_return,
                descriptor_params => format!("ValueLayout.ADDRESS{extra_descriptor}"),
                returns_void => false,
            });
        }
    }

    // Build stub allocations for trait methods
    for method in &bridge_methods {
        let method_pascal = method.name.to_pascal_case();
        let handle_name = format!("handle{method_pascal}");
        let stub_name = format!("stub{method_pascal}");

        let mut method_type_params = vec!["MemorySegment.class".to_string()];
        for param in &method.params {
            let class_literal = match &param.ty {
                TypeRef::Primitive(PrimitiveType::Bool) => "int.class".to_string(),
                TypeRef::Primitive(p) => format!("{}.class", java_type(&TypeRef::Primitive(p.clone()))),
                _ => "MemorySegment.class".to_string(),
            };
            method_type_params.push(class_literal);
            // Bytes params carry a companion long length (mirrors vtable.rs pattern).
            if matches!(param.ty, TypeRef::Bytes) {
                method_type_params.push("long.class".to_string());
            }
        }
        if !matches!(method.return_type, TypeRef::Unit) {
            method_type_params.push("MemorySegment.class".to_string());
        }
        method_type_params.push("MemorySegment.class".to_string());

        let mut func_desc_params = vec!["ValueLayout.ADDRESS".to_string()];
        for param in &method.params {
            let ffi_layout = match &param.ty {
                TypeRef::Primitive(p) => java_ffi_type(p).to_string(),
                _ => "ValueLayout.ADDRESS".to_string(),
            };
            func_desc_params.push(ffi_layout);
            // Bytes params carry a companion ValueLayout.JAVA_LONG length (mirrors vtable.rs).
            if matches!(param.ty, TypeRef::Bytes) {
                func_desc_params.push("ValueLayout.JAVA_LONG".to_string());
            }
        }
        if !matches!(method.return_type, TypeRef::Unit) {
            func_desc_params.push("ValueLayout.ADDRESS".to_string());
        }
        func_desc_params.push("ValueLayout.ADDRESS".to_string());

        stubs.push(minijinja::context! {
            var_name => &stub_name,
            pascal_name => &method_pascal,
            handle_name => &handle_name,
            return_type => "int.class",
            method_type_params => method_type_params.join(", "),
            // Trait method stubs return i32 status codes, so the FunctionDescriptor return
            // layout must be JAVA_INT (matching the `int` upcall return). The companion
            // bytes-length param above stays JAVA_LONG.
            descriptor_return => "ValueLayout.JAVA_INT",
            descriptor_params => func_desc_params.join(", "),
            returns_void => false,
        });
    }

    // Build trait method handlers
    let methods: Vec<Value> = bridge_methods
        .iter()
        .map(|method| {
            let handle = format!("handle{}", method.name.to_pascal_case());
            let mut sig_params = vec!["MemorySegment userData".to_string()];
            for param in &method.params {
                let local = java_param_name(&param.name);
                match &param.ty {
                    TypeRef::Primitive(PrimitiveType::Bool) => {
                        sig_params.push(format!("int {local}_raw"));
                    }
                    TypeRef::Primitive(p) => {
                        sig_params.push(format!("{} {local}", java_type(&TypeRef::Primitive(p.clone()))));
                    }
                    _ => {
                        sig_params.push(format!("MemorySegment {local}_in"));
                    }
                }
                // Bytes params carry a companion long length so the handler can
                // bound the MemorySegment read (fixes issue #114).
                if matches!(param.ty, TypeRef::Bytes) {
                    sig_params.push(format!("long {local}Len"));
                }
            }
            if !matches!(method.return_type, TypeRef::Unit) {
                sig_params.push("MemorySegment outResult".to_string());
            }
            sig_params.push("MemorySegment outError".to_string());

            // Build unmarshal params
            let mut unmarshal_params: Vec<String> = vec![];
            for param in &method.params {
                let local = java_param_name(&param.name);
                if matches!(param.ty, TypeRef::Primitive(PrimitiveType::Bool)) {
                    unmarshal_params.push(format!("boolean {local} = {local}_raw != 0;"));
                } else if !matches!(param.ty, TypeRef::Primitive(_)) {
                    let segment = format!("{local}_in");
                    // Named types not in visible_type_names (e.g. InternalDocument) OR in excluded_types
                    // are JSON-bridged as opaque Strings — no companion Java class is generated
                    // for them, so unmarshal as a plain String rather than deserialising
                    // into a missing class.
                    if let TypeRef::Named(name) = &param.ty {
                        if !visible_type_names.contains(name.as_str()) || excluded_types.contains(name) {
                            // Unmarshal as String (TypeRef::String), not as the excluded Named type
                            unmarshal_params.push(format_unmarshal_param(&local, &segment, &TypeRef::String, None));
                            continue;
                        }
                    }
                    // Pass the len variable name for Bytes so the unmarshal can use it.
                    let bytes_len = if matches!(param.ty, TypeRef::Bytes) {
                        Some(format!("{local}Len"))
                    } else {
                        None
                    };
                    unmarshal_params.push(format_unmarshal_param(
                        &local,
                        &segment,
                        &param.ty,
                        bytes_len.as_deref(),
                    ));
                }
            }

            let java_args: Vec<String> = method.params.iter().map(|p| java_param_name(&p.name)).collect();
            let has_return = !matches!(method.return_type, TypeRef::Unit);
            let return_type_str = if has_return {
                java_type_visible(&method.return_type, visible_type_names, excluded_types)
            } else {
                String::new()
            };
            let raw_result = match &method.return_type {
                TypeRef::String | TypeRef::Char | TypeRef::Path => true,
                TypeRef::Named(name) => !visible_type_names.contains(name.as_str()) || excluded_types.contains(name),
                _ => false,
            };

            minijinja::context! {
                name => &method.name,
                handle_name => &handle,
                sig_params => sig_params.join(", "),
                unmarshal_params => unmarshal_params,
                return_type => &return_type_str,
                call_args => java_args.join(", "),
                has_return => has_return,
                raw_result => raw_result,
            }
        })
        .collect();

    let num_methods = bridge_methods.len();
    let num_super_slots = if has_super_trait { 4usize } else { 0usize };
    stubs.push(minijinja::context! {
        var_name => "stubFreeString",
        pascal_name => "FreeString",
        handle_name => "freeString",
        return_type => "void.class",
        method_type_params => "MemorySegment.class",
        descriptor_return => "ValueLayout.ADDRESS",
        descriptor_params => "ValueLayout.ADDRESS",
        returns_void => true,
    });

    stubs.push(minijinja::context! {
        var_name => "stubFreeUserData",
        pascal_name => "FreeUserData",
        handle_name => "freeUserData",
        return_type => "void.class",
        method_type_params => "MemorySegment.class",
        descriptor_return => "ValueLayout.ADDRESS",
        descriptor_params => "ValueLayout.ADDRESS",
        returns_void => true,
    });

    let num_vtable_fields = num_super_slots + num_methods + 2;
    let register_takes_name = has_super_trait;

    let trait_snake_upper = trait_snake.to_uppercase();
    let unregister_method = gen_unregistration_fn(
        &trait_pascal,
        &trait_snake_upper,
        &prefix_upper,
        &bridge_class,
        &registry_field,
        unregister_fn,
    );
    let clear_method = gen_clear_fn(
        &trait_pascal,
        &trait_snake_upper,
        &prefix_upper,
        &bridge_class,
        &registry_field,
        clear_fn,
    );

    // Render-body-then-scan-imports: the conditional imports (`List`, `Map`) are
    // driven by what actually appears in the trait-method handler bodies — the
    // return-type declaration (`{return_type} result = impl.method(...)`) and the
    // unmarshal expressions (`List<...> {local} = JSON.readValue(...)`). Synthesising
    // a separate signature string was lossy and produced false positives, so we
    // scan the already-rendered fragments instead. This mirrors `ffi_class.rs`.
    let methods_body: String = methods
        .iter()
        .flat_map(|m| {
            let ret = m
                .get_attr("return_type")
                .ok()
                .map(|v| v.to_string())
                .unwrap_or_default();
            let unmarshal = m
                .get_attr("unmarshal_params")
                .ok()
                .map(|v| v.to_string())
                .unwrap_or_default();
            [ret, unmarshal]
        })
        .collect::<Vec<_>>()
        .join("\n");

    let needs_list = methods_body.contains("List<");
    let needs_map = methods_body.contains("Map<");

    let ctx = minijinja::context! {
        package => package,
        needs_list => needs_list,
        needs_map => needs_map,
        trait_pascal => &trait_pascal,
        trait_snake_upper => &trait_snake_upper,
        bridge_class => &bridge_class,
        registry_field => &registry_field,
        prefix_upper => &prefix_upper,
        num_methods => num_methods,
        num_super_slots => num_super_slots,
        num_vtable_fields => num_vtable_fields,
        vtable_layout_fields_list => vec!["ValueLayout.ADDRESS"; num_vtable_fields],
        lifecycle_methods => lifecycle_methods,
        stubs => stubs,
        methods => methods,
        register_takes_name => register_takes_name,
        name_expr => if has_super_trait { "impl.name()" } else { "name" },
        unregister_method => &unregister_method,
        clear_method => &clear_method,
    };

    template_env::render("trait_bridge.jinja", ctx)
}

/// Format unmarshal code for a single parameter without writing to a string.
///
/// `bytes_len` must be `Some(len_var)` when `ty` is `TypeRef::Bytes` — the len var
/// bounds the `reinterpret()` call so the full binary payload is readable even when
/// it contains embedded NUL bytes (fixes issue #114).
fn format_unmarshal_param(local: &str, segment: &str, ty: &TypeRef, bytes_len: Option<&str>) -> String {
    match ty {
        TypeRef::Primitive(_) => {
            // Primitives are declared directly in the handler signature with their Java primitive
            // type (e.g. `byte _level`). No extraction from a MemorySegment is needed.
            // This branch is intentionally unreachable — callers skip unmarshal_param for
            // primitive params — but is kept to keep the match exhaustive.
            String::new()
        }
        TypeRef::Bytes => {
            let len = bytes_len.unwrap_or("Long.MAX_VALUE");
            format!("byte[] {local} = {segment}.reinterpret({len}).toArray(ValueLayout.JAVA_BYTE);")
        }
        TypeRef::String => {
            format!("String {local} = {segment}.reinterpret(Long.MAX_VALUE).getString(0);")
        }
        TypeRef::Path => {
            format!(
                "java.nio.file.Path {local} = java.nio.file.Paths.get({segment}.reinterpret(Long.MAX_VALUE).getString(0));"
            )
        }
        TypeRef::Named(type_name) => {
            format!(
                "String {local}_json = {segment}.reinterpret(Long.MAX_VALUE).getString(0);\n            {type_name} {local} = JSON.readValue({local}_json, {type_name}.class);"
            )
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Optional(_) => {
            let java_ty = java_type(ty);
            format!(
                "String {local}_json = {segment}.reinterpret(Long.MAX_VALUE).getString(0);\n            {java_ty} {local} = JSON.readValue({local}_json, new com.fasterxml.jackson.core.type.TypeReference<{java_ty}>() {{ }});"
            )
        }
        TypeRef::Json | TypeRef::Duration | TypeRef::Char | TypeRef::Unit => {
            let java_ty = java_type(ty);
            format!(
                "String {local}_json = {segment}.reinterpret(Long.MAX_VALUE).getString(0);\n            {java_ty} {local} = JSON.readValue({local}_json, {java_ty}.class);"
            )
        }
    }
}

/// Java reserves several keywords; sanitize parameter names that would clash.
fn java_param_name(name: &str) -> String {
    match name {
        "default" | "class" | "package" | "new" | "return" | "this" | "void" | "interface" | "enum" | "switch"
        | "case" | "for" | "while" | "do" | "if" | "else" | "throw" | "throws" | "try" | "catch" | "finally"
        | "int" | "long" | "short" | "byte" | "boolean" | "float" | "double" | "char" | "synchronized" | "volatile"
        | "transient" | "abstract" | "static" | "final" | "private" | "protected" | "public" | "native"
        | "strictfp" | "extends" | "implements" | "instanceof" | "super" | "import" | "true" | "false" | "null" => {
            format!("{name}_")
        }
        _ => name.to_string(),
    }
}

#[cfg(test)]
mod tests;
