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
use std::fmt::Write;

use crate::type_map::{java_ffi_type, java_type};

/// The two generated Java files for one trait bridge.
pub struct BridgeFiles {
    pub interface_content: String,
    pub bridge_content: String,
}

/// Generate both the managed interface file and the bridge class file for one trait.
pub fn gen_trait_bridge_files(trait_def: &TypeDef, prefix: &str, package: &str, has_super_trait: bool) -> BridgeFiles {
    BridgeFiles {
        interface_content: gen_interface_file(trait_def, package, has_super_trait),
        bridge_content: gen_bridge_file(trait_def, prefix, package, has_super_trait),
    }
}

/// Generate the standalone managed `I{Trait}` interface compilation unit.
fn gen_interface_file(trait_def: &TypeDef, package: &str, has_super_trait: bool) -> String {
    let trait_pascal = trait_def.name.to_pascal_case();
    let mut out = String::with_capacity(2048);

    writeln!(out, "package {package};").ok();
    writeln!(out).ok();
    writeln!(out, "import java.util.List;").ok();
    writeln!(out, "import java.util.Map;").ok();
    writeln!(out).ok();

    writeln!(out, "/**").ok();
    writeln!(out, " * Bridge interface for the {trait_pascal} plugin system.").ok();
    writeln!(out, " *").ok();
    writeln!(
        out,
        " * Implementations are wrapped by {trait_pascal}Bridge and exposed to the native"
    )
    .ok();
    writeln!(out, " * runtime through Panama FFM upcall stubs.").ok();
    writeln!(out, " */").ok();
    writeln!(out, "public interface I{trait_pascal} {{").ok();
    writeln!(out).ok();

    if has_super_trait {
        writeln!(out, "    /** Plugin name (used for registry keying). */").ok();
        writeln!(out, "    String name();").ok();
        writeln!(out).ok();
        writeln!(out, "    /** Plugin version. */").ok();
        writeln!(out, "    String version();").ok();
        writeln!(out).ok();
        writeln!(out, "    /** Initialize the plugin. */").ok();
        writeln!(out, "    default void initialize() throws Exception {{}}").ok();
        writeln!(out).ok();
        writeln!(out, "    /** Shut down the plugin. */").ok();
        writeln!(out, "    default void shutdown() throws Exception {{}}").ok();
        writeln!(out).ok();
    }

    for method in &trait_def.methods {
        let return_type_str = java_type(&method.return_type);
        let params_str = method
            .params
            .iter()
            .map(|p| format!("{} {}", java_type(&p.ty), java_param_name(&p.name)))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(out, "    /** {}. */", method.name).ok();
        writeln!(
            out,
            "    {} {}({}) throws Exception;",
            return_type_str, method.name, params_str
        )
        .ok();
        writeln!(out).ok();
    }

    writeln!(out, "}}").ok();
    out
}

