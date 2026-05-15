//! Java (Panama FFM) trait bridge code generation for plugin systems.
//!
//! Emits two files per `[[trait_bridges]]` entry, both as syntactically valid
//! single-class Java compilation units:
//!
//! 1. `I{TraitName}.java` — managed interface users implement
//! 2. `{TraitName}Bridge.java` — Panama upcall-stub bridge class with nested
//!    static fields for the live-bridge registry plus
//!    `register{TraitName}` / `unregister{TraitName}` static helpers
//!
//! All complex parameter and return marshalling goes through Jackson JSON, matching
//! how the FFI vtable receives values from native callers.

use alef_core::ir::{TypeDef, TypeRef};
use heck::{ToPascalCase, ToSnakeCase};
use minijinja::Value;
use std::collections::HashSet;

use crate::template_env;
use crate::type_map::{java_ffi_type, java_type};

/// Map a `TypeRef` to its Java representation, substituting `String` for any
/// `Named` type that is not in the set of visible (i.e. generated) types.
///
/// This prevents internal/excluded types like `InternalDocument` from leaking
/// into public trait interface signatures and Panama upcall bridge methods.
/// Excluded types are JSON-serialised over the FFI boundary as `String`.
fn java_type_visible(ty: &TypeRef, visible_type_names: &HashSet<&str>) -> String {
    match ty {
        TypeRef::Named(name) => {
            if visible_type_names.contains(name.as_str()) {
                java_type(ty).into_owned()
            } else {
                "String".to_string()
            }
        }
        TypeRef::Optional(inner) => java_type_visible(inner, visible_type_names),
        TypeRef::Vec(inner) => format!("List<{}>", java_type_visible_boxed(inner, visible_type_names)),
        TypeRef::Map(k, v) => format!(
            "Map<{}, {}>",
            java_type_visible_boxed(k, visible_type_names),
            java_type_visible_boxed(v, visible_type_names)
        ),
        _ => java_type(ty).into_owned(),
    }
}

