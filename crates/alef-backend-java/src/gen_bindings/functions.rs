use crate::type_map::{java_boxed_type, java_type};
use ahash::AHashSet;
use alef_codegen::naming::to_java_name;
use alef_core::config::AlefConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, FunctionDef, TypeRef};
use heck::ToSnakeCase;
use std::collections::HashSet;
use std::fmt::Write;

use super::helpers::is_bridge_param_java;
use super::marshal::{
    ffi_param_name, gen_ffi_layout, gen_function_descriptor, gen_helper_methods, is_ffi_string_return,
    java_ffi_return_cast, marshal_param_to_ffi,
};

pub(crate) fn gen_native_lib(
    api: &ApiSurface,
    config: &AlefConfig,
    package: &str,
    prefix: &str,
    has_visitor_bridge: bool,
) -> String {
    // Generate the class body first, then scan it to determine which imports are needed.
    let mut body = String::with_capacity(2048);
    // Derive the native library name from the FFI output path (directory name with hyphens replaced
    // by underscores), falling back to `{ffi_prefix}_ffi`.
    let lib_name = config.ffi_lib_name();

    writeln!(body, "final class NativeLib {{").ok();
    writeln!(body, "    private static final Linker LINKER = Linker.nativeLinker();").ok();
    writeln!(body, "    private static final SymbolLookup LIB;").ok();
    writeln!(
        body,
        "    private static final String NATIVES_RESOURCE_ROOT = \"/natives\";"
    )
    .ok();
    writeln!(
        body,
        "    private static final Object NATIVE_EXTRACT_LOCK = new Object();"
    )
    .ok();
    writeln!(body, "    private static String cachedExtractKey;").ok();
    writeln!(body, "    private static Path cachedExtractDir;").ok();
    writeln!(body).ok();
    writeln!(body, "    static {{").ok();
    writeln!(body, "        loadNativeLibrary();").ok();
    writeln!(body, "        LIB = SymbolLookup.loaderLookup();").ok();
    writeln!(body, "    }}").ok();
    writeln!(body).ok();
    writeln!(body, "    private static void loadNativeLibrary() {{").ok();
    writeln!(
        body,
        "        String osName = System.getProperty(\"os.name\", \"\").toLowerCase(java.util.Locale.ROOT);"
    )
    .ok();
    writeln!(
        body,
        "        String osArch = System.getProperty(\"os.arch\", \"\").toLowerCase(java.util.Locale.ROOT);"
    )
    .ok();
    writeln!(body).ok();
    writeln!(body, "        String libName;").ok();
    writeln!(body, "        String libExt;").ok();
    writeln!(
        body,
        "        if (osName.contains(\"mac\") || osName.contains(\"darwin\")) {{"
    )
    .ok();
    writeln!(body, "            libName = \"lib{}\";", lib_name).ok();
    writeln!(body, "            libExt = \".dylib\";").ok();
    writeln!(body, "        }} else if (osName.contains(\"win\")) {{").ok();
    writeln!(body, "            libName = \"{}\";", lib_name).ok();
    writeln!(body, "            libExt = \".dll\";").ok();
    writeln!(body, "        }} else {{").ok();
    writeln!(body, "            libName = \"lib{}\";", lib_name).ok();
    writeln!(body, "            libExt = \".so\";").ok();
    writeln!(body, "        }}").ok();
    writeln!(body).ok();
    writeln!(body, "        String nativesRid = resolveNativesRid(osName, osArch);").ok();
    writeln!(
        body,
        "        String nativesDir = NATIVES_RESOURCE_ROOT + \"/\" + nativesRid;"
    )
    .ok();
    writeln!(body).ok();
    writeln!(
        body,
        "        Path extracted = tryExtractAndLoadFromResources(nativesDir, libName, libExt);"
    )
    .ok();
    writeln!(body, "        if (extracted != null) {{").ok();
    writeln!(body, "            return;").ok();
    writeln!(body, "        }}").ok();
    writeln!(body).ok();
    writeln!(body, "        try {{").ok();
    writeln!(body, "            System.loadLibrary(\"{}\");", lib_name).ok();
    writeln!(body, "        }} catch (UnsatisfiedLinkError e) {{").ok();
    writeln!(
        body,
        "            String msg = \"Failed to load {} native library. Expected resource: \" + nativesDir + \"/\" + libName",
        lib_name
    ).ok();
    writeln!(
        body,
        "                    + libExt + \" (RID: \" + nativesRid + \"). \""
    )
    .ok();
    writeln!(
        body,
        "                    + \"Ensure the library is bundled in the JAR under natives/{{os-arch}}/, \""
    )
    .ok();
    writeln!(
        body,
        "                    + \"or place it on the system library path (java.library.path).\";",
    )
    .ok();
    writeln!(
        body,
        "            UnsatisfiedLinkError out = new UnsatisfiedLinkError(msg + \" Original error: \" + e.getMessage());"
    )
    .ok();
    writeln!(body, "            out.initCause(e);").ok();
    writeln!(body, "            throw out;").ok();
    writeln!(body, "        }}").ok();
    writeln!(body, "    }}").ok();
    writeln!(body).ok();
    writeln!(
        body,
        "    private static Path tryExtractAndLoadFromResources(String nativesDir, String libName, String libExt) {{"
    )
    .ok();
    writeln!(
        body,
        "        String resourcePath = nativesDir + \"/\" + libName + libExt;"
    )
    .ok();
    writeln!(
        body,
        "        URL resource = NativeLib.class.getResource(resourcePath);"
    )
    .ok();
    writeln!(body, "        if (resource == null) {{").ok();
    writeln!(body, "            return null;").ok();
    writeln!(body, "        }}").ok();
    writeln!(body).ok();
    writeln!(body, "        try {{").ok();
    writeln!(
        body,
        "            Path tempDir = extractOrReuseNativeDirectory(nativesDir);"
    )
    .ok();
    writeln!(body, "            Path libPath = tempDir.resolve(libName + libExt);").ok();
    writeln!(body, "            if (!Files.exists(libPath)) {{").ok();
    writeln!(
        body,
        "                throw new UnsatisfiedLinkError(\"Missing extracted native library: \" + libPath);"
    )
    .ok();
    writeln!(body, "            }}").ok();
    writeln!(body, "            System.load(libPath.toAbsolutePath().toString());").ok();
    writeln!(body, "            return libPath;").ok();
    writeln!(body, "        }} catch (Exception e) {{").ok();
    writeln!(body, "            System.err.println(\"[NativeLib] Failed to extract and load native library from resources: \" + e.getMessage());").ok();
    writeln!(body, "            return null;").ok();
    writeln!(body, "        }}").ok();
    writeln!(body, "    }}").ok();
    writeln!(body).ok();
    writeln!(
        body,
        "    private static Path extractOrReuseNativeDirectory(String nativesDir) throws Exception {{"
    )
    .ok();
    writeln!(
        body,
        "        URL location = NativeLib.class.getProtectionDomain().getCodeSource().getLocation();"
    )
    .ok();
    writeln!(body, "        if (location == null) {{").ok();
    writeln!(
        body,
        "            throw new IllegalStateException(\"Missing code source location for {} JAR\");",
        lib_name
    )
    .ok();
    writeln!(body, "        }}").ok();
    writeln!(body).ok();
    writeln!(body, "        Path codePath = Path.of(location.toURI());").ok();
    writeln!(
        body,
        "        String key = codePath.toAbsolutePath() + \"::\" + nativesDir;"
    )
    .ok();
    writeln!(body).ok();
    writeln!(body, "        synchronized (NATIVE_EXTRACT_LOCK) {{").ok();
    writeln!(
        body,
        "            if (cachedExtractDir != null && key.equals(cachedExtractKey)) {{"
    )
    .ok();
    writeln!(body, "                return cachedExtractDir;").ok();
    writeln!(body, "            }}").ok();
    writeln!(
        body,
        "            Path tempDir = Files.createTempDirectory(\"{}_native\");",
        lib_name
    )
    .ok();
    writeln!(body, "            tempDir.toFile().deleteOnExit();").ok();
    writeln!(
        body,
        "            List<Path> extracted = extractNativeDirectory(codePath, nativesDir, tempDir);"
    )
    .ok();
    writeln!(body, "            if (extracted.isEmpty()) {{").ok();
    writeln!(body, "                throw new IllegalStateException(\"No native files extracted from resources dir: \" + nativesDir);").ok();
    writeln!(body, "            }}").ok();
    writeln!(body, "            cachedExtractKey = key;").ok();
    writeln!(body, "            cachedExtractDir = tempDir;").ok();
    writeln!(body, "            return tempDir;").ok();
    writeln!(body, "        }}").ok();
    writeln!(body, "    }}").ok();
    writeln!(body).ok();
    writeln!(body, "    private static List<Path> extractNativeDirectory(Path codePath, String nativesDir, Path destDir) throws Exception {{").ok();
    writeln!(
        body,
        "        if (!Files.exists(destDir) || !Files.isDirectory(destDir)) {{"
    )
    .ok();
    writeln!(
        body,
        "            throw new IllegalArgumentException(\"Destination directory does not exist: \" + destDir);"
    )
    .ok();
    writeln!(body, "        }}").ok();
    writeln!(body).ok();
    writeln!(
        body,
        "        String prefix = nativesDir.startsWith(\"/\") ? nativesDir.substring(1) : nativesDir;"
    )
    .ok();
    writeln!(body, "        if (!prefix.endsWith(\"/\")) {{").ok();
    writeln!(body, "            prefix = prefix + \"/\";").ok();
    writeln!(body, "        }}").ok();
    writeln!(body).ok();
    writeln!(body, "        if (Files.isDirectory(codePath)) {{").ok();
    writeln!(body, "            Path nativesPath = codePath.resolve(prefix);").ok();
    writeln!(
        body,
        "            if (!Files.exists(nativesPath) || !Files.isDirectory(nativesPath)) {{"
    )
    .ok();
    writeln!(body, "                return List.of();").ok();
    writeln!(body, "            }}").ok();
    writeln!(body, "            return copyDirectory(nativesPath, destDir);").ok();
    writeln!(body, "        }}").ok();
    writeln!(body).ok();
    writeln!(body, "        List<Path> extracted = new ArrayList<>();").ok();
    writeln!(body, "        try (JarFile jar = new JarFile(codePath.toFile())) {{").ok();
    writeln!(body, "            Enumeration<JarEntry> entries = jar.entries();").ok();
    writeln!(body, "            while (entries.hasMoreElements()) {{").ok();
    writeln!(body, "                JarEntry entry = entries.nextElement();").ok();
    writeln!(body, "                String name = entry.getName();").ok();
    writeln!(
        body,
        "                if (!name.startsWith(prefix) || entry.isDirectory()) {{"
    )
    .ok();
    writeln!(body, "                    continue;").ok();
    writeln!(body, "                }}").ok();
    writeln!(
        body,
        "                String relative = name.substring(prefix.length());"
    )
    .ok();
    writeln!(body, "                Path out = safeResolve(destDir, relative);").ok();
    writeln!(body, "                Files.createDirectories(out.getParent());").ok();
    writeln!(body, "                try (var in = jar.getInputStream(entry)) {{").ok();
    writeln!(
        body,
        "                    Files.copy(in, out, StandardCopyOption.REPLACE_EXISTING);"
    )
    .ok();
    writeln!(body, "                }}").ok();
    writeln!(body, "                out.toFile().deleteOnExit();").ok();
    writeln!(body, "                extracted.add(out);").ok();
    writeln!(body, "            }}").ok();
    writeln!(body, "        }}").ok();
    writeln!(body, "        return extracted;").ok();
    writeln!(body, "    }}").ok();
    writeln!(body).ok();
    writeln!(
        body,
        "    private static List<Path> copyDirectory(Path srcDir, Path destDir) throws Exception {{"
    )
    .ok();
    writeln!(body, "        List<Path> copied = new ArrayList<>();").ok();
    writeln!(body, "        try (var paths = Files.walk(srcDir)) {{").ok();
    writeln!(body, "            for (Path src : (Iterable<Path>) paths::iterator) {{").ok();
    writeln!(body, "                if (Files.isDirectory(src)) {{").ok();
    writeln!(body, "                    continue;").ok();
    writeln!(body, "                }}").ok();
    writeln!(body, "                Path relative = srcDir.relativize(src);").ok();
    writeln!(
        body,
        "                Path out = safeResolve(destDir, relative.toString());"
    )
    .ok();
    writeln!(body, "                Files.createDirectories(out.getParent());").ok();
    writeln!(
        body,
        "                Files.copy(src, out, StandardCopyOption.REPLACE_EXISTING);"
    )
    .ok();
    writeln!(body, "                out.toFile().deleteOnExit();").ok();
    writeln!(body, "                copied.add(out);").ok();
    writeln!(body, "            }}").ok();
    writeln!(body, "        }}").ok();
    writeln!(body, "        return copied;").ok();
    writeln!(body, "    }}").ok();
    writeln!(body).ok();
    writeln!(
        body,
        "    private static Path safeResolve(Path destDir, String relative) throws Exception {{"
    )
    .ok();
    writeln!(
        body,
        "        Path normalizedDest = destDir.toAbsolutePath().normalize();"
    )
    .ok();
    writeln!(body, "        Path out = normalizedDest.resolve(relative).normalize();").ok();
    writeln!(body, "        if (!out.startsWith(normalizedDest)) {{").ok();
    writeln!(body, "            throw new SecurityException(\"Blocked extracting native file outside destination directory: \" + relative);").ok();
    writeln!(body, "        }}").ok();
    writeln!(body, "        return out;").ok();
    writeln!(body, "    }}").ok();
    writeln!(body).ok();
    writeln!(
        body,
        "    private static String resolveNativesRid(String osName, String osArch) {{"
    )
    .ok();
    writeln!(body, "        String arch;").ok();
    writeln!(
        body,
        "        if (osArch.contains(\"aarch64\") || osArch.contains(\"arm64\")) {{"
    )
    .ok();
    writeln!(body, "            arch = \"arm64\";").ok();
    writeln!(
        body,
        "        }} else if (osArch.contains(\"x86_64\") || osArch.contains(\"amd64\")) {{"
    )
    .ok();
    writeln!(body, "            arch = \"x86_64\";").ok();
    writeln!(body, "        }} else {{").ok();
    writeln!(body, "            arch = osArch.replaceAll(\"[^a-z0-9_]+\", \"\");").ok();
    writeln!(body, "        }}").ok();
    writeln!(body).ok();
    writeln!(body, "        String os;").ok();
    writeln!(
        body,
        "        if (osName.contains(\"mac\") || osName.contains(\"darwin\")) {{"
    )
    .ok();
    writeln!(body, "            os = \"macos\";").ok();
    writeln!(body, "        }} else if (osName.contains(\"win\")) {{").ok();
    writeln!(body, "            os = \"windows\";").ok();
    writeln!(body, "        }} else {{").ok();
    writeln!(body, "            os = \"linux\";").ok();
    writeln!(body, "        }}").ok();
    writeln!(body).ok();
    writeln!(body, "        return os + \"-\" + arch;").ok();
    writeln!(body, "    }}").ok();
    writeln!(body).ok();

    // Generate method handles for free functions.
    // All functions get handles regardless of is_async — the FFI layer always exposes
    // synchronous C functions, and the Java async wrapper delegates to the sync method.
    for func in &api.functions {
        let ffi_name = format!("{}_{}", prefix, func.name.to_lowercase());
        let return_layout = gen_ffi_layout(&func.return_type);
        let param_layouts: Vec<String> = func.params.iter().map(|p| gen_ffi_layout(&p.ty)).collect();

        let layout_str = gen_function_descriptor(&return_layout, &param_layouts);

        let handle_name = format!("{}_{}", prefix.to_uppercase(), func.name.to_uppercase());

        writeln!(
            body,
            "    static final MethodHandle {} = LINKER.downcallHandle(",
            handle_name
        )
        .ok();
        writeln!(body, "        LIB.find(\"{}\").orElseThrow(),", ffi_name).ok();
        writeln!(body, "        {}", layout_str).ok();
        writeln!(body, "    );").ok();
    }

    // free_string handle for releasing FFI-allocated strings
    {
        let free_name = format!("{}_free_string", prefix);
        let handle_name = format!("{}_FREE_STRING", prefix.to_uppercase());
        writeln!(body).ok();
        writeln!(
            body,
            "    static final MethodHandle {} = LINKER.downcallHandle(",
            handle_name
        )
        .ok();
        writeln!(body, "        LIB.find(\"{}\").orElseThrow(),", free_name).ok();
        writeln!(body, "        FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)").ok();
        writeln!(body, "    );").ok();
    }

    // Error handling — use the FFI's last_error_code and last_error_context symbols
    {
        writeln!(
            body,
            "    static final MethodHandle {}_LAST_ERROR_CODE = LINKER.downcallHandle(",
            prefix.to_uppercase()
        )
        .ok();
        writeln!(body, "        LIB.find(\"{}_last_error_code\").orElseThrow(),", prefix).ok();
        writeln!(body, "        FunctionDescriptor.of(ValueLayout.JAVA_INT)").ok();
        writeln!(body, "    );").ok();

        writeln!(
            body,
            "    static final MethodHandle {}_LAST_ERROR_CONTEXT = LINKER.downcallHandle(",
            prefix.to_uppercase()
        )
        .ok();
        writeln!(
            body,
            "        LIB.find(\"{}_last_error_context\").orElseThrow(),",
            prefix
        )
        .ok();
        writeln!(body, "        FunctionDescriptor.of(ValueLayout.ADDRESS)").ok();
        writeln!(body, "    );").ok();
    }

    // Track emitted free handles to avoid duplicates (a type may appear both as
    // a function return type AND as an opaque type).
    let mut emitted_free_handles: AHashSet<String> = AHashSet::new();

    // Build the set of opaque type names so we can pick the right accessor below.
    let opaque_type_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    // Accessor handles for Named return types (struct pointer → field accessor + free)
    for func in &api.functions {
        if let TypeRef::Named(name) = &func.return_type {
            let type_snake = name.to_snake_case();
            let type_upper = type_snake.to_uppercase();
            let is_opaque = opaque_type_names.contains(name.as_str());

            if is_opaque {
                // Opaque handles: the caller wraps the pointer directly, no JSON needed.
                // No content accessor is emitted for opaque types.
            } else {
                // Non-opaque record types: use _to_json to serialize the full struct to JSON,
                // which the Java side then deserializes with ObjectMapper.
                // NOTE: _content returns only the markdown string field, not the full JSON.
                let to_json_handle = format!("{}_{}_TO_JSON", prefix.to_uppercase(), type_upper);
                let to_json_ffi = format!("{}_{}_to_json", prefix, type_snake);
                writeln!(body).ok();
                writeln!(
                    body,
                    "    static final MethodHandle {} = LINKER.downcallHandle(",
                    to_json_handle
                )
                .ok();
                writeln!(body, "        LIB.find(\"{}\").orElseThrow(),", to_json_ffi).ok();
                writeln!(
                    body,
                    "        FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS)"
                )
                .ok();
                writeln!(body, "    );").ok();
            }

            // _free: (struct_ptr) -> void
            let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_upper);
            let free_ffi = format!("{}_{}_free", prefix, type_snake);
            if emitted_free_handles.insert(free_handle.clone()) {
                writeln!(body).ok();
                writeln!(
                    body,
                    "    static final MethodHandle {} = LINKER.downcallHandle(",
                    free_handle
                )
                .ok();
                writeln!(body, "        LIB.find(\"{}\").orElseThrow(),", free_ffi).ok();
                writeln!(body, "        FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)").ok();
                writeln!(body, "    );").ok();
            }
        }
    }

    // FROM_JSON + FREE handles for non-opaque Named types used as parameters.
    // These allow serializing a Java record to JSON and passing it to the FFI.
    let mut emitted_from_json_handles: AHashSet<String> = AHashSet::new();
    for func in &api.functions {
        for param in &func.params {
            // Handle both Named and Optional<Named> params
            let inner_name = match &param.ty {
                TypeRef::Named(n) => Some(n.clone()),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        Some(n.clone())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(name) = inner_name {
                if !opaque_type_names.contains(name.as_str()) {
                    let type_snake = name.to_snake_case();
                    let type_upper = type_snake.to_uppercase();

                    // _from_json: (char*) -> struct_ptr
                    let from_json_handle = format!("{}_{}_FROM_JSON", prefix.to_uppercase(), type_upper);
                    let from_json_ffi = format!("{}_{}_from_json", prefix, type_snake);
                    if emitted_from_json_handles.insert(from_json_handle.clone()) {
                        writeln!(body).ok();
                        writeln!(
                            body,
                            "    static final MethodHandle {} = LINKER.downcallHandle(",
                            from_json_handle
                        )
                        .ok();
                        writeln!(body, "        LIB.find(\"{}\").orElseThrow(),", from_json_ffi).ok();
                        writeln!(
                            body,
                            "        FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS)"
                        )
                        .ok();
                        writeln!(body, "    );").ok();
                    }

                    // _free: (struct_ptr) -> void
                    let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_upper);
                    let free_ffi = format!("{}_{}_free", prefix, type_snake);
                    if emitted_free_handles.insert(free_handle.clone()) {
                        writeln!(body).ok();
                        writeln!(
                            body,
                            "    static final MethodHandle {} = LINKER.downcallHandle(",
                            free_handle
                        )
                        .ok();
                        writeln!(body, "        LIB.find(\"{}\").orElseThrow(),", free_ffi).ok();
                        writeln!(body, "        FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)").ok();
                        writeln!(body, "    );").ok();
                    }
                }
            }
        }
    }

    // Collect builder class names from record types with defaults, so we skip
    // opaque types that are superseded by a pure-Java builder class.
    let builder_class_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| !t.is_opaque && !t.fields.is_empty() && t.has_default)
        .map(|t| format!("{}Builder", t.name))
        .collect();

    // Free handles for opaque types (handle pointer → void)
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if typ.is_opaque && !builder_class_names.contains(&typ.name) {
            let type_snake = typ.name.to_snake_case();
            let type_upper = type_snake.to_uppercase();
            let free_handle = format!("{}_{}_FREE", prefix.to_uppercase(), type_upper);
            let free_ffi = format!("{}_{}_free", prefix, type_snake);
            if emitted_free_handles.insert(free_handle.clone()) {
                writeln!(body).ok();
                writeln!(
                    body,
                    "    static final MethodHandle {} = LINKER.downcallHandle(",
                    free_handle
                )
                .ok();
                writeln!(body, "        LIB.find(\"{}\").orElseThrow(),", free_ffi).ok();
                writeln!(body, "        FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)").ok();
                writeln!(body, "    );").ok();
            }
        }
    }

    // Inject visitor FFI method handles when a trait bridge is configured.
    if has_visitor_bridge {
        body.push_str(&crate::gen_visitor::gen_native_lib_visitor_handles(prefix));
    }

    writeln!(body, "}}").ok();

    // Now assemble the file with only the imports that are actually used in the body.
    let mut out = String::with_capacity(body.len() + 512);

    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "package {};", package).ok();
    writeln!(out).ok();
    if body.contains("Arena") {
        writeln!(out, "import java.lang.foreign.Arena;").ok();
    }
    if body.contains("FunctionDescriptor") {
        writeln!(out, "import java.lang.foreign.FunctionDescriptor;").ok();
    }
    if body.contains("Linker") {
        writeln!(out, "import java.lang.foreign.Linker;").ok();
    }
    if body.contains("MemorySegment") {
        writeln!(out, "import java.lang.foreign.MemorySegment;").ok();
    }
    if body.contains("SymbolLookup") {
        writeln!(out, "import java.lang.foreign.SymbolLookup;").ok();
    }
    if body.contains("ValueLayout") {
        writeln!(out, "import java.lang.foreign.ValueLayout;").ok();
    }
    if body.contains("MethodHandle") {
        writeln!(out, "import java.lang.invoke.MethodHandle;").ok();
    }
    // Imports required by the JAR-extraction native loader (always present).
    writeln!(out, "import java.net.URL;").ok();
    writeln!(out, "import java.nio.file.Files;").ok();
    writeln!(out, "import java.nio.file.Path;").ok();
    writeln!(out, "import java.nio.file.StandardCopyOption;").ok();
    writeln!(out, "import java.util.ArrayList;").ok();
    writeln!(out, "import java.util.Enumeration;").ok();
    writeln!(out, "import java.util.List;").ok();
    writeln!(out, "import java.util.jar.JarEntry;").ok();
    writeln!(out, "import java.util.jar.JarFile;").ok();
    writeln!(out).ok();

    out.push_str(&body);

    out
}