/// Generate the bridge class compilation unit with upcall stubs, registry, and
/// register/unregister helpers all nested inside the public top-level class.
fn gen_bridge_file(trait_def: &TypeDef, prefix: &str, package: &str, has_super_trait: bool) -> String {
    let trait_pascal = trait_def.name.to_pascal_case();
    let trait_snake = trait_def.name.to_snake_case();
    let prefix_upper = prefix.to_uppercase();
    let registry_field = format!("{}_BRIDGES", trait_snake.to_uppercase());
    let bridge_class = format!("{trait_pascal}Bridge");

    let mut out = String::with_capacity(8192);

    writeln!(out, "package {package};").ok();
    writeln!(out).ok();
    writeln!(out, "import java.lang.foreign.Arena;").ok();
    writeln!(out, "import java.lang.foreign.FunctionDescriptor;").ok();
    writeln!(out, "import java.lang.foreign.Linker;").ok();
    writeln!(out, "import java.lang.foreign.MemorySegment;").ok();
    writeln!(out, "import java.lang.foreign.ValueLayout;").ok();
    writeln!(out, "import java.lang.invoke.MethodHandles;").ok();
    writeln!(out, "import java.lang.invoke.MethodType;").ok();
    writeln!(out, "import java.util.List;").ok();
    writeln!(out, "import java.util.Map;").ok();
    writeln!(out, "import java.util.concurrent.ConcurrentHashMap;").ok();
    writeln!(out, "import com.fasterxml.jackson.databind.ObjectMapper;").ok();
    writeln!(out).ok();

    writeln!(out, "/**").ok();
    writeln!(
        out,
        " * Allocates Panama FFM upcall stubs for an I{trait_pascal} implementation,"
    )
    .ok();
    writeln!(out, " * assembles the C vtable in native memory, and provides static").ok();
    writeln!(out, " * register{trait_pascal}/unregister{trait_pascal} helpers.").ok();
    writeln!(out, " */").ok();
    writeln!(out, "public final class {bridge_class} implements AutoCloseable {{").ok();
    writeln!(out).ok();

    writeln!(out, "    private static final Linker LINKER = Linker.nativeLinker();").ok();
    writeln!(
        out,
        "    private static final MethodHandles.Lookup LOOKUP = MethodHandles.lookup();"
    )
    .ok();
    writeln!(out, "    private static final ObjectMapper JSON = new ObjectMapper();").ok();
    writeln!(out).ok();

    writeln!(
        out,
        "    /** Live registry — keeps Arenas and upcall stubs alive past the register call. */"
    )
    .ok();
    writeln!(
        out,
        "    private static final ConcurrentHashMap<String, {bridge_class}> {registry_field} = new ConcurrentHashMap<>();"
    )
    .ok();
    writeln!(out).ok();

    let num_methods = trait_def.methods.len();
    let num_super_slots = if has_super_trait { 4usize } else { 0usize };
    let num_vtable_fields = num_super_slots + num_methods + 1;
    writeln!(
        out,
        "    // C vtable: {num_vtable_fields} fields ({num_super_slots} plugin methods + {num_methods} trait methods + free_user_data)"
    )
    .ok();
    writeln!(
        out,
        "    private static final long VTABLE_SIZE = (long) ValueLayout.ADDRESS.byteSize() * {num_vtable_fields}L;"
    )
    .ok();
    writeln!(out).ok();

    writeln!(out, "    private final Arena arena;").ok();
    writeln!(out, "    private final MemorySegment vtable;").ok();
    writeln!(out, "    private final I{trait_pascal} impl;").ok();
    writeln!(out).ok();

    // Constructor — wires every vtable slot to a method handle bound to this instance.
    writeln!(out, "    {bridge_class}(final I{trait_pascal} impl) {{").ok();
    writeln!(out, "        this.impl = impl;").ok();
    writeln!(out, "        this.arena = Arena.ofShared();").ok();
    writeln!(out, "        this.vtable = arena.allocate(VTABLE_SIZE);").ok();
    writeln!(out).ok();
    writeln!(out, "        try {{").ok();
    writeln!(out, "            long offset = 0L;").ok();
    writeln!(out).ok();

    if has_super_trait {
        emit_lifecycle_stub(&mut out, "Name", "MemorySegment.class", "ValueLayout.ADDRESS");
        emit_lifecycle_stub(&mut out, "Version", "MemorySegment.class", "ValueLayout.ADDRESS");
        emit_lifecycle_stub(&mut out, "Initialize", "int.class", "ValueLayout.JAVA_INT");
        emit_lifecycle_stub(&mut out, "Shutdown", "int.class", "ValueLayout.JAVA_INT");
    }

    for method in &trait_def.methods {
        let handle_name = format!("handle{}", method.name.to_pascal_case());
        let stub_name = format!("stub{}", method.name.to_pascal_case());

        let mut method_type_params = vec!["MemorySegment.class".to_string()];
        for _param in &method.params {
            method_type_params.push("MemorySegment.class".to_string());
        }
        if !matches!(method.return_type, TypeRef::Unit) {
            method_type_params.push("MemorySegment.class".to_string());
        }
        method_type_params.push("MemorySegment.class".to_string());

        writeln!(
            out,
            "            var {stub_name} = LINKER.upcallStub(LOOKUP.bind(this, \"{handle_name}\","
        )
        .ok();
        writeln!(
            out,
            "                MethodType.methodType(int.class, {})),",
            method_type_params.join(", ")
        )
        .ok();

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

        writeln!(
            out,
            "                FunctionDescriptor.of(ValueLayout.JAVA_INT, {}),",
            func_desc_params.join(", ")
        )
        .ok();
        writeln!(out, "                arena);").ok();
        writeln!(out, "            vtable.set(ValueLayout.ADDRESS, offset, {stub_name});").ok();
        writeln!(out, "            offset += ValueLayout.ADDRESS.byteSize();").ok();
        writeln!(out).ok();
    }

    writeln!(
        out,
        "            vtable.set(ValueLayout.ADDRESS, offset, MemorySegment.NULL);"
    )
    .ok();
    writeln!(out).ok();
    writeln!(out, "        }} catch (ReflectiveOperationException e) {{").ok();
    writeln!(out, "            arena.close();").ok();
    writeln!(
        out,
        "            throw new RuntimeException(\"Failed to create trait bridge stubs\", e);"
    )
    .ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    writeln!(out, "    MemorySegment vtableSegment() {{ return vtable; }}").ok();
    writeln!(out).ok();

    if has_super_trait {
        writeln!(out, "    private MemorySegment handleName(MemorySegment userData) {{").ok();
        writeln!(out, "        try {{").ok();
        writeln!(out, "            return arena.allocateFrom(impl.name());").ok();
        writeln!(out, "        }} catch (Throwable e) {{ return MemorySegment.NULL; }}").ok();
        writeln!(out, "    }}").ok();
        writeln!(out).ok();

        writeln!(
            out,
            "    private MemorySegment handleVersion(MemorySegment userData) {{"
        )
        .ok();
        writeln!(out, "        try {{").ok();
        writeln!(out, "            return arena.allocateFrom(impl.version());").ok();
        writeln!(out, "        }} catch (Throwable e) {{ return MemorySegment.NULL; }}").ok();
        writeln!(out, "    }}").ok();
        writeln!(out).ok();

        writeln!(
            out,
            "    private int handleInitialize(MemorySegment userData, MemorySegment outError) {{"
        )
        .ok();
        writeln!(out, "        try {{ impl.initialize(); return 0; }}").ok();
        writeln!(
            out,
            "        catch (Throwable e) {{ writeError(outError, e); return 1; }}"
        )
        .ok();
        writeln!(out, "    }}").ok();
        writeln!(out).ok();

        writeln!(
            out,
            "    private int handleShutdown(MemorySegment userData, MemorySegment outError) {{"
        )
        .ok();
        writeln!(out, "        try {{ impl.shutdown(); return 0; }}").ok();
        writeln!(
            out,
            "        catch (Throwable e) {{ writeError(outError, e); return 1; }}"
        )
        .ok();
        writeln!(out, "    }}").ok();
        writeln!(out).ok();
    }

    // Trait method handlers.
    for method in &trait_def.methods {
        emit_method_handler(&mut out, method);
    }

    // Shared error-writer.
    writeln!(
        out,
        "    private void writeError(MemorySegment outError, Throwable e) {{"
    )
    .ok();
    writeln!(
        out,
        "        try {{ outError.set(ValueLayout.ADDRESS, 0, arena.allocateFrom(e.getClass().getSimpleName() + \": \" + e.getMessage())); }}"
    )
    .ok();
    writeln!(out, "        catch (Throwable ignored) {{ /* swallow */ }}").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    // close()
    writeln!(out, "    @Override").ok();
    writeln!(out, "    public void close() {{ arena.close(); }}").ok();
    writeln!(out).ok();

    // Static register / unregister.
    writeln!(
        out,
        "    /** Register a {trait_pascal} implementation via Panama FFM upcall stubs. */"
    )
    .ok();
    writeln!(
        out,
        "    public static void register{trait_pascal}(final I{trait_pascal} impl) throws Exception {{"
    )
    .ok();
    writeln!(out, "        var bridge = new {bridge_class}(impl);").ok();
    writeln!(out, "        try {{").ok();
    writeln!(out, "            try (var nameArena = Arena.ofConfined()) {{").ok();
    writeln!(out, "                var nameCs = nameArena.allocateFrom(impl.name());").ok();
    writeln!(
        out,
        "                MemorySegment outErr = nameArena.allocate(ValueLayout.ADDRESS);"
    )
    .ok();
    writeln!(
        out,
        "                int rc = (int) NativeLib.{prefix_upper}_REGISTER_{}.invoke(nameCs, bridge.vtableSegment(), MemorySegment.NULL, outErr);",
        trait_snake.to_uppercase()
    )
    .ok();
    writeln!(out, "                if (rc != 0) {{").ok();
    writeln!(
        out,
        "                    MemorySegment errPtr = outErr.get(ValueLayout.ADDRESS, 0);"
    )
    .ok();
    writeln!(
        out,
        "                    String msg = errPtr.equals(MemorySegment.NULL) ? \"registration failed (rc=\" + rc + \")\" : errPtr.reinterpret(Long.MAX_VALUE).getString(0);"
    )
    .ok();
    writeln!(
        out,
        "                    throw new RuntimeException(\"register{trait_pascal}: \" + msg);"
    )
    .ok();
    writeln!(out, "                }}").ok();
    writeln!(out, "            }}").ok();
    writeln!(out, "        }} catch (Throwable t) {{").ok();
    writeln!(out, "            bridge.close();").ok();
    writeln!(out, "            throw t;").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "        {registry_field}.put(impl.name(), bridge);").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    writeln!(out, "    /** Unregister a {trait_pascal} implementation by name. */").ok();
    writeln!(
        out,
        "    public static void unregister{trait_pascal}(String name) throws Exception {{"
    )
    .ok();
    writeln!(out, "        try (var nameArena = Arena.ofConfined()) {{").ok();
    writeln!(out, "            var nameCs = nameArena.allocateFrom(name);").ok();
    writeln!(
        out,
        "            MemorySegment outErr = nameArena.allocate(ValueLayout.ADDRESS);"
    )
    .ok();
    writeln!(
        out,
        "            int rc = (int) NativeLib.{prefix_upper}_UNREGISTER_{}.invoke(nameCs, outErr);",
        trait_snake.to_uppercase()
    )
    .ok();
    writeln!(out, "            if (rc != 0) {{").ok();
    writeln!(
        out,
        "                MemorySegment errPtr = outErr.get(ValueLayout.ADDRESS, 0);"
    )
    .ok();
    writeln!(
        out,
        "                String msg = errPtr.equals(MemorySegment.NULL) ? \"unregistration failed (rc=\" + rc + \")\" : errPtr.reinterpret(Long.MAX_VALUE).getString(0);"
    )
    .ok();
    writeln!(
        out,
        "                throw new RuntimeException(\"unregister{trait_pascal}: \" + msg);"
    )
    .ok();
    writeln!(out, "            }}").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "        {bridge_class} old = {registry_field}.remove(name);").ok();
    writeln!(out, "        if (old != null) {{ old.close(); }}").ok();
    writeln!(out, "    }}").ok();

    writeln!(out, "}}").ok();
    out
}