/// Boxed variant of `java_type_visible` for use inside `List<...>` / `Map<...>`
/// generics. Substitutes excluded Named types with `String` (already boxed).
fn java_type_visible_boxed(ty: &TypeRef, visible_type_names: &HashSet<&str>) -> String {
    match ty {
        TypeRef::Named(name) => {
            if visible_type_names.contains(name.as_str()) {
                crate::type_map::java_boxed_type(ty).into_owned()
            } else {
                "String".to_string()
            }
        }
        TypeRef::Optional(inner) => java_type_visible_boxed(inner, visible_type_names),
        TypeRef::Vec(inner) => format!("List<{}>", java_type_visible_boxed(inner, visible_type_names)),
        TypeRef::Map(k, v) => format!(
            "Map<{}, {}>",
            java_type_visible_boxed(k, visible_type_names),
            java_type_visible_boxed(v, visible_type_names)
        ),
        _ => crate::type_map::java_boxed_type(ty).into_owned(),
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
/// `"unregister_ocr_backend"`). When `Some`, a `public static void unregister{Trait}(String
/// name)` helper is emitted in the bridge class. When `None`, the method is omitted.
///
/// `clear_fn` is the configured name of the host-crate clear-all function (e.g.
/// `"clear_ocr_backends"`). When `Some`, a `public static void clearAll{Trait}()` helper is
/// emitted. When `None`, the method is omitted.
pub fn gen_trait_bridge_files(
    trait_def: &TypeDef,
    prefix: &str,
    package: &str,
    has_super_trait: bool,
    unregister_fn: Option<&str>,
    clear_fn: Option<&str>,
    visible_type_names: &HashSet<&str>,
) -> BridgeFiles {
    BridgeFiles {
        interface_content: gen_interface_file(trait_def, package, has_super_trait, visible_type_names),
        bridge_content: gen_bridge_file(
            trait_def,
            prefix,
            package,
            has_super_trait,
            unregister_fn,
            clear_fn,
            visible_type_names,
        ),
    }
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

/// Generate the Java `clearAll{Trait}` static helper body.
///
/// Returns an empty string when `clear_fn` is `None` (opt-in). The emitted method calls
/// `NativeLib.{PREFIX}_CLEAR_{TRAIT}` via Panama FFM (no arguments other than the out-error
/// pointer), then closes and removes every live bridge from the local registry.
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
    template_env::render(
        "bridge_clear_method.jinja",
        minijinja::context! {
            trait_pascal => trait_pascal,
            trait_snake_upper => trait_snake_upper,
            prefix_upper => prefix_upper,
            bridge_class => bridge_class,
            registry_field => registry_field,
        },
    )
}

/// Generate the standalone managed `I{Trait}` interface compilation unit.
fn gen_interface_file(
    trait_def: &TypeDef,
    package: &str,
    has_super_trait: bool,
    visible_type_names: &HashSet<&str>,
) -> String {
    let trait_pascal = trait_def.name.to_pascal_case();

    // Determine which imports are needed based on method signatures
    let signatures_text: String = trait_def
        .methods
        .iter()
        .map(|m| {
            let ret = java_type_visible(&m.return_type, visible_type_names);
            let params = m
                .params
                .iter()
                .map(|p| java_type_visible(&p.ty, visible_type_names))
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
        .map(|m| {
            let return_type_str = java_type_visible(&m.return_type, visible_type_names);
            let params_str = m
                .params
                .iter()
                .map(|p| {
                    format!(
                        "{} {}",
                        java_type_visible(&p.ty, visible_type_names),
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
fn gen_bridge_file(
    trait_def: &TypeDef,
    prefix: &str,
    package: &str,
    has_super_trait: bool,
    unregister_fn: Option<&str>,
    clear_fn: Option<&str>,
    visible_type_names: &HashSet<&str>,
) -> String {
    let trait_pascal = trait_def.name.to_pascal_case();
    let trait_snake = trait_def.name.to_snake_case();
    let prefix_upper = prefix.to_uppercase();
    let registry_field = format!("{}_BRIDGES", trait_snake.to_uppercase());
    let bridge_class = format!("{trait_pascal}Bridge");

    // Determine which imports are needed based on method signatures
    let bridge_signatures: String = trait_def
        .methods
        .iter()
        .map(|m| {
            let ret = java_type_visible(&m.return_type, visible_type_names);
            let params = m
                .params
                .iter()
                .map(|p| java_type_visible(&p.ty, visible_type_names))
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
                signature => "MemorySegment handleName(MemorySegment userData)",
                body => "arena.allocateFrom(impl.name())",
                error_return => "MemorySegment.NULL",
                void_call => false,
                success_return => "",
            },
            minijinja::context! {
                signature => "MemorySegment handleVersion(MemorySegment userData)",
                body => "arena.allocateFrom(impl.version())",
                error_return => "MemorySegment.NULL",
                void_call => false,
                success_return => "",
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
            ("Name", "MemorySegment.class", "ValueLayout.ADDRESS", ""),
            ("Version", "MemorySegment.class", "ValueLayout.ADDRESS", ""),
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
            let extra_descriptor = if pascal == "Initialize" || pascal == "Shutdown" {
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
            });
        }
    }

    // Build stub allocations for trait methods
    for method in &trait_def.methods {
        let handle_name = format!("handle{}", method.name.to_pascal_case());
        let stub_name = format!("stub{}", method.name.to_pascal_case());

        let mut method_type_params = vec!["MemorySegment.class".to_string()];
        for param in &method.params {
            let class_literal = match &param.ty {
                TypeRef::Primitive(p) => format!("{}.class", java_type(&TypeRef::Primitive(p.clone()))),
                _ => "MemorySegment.class".to_string(),
            };
            method_type_params.push(class_literal);
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
        });
    }

    // Build trait method handlers
    let methods: Vec<Value> = trait_def
        .methods
        .iter()
        .map(|method| {
            let handle = format!("handle{}", method.name.to_pascal_case());
            let mut sig_params = vec!["MemorySegment userData".to_string()];
            for param in &method.params {
                let local = java_param_name(&param.name);
                match &param.ty {
                    TypeRef::Primitive(p) => {
                        sig_params.push(format!("{} {local}", java_type(&TypeRef::Primitive(p.clone()))));
                    }
                    _ => {
                        sig_params.push(format!("MemorySegment {local}_in"));
                    }
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
                if !matches!(param.ty, TypeRef::Primitive(_)) {
                    let segment = format!("{local}_in");
                    // Named types not in visible_type_names (e.g. InternalDocument) are
                    // JSON-bridged as opaque Strings — no companion Java class is generated
                    // for them, so unmarshal as a plain String rather than deserialising
                    // into a missing class.
                    if let TypeRef::Named(name) = &param.ty {
                        if !visible_type_names.contains(name.as_str()) {
                            unmarshal_params.push(format_unmarshal_param(&local, &segment, &TypeRef::String));
                            continue;
                        }
                    }
                    unmarshal_params.push(format_unmarshal_param(&local, &segment, &param.ty));
                }
            }

            let java_args: Vec<String> = method.params.iter().map(|p| java_param_name(&p.name)).collect();
            let has_return = !matches!(method.return_type, TypeRef::Unit);
            let return_type_str = if has_return {
                java_type_visible(&method.return_type, visible_type_names)
            } else {
                String::new()
            };

            minijinja::context! {
                name => &method.name,
                handle_name => &handle,
                sig_params => sig_params.join(", "),
                unmarshal_params => unmarshal_params,
                return_type => &return_type_str,
                call_args => java_args.join(", "),
                has_return => has_return,
            }
        })
        .collect();

    let num_methods = trait_def.methods.len();
    let num_super_slots = if has_super_trait { 4usize } else { 0usize };
    let num_vtable_fields = num_super_slots + num_methods + 1;
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
fn format_unmarshal_param(local: &str, segment: &str, ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(_) => {
            // Primitives are declared directly in the handler signature with their Java primitive
            // type (e.g. `byte _level`). No extraction from a MemorySegment is needed.
            // This branch is intentionally unreachable — callers skip unmarshal_param for
            // primitive params — but is kept to keep the match exhaustive.
            String::new()
        }
        TypeRef::Bytes => {
            format!("byte[] {local} = {segment}.reinterpret(Long.MAX_VALUE).toArray(ValueLayout.JAVA_BYTE);")
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
    use alef_core::ir::{MethodDef, ParamDef, PrimitiveType};

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
            receiver: Some(alef_core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    fn make_trait(name: &str, methods: Vec<MethodDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("kreuzberg::{name}"),
            original_rust_path: format!("kreuzberg::{name}"),
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
        }
    }

    #[test]
    fn interface_emits_package_and_lifecycle_when_super_trait() {
        let trait_def = make_trait("OcrBackend", vec![make_method("process", TypeRef::String, vec![])]);
        let visible = all_named_visible(&trait_def.methods);
        let files = gen_trait_bridge_files(&trait_def, "krz", "dev.kreuzberg", true, None, None, &visible);
        assert!(files.interface_content.starts_with("package dev.kreuzberg;"));
        assert!(files.interface_content.contains("public interface IOcrBackend"));
        assert!(files.interface_content.contains("String name();"));
        assert!(files.interface_content.contains("default void initialize()"));
        assert!(files.interface_content.contains("String process()"));
    }

    #[test]
    fn interface_omits_lifecycle_when_no_super_trait() {
        let trait_def = make_trait("Filter", vec![make_method("apply", TypeRef::String, vec![])]);
        let visible = all_named_visible(&trait_def.methods);
        let files = gen_trait_bridge_files(&trait_def, "krz", "dev.kreuzberg", false, None, None, &visible);
        assert!(!files.interface_content.contains("String name();"));
        assert!(files.interface_content.contains("String apply()"));
    }

    #[test]
    fn bridge_class_has_register_helper_and_registry() {
        let trait_def = make_trait("OcrBackend", vec![]);
        let visible = all_named_visible(&trait_def.methods);
        // No unregister/clear configured: neither method should appear
        let files = gen_trait_bridge_files(&trait_def, "krz", "dev.kreuzberg", true, None, None, &visible);
        let body = files.bridge_content.as_str();
        assert!(body.starts_with("package dev.kreuzberg;"));
        assert!(body.contains("public final class OcrBackendBridge"));
        assert!(body.contains("public static void registerOcrBackend(final IOcrBackend impl)"));
        assert!(!body.contains("public static void unregisterOcrBackend(String name)"));
        assert!(!body.contains("public static void clearAllOcrBackend()"));
        assert!(body.contains("ConcurrentHashMap<String, OcrBackendBridge>"));
        assert!(body.contains("OCR_BACKEND_BRIDGES = new ConcurrentHashMap<>()"));
        assert!(body.contains("KRZ_REGISTER_OCR_BACKEND"));
    }

    #[test]
    fn gen_unregistration_fn_emits_method_when_configured() {
        let trait_def = make_trait("OcrBackend", vec![]);
        let visible = all_named_visible(&trait_def.methods);
        let files = gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.kreuzberg",
            true,
            Some("unregister_ocr_backend"),
            None,
            &visible,
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
        let files = gen_trait_bridge_files(&trait_def, "krz", "dev.kreuzberg", true, None, None, &visible);
        let body = files.bridge_content.as_str();
        assert!(!body.contains("public static void unregisterOcrBackend(String name)"));
    }

    #[test]
    fn gen_clear_fn_emits_method_when_configured() {
        let trait_def = make_trait("OcrBackend", vec![]);
        let visible = all_named_visible(&trait_def.methods);
        let files = gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.kreuzberg",
            true,
            None,
            Some("clear_ocr_backends"),
            &visible,
        );
        let body = files.bridge_content.as_str();
        assert!(body.contains("public static void clearAllOcrBackend()"));
        assert!(body.contains("KRZ_CLEAR_OCR_BACKEND"));
        assert!(body.contains("OCR_BACKEND_BRIDGES.values().forEach(OcrBackendBridge::close)"));
        assert!(body.contains("OCR_BACKEND_BRIDGES.clear()"));
        assert!(!body.contains("public static void unregisterOcrBackend(String name)"));
    }

    #[test]
    fn gen_clear_fn_omits_method_when_none() {
        let trait_def = make_trait("OcrBackend", vec![]);
        let visible = all_named_visible(&trait_def.methods);
        let files = gen_trait_bridge_files(&trait_def, "krz", "dev.kreuzberg", true, None, None, &visible);
        let body = files.bridge_content.as_str();
        assert!(!body.contains("public static void clearAllOcrBackend()"));
    }

    #[test]
    fn both_unregister_and_clear_emitted_when_both_configured() {
        let trait_def = make_trait("OcrBackend", vec![]);
        let visible = all_named_visible(&trait_def.methods);
        let files = gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.kreuzberg",
            true,
            Some("unregister_ocr_backend"),
            Some("clear_ocr_backends"),
            &visible,
        );
        let body = files.bridge_content.as_str();
        assert!(body.contains("public static void unregisterOcrBackend(String name)"));
        assert!(body.contains("public static void clearAllOcrBackend()"));
    }

    #[test]
    fn java_param_name_sanitizes_keywords() {
        assert_eq!(java_param_name("default"), "default_");
        assert_eq!(java_param_name("config"), "config");
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
                    },
                ],
            )],
        );
        let visible = all_named_visible(&trait_def.methods);
        let files = gen_trait_bridge_files(&trait_def, "krz", "dev.kreuzberg", false, None, None, &visible);
        let body = files.bridge_content.as_str();
        assert!(body.contains("toArray(ValueLayout.JAVA_BYTE)"));
        assert!(body.contains("OcrConfig"));
        assert!(body.contains("Paths.get("));
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
                    },
                ],
            )],
        );
        let visible = all_named_visible(&trait_def.methods);
        let files = gen_trait_bridge_files(&trait_def, "krz", "dev.kreuzberg", false, None, None, &visible);
        let body = files.bridge_content.as_str();
        // The handler signature should have `byte level`, not `MemorySegment level_in`
        assert!(body.contains(
            "private int handleLog(MemorySegment userData, byte level, MemorySegment msg_in, MemorySegment outError)"
        ));
    }
}