// ---------------------------------------------------------------------------
// Main wrapper class
// ---------------------------------------------------------------------------

pub(crate) fn gen_main_class(
    api: &ApiSurface,
    _config: &AlefConfig,
    package: &str,
    class_name: &str,
    prefix: &str,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    has_visitor_bridge: bool,
) -> String {
    // Build the set of opaque type names so we can distinguish opaque handles from records
    let opaque_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    // Generate the class body first, then scan it to determine which imports are needed.
    let mut body = String::with_capacity(4096);

    writeln!(body, "public final class {} {{", class_name).ok();
    writeln!(body, "    private {}() {{ }}", class_name).ok();
    writeln!(body).ok();

    // Generate static methods for free functions
    for func in &api.functions {
        // Always generate sync method (bridge params stripped from signature)
        gen_sync_function_method(
            &mut body,
            func,
            prefix,
            class_name,
            &opaque_types,
            bridge_param_names,
            bridge_type_aliases,
        );
        writeln!(body).ok();

        // Also generate async wrapper if marked as async
        if func.is_async {
            gen_async_wrapper_method(&mut body, func, bridge_param_names, bridge_type_aliases);
            writeln!(body).ok();
        }
    }

    // Inject convertWithVisitor when a visitor bridge is configured.
    if has_visitor_bridge {
        body.push_str(&crate::gen_visitor::gen_convert_with_visitor_method(class_name, prefix));
        writeln!(body).ok();
    }

    // Add helper methods only if they are referenced in the body
    gen_helper_methods(&mut body, prefix, class_name);

    writeln!(body, "}}").ok();

    // Now assemble the file with only the imports that are actually used in the body.
    let mut out = String::with_capacity(body.len() + 512);

    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "package {};", package).ok();
    writeln!(out).ok();
    if body.contains("Arena") {
        writeln!(out, "import java.lang.foreign.Arena;").ok();
    }
    if body.contains("FunctionDescriptor") {
        writeln!(out, "import java.lang.foreign.FunctionDescriptor;").ok();
    }
    if body.contains("Linker") {
        writeln!(out, "import java.lang.foreign.Linker;").ok();
    }
    if body.contains("MemorySegment") {
        writeln!(out, "import java.lang.foreign.MemorySegment;").ok();
    }
    if body.contains("SymbolLookup") {
        writeln!(out, "import java.lang.foreign.SymbolLookup;").ok();
    }
    if body.contains("ValueLayout") {
        writeln!(out, "import java.lang.foreign.ValueLayout;").ok();
    }
    if body.contains("List<") {
        writeln!(out, "import java.util.List;").ok();
    }
    if body.contains("Map<") {
        writeln!(out, "import java.util.Map;").ok();
    }
    if body.contains("Optional<") {
        writeln!(out, "import java.util.Optional;").ok();
    }
    if body.contains("HashMap<") || body.contains("new HashMap") {
        writeln!(out, "import java.util.HashMap;").ok();
    }
    if body.contains("CompletableFuture") {
        writeln!(out, "import java.util.concurrent.CompletableFuture;").ok();
    }
    if body.contains("CompletionException") {
        writeln!(out, "import java.util.concurrent.CompletionException;").ok();
    }
    // Only import the short name `ObjectMapper` when it's used as a type reference (not just via
    // `createObjectMapper()` which uses fully qualified names internally).
    // Check for " ObjectMapper" (space before) which indicates use as a type, not a method name suffix.
    if body.contains(" ObjectMapper") {
        writeln!(out, "import com.fasterxml.jackson.databind.ObjectMapper;").ok();
    }
    writeln!(out).ok();

    out.push_str(&body);

    out
}