/// Emit the upcall-stub-allocation block for a Plugin lifecycle slot.
fn emit_lifecycle_stub(out: &mut String, pascal: &str, method_return_type: &str, descriptor_return: &str) {
    let handle = format!("handle{pascal}");
    let stub_var = format!("stub{pascal}");
    let extra_param = if pascal == "Initialize" || pascal == "Shutdown" {
        ", MemorySegment.class"
    } else {
        ""
    };
    let extra_descriptor = if pascal == "Initialize" || pascal == "Shutdown" {
        ", ValueLayout.ADDRESS"
    } else {
        ""
    };
    writeln!(
        out,
        "            var {stub_var} = LINKER.upcallStub(LOOKUP.bind(this, \"{handle}\","
    )
    .ok();
    writeln!(
        out,
        "                MethodType.methodType({method_return_type}, MemorySegment.class{extra_param})),"
    )
    .ok();
    writeln!(
        out,
        "                FunctionDescriptor.of({descriptor_return}, ValueLayout.ADDRESS{extra_descriptor}),"
    )
    .ok();
    writeln!(out, "                arena);").ok();
    writeln!(out, "            vtable.set(ValueLayout.ADDRESS, offset, {stub_var});").ok();
    writeln!(out, "            offset += ValueLayout.ADDRESS.byteSize();").ok();
    writeln!(out).ok();
}

