use ahash::AHashSet;
use alef_core::config::ResolvedCrateConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, TypeRef};
use heck::ToSnakeCase;
use std::fmt::Write;

use super::marshal::{gen_ffi_layout, gen_function_descriptor};

pub(crate) fn gen_native_lib(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    package: &str,
    prefix: &str,
    has_visitor_pattern: bool,
) -> String {
    // Generate the class body first, then scan it to determine which imports are needed.
    let mut body = String::with_capacity(2048);
    // Derive the native library name from the FFI output path (directory name with hyphens replaced
    // by underscores), falling back to `{ffi_prefix}_ffi`.
    let lib_name = config.ffi_lib_name();

    writeln!(body, "final class NativeLib {{").ok();
    writeln!(body, "    private static final Linker LINKER = Linker.nativeLinker();").ok();
    writeln!(body, "    private static SymbolLookup LIB;").ok();
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
    writeln!(body, "    private static String loadedLibraryName;").ok();
    writeln!(body).ok();
    writeln!(body, "    static {{").ok();
    writeln!(body, "        loadNativeLibrary();").ok();
    writeln!(body, "        try {{").ok();
    writeln!(
        body,
        "            java.lang.foreign.Arena arena = java.lang.foreign.Arena.ofConfined();"
    )
    .ok();
    writeln!(
        body,
        "            // Try the loaded library name first (for System.load() path case)"
    )
    .ok();
    writeln!(body, "            try {{").ok();
    writeln!(
        body,
        "                LIB = SymbolLookup.libraryLookup(loadedLibraryName, arena);"
    )
    .ok();
    writeln!(body, "            }} catch (Throwable inner1) {{").ok();
    writeln!(
        body,
        "                // Try with 'lib' prefix if not already present (for System.loadLibrary() case)"
    )
    .ok();
    writeln!(body, "                String nameWithLib = loadedLibraryName.startsWith(\"lib\") ? loadedLibraryName : \"lib\" + loadedLibraryName;").ok();
    writeln!(body, "                try {{").ok();
    writeln!(
        body,
        "                    LIB = SymbolLookup.libraryLookup(nameWithLib, arena);"
    )
    .ok();
    writeln!(body, "                }} catch (Throwable inner2) {{").ok();
    writeln!(body, "                    // Last fallback: use LINKER.defaultLookup()").ok();
    writeln!(body, "                    LIB = LINKER.defaultLookup();").ok();
    writeln!(body, "                }}").ok();
    writeln!(body, "            }}").ok();
    writeln!(body, "        }} catch (Throwable e) {{").ok();
    writeln!(body, "            throw new ExceptionInInitializerError(\"Failed to initialize library symbols: \" + e.getMessage());").ok();
    writeln!(body, "        }}").ok();
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
    writeln!(body, "            // Find the full path by searching java.library.path").ok();
    writeln!(
        body,
        "            loadedLibraryName = findLoadedLibraryPath(\"{}\", libName, libExt);",
        lib_name
    )
    .ok();
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
    writeln!(
        body,
        "            loadedLibraryName = libPath.toAbsolutePath().toString();"
    )
    .ok();
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
    writeln!(
        body,
        "    private static String findLoadedLibraryPath(String libName, String fullLibName, String libExt) {{"
    )
    .ok();
    writeln!(body, "        // Search java.library.path for the library file").ok();
    writeln!(
        body,
        "        String javaLibPath = System.getProperty(\"java.library.path\");"
    )
    .ok();
    writeln!(body, "        if (javaLibPath != null) {{").ok();
    writeln!(
        body,
        "            for (String path : javaLibPath.split(File.pathSeparator)) {{"
    )
    .ok();
    writeln!(
        body,
        "                java.nio.file.Path libPath = java.nio.file.Paths.get(path, fullLibName + libExt);"
    )
    .ok();
    writeln!(body, "                if (java.nio.file.Files.exists(libPath)) {{").ok();
    writeln!(body, "                    try {{").ok();
    writeln!(body, "                        return libPath.toRealPath().toString();").ok();
    writeln!(body, "                    }} catch (java.io.IOException e) {{").ok();
    writeln!(
        body,
        "                        return libPath.toAbsolutePath().toString();"
    )
    .ok();
    writeln!(body, "                    }}").ok();
    writeln!(body, "                }}").ok();
    writeln!(body, "            }}").ok();
    writeln!(body, "        }}").ok();
    writeln!(
        body,
        "        // Fallback: try just the library name (may work on some systems)"
    )
    .ok();
    writeln!(body, "        return libName;").ok();
    writeln!(body, "    }}").ok();
    writeln!(body).ok();

    // Collect trait bridge handle names that will be emitted later, so we can skip them
    // in the functions loop (prevents duplicate handle emission with wrong descriptors).
    let trait_bridge_handles: AHashSet<String> = config
        .trait_bridges
        .iter()
        .filter(|b| {
            !b.exclude_languages
                .contains(&alef_core::config::Language::Java.to_string())
        })
        .flat_map(|b| {
            let trait_snake = b.trait_name.to_snake_case();
            let trait_upper = trait_snake.to_uppercase();
            vec![
                format!("{}_REGISTER_{}", prefix.to_uppercase(), trait_upper),
                format!("{}_UNREGISTER_{}", prefix.to_uppercase(), trait_upper),
            ]
        })
        .collect();

    // Collect FFI-excluded function names so we can emit nullable handles for them.
    // Functions excluded from the FFI layer are still present in the IR (and thus appear
    // in the Java facade) but their native symbols are not compiled into the shared library.
    // Using orElse(null) prevents class initialization failure; callers must null-check before
    // invoking these handles.
    let ffi_excluded: AHashSet<String> = config
        .ffi
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();

    // Generate method handles for free functions.
    // All functions get handles regardless of is_async — the FFI layer always exposes
    // synchronous C functions, and the Java async wrapper delegates to the sync method.
    for func in &api.functions {
        let handle_name = format!("{}_{}", prefix.to_uppercase(), func.name.to_uppercase());

        // Skip if this function's handle will be emitted by trait bridge code (with correct descriptor).
        if trait_bridge_handles.contains(&handle_name) {
            continue;
        }

        let ffi_name = format!("{}_{}", prefix, func.name.to_lowercase());
        let return_layout = gen_ffi_layout(&func.return_type);
        let param_layouts: Vec<String> = func.params.iter().map(|p| gen_ffi_layout(&p.ty)).collect();

        let layout_str = gen_function_descriptor(&return_layout, &param_layouts);

        if ffi_excluded.contains(&func.name) {
            // Use orElse(null) for FFI-excluded functions — their native symbol may be absent.
            // Callers must null-check before invoking these handles.
            writeln!(body).ok();
            writeln!(
                body,
                "    static final MethodHandle {} = LIB.find(\"{}\").map(s -> LINKER.downcallHandle(s, {})).orElse(null);",
                handle_name, ffi_name, layout_str
            )
            .ok();
        } else {
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

    // Track emitted handles to avoid duplicates (a type may appear both as
    // a function return type AND as an opaque type, or as both return and parameter type).
    let mut emitted_free_handles: AHashSet<String> = AHashSet::new();
    // Same dedup for `_to_json` handles — when multiple functions return the
    // same Named type we'd otherwise emit the constant twice.
    let mut emitted_to_json_handles: AHashSet<String> = AHashSet::new();

    // Build the set of opaque type names so we can pick the right accessor below.
    let opaque_type_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    // Accessor handles for Named return types (struct pointer → field accessor + free).
    // Also handles `Option<Named>` return types — the FFI layer flattens nullable returns
    // to a raw pointer that's NULL when the optional is empty.
    for func in &api.functions {
        let inner_named = match &func.return_type {
            TypeRef::Named(n) => Some(n),
            TypeRef::Optional(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() {
                    Some(n)
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(name) = inner_named {
            let type_snake = name.to_snake_case();
            let type_upper = type_snake.to_uppercase();
            let is_opaque = opaque_type_names.contains(name.as_str());

            // Emit `_to_json` method handle whenever the FFI exposes one for this type.
            // Both opaque and non-opaque types may have a `_to_json` exporter — the Java
            // wrapper code uses it to serialize for inspection (e.g. `EmbeddingPreset`).
            // We use `LIB.find(...).map(...).orElse(null)` so generation is robust if the
            // function is absent in this build (compile-time presence isn't always guaranteed).
            let to_json_handle = format!("{}_{}_TO_JSON", prefix.to_uppercase(), type_upper);
            let to_json_ffi = format!("{}_{}_to_json", prefix, type_snake);
            if emitted_to_json_handles.insert(to_json_handle.clone()) {
                writeln!(body).ok();
                writeln!(
                    body,
                    "    static final MethodHandle {} = LIB.find(\"{}\").map(s -> LINKER.downcallHandle(s, FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS))).orElse(null);",
                    to_json_handle, to_json_ffi
                )
                .ok();
            }
            let _ = is_opaque;

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
    //
    // Note: Even enums need _free here. `{prefix}_{type}_from_json` returns *mut T
    // (a heap-allocated pointer) regardless of whether T is an enum or struct, so the
    // matching _free is required to avoid leaking that allocation.
    //
    // We scan ALL functions (including ffi-excluded ones) because parameter type helpers
    // like _from_json/_free may be needed for the generated wrapper regardless of whether
    // the main function handle uses orElse(null). The dylib always exports these helpers
    // for parameter types that appear in non-excluded functions of the same type.
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

    // Trait bridge register/unregister FFI handles (de-duplicated with function-related handles)
    let mut emitted_register_handles: AHashSet<String> = AHashSet::new();
    let mut emitted_unregister_handles: AHashSet<String> = AHashSet::new();

    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg
            .exclude_languages
            .contains(&alef_core::config::Language::Java.to_string())
        {
            continue;
        }

        let trait_snake = bridge_cfg.trait_name.to_snake_case();
        let trait_upper = trait_snake.to_uppercase();

        // Register handle
        let register_handle_name = format!("{}_REGISTER_{}", prefix.to_uppercase(), trait_upper);
        let register_ffi_name = format!("{}_register_{}", prefix, trait_snake);
        if emitted_register_handles.insert(register_handle_name.clone()) {
            writeln!(body).ok();
            // Use orElse(null): the register symbol may be absent when the trait bridge
            // is not compiled into the dylib. Callers must null-check before invoking.
            writeln!(
                body,
                "    static final MethodHandle {} = LIB.find(\"{}\").map(s -> LINKER.downcallHandle(s,",
                register_handle_name, register_ffi_name
            )
            .ok();
            writeln!(
                body,
                "        FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS))).orElse(null);"
            )
            .ok();
        }

        // Unregister handle
        let unregister_handle_name = format!("{}_UNREGISTER_{}", prefix.to_uppercase(), trait_upper);
        let unregister_ffi_name = format!("{}_unregister_{}", prefix, trait_snake);
        if emitted_unregister_handles.insert(unregister_handle_name.clone()) {
            writeln!(body).ok();
            // Use orElse(null): the unregister symbol may be absent when the trait bridge
            // is not compiled into the dylib. Callers must null-check before invoking.
            writeln!(
                body,
                "    static final MethodHandle {} = LIB.find(\"{}\").map(s -> LINKER.downcallHandle(s,",
                unregister_handle_name, unregister_ffi_name
            )
            .ok();
            writeln!(
                body,
                "        FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS))).orElse(null);"
            )
            .ok();
        }
    }

    // Inject visitor FFI method handles when a trait bridge is configured.
    if has_visitor_pattern {
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
    writeln!(out, "import java.io.File;").ok();
    writeln!(out, "import java.net.URL;").ok();
    writeln!(out, "import java.nio.file.Files;").ok();
    writeln!(out, "import java.nio.file.Path;").ok();
    writeln!(out, "import java.nio.file.Paths;").ok();
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