pub(crate) fn gen_sync_function_method(
    out: &mut String,
    func: &FunctionDef,
    prefix: &str,
    class_name: &str,
    opaque_types: &AHashSet<String>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) {
    // Exclude bridge params from the public Java signature.
    let params: Vec<String> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
        .map(|p| {
            let ptype = java_type(&p.ty);
            format!("final {} {}", ptype, to_java_name(&p.name))
        })
        .collect();

    let return_type = java_type(&func.return_type);

    writeln!(
        out,
        "    public static {} {}({}) throws {}Exception {{",
        return_type,
        to_java_name(&func.name),
        params.join(", "),
        class_name
    )
    .ok();

    writeln!(out, "        try (var arena = Arena.ofConfined()) {{").ok();

    // Collect non-opaque Named params that need FFI pointer cleanup after the call.
    // These are Rust-allocated by _from_json and must be freed with _free.
    // Bridge params are excluded — they are passed as NULL.
    let ffi_ptr_params: Vec<(String, String)> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
        .filter_map(|p| {
            let inner_name = match &p.ty {
                TypeRef::Named(n) if !opaque_types.contains(n.as_str()) => Some(n.clone()),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        if !opaque_types.contains(n.as_str()) {
                            Some(n.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };
            inner_name.map(|type_name| {
                let cname = "c".to_string() + &to_java_name(&p.name);
                let type_snake = type_name.to_snake_case();
                let free_handle = format!("NativeLib.{}_{}_FREE", prefix.to_uppercase(), type_snake.to_uppercase());
                (cname, free_handle)
            })
        })
        .collect();

    // Marshal non-bridge parameters (use camelCase Java names)
    for param in &func.params {
        if is_bridge_param_java(param, bridge_param_names, bridge_type_aliases) {
            continue;
        }
        marshal_param_to_ffi(out, &to_java_name(&param.name), &param.ty, opaque_types, prefix);
    }

    // Call FFI
    let ffi_handle = format!("NativeLib.{}_{}", prefix.to_uppercase(), func.name.to_uppercase());

    // Build call args: bridge params get MemorySegment.NULL, others are marshalled normally.
    let call_args: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            if is_bridge_param_java(p, bridge_param_names, bridge_type_aliases) {
                "MemorySegment.NULL".to_string()
            } else {
                ffi_param_name(&to_java_name(&p.name), &p.ty, opaque_types)
            }
        })
        .collect();

    // Emit a helper closure to free FFI-allocated param pointers (e.g. options created by _from_json)
    let emit_ffi_ptr_cleanup = |out: &mut String| {
        for (cname, free_handle) in &ffi_ptr_params {
            writeln!(out, "            if (!{}.equals(MemorySegment.NULL)) {{", cname).ok();
            writeln!(out, "                {}.invoke({});", free_handle, cname).ok();
            writeln!(out, "            }}").ok();
        }
    };

    if matches!(func.return_type, TypeRef::Unit) {
        writeln!(out, "            {}.invoke({});", ffi_handle, call_args.join(", ")).ok();
        emit_ffi_ptr_cleanup(out);
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(
            out,
            "            throw new {}Exception(\"FFI call failed\", e);",
            class_name
        )
        .ok();
        writeln!(out, "        }}").ok();
    } else if is_ffi_string_return(&func.return_type) {
        let free_handle = format!("NativeLib.{}_FREE_STRING", prefix.to_uppercase());
        writeln!(
            out,
            "            var resultPtr = (MemorySegment) {}.invoke({});",
            ffi_handle,
            call_args.join(", ")
        )
        .ok();
        emit_ffi_ptr_cleanup(out);
        writeln!(out, "            if (resultPtr.equals(MemorySegment.NULL)) {{").ok();
        writeln!(out, "                checkLastError();").ok();
        writeln!(out, "                return null;").ok();
        writeln!(out, "            }}").ok();
        writeln!(
            out,
            "            String result = resultPtr.reinterpret(Long.MAX_VALUE).getString(0);"
        )
        .ok();
        writeln!(out, "            {}.invoke(resultPtr);", free_handle).ok();
        writeln!(out, "            return result;").ok();
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(
            out,
            "            throw new {}Exception(\"FFI call failed\", e);",
            class_name
        )
        .ok();
        writeln!(out, "        }}").ok();
    } else if matches!(func.return_type, TypeRef::Named(_)) {
        // Named return types: FFI returns a struct pointer.
        let return_type_name = match &func.return_type {
            TypeRef::Named(name) => name,
            _ => unreachable!(),
        };
        let is_opaque = opaque_types.contains(return_type_name.as_str());

        writeln!(
            out,
            "            var resultPtr = (MemorySegment) {}.invoke({});",
            ffi_handle,
            call_args.join(", ")
        )
        .ok();
        emit_ffi_ptr_cleanup(out);
        writeln!(out, "            if (resultPtr.equals(MemorySegment.NULL)) {{").ok();
        writeln!(out, "                checkLastError();").ok();
        writeln!(out, "                return null;").ok();
        writeln!(out, "            }}").ok();

        if is_opaque {
            // Opaque handles: wrap the raw pointer directly, caller owns and will close()
            writeln!(out, "            return new {}(resultPtr);", return_type_name).ok();
        } else {
            // Record types: use _to_json to serialize the full struct to JSON, then deserialize.
            // NOTE: _content only returns the markdown string field, not a full JSON object.
            let type_snake = return_type_name.to_snake_case();
            let free_handle = format!("NativeLib.{}_{}_FREE", prefix.to_uppercase(), type_snake.to_uppercase());
            let to_json_handle = format!(
                "NativeLib.{}_{}_TO_JSON",
                prefix.to_uppercase(),
                type_snake.to_uppercase()
            );
            writeln!(
                out,
                "            var jsonPtr = (MemorySegment) {}.invoke(resultPtr);",
                to_json_handle
            )
            .ok();
            writeln!(out, "            {}.invoke(resultPtr);", free_handle).ok();
            writeln!(out, "            if (jsonPtr.equals(MemorySegment.NULL)) {{").ok();
            writeln!(out, "                checkLastError();").ok();
            writeln!(out, "                return null;").ok();
            writeln!(out, "            }}").ok();
            writeln!(
                out,
                "            String json = jsonPtr.reinterpret(Long.MAX_VALUE).getString(0);"
            )
            .ok();
            writeln!(
                out,
                "            NativeLib.{}_FREE_STRING.invoke(jsonPtr);",
                prefix.to_uppercase()
            )
            .ok();
            writeln!(
                out,
                "            return createObjectMapper().readValue(json, {}.class);",
                return_type_name
            )
            .ok();
        }

        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(
            out,
            "            throw new {}Exception(\"FFI call failed\", e);",
            class_name
        )
        .ok();
        writeln!(out, "        }}").ok();
    } else if matches!(func.return_type, TypeRef::Vec(_)) {
        // Vec return types: FFI returns a JSON string pointer; deserialize into List<T>.
        let free_handle = format!("NativeLib.{}_FREE_STRING", prefix.to_uppercase());
        writeln!(
            out,
            "            var resultPtr = (MemorySegment) {}.invoke({});",
            ffi_handle,
            call_args.join(", ")
        )
        .ok();
        emit_ffi_ptr_cleanup(out);
        writeln!(out, "            if (resultPtr.equals(MemorySegment.NULL)) {{").ok();
        writeln!(out, "                return java.util.List.of();").ok();
        writeln!(out, "            }}").ok();
        writeln!(
            out,
            "            String json = resultPtr.reinterpret(Long.MAX_VALUE).getString(0);"
        )
        .ok();
        writeln!(out, "            {}.invoke(resultPtr);", free_handle).ok();
        // Determine the element type for deserialization
        let element_type = match &func.return_type {
            TypeRef::Vec(inner) => java_type(inner),
            _ => unreachable!(),
        };
        writeln!(
            out,
            "            return createObjectMapper().readValue(json, new com.fasterxml.jackson.core.type.TypeReference<java.util.List<{}>>() {{ }});",
            element_type
        )
        .ok();
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(
            out,
            "            throw new {}Exception(\"FFI call failed\", e);",
            class_name
        )
        .ok();
        writeln!(out, "        }}").ok();
    } else {
        writeln!(
            out,
            "            var primitiveResult = ({}) {}.invoke({});",
            java_ffi_return_cast(&func.return_type),
            ffi_handle,
            call_args.join(", ")
        )
        .ok();
        emit_ffi_ptr_cleanup(out);
        writeln!(out, "            return primitiveResult;").ok();
        writeln!(out, "        }} catch (Throwable e) {{").ok();
        writeln!(
            out,
            "            throw new {}Exception(\"FFI call failed\", e);",
            class_name
        )
        .ok();
        writeln!(out, "        }}").ok();
    }

    writeln!(out, "    }}").ok();
}