/// Emit one trait-method handler.
fn emit_method_handler(out: &mut String, method: &alef_core::ir::MethodDef) {
    let handle = format!("handle{}", method.name.to_pascal_case());
    let mut sig_params = vec!["MemorySegment userData".to_string()];
    for param in &method.params {
        // Use a `_in` suffix on the segment name so we can declare the unmarshalled
        // local under the original identifier without shadowing.
        sig_params.push(format!("MemorySegment {}_in", java_param_name(&param.name)));
    }
    if !matches!(method.return_type, TypeRef::Unit) {
        sig_params.push("MemorySegment outResult".to_string());
    }
    sig_params.push("MemorySegment outError".to_string());

    writeln!(out, "    private int {handle}({}) {{", sig_params.join(", ")).ok();
    writeln!(out, "        try {{").ok();

    for param in &method.params {
        let local = java_param_name(&param.name);
        let segment = format!("{local}_in");
        unmarshal_param(out, &local, &segment, &param.ty);
    }

    let java_args: Vec<String> = method.params.iter().map(|p| java_param_name(&p.name)).collect();

    if matches!(method.return_type, TypeRef::Unit) {
        writeln!(out, "            impl.{}({});", method.name, java_args.join(", ")).ok();
    } else {
        let return_type_str = java_type(&method.return_type);
        writeln!(
            out,
            "            {return_type_str} result = impl.{}({});",
            method.name,
            java_args.join(", ")
        )
        .ok();
        writeln!(out, "            String json = JSON.writeValueAsString(result);").ok();
        writeln!(out, "            MemorySegment jsonCs = arena.allocateFrom(json);").ok();
        writeln!(out, "            outResult.set(ValueLayout.ADDRESS, 0, jsonCs);").ok();
    }

    writeln!(out, "            return 0;").ok();
    writeln!(out, "        }} catch (Throwable e) {{").ok();
    writeln!(out, "            writeError(outError, e);").ok();
    writeln!(out, "            return 1;").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();
}

