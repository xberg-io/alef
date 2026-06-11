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

    // Determine which imports are needed based on method signatures
    let signatures_text: String = trait_def
        .methods
        .iter()
        .filter(|m| !skipped.contains(m.name.as_str()))
        .map(|m| {
            let ret = java_type_visible(&m.return_type, visible_type_names, excluded_types);
            let params = m
                .params
                .iter()
                .map(|p| java_type_visible(&p.ty, visible_type_names, excluded_types))
                .collect::<Vec<_>>()
                .join(",");
            format!("{ret}({params})")
        })
        .collect::<Vec<_>>()
        .join(";");

    let mut imports = vec![];
    if signatures_text.contains("List<") {
        imports.push("java.util.List");
    }
    if signatures_text.contains("Map<") {
        imports.push("java.util.Map");
    }

    // Build method list for template
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

    let ctx = minijinja::context! {
        package => package,
        imports => imports,
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

    // Determine which imports are needed based on method signatures
    let bridge_signatures: String = trait_def
        .methods
        .iter()
        .filter(|m| m.trait_source.is_none() && !skipped.contains(m.name.as_str()))
        .map(|m| {
            let ret = java_type_visible(&m.return_type, visible_type_names, excluded_types);
            let params = m
                .params
                .iter()
                .map(|p| java_type_visible(&p.ty, visible_type_names, excluded_types))
                .collect::<Vec<_>>()
                .join(",");
            format!("{ret}({params})")
        })
        .collect::<Vec<_>>()
        .join(";");

    let mut imports = vec![];
    if bridge_signatures.contains("List<") {
        imports.push("java.util.List");
    }
    if bridge_signatures.contains("Map<") {
        imports.push("java.util.Map");
    }

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
            ("Shutdown", "int.class", "ValueLayout.JAVA_INT", ", MemorySegment.class"),
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
        let handle_name = format!("handle{}", method.name.to_pascal_case());
        let stub_name = format!("stub{}", method.name.to_pascal_case());

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
            handle_name => &handle_name,
            return_type => "int.class",
            method_type_params => method_type_params.join(", "),
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
        handle_name => "freeString",
        return_type => "void.class",
        method_type_params => "MemorySegment.class",
        descriptor_return => "ValueLayout.ADDRESS",
        descriptor_params => "ValueLayout.ADDRESS",
        returns_void => true,
    });

    stubs.push(minijinja::context! {
        var_name => "stubFreeUserData",
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

    let ctx = minijinja::context! {
        package => package,
        imports => imports,
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
mod tests {
    use super::*;
    use crate::core::ir::{MethodDef, ParamDef, PrimitiveType};

    /// Build a `visible_type_names` set containing every `Named` type referenced
    /// by the trait method's params or return type, so tests behave as if those
    /// types are visible in the generated API.
    fn all_named_visible(methods: &[MethodDef]) -> HashSet<&str> {
        fn collect<'a>(ty: &'a TypeRef, out: &mut HashSet<&'a str>) {
            match ty {
                TypeRef::Named(n) => {
                    out.insert(n.as_str());
                }
                TypeRef::Optional(inner) | TypeRef::Vec(inner) => collect(inner, out),
                TypeRef::Map(k, v) => {
                    collect(k, out);
                    collect(v, out);
                }
                _ => {}
            }
        }
        let mut set = HashSet::new();
        for m in methods {
            collect(&m.return_type, &mut set);
            for p in &m.params {
                collect(&p.ty, &mut set);
            }
        }
        set
    }

    fn make_method(name: &str, return_type: TypeRef, params: Vec<ParamDef>) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params,
            return_type,
            is_async: false,
            is_static: false,
            error_type: Some("Error".to_string()),
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    fn make_trait(name: &str, methods: Vec<MethodDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("sample_crate::{name}"),
            original_rust_path: format!("sample_crate::{name}"),
            fields: vec![],
            methods,
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            doc: String::new(),
            cfg: None,
            is_trait: true,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }
    }

    #[test]
    fn interface_emisample_package_and_lifecycle_when_super_trait() {
        let trait_def = make_trait("OcrBackend", vec![make_method("process", TypeRef::String, vec![])]);
        let visible = all_named_visible(&trait_def.methods);
        let excluded = HashSet::new();
        let files = gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.sample_crate",
            true,
            None,
            None,
            &visible,
            &excluded,
            &[],
        );
        assert!(files.interface_content.starts_with("package dev.sample_crate;"));
        assert!(files.interface_content.contains("public interface IOcrBackend"));
        assert!(files.interface_content.contains("String name();"));
        assert!(files.interface_content.contains("default void initialize()"));
        assert!(files.interface_content.contains("String process()"));
    }

    #[test]
    fn interface_omits_lifecycle_when_no_super_trait() {
        let trait_def = make_trait("Filter", vec![make_method("apply", TypeRef::String, vec![])]);
        let visible = all_named_visible(&trait_def.methods);
        let excluded = HashSet::new();
        let files = gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.sample_crate",
            false,
            None,
            None,
            &visible,
            &excluded,
            &[],
        );
        assert!(!files.interface_content.contains("String name();"));
        assert!(files.interface_content.contains("String apply()"));
    }

    #[test]
    fn bridge_class_has_register_helper_and_registry() {
        let trait_def = make_trait("OcrBackend", vec![]);
        let visible = all_named_visible(&trait_def.methods);
        let excluded = HashSet::new();
        // No unregister/clear configured: neither method should appear
        let files = gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.sample_crate",
            true,
            None,
            None,
            &visible,
            &excluded,
            &[],
        );
        let body = files.bridge_content.as_str();
        assert!(body.starts_with("package dev.sample_crate;"));
        assert!(body.contains("public final class OcrBackendBridge"));
        assert!(body.contains("public static void registerOcrBackend(final IOcrBackend impl)"));
        assert!(!body.contains("public static void unregisterOcrBackend(String name)"));
        assert!(!body.contains("public static void clearAllOcrBackend()"));
        assert!(body.contains("ConcurrentHashMap<String, OcrBackendBridge>"));
        assert!(body.contains("OCR_BACKEND_BRIDGES = new ConcurrentHashMap<>()"));
        assert!(body.contains("KRZ_REGISTER_OCR_BACKEND"));
        assert!(body.contains("private void freeString(MemorySegment ptr)"));
        assert!(body.contains("FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)"));
    }

    #[test]
    fn lifecycle_string_callbacks_use_status_and_out_error() {
        let trait_def = make_trait("OcrBackend", vec![]);
        let visible = all_named_visible(&trait_def.methods);
        let excluded = HashSet::new();
        let files = gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.sample_crate",
            true,
            None,
            None,
            &visible,
            &excluded,
            &[],
        );
        let body = files.bridge_content.as_str();

        assert!(body.contains("int handleName(MemorySegment userData, MemorySegment outName, MemorySegment outError)"));
        assert!(
            body.contains(
                "int handleVersion(MemorySegment userData, MemorySegment outVersion, MemorySegment outError)"
            )
        );
        assert!(body.contains(
            "MethodType.methodType(int.class, MemorySegment.class, MemorySegment.class, MemorySegment.class)"
        ));
        assert!(body.contains(
            "FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)"
        ));
    }

    #[test]
    fn gen_unregistration_fn_emits_method_when_configured() {
        let trait_def = make_trait("OcrBackend", vec![]);
        let visible = all_named_visible(&trait_def.methods);
        let excluded = HashSet::new();
        let files = gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.sample_crate",
            true,
            Some("unregister_ocr_backend"),
            None,
            &visible,
            &excluded,
            &[],
        );
        let body = files.bridge_content.as_str();
        assert!(body.contains("public static void unregisterOcrBackend(String name)"));
        assert!(body.contains("KRZ_UNREGISTER_OCR_BACKEND"));
        assert!(body.contains("OCR_BACKEND_BRIDGES.remove(name)"));
        assert!(!body.contains("public static void clearAllOcrBackend()"));
    }

    #[test]
    fn gen_unregistration_fn_omits_method_when_none() {
        let trait_def = make_trait("OcrBackend", vec![]);
        let visible = all_named_visible(&trait_def.methods);
        let excluded = HashSet::new();
        let files = gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.sample_crate",
            true,
            None,
            None,
            &visible,
            &excluded,
            &[],
        );
        let body = files.bridge_content.as_str();
        assert!(!body.contains("public static void unregisterOcrBackend(String name)"));
    }

    #[test]
    fn gen_clear_fn_emits_method_when_configured() {
        let trait_def = make_trait("OcrBackend", vec![]);
        let visible = all_named_visible(&trait_def.methods);
        let excluded = HashSet::new();
        let files = gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.sample_crate",
            true,
            None,
            Some("clear_ocr_backends"),
            &visible,
            &excluded,
            &[],
        );
        let body = files.bridge_content.as_str();
        assert!(body.contains("public static void clearOcrBackends()"));
        assert!(body.contains("KRZ_CLEAR_OCR_BACKEND"));
        assert!(body.contains("OCR_BACKEND_BRIDGES.values().forEach(OcrBackendBridge::close)"));
        assert!(body.contains("OCR_BACKEND_BRIDGES.clear()"));
        assert!(!body.contains("public static void unregisterOcrBackend(String name)"));
    }

    #[test]
    fn gen_clear_fn_omits_method_when_none() {
        let trait_def = make_trait("OcrBackend", vec![]);
        let visible = all_named_visible(&trait_def.methods);
        let excluded = HashSet::new();
        let files = gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.sample_crate",
            true,
            None,
            None,
            &visible,
            &excluded,
            &[],
        );
        let body = files.bridge_content.as_str();
        assert!(!body.contains("public static void clearOcrBackends()"));
    }

    #[test]
    fn both_unregister_and_clear_emitted_when_both_configured() {
        let trait_def = make_trait("OcrBackend", vec![]);
        let visible = all_named_visible(&trait_def.methods);
        let excluded = HashSet::new();
        let files = gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.sample_crate",
            true,
            Some("unregister_ocr_backend"),
            Some("clear_ocr_backends"),
            &visible,
            &excluded,
            &[],
        );
        let body = files.bridge_content.as_str();
        assert!(body.contains("public static void unregisterOcrBackend(String name)"));
        assert!(body.contains("public static void clearOcrBackends()"));
    }

    #[test]
    fn java_param_name_sanitizes_keywords() {
        assert_eq!(java_param_name("default"), "default_");
        assert_eq!(java_param_name("config"), "config");
    }

    #[test]
    fn bridge_class_does_not_json_quote_raw_string_results() {
        let trait_def = make_trait("Renderer", vec![make_method("render", TypeRef::String, vec![])]);
        let visible = all_named_visible(&trait_def.methods);
        let excluded = HashSet::new();
        let files = gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.sample_crate",
            false,
            None,
            None,
            &visible,
            &excluded,
            &[],
        );
        let body = files.bridge_content.as_str();

        assert!(
            body.contains("MemorySegment jsonCs = arena.allocateFrom(result);"),
            "String callback results must be returned as raw UTF-8, got:\n{body}"
        );
        assert!(
            !body.contains("String json = JSON.writeValueAsString(result);"),
            "String callback results must not be JSON-quoted, got:\n{body}"
        );
    }

    #[test]
    fn bridge_class_does_not_double_encode_excluded_named_json_results() {
        let trait_def = make_trait(
            "Renderer",
            vec![make_method(
                "render",
                TypeRef::Named("InternalDocument".to_string()),
                vec![],
            )],
        );
        let visible = HashSet::new();
        let excluded = HashSet::from(["InternalDocument".to_string()]);
        let files = gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.sample_crate",
            false,
            None,
            None,
            &visible,
            &excluded,
            &[],
        );
        let body = files.bridge_content.as_str();

        assert!(
            body.contains("String result = impl.render();"),
            "excluded named return should surface as a raw JSON String, got:\n{body}"
        );
        assert!(
            body.contains("MemorySegment jsonCs = arena.allocateFrom(result);"),
            "excluded named JSON return must be passed through without writeValueAsString, got:\n{body}"
        );
    }

    #[test]
    fn bridge_class_unmarshals_path_and_bytes() {
        let trait_def = make_trait(
            "OcrBackend",
            vec![make_method(
                "process_image",
                TypeRef::String,
                vec![
                    ParamDef {
                        name: "image_bytes".to_string(),
                        ty: TypeRef::Bytes,
                        optional: false,
                        default: None,
                        sanitized: false,
                        typed_default: None,
                        is_ref: true,
                        is_mut: false,
                        newtype_wrapper: None,
                        original_type: None,
                        map_is_ahash: false,
                        map_key_is_cow: false,
                        vec_inner_is_ref: false,
                        map_is_btree: false,
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "config".to_string(),
                        ty: TypeRef::Named("OcrConfig".to_string()),
                        optional: false,
                        default: None,
                        sanitized: false,
                        typed_default: None,
                        is_ref: true,
                        is_mut: false,
                        newtype_wrapper: None,
                        original_type: None,
                        map_is_ahash: false,
                        map_key_is_cow: false,
                        vec_inner_is_ref: false,
                        map_is_btree: false,
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "path".to_string(),
                        ty: TypeRef::Path,
                        optional: false,
                        default: None,
                        sanitized: false,
                        typed_default: None,
                        is_ref: true,
                        is_mut: false,
                        newtype_wrapper: None,
                        original_type: None,
                        map_is_ahash: false,
                        map_key_is_cow: false,
                        vec_inner_is_ref: false,
                        map_is_btree: false,
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                ],
            )],
        );
        let visible = all_named_visible(&trait_def.methods);
        let excluded = HashSet::new();
        let files = gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.sample_crate",
            false,
            None,
            None,
            &visible,
            &excluded,
            &[],
        );
        let body = files.bridge_content.as_str();
        assert!(body.contains("toArray(ValueLayout.JAVA_BYTE)"));
        assert!(body.contains("OcrConfig"));
        assert!(body.contains("Paths.get("));
    }

    /// Regression (#114): the Panama FFM handler signature for a Bytes parameter must include
    /// a `long {name}Len` companion, and the unmarshal expression must use that length to
    /// bound the MemorySegment read (`reinterpret(len)` not `reinterpret(Long.MAX_VALUE)`).
    /// Without the companion parameter, embedded NUL bytes (0x00) in the payload cause the
    /// callee to read past the end of the buffer.
    #[test]
    fn bridge_handler_bytes_param_includes_len_companion_and_bounded_reinterpret() {
        let trait_def = make_trait(
            "Processor",
            vec![make_method(
                "ingest",
                TypeRef::Unit,
                vec![crate::core::ir::ParamDef {
                    name: "payload".to_string(),
                    ty: TypeRef::Bytes,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: true,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                }],
            )],
        );
        let visible = all_named_visible(&trait_def.methods);
        let excluded = HashSet::new();
        let files = gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.sample_crate",
            false,
            None,
            None,
            &visible,
            &excluded,
            &[],
        );
        let body = files.bridge_content.as_str();

        // The handler method signature must carry the length companion.
        assert!(
            body.contains("long payloadLen"),
            "handler signature must include `long payloadLen` for Bytes param;\nactual:\n{body}"
        );
        // The unmarshal must use the bounded reinterpret(payloadLen), never Long.MAX_VALUE.
        assert!(
            body.contains("reinterpret(payloadLen)"),
            "Bytes unmarshal must use `reinterpret(payloadLen)`;\nactual:\n{body}"
        );
        assert!(
            !body.contains("Long.MAX_VALUE"),
            "Bytes unmarshal must not use `Long.MAX_VALUE` (unbounded read);\nactual:\n{body}"
        );
    }

    #[test]
    fn bridge_handler_emits_primitive_param_as_java_primitive_not_memory_segment() {
        let trait_def = make_trait(
            "Logger",
            vec![make_method(
                "log",
                TypeRef::Unit,
                vec![
                    ParamDef {
                        name: "level".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::U8),
                        optional: false,
                        default: None,
                        sanitized: false,
                        typed_default: None,
                        is_ref: false,
                        is_mut: false,
                        newtype_wrapper: None,
                        original_type: None,
                        map_is_ahash: false,
                        map_key_is_cow: false,
                        vec_inner_is_ref: false,
                        map_is_btree: false,
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "msg".to_string(),
                        ty: TypeRef::String,
                        optional: false,
                        default: None,
                        sanitized: false,
                        typed_default: None,
                        is_ref: true,
                        is_mut: false,
                        newtype_wrapper: None,
                        original_type: None,
                        map_is_ahash: false,
                        map_key_is_cow: false,
                        vec_inner_is_ref: false,
                        map_is_btree: false,
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                ],
            )],
        );
        let visible = all_named_visible(&trait_def.methods);
        let excluded = HashSet::new();
        let files = gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.sample_crate",
            false,
            None,
            None,
            &visible,
            &excluded,
            &[],
        );
        let body = files.bridge_content.as_str();
        // The handler signature should have `byte level`, not `MemorySegment level_in`
        assert!(body.contains(
            "private int handleLog(MemorySegment userData, byte level, MemorySegment msg_in, MemorySegment outError)"
        ));
    }
}