pub(crate) fn gen_async_wrapper_method(
    out: &mut String,
    func: &FunctionDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) {
    let params: Vec<String> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
        .map(|p| {
            let ptype = java_type(&p.ty);
            format!("final {} {}", ptype, to_java_name(&p.name))
        })
        .collect();

    let return_type = match &func.return_type {
        TypeRef::Unit => "Void".to_string(),
        other => java_boxed_type(other).to_string(),
    };

    let sync_method_name = to_java_name(&func.name);
    let async_method_name = format!("{}Async", sync_method_name);
    let param_names: Vec<String> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
        .map(|p| to_java_name(&p.name))
        .collect();

    writeln!(
        out,
        "    public static CompletableFuture<{}> {}({}) {{",
        return_type,
        async_method_name,
        params.join(", ")
    )
    .ok();
    writeln!(out, "        return CompletableFuture.supplyAsync(() -> {{").ok();
    writeln!(out, "            try {{").ok();
    writeln!(
        out,
        "                return {}({});",
        sync_method_name,
        param_names.join(", ")
    )
    .ok();
    writeln!(out, "            }} catch (Throwable e) {{").ok();
    writeln!(out, "                throw new CompletionException(e);").ok();
    writeln!(out, "            }}").ok();
    writeln!(out, "        }});").ok();
    writeln!(out, "    }}").ok();
}