/// Emit code that materializes the Java-side parameter `local` (declared here)
/// from the FFI MemorySegment `segment`.
fn unmarshal_param(out: &mut String, local: &str, segment: &str, ty: &TypeRef) {
    match ty {
        TypeRef::Primitive(_) => {
            // Primitives arrive as their primitive Java type; the bridge signature is generated
            // with MemorySegment for every parameter, so primitive support is currently
            // limited — kreuzberg's traits do not exercise this path.
            writeln!(
                out,
                "            // primitive parameter '{local}' is treated as MemorySegment placeholder; not used."
            )
            .ok();
            writeln!(
                out,
                "            // (fix me when a trait exposes a primitive method param)"
            )
            .ok();
            writeln!(out, "            {local} = 0; /* unsupported primitive bridge */").ok();
        }
        TypeRef::Bytes => {
            writeln!(
                out,
                "            byte[] {local} = {segment}.reinterpret(Long.MAX_VALUE).toArray(ValueLayout.JAVA_BYTE);"
            )
            .ok();
        }
        TypeRef::String => {
            writeln!(
                out,
                "            String {local} = {segment}.reinterpret(Long.MAX_VALUE).getString(0);"
            )
            .ok();
        }
        TypeRef::Path => {
            writeln!(
                out,
                "            java.nio.file.Path {local} = java.nio.file.Paths.get({segment}.reinterpret(Long.MAX_VALUE).getString(0));"
            )
            .ok();
        }
        TypeRef::Named(type_name) => {
            writeln!(
                out,
                "            String {local}_json = {segment}.reinterpret(Long.MAX_VALUE).getString(0);"
            )
            .ok();
            writeln!(
                out,
                "            {type_name} {local} = JSON.readValue({local}_json, {type_name}.class);"
            )
            .ok();
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Optional(_) => {
            let java_ty = java_type(ty);
            writeln!(
                out,
                "            String {local}_json = {segment}.reinterpret(Long.MAX_VALUE).getString(0);"
            )
            .ok();
            writeln!(
                out,
                "            {java_ty} {local} = JSON.readValue({local}_json, new com.fasterxml.jackson.core.type.TypeReference<{java_ty}>() {{ }});"
            )
            .ok();
        }
        TypeRef::Json | TypeRef::Duration | TypeRef::Char | TypeRef::Unit => {
            let java_ty = java_type(ty);
            writeln!(
                out,
                "            String {local}_json = {segment}.reinterpret(Long.MAX_VALUE).getString(0);"
            )
            .ok();
            writeln!(
                out,
                "            {java_ty} {local} = JSON.readValue({local}_json, {java_ty}.class);"
            )
            .ok();
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
            doc: String::new(),
            cfg: None,
            is_trait: true,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
        }
    }

    #[test]
    fn interface_emits_package_and_lifecycle_when_super_trait() {
        let trait_def = make_trait("OcrBackend", vec![make_method("process", TypeRef::String, vec![])]);
        let files = gen_trait_bridge_files(&trait_def, "krz", "dev.kreuzberg", true);
        assert!(files.interface_content.starts_with("package dev.kreuzberg;"));
        assert!(files.interface_content.contains("public interface IOcrBackend"));
        assert!(files.interface_content.contains("String name();"));
        assert!(files.interface_content.contains("default void initialize()"));
        assert!(files.interface_content.contains("String process()"));
    }

    #[test]
    fn interface_omits_lifecycle_when_no_super_trait() {
        let trait_def = make_trait("Filter", vec![make_method("apply", TypeRef::String, vec![])]);
        let files = gen_trait_bridge_files(&trait_def, "krz", "dev.kreuzberg", false);
        assert!(!files.interface_content.contains("String name();"));
        assert!(files.interface_content.contains("String apply()"));
    }

    #[test]
    fn bridge_class_has_register_helper_and_registry() {
        let trait_def = make_trait("OcrBackend", vec![]);
        let files = gen_trait_bridge_files(&trait_def, "krz", "dev.kreuzberg", true);
        let body = files.bridge_content.as_str();
        assert!(body.starts_with("package dev.kreuzberg;"));
        assert!(body.contains("public final class OcrBackendBridge"));
        assert!(body.contains("public static void registerOcrBackend(final IOcrBackend impl)"));
        assert!(body.contains("public static void unregisterOcrBackend(String name)"));
        assert!(body.contains("ConcurrentHashMap<String, OcrBackendBridge> OCR_BACKEND_BRIDGES"));
        assert!(body.contains("KRZ_REGISTER_OCR_BACKEND"));
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
                ],
            )],
        );
        let files = gen_trait_bridge_files(&trait_def, "krz", "dev.kreuzberg", true);
        let body = files.bridge_content.as_str();
        assert!(body.contains("byte[] image_bytes = image_bytes_in.reinterpret"));
        assert!(body.contains("String config_json = config_in.reinterpret"));
        let _ = PrimitiveType::Bool;
    }
}
