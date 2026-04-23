//! Java (Panama FFM) trait bridge code generation for plugin systems.
//!
//! This module generates Java code that wraps C FFI vtables for plugin registration.
//! Since Java cannot expose method references as raw C function pointers, we use
//! Java 21+ Foreign Function & Memory API (Panama) upcall stubs to bridge Java
//! implementations into the C vtable structure.
//!
//! For each `[[trait_bridges]]` entry, this module generates:
//!
//! 1. A `public interface {TraitName} { ... }` with methods matching the trait's methods
//!    plus Plugin lifecycle methods (name, version, initialize, shutdown).
//! 2. A `{TraitName}Bridge` class that:
//!    - Allocates Panama FFM upcall stubs for each trait method
//!    - Builds the C vtable as a MemorySegment
//!    - Manages memory lifecycle with AutoCloseable
//! 3. Registration helper: `public static void register{TraitName}({TraitName} impl)`
//!    that builds the vtable and calls the C registration function.
//! 4. Unregistration helper: `public static void unregister{TraitName}(String name)`.

use heck::{ToPascalCase, ToSnakeCase};
use std::fmt::Write;

/// Generate all trait bridge code for a single `[[trait_bridges]]` entry.
/// Returns Java source code as a String.
///
/// `has_super_trait`: when `true`, the C vtable includes Plugin lifecycle slots
/// (name_fn, version_fn, initialize_fn, shutdown_fn) and the bridge emits matching
/// upcall stubs. When `false`, only the trait-method slots are emitted, matching
/// the Rust-side vtable layout exactly.
pub fn gen_trait_bridge(
    trait_name: &str,
    prefix: &str,
    trait_methods: &[(&str, &str)], // [(method_name, return_type), ...]
    has_super_trait: bool,
) -> String {
    let trait_pascal = trait_name.to_pascal_case();
    let trait_snake = trait_name.to_snake_case();
    let prefix_upper = prefix.to_uppercase();

    let mut out = String::with_capacity(4096);

    // --- Trait interface ---
    writeln!(out, "/**").ok();
    writeln!(out, " * Bridge trait for {} plugin system.", trait_pascal).ok();
    writeln!(out, " *").ok();
    writeln!(
        out,
        " * Implementations provide methods that are called via upcall stubs"
    )
    .ok();
    writeln!(out, " * into the C vtable during registration.").ok();
    writeln!(out, " */").ok();
    writeln!(out, "public interface {} {{", trait_pascal).ok();
    writeln!(out).ok();

    // Plugin lifecycle methods — only when a super_trait (Plugin) is configured
    if has_super_trait {
        writeln!(out, "    /** Return the plugin name. */").ok();
        writeln!(out, "    String name();").ok();
        writeln!(out).ok();

        writeln!(out, "    /** Return the plugin version. */").ok();
        writeln!(out, "    String version();").ok();
        writeln!(out).ok();

        writeln!(out, "    /** Initialize the plugin. */").ok();
        writeln!(out, "    void initialize() throws Exception;").ok();
        writeln!(out).ok();

        writeln!(out, "    /** Shut down the plugin. */").ok();
        writeln!(out, "    void shutdown() throws Exception;").ok();
        writeln!(out).ok();
    }

    // Trait methods
    for (method_name, return_type) in trait_methods {
        writeln!(out, "    /** Trait method: {}. */", method_name).ok();
        writeln!(out, "    {} {}();", return_type, method_name).ok();
        writeln!(out).ok();
    }

    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // --- Bridge class for FFI upcall stubs ---
    writeln!(out, "/**").ok();
    writeln!(
        out,
        " * Allocates Panama FFM upcall stubs for a {} trait implementation",
        trait_pascal
    )
    .ok();
    writeln!(out, " * and assembles the C vtable in native memory.").ok();
    writeln!(out, " */").ok();
    writeln!(out, "final class {}Bridge implements AutoCloseable {{", trait_pascal).ok();
    writeln!(out).ok();

    writeln!(out, "    private static final Linker LINKER = Linker.nativeLinker();").ok();
    writeln!(
        out,
        "    private static final MethodHandles.Lookup LOOKUP = MethodHandles.lookup();"
    )
    .ok();
    writeln!(out).ok();

    // Number of vtable fields: optionally name_fn, version_fn, initialize_fn, shutdown_fn,
    // then trait methods, then free_user_data.
    let num_methods = trait_methods.len();
    let num_super_slots = if has_super_trait { 4usize } else { 0usize };
    let num_vtable_fields = num_super_slots + num_methods + 1; // (super-trait methods +) trait methods + free_user_data
    writeln!(
        out,
        "    // C vtable: {} fields ({} plugin methods + {} trait methods + free_user_data)",
        num_vtable_fields, num_super_slots, num_methods
    )
    .ok();
    writeln!(
        out,
        "    private static final long VTABLE_SIZE = (long) ValueLayout.ADDRESS.byteSize() * {}L;",
        num_vtable_fields
    )
    .ok();
    writeln!(out).ok();

    writeln!(out, "    private final Arena arena;").ok();
    writeln!(out, "    private final MemorySegment vtable;").ok();
    writeln!(out, "    private final {} impl;", trait_pascal).ok();
    writeln!(out).ok();

    // Constructor
    writeln!(out, "    {}Bridge(final {} impl) {{", trait_pascal, trait_pascal).ok();
    writeln!(out, "        this.impl = impl;").ok();
    writeln!(out, "        this.arena = Arena.ofConfined();").ok();
    writeln!(out, "        this.vtable = arena.allocate(VTABLE_SIZE);").ok();
    writeln!(out).ok();
    writeln!(out, "        try {{").ok();
    writeln!(out, "            long offset = 0L;").ok();
    writeln!(out).ok();

    if has_super_trait {
        // Register name_fn
        writeln!(
            out,
            "            var stubName = LINKER.upcallStub(LOOKUP.bind(this, \"handleName\","
        )
        .ok();
        writeln!(out, "                MethodType.methodType(MemorySegment.class)),").ok();
        writeln!(
            out,
            "                FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS),"
        )
        .ok();
        writeln!(out, "                arena);").ok();
        writeln!(out, "            vtable.set(ValueLayout.ADDRESS, offset, stubName);").ok();
        writeln!(out, "            offset += ValueLayout.ADDRESS.byteSize();").ok();
        writeln!(out).ok();

        // Register version_fn
        writeln!(
            out,
            "            var stubVersion = LINKER.upcallStub(LOOKUP.bind(this, \"handleVersion\","
        )
        .ok();
        writeln!(out, "                MethodType.methodType(MemorySegment.class)),").ok();
        writeln!(
            out,
            "                FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS),"
        )
        .ok();
        writeln!(out, "                arena);").ok();
        writeln!(out, "            vtable.set(ValueLayout.ADDRESS, offset, stubVersion);").ok();
        writeln!(out, "            offset += ValueLayout.ADDRESS.byteSize();").ok();
        writeln!(out).ok();

        // Register initialize_fn
        writeln!(
            out,
            "            var stubInitialize = LINKER.upcallStub(LOOKUP.bind(this, \"handleInitialize\","
        )
        .ok();
        writeln!(out, "                MethodType.methodType(int.class)),").ok();
        writeln!(
            out,
            "                FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS),"
        )
        .ok();
        writeln!(out, "                arena);").ok();
        writeln!(
            out,
            "            vtable.set(ValueLayout.ADDRESS, offset, stubInitialize);"
        )
        .ok();
        writeln!(out, "            offset += ValueLayout.ADDRESS.byteSize();").ok();
        writeln!(out).ok();

        // Register shutdown_fn
        writeln!(
            out,
            "            var stubShutdown = LINKER.upcallStub(LOOKUP.bind(this, \"handleShutdown\","
        )
        .ok();
        writeln!(out, "                MethodType.methodType(int.class)),").ok();
        writeln!(
            out,
            "                FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS),"
        )
        .ok();
        writeln!(out, "                arena);").ok();
        writeln!(
            out,
            "            vtable.set(ValueLayout.ADDRESS, offset, stubShutdown);"
        )
        .ok();
        writeln!(out, "            offset += ValueLayout.ADDRESS.byteSize();").ok();
        writeln!(out).ok();
    }

    // Register trait methods
    for (method_name, _) in trait_methods {
        let handle_name = format!("handle{}", method_name.to_pascal_case());
        writeln!(
            out,
            "            var stub{} = LINKER.upcallStub(LOOKUP.bind(this, \"{}\",",
            method_name.to_pascal_case(),
            handle_name
        )
        .ok();
        writeln!(out, "                MethodType.methodType(MemorySegment.class)),").ok();
        writeln!(
            out,
            "                FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS),"
        )
        .ok();
        writeln!(out, "                arena);").ok();
        writeln!(
            out,
            "            vtable.set(ValueLayout.ADDRESS, offset, stub{});",
            method_name.to_pascal_case()
        )
        .ok();
        writeln!(out, "            offset += ValueLayout.ADDRESS.byteSize();").ok();
        writeln!(out).ok();
    }

    // Register free_user_data (NULL for now)
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

    // Accessor method
    writeln!(out, "    MemorySegment vtableSegment() {{").ok();
    writeln!(out, "        return vtable;").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    // Handle methods
    writeln!(
        out,
        "    // --- Upcall handlers (return MemorySegment pointing to allocated strings) ---"
    )
    .ok();
    writeln!(out).ok();

    if has_super_trait {
        writeln!(out, "    private MemorySegment handleName() {{").ok();
        writeln!(out, "        try {{").ok();
        writeln!(out, "            String name = impl.name();").ok();
        writeln!(out, "            return arena.allocateFrom(name);").ok();
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(out, "            return MemorySegment.NULL;").ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "    }}").ok();
        writeln!(out).ok();

        writeln!(out, "    private MemorySegment handleVersion() {{").ok();
        writeln!(out, "        try {{").ok();
        writeln!(out, "            String version = impl.version();").ok();
        writeln!(out, "            return arena.allocateFrom(version);").ok();
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(out, "            return MemorySegment.NULL;").ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "    }}").ok();
        writeln!(out).ok();

        writeln!(out, "    private int handleInitialize() {{").ok();
        writeln!(out, "        try {{").ok();
        writeln!(out, "            impl.initialize();").ok();
        writeln!(out, "            return 0;").ok();
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(out, "            return 1;").ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "    }}").ok();
        writeln!(out).ok();

        writeln!(out, "    private int handleShutdown() {{").ok();
        writeln!(out, "        try {{").ok();
        writeln!(out, "            impl.shutdown();").ok();
        writeln!(out, "            return 0;").ok();
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(out, "            return 1;").ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "    }}").ok();
        writeln!(out).ok();
    }

    // Trait method handlers
    for (method_name, return_type) in trait_methods {
        writeln!(
            out,
            "    private MemorySegment handle{}() {{",
            method_name.to_pascal_case()
        )
        .ok();
        writeln!(out, "        try {{").ok();
        if *return_type == "void" || *return_type == "Void" {
            writeln!(out, "            impl.{}();", method_name).ok();
            writeln!(out, "            return MemorySegment.NULL;").ok();
        } else {
            writeln!(out, "            {} result = impl.{}();", return_type, method_name).ok();
            writeln!(out, "            String json = new com.fasterxml.jackson.databind.ObjectMapper().writeValueAsString(result);").ok();
            writeln!(out, "            return arena.allocateFrom(json);").ok();
        }
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(out, "            return MemorySegment.NULL;").ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "    }}").ok();
        writeln!(out).ok();
    }

    writeln!(out, "    @Override").ok();
    writeln!(out, "    public void close() {{").ok();
    writeln!(out, "        arena.close();").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // --- Bridge registry (keeps arenas + upcall stubs alive) ---
    writeln!(
        out,
        "/** Registry of live {} bridges — keeps upcall stubs and arenas alive. */",
        trait_pascal
    )
    .ok();
    writeln!(
        out,
        "private static final java.util.concurrent.ConcurrentHashMap<String, {}Bridge> {}_BRIDGES",
        trait_pascal,
        trait_snake.to_uppercase()
    )
    .ok();
    writeln!(out, "    = new java.util.concurrent.ConcurrentHashMap<>();").ok();
    writeln!(out).ok();

    // --- Registration helpers ---
    writeln!(
        out,
        "/** Register a {} implementation via Panama FFM upcall stubs. */",
        trait_pascal
    )
    .ok();
    writeln!(
        out,
        "public static void register{}(final {} impl) throws Exception {{",
        trait_pascal, trait_pascal
    )
    .ok();
    // Do NOT use try-with-resources: the bridge arena must stay open for the lifetime
    // of the plugin. We store it in the static registry so it is not GC'd or closed early.
    writeln!(out, "    var bridge = new {}Bridge(impl);", trait_pascal).ok();
    writeln!(out, "    try {{").ok();
    writeln!(out, "        try (var nameArena = Arena.ofConfined()) {{").ok();
    writeln!(out, "            var nameCs = nameArena.allocateFrom(impl.name());").ok();
    writeln!(out, "            var outErrArena = Arena.ofConfined();").ok();
    writeln!(
        out,
        "            MemorySegment outErr = outErrArena.allocate(ValueLayout.ADDRESS);"
    )
    .ok();
    writeln!(
        out,
        "            int rc = (int) NativeLib.{}_REGISTER_{}.invoke(nameCs, bridge.vtableSegment(), MemorySegment.NULL, outErr);",
        prefix_upper,
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
        "                String msg = errPtr.equals(MemorySegment.NULL) ? \"registration failed (rc=\" + rc + \")\" : errPtr.reinterpret(Long.MAX_VALUE).getString(0);"
    )
    .ok();
    writeln!(
        out,
        "                throw new RuntimeException(\"register{}: \" + msg);",
        trait_pascal
    )
    .ok();
    writeln!(out, "            }}").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }} catch (Throwable t) {{").ok();
    writeln!(out, "        bridge.close();").ok();
    writeln!(out, "        throw t;").ok();
    writeln!(out, "    }}").ok();
    writeln!(
        out,
        "    {}_BRIDGES.put(impl.name(), bridge);",
        trait_snake.to_uppercase()
    )
    .ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    writeln!(out, "/** Unregister a {} implementation. */", trait_pascal).ok();
    writeln!(
        out,
        "public static void unregister{}(String name) throws Exception {{",
        trait_pascal
    )
    .ok();
    writeln!(out, "    try (var nameArena = Arena.ofConfined()) {{").ok();
    writeln!(out, "        var nameCs = nameArena.allocateFrom(name);").ok();
    writeln!(out, "        var outErrArena = Arena.ofConfined();").ok();
    writeln!(
        out,
        "        MemorySegment outErr = outErrArena.allocate(ValueLayout.ADDRESS);"
    )
    .ok();
    writeln!(
        out,
        "        int rc = (int) NativeLib.{}_UNREGISTER_{}.invoke(nameCs, outErr);",
        prefix_upper,
        trait_snake.to_uppercase()
    )
    .ok();
    writeln!(out, "        if (rc != 0) {{").ok();
    writeln!(
        out,
        "            MemorySegment errPtr = outErr.get(ValueLayout.ADDRESS, 0);"
    )
    .ok();
    writeln!(
        out,
        "            String msg = errPtr.equals(MemorySegment.NULL) ? \"unregistration failed (rc=\" + rc + \")\" : errPtr.reinterpret(Long.MAX_VALUE).getString(0);"
    )
    .ok();
    writeln!(
        out,
        "            throw new RuntimeException(\"unregister{}: \" + msg);",
        trait_pascal
    )
    .ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    // Close and remove the bridge after the C unregister call succeeds
    writeln!(
        out,
        "    {}Bridge old = {}_BRIDGES.remove(name);",
        trait_pascal,
        trait_snake.to_uppercase()
    )
    .ok();
    writeln!(out, "    if (old != null) {{ old.close(); }}").ok();
    writeln!(out, "}}").ok();

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gen_trait_bridge_basic() {
        let code = gen_trait_bridge("MyPlugin", "mylib", &[("doWork", "String"), ("getStatus", "int")], true);

        // Basic sanity checks
        assert!(code.contains("public interface MyPlugin"));
        assert!(code.contains("String name()"));
        assert!(code.contains("String version()"));
        assert!(code.contains("void initialize()"));
        assert!(code.contains("void shutdown()"));
        assert!(code.contains("doWork"));
        assert!(code.contains("getStatus"));
        assert!(code.contains("MyPluginBridge"));
        assert!(code.contains("registerMyPlugin"));
        assert!(code.contains("unregisterMyPlugin"));
    }

    #[test]
    fn test_gen_trait_bridge_vtable_stubs() {
        let code = gen_trait_bridge("Handler", "lib", &[], true);

        // Verify Panama FFM upcall stubs are generated
        assert!(code.contains("LINKER.upcallStub"));
        assert!(code.contains("handleName"));
        assert!(code.contains("handleVersion"));
        assert!(code.contains("handleInitialize"));
        assert!(code.contains("handleShutdown"));
    }

    #[test]
    fn test_gen_trait_bridge_lifecycle_methods() {
        let code = gen_trait_bridge("Processor", "pfx", &[("process", "Object")], true);

        // Verify Plugin lifecycle methods are present in Java interface
        assert!(code.contains("String name()"));
        assert!(code.contains("String version()"));
        assert!(code.contains("void initialize()"));
        assert!(code.contains("void shutdown()"));
    }

    #[test]
    fn test_gen_trait_bridge_no_super_trait_omits_lifecycle() {
        let code = gen_trait_bridge("Transformer", "lib", &[("transform", "String")], false);

        // Without super_trait, no lifecycle slots should be emitted
        assert!(
            !code.contains("String name()"),
            "no lifecycle methods without super_trait"
        );
        assert!(
            !code.contains("handleName"),
            "no lifecycle handlers without super_trait"
        );
        assert!(code.contains("transform"), "trait method must still be emitted");
    }
}