// ---------------------------------------------------------------------------
// Exception class
// ---------------------------------------------------------------------------

pub(crate) fn gen_facade_class(
    api: &ApiSurface,
    package: &str,
    public_class: &str,
    raw_class: &str,
    _prefix: &str,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    has_visitor_bridge: bool,
) -> String {
    let mut body = String::with_capacity(4096);

    writeln!(body, "public final class {} {{", public_class).ok();
    writeln!(body, "    private {}() {{ }}", public_class).ok();
    writeln!(body).ok();

    // Generate static methods for free functions
    for func in &api.functions {
        // Sync method — bridge params stripped from public signature
        let params: Vec<String> = func
            .params
            .iter()
            .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
            .map(|p| {
                let ptype = java_type(&p.ty);
                format!("final {} {}", ptype, to_java_name(&p.name))
            })
            .collect();

        let return_type = java_type(&func.return_type);

        if !func.doc.is_empty() {
            writeln!(body, "    /**").ok();
            for line in func.doc.lines() {
                writeln!(body, "     * {}", line).ok();
            }
            writeln!(body, "     */").ok();
        }

        writeln!(
            body,
            "    public static {} {}({}) throws {}Exception {{",
            return_type,
            to_java_name(&func.name),
            params.join(", "),
            raw_class
        )
        .ok();

        // Null checks for required non-bridge parameters
        for param in &func.params {
            if !param.optional && !is_bridge_param_java(param, bridge_param_names, bridge_type_aliases) {
                let pname = to_java_name(&param.name);
                writeln!(
                    body,
                    "        java.util.Objects.requireNonNull({}, \"{} must not be null\");",
                    pname, pname
                )
                .ok();
            }
        }

        // Delegate to raw FFI class — bridge params are stripped from the raw class
        // signature, so we must exclude them entirely (not pass null) to match the
        // raw class's parameter count.
        let call_args: Vec<String> = func
            .params
            .iter()
            .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
            .map(|p| to_java_name(&p.name))
            .collect();

        if matches!(func.return_type, TypeRef::Unit) {
            writeln!(
                body,
                "        {}.{}({});",
                raw_class,
                to_java_name(&func.name),
                call_args.join(", ")
            )
            .ok();
        } else {
            writeln!(
                body,
                "        return {}.{}({});",
                raw_class,
                to_java_name(&func.name),
                call_args.join(", ")
            )
            .ok();
        }

        writeln!(body, "    }}").ok();
        writeln!(body).ok();

        // Generate overload without optional params (convenience method).
        // Only non-bridge params are considered here.
        let has_optional = func
            .params
            .iter()
            .any(|p| p.optional && !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases));
        if has_optional {
            let required_params: Vec<String> = func
                .params
                .iter()
                .filter(|p| !p.optional && !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
                .map(|p| {
                    let ptype = java_type(&p.ty);
                    format!("final {} {}", ptype, to_java_name(&p.name))
                })
                .collect();

            writeln!(
                body,
                "    public static {} {}({}) throws {}Exception {{",
                return_type,
                to_java_name(&func.name),
                required_params.join(", "),
                raw_class
            )
            .ok();

            // Build call to raw class: bridge params are excluded (stripped from raw
            // class signature), optional params passed as null.
            let full_args: Vec<String> = func
                .params
                .iter()
                .filter(|p| !is_bridge_param_java(p, bridge_param_names, bridge_type_aliases))
                .map(|p| {
                    if p.optional {
                        "null".to_string()
                    } else {
                        to_java_name(&p.name)
                    }
                })
                .collect();

            if matches!(func.return_type, TypeRef::Unit) {
                writeln!(
                    body,
                    "        {}.{}({});",
                    raw_class,
                    to_java_name(&func.name),
                    full_args.join(", ")
                )
                .ok();
            } else {
                writeln!(
                    body,
                    "        return {}.{}({});",
                    raw_class,
                    to_java_name(&func.name),
                    full_args.join(", ")
                )
                .ok();
            }

            writeln!(body, "    }}").ok();
            writeln!(body).ok();
        }
    }

    // Expose convertWithVisitor in the public facade when visitor bridge is configured.
    if has_visitor_bridge {
        writeln!(body, "    /**").ok();
        writeln!(
            body,
            "     * Convert HTML to Markdown, invoking visitor callbacks during processing."
        )
        .ok();
        writeln!(body, "     */").ok();
        writeln!(
            body,
            "    public static ConversionResult convertWithVisitor(String html, ConversionOptions options, Visitor visitor)"
        )
        .ok();
        writeln!(body, "            throws {}Exception {{", raw_class).ok();
        writeln!(
            body,
            "        return {}.convertWithVisitor(html, options, visitor);",
            raw_class
        )
        .ok();
        writeln!(body, "    }}").ok();
        writeln!(body).ok();
    }

    writeln!(body, "}}").ok();

    // Now assemble the file with imports
    let mut out = String::with_capacity(body.len() + 512);

    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "package {};", package).ok();

    // Check what imports are needed based on content
    let has_list = body.contains("List<");
    let has_map = body.contains("Map<");
    let has_optional = body.contains("Optional<");
    let has_imports = has_list || has_map || has_optional;

    if has_imports {
        writeln!(out).ok();
        if has_list {
            writeln!(out, "import java.util.List;").ok();
        }
        if has_map {
            writeln!(out, "import java.util.Map;").ok();
        }
        if has_optional {
            writeln!(out, "import java.util.Optional;").ok();
        }
    }

    writeln!(out).ok();
    out.push_str(&body);

    out
}

// ---------------------------------------------------------------------------
// Opaque handle classes
// ---------------------------------------------------------------------------
