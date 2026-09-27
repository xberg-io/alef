use crate::codegen::naming::{PublicIdentifierKind, public_casing};
use crate::core::config::Language;
use heck::{ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};

pub(crate) fn lang_display_name(lang: Language) -> &'static str {
    match lang {
        Language::Python => "Python",
        Language::Node => "TypeScript",
        Language::Ruby => "Ruby",
        Language::Php => "PHP",
        Language::Elixir => "Elixir",
        Language::Go => "Go",
        Language::Java => "Java",
        Language::Csharp => "C#",
        Language::Ffi | Language::C | Language::Jni => "C",
        Language::Wasm => "WebAssembly",
        Language::R => "R",
        Language::Rust => "Rust",
        Language::Kotlin => "Kotlin",
        Language::KotlinAndroid => "Kotlin (Android)",
        Language::Swift => "Swift",
        Language::Dart => "Dart",
        Language::Gleam => "Gleam",
        Language::Zig => "Zig",
    }
}

/// Get the slug used in file names (e.g. `typescript` for `Node`).
pub(crate) fn lang_slug(lang: Language) -> &'static str {
    match lang {
        Language::Python => "python",
        Language::Node => "typescript",
        Language::Ruby => "ruby",
        Language::Php => "php",
        Language::Elixir => "elixir",
        Language::Go => "go",
        Language::Java => "java",
        Language::Csharp => "csharp",
        Language::Ffi | Language::C | Language::Jni => "c",
        Language::Wasm => "wasm",
        Language::R => "r",
        Language::Rust => "rust",
        Language::Kotlin => "kotlin",
        Language::KotlinAndroid => "kotlin-android",
        Language::Swift => "swift",
        Language::Dart => "dart",
        Language::Gleam => "gleam",
        Language::Zig => "zig",
    }
}

/// Get the code fence language identifier.
pub(crate) fn lang_code_fence(lang: Language) -> &'static str {
    match lang {
        Language::Python => "python",
        Language::Node | Language::Wasm => "typescript",
        Language::Ruby => "ruby",
        Language::Php => "php",
        Language::Elixir => "elixir",
        Language::Go => "go",
        Language::Java => "java",
        Language::Csharp => "csharp",
        Language::Ffi | Language::C | Language::Jni => "c",
        Language::R => "r",
        Language::Rust => "rust",
        Language::Kotlin | Language::KotlinAndroid => "kotlin",
        Language::Swift => "swift",
        Language::Dart => "dart",
        Language::Gleam => "gleam",
        Language::Zig => "zig",
    }
}

/// Convert a Rust type name to the idiomatic name for the target language.
///
/// Delegates casing to [`public_casing`] -- the same per-language authority every codegen
/// backend's type declaration goes through via `public_host_identifier` -- so this can no
/// longer independently decide that Go wants `BaseUrl` where the Go backend actually emits
/// `BaseURL` (`crate::codegen::naming::to_go_name`/`go_type_name` apply initialism
/// uppercasing; a bare `heck::to_pascal_case` does not). ~keep
pub(crate) fn type_name(name: &str, lang: Language, ffi_prefix: &str) -> String {
    let short = name.rsplit("::").next().unwrap_or(name);
    let cased = public_casing(lang, PublicIdentifierKind::Type, short);
    match lang {
        // cbindgen renames every exported type with `[export] prefix`. Callers hand this
        // function the PascalCase form of `[ffi] prefix` (docs::generate_docs), so a
        // case-restoring conversion is required to recover the header spelling: for a consumer
        // whose `[ffi] prefix` is a single lowercase word such as `demoapi`, the header really
        // does say `DEMOAPIDefaultClient`, and emitting `DemoapiDefaultClient` names a symbol
        // that occurs zero times in it.
        //
        // Both sides now use the same conversion: `gen_cbindgen_toml`
        // (backends/ffi/gen_bindings/helpers.rs:392) computes the real `[export] prefix` as
        // `prefix.to_shouty_snake_case()`, which is what this arm applies too. An earlier
        // version of this note recorded a divergence -- `gen_cbindgen_toml` using plain
        // `.to_uppercase()`, so `SampleCore` became `SAMPLECORE` in the header but
        // `SAMPLE_CORE` here -- and instructed readers not to "fix" it. That divergence is
        // gone; the instruction would now reintroduce it. Matches the `enum_variant_name`
        // arm below. ~keep
        Language::Ffi | Language::C | Language::Jni => format!("{}{}", ffi_prefix.to_shouty_snake_case(), cased),
        _ => cased,
    }
}

/// Convert a Rust function name to the idiomatic name for the target language.
///
/// The `base` casing below delegates to [`public_casing`] -- the same authority
/// `public_host_identifier` applies for every codegen backend's function/method declaration --
/// so this function cannot independently decide Go wants `ParseUrl` where the Go backend emits
/// `ParseURL`, or Gleam wants `parseUrl` where Gleam's own snake_case idiom (matching
/// `field_name` below) wants `parse_url`. The per-language override table that follows must
/// still run on the *un-escaped* `base`, which is exactly what [`public_casing`] returns
/// (unlike `public_host_identifier`, which would already have turned `new` into `new_` before
/// the Java-specific `new` -> `create` arm below ever saw it). ~keep
pub(crate) fn func_name(name: &str, lang: Language, ffi_prefix: &str) -> String {
    let base = match lang {
        // `func_name` is fed Rust `fn` names, which arrive already snake_case, so routing
        // through `c_consumer::free_function_symbol` (which applies `pascal_to_snake` rather
        // than heck's `to_snake_case`) is a no-op rename here -- both conversions are the
        // identity on an already-snake_case input. Kept as the single source of the free-
        // function symbol shape so this and `render_c_fn_sig` cannot re-derive it
        // independently. ~keep
        Language::Ffi | Language::C | Language::Jni => {
            crate::codegen::c_consumer::free_function_symbol(&ffi_prefix.to_snake_case(), name)
        }
        _ => public_casing(lang, PublicIdentifierKind::Function, name),
    };
    // ~keep Java's keyword renames must match `safe_java_method_name`
    // (backends/java/gen_bindings/helpers.rs:207-215), which is what the Java backend applies
    // to every opaque-type method: `default` -> `defaultInstance`, `new` -> `create`, any
    // other keyword collision -> a trailing-underscore form. The previous hand-written arm
    // emitted `defaultOptions`, a name the backend never generates, and carried no `new` arm
    // at all -- so an opaque type's default static constructor (`pub fn new`, the shape the
    // IR carries for it) arrived at the identifier gate as the Java reserved word `new` and
    // panicked the whole docs run instead of documenting `create`. Mirrored rather than
    // delegated because the backend applies it only to opaque methods, while this function
    // also names free functions (examples.rs, language_pages/function_render.rs); the mirror
    // is arm-for-arm and shares the same `JAVA_KEYWORDS` constant, and
    // `test_func_name_java_matches_backend_safe_java_method_name` pins it across that whole
    // constant rather than a sample. The two camel-case helpers differ in name only for these
    // inputs -- every entry in `JAVA_KEYWORDS` is a plain lowercase word, which both leave
    // unchanged, so the membership arm keys identically on both sides.
    //
    // ~keep `true`, `false`, and `null` used to be a shared blind spot here: they are reserved
    // *literals*, not keywords, and were absent from `JAVA_KEYWORDS` -- so neither this table's
    // membership arm nor `safe_java_method_name` renamed a method literally named one of them,
    // and the backend emitted non-compiling Java that only the docs gate's separate, wider
    // `reserved_words(Java)` table (formatting.rs) ever caught. Closed at the source:
    // `JAVA_KEYWORDS` (core/keywords.rs) now includes all three, so `safe_java_method_name`
    // and this table's membership arm both escape them the same way they escape any other
    // keyword, and `reserved_words(Java)` no longer needs its own copy of the three literals.
    // ~keep Dart's `new` is an ordinary reserved-word collision, not a declaration-shape
    // mismatch the way Swift's `init` is (see `is_swift_static_constructor` in signatures.rs):
    // the docs pipeline already renders a static factory method for a static `new` returning
    // `Self` (`static Future<T> {name}(...)`), which is a legal Dart declaration for any legal
    // name -- only the identifier `new` itself is reserved. Mirrors the Java `new` -> `create`
    // arm above for the same reason: a curated or default opaque constructor used to reach the
    // identifier gate as the raw reserved word `new`, and the docs pipeline shipped
    // `static Future<DownloadManager> new(String version)` -- three such sites in one real
    // consumer's `api-dart.md`, none of which compile. Given its own named arm ahead of the
    // general sweep below so it keeps the more idiomatic `create` instead of falling through
    // to the mechanical `new_`.
    //
    // ~keep The class this whole function exists to close is not "constructors named `new`" --
    // it is "any emitted `pub fn` whose name is a keyword in the target language". A second,
    // independently measured consumer tree hit `global` (Python), `get` (Dart, Kotlin), and
    // `new` (Dart) on four *ordinary* methods with no constructor shape at all
    // (`pub fn global() -> &'static Registry`, `pub fn get(&self, id: &str) -> Option<&Preset>`
    // repeated across five registries). None of those collide in every language --
    // `identifier_violation`'s own per-language tables are the single source of truth for
    // which word collides where (Kotlin's `get` is a conservative soft-keyword inclusion,
    // documented on `reserved_words`; a narrower per-backend list would let it through and
    // silently disagree with the gate that judges the output). So: any language whose grammar
    // reserves the word in every position (`!reserved_words_bind_in_member_position`, i.e.
    // everyone except Node/Wasm/Php) gets the trailing-underscore escape already used
    // throughout `core::keywords` for exactly this purpose (`python_ident`, `kotlin_ident`,
    // `swift_ident`, `dart_ident`, ...) -- checked against `identifier_violation` itself, not
    // a second hand-copied list, so the escape and the gate can never disagree about what
    // needs escaping. Node/Wasm/Php are excluded because `func_name` has no position
    // parameter: those two languages only relax the reserved-word rule in *member* position
    // (formatting.rs's `reserved_words_bind_in_member_position`), and a free-function
    // *declaration* using the same name is still illegal there -- escaping it here would hide
    // that real declaration-position violation instead of reporting it. Java is excluded
    // because its own arms above already cover it with a more idiomatic mapping; running both
    // would be redundant, not wrong, but the explicit arms document *why* those two words are
    // special-cased instead of falling through to the mechanical escape.
    match (lang, base.as_str()) {
        (Language::Java, "default") => "defaultInstance".to_string(),
        (Language::Java, "new") => "create".to_string(),
        (Language::Java, other) if crate::core::keywords::JAVA_KEYWORDS.contains(&other) => format!("{other}_"),
        (Language::Csharp, "Default") => "CreateDefault".to_string(),
        (Language::Dart, "new") => "create".to_string(),
        (lang, other)
            if !matches!(lang, Language::Java | Language::Node | Language::Wasm | Language::Php)
                && crate::docs::formatting::identifier_violation(
                    other,
                    lang,
                    crate::docs::formatting::IdentifierPosition::Declaration,
                ) == Some("reserved word") =>
        {
            format!("{other}_")
        }
        _ => base,
    }
}

/// Convert a Rust method name to the idiomatic name for the target language, folding in the
/// owning type for C.
///
/// A C symbol has no namespace, so the backend folds the owning type into the name
/// (`gen_method_wrapper` / `gen_streaming_method_wrapper` emit `{prefix}_{type_snake}_{method}`).
/// Every other language documents a method as a member of its owning type already -- the type
/// is spelled once, in the signature's receiver or class heading -- so `func_name`'s per-language
/// rules (including the Java keyword renames) keep applying unchanged there.
pub(crate) fn method_name(owner_type: &str, name: &str, lang: Language, ffi_prefix: &str) -> String {
    match lang {
        Language::Ffi | Language::C | Language::Jni => {
            crate::codegen::c_consumer::method_symbol(&ffi_prefix.to_snake_case(), owner_type, name)
        }
        _ => func_name(name, lang, ffi_prefix),
    }
}

/// Suffix a C# member name with `Async`, the same rule the C# backend applies to every emitted
/// async free function, bridge-field wrapper, and opaque method (`gen_bindings/methods/
/// wrappers.rs`, `gen_bindings/methods/bridge_fields.rs`, `gen_bindings/types/opaque.rs`: each
/// independently does `if is_async && !name.ends_with("Async") { name.push_str("Async") }`).
/// `signatures.rs`'s C# function/method signature renderers and `examples.rs`'s C# call-site
/// renderer both name the same emitted member and must not re-derive this rule separately --
/// that duplication is exactly how a real consumer ended up with example code calling
/// `Extract`/`ExtractBatch` against a binding that only exports `ExtractAsync`/
/// `ExtractBatchAsync`. ~keep
pub(crate) fn csharp_async_member_name(name: &str, is_async: bool) -> String {
    if is_async && !name.ends_with("Async") {
        format!("{name}Async")
    } else {
        name.to_string()
    }
}

/// Convert a Rust field name to the idiomatic name for the target language.
///
/// Delegates to [`public_casing`], the same per-language authority the codegen backends use for
/// a struct field's declaration. Previously hand-rolled its own table that put Gleam in the
/// camelCase group; Gleam fields (and functions/parameters) are snake_case, matching Rust,
/// Elixir and Erlang -- see the same fix in
/// `crate::codegen::naming::host::public_enum_variant_name`'s doc comment for why enum variant
/// casing stayed PascalCase for Gleam while member casing did not. ~keep
pub(crate) fn field_name(name: &str, lang: Language) -> String {
    public_casing(lang, PublicIdentifierKind::Field, name)
}

/// Convert a Rust enum variant name to the idiomatic name for the target language.
///
/// Delegates to [`public_casing`], which fixes three independent bugs this function used to
/// carry on its own:
/// - Java enum constants are SCREAMING_SNAKE (`INLINE`), not PascalCase (`Inline`).
/// - Gleam constructors are PascalCase (`Circle`), not the snake_case this function previously
///   gave every non-JVM/C#/Go language uniformly.
/// - The acronym-run splitting `heck::to_snake_case`/`to_shouty_snake_case` get wrong for a
///   name like the real Rust variant `RDFa` (three-letter acronym run + a one-letter suffix)
///   is exactly what `crate::codegen::naming::case::pascal_to_snake` exists to get right; this
///   function used to paper over the gap with a hardcoded `if name == "RDFa"` table instead of
///   fixing the acronym-aware conversion (see that function's `MIN_TRAILING_WORD_LEN` doc
///   comment for the fix). Deleting the hardcode also means every *other* current or future
///   variant with the same acronym-run shape is now handled correctly too, not just this one
///   literal name. ~keep
pub(crate) fn enum_variant_name(name: &str, lang: Language, ffi_prefix: &str) -> String {
    let cased = public_casing(lang, PublicIdentifierKind::EnumVariant, name);
    match lang {
        Language::Ffi | Language::C | Language::Jni => format!("{}_{}", ffi_prefix.to_shouty_snake_case(), cased),
        _ => cased,
    }
}

/// Convert snake_case or PascalCase to camelCase.
pub(crate) fn to_camel_case(s: &str) -> String {
    let pascal = s.to_upper_camel_case();
    let mut chars = pascal.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_lowercase().to_string() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::Language;
    use crate::docs::test_helpers::TEST_PREFIX;

    #[test]
    fn test_enum_variant_name_python() {
        assert_eq!(enum_variant_name("Atx", Language::Python, TEST_PREFIX), "ATX");
        assert_eq!(
            enum_variant_name("SnakeCase", Language::Python, TEST_PREFIX),
            "SNAKE_CASE"
        );
    }

    #[test]
    fn test_enum_variant_name_java() {
        assert_eq!(enum_variant_name("Atx", Language::Java, TEST_PREFIX), "ATX");
    }

    #[test]
    fn test_enum_variant_name_ffi() {
        assert_eq!(enum_variant_name("Atx", Language::Ffi, TEST_PREFIX), "HTM_ATX");
    }

    #[test]
    fn test_type_name_ffi_uses_prefix() {
        assert_eq!(
            type_name("ParseOptions", Language::Ffi, "SampleCrate"),
            "SAMPLE_CRATEParseOptions"
        );
        assert_eq!(
            type_name("ParseOutput", Language::Ffi, "SampleCrate"),
            "SAMPLE_CRATEParseOutput"
        );
    }

    /// The C reference page must spell a type exactly as it appears in the emitted header.
    ///
    /// `alef all` used to publish `DemoapiDefaultClient` while the emitted `demo_api.h` declared
    /// `DEMOAPIDefaultClient` -- a name that occurs zero times in the header, so every C
    /// snippet on the same site contradicted the reference page. Observed in a consumer repo
    /// whose `[ffi] prefix` is a single lowercase word.
    ///
    /// The lowercase, single-word and already-underscored prefixes below do not discriminate
    /// between conversions -- `to_shouty_snake_case` and a plain `.to_uppercase()` return the
    /// same string for all of them, so pinning only those would pass no matter which one
    /// `gen_cbindgen_toml` used. `SampleCore` is the discriminating case: it is the one shape
    /// where the two disagree (`SAMPLE_CORE` vs `SAMPLECORE`), so it is the only row here that
    /// can actually fail if the header side stops using `to_shouty_snake_case`
    /// (backends/ffi/gen_bindings/helpers.rs:392). ~keep
    #[test]
    fn type_name_ffi_matches_cbindgen_export_prefix() {
        for (ffi_prefix, expected_export_prefix) in [
            ("demoapi", "DEMOAPI"),
            ("Demoapi", "DEMOAPI"),
            ("demo_api", "DEMO_API"),
            ("SampleCore", "SAMPLE_CORE"),
        ] {
            assert_eq!(
                type_name("DefaultClient", Language::C, ffi_prefix),
                format!("{expected_export_prefix}DefaultClient"),
                "docs must use cbindgen's export prefix for `{ffi_prefix}`"
            );
            assert_eq!(
                type_name("sample_crate::DefaultClient", Language::Ffi, ffi_prefix),
                format!("{expected_export_prefix}DefaultClient"),
                "a fully qualified rust path must resolve to the same header symbol"
            );
        }
    }

    #[test]
    fn test_func_name_ffi_uses_prefix() {
        assert_eq!(
            func_name("convert", Language::Ffi, "SampleCrate"),
            "sample_crate_convert"
        );
    }

    /// The whole point of `method_name`: a C symbol has no namespace, so the owning type must
    /// be folded into the name, or the docs publish a symbol that occurs zero times in the
    /// emitted header (the backend always emits `{prefix}_{type_snake}_{method}`, never
    /// `{prefix}_{method}`). Regresses if a call site falls back to `func_name` for C.
    #[test]
    fn test_method_name_ffi_folds_in_owning_type() {
        assert_eq!(
            method_name("Converter", "convert", Language::Ffi, TEST_PREFIX),
            "htm_converter_convert"
        );
        assert_ne!(
            method_name("Converter", "convert", Language::Ffi, TEST_PREFIX),
            func_name("convert", Language::Ffi, TEST_PREFIX),
            "a method symbol must not collide with the free-function symbol of the same name"
        );
    }

    /// For every non-C language, `method_name` must be an exact passthrough to `func_name` --
    /// including its Java keyword renames -- since the owning type is already spelled
    /// elsewhere in those languages' method docs (receiver, class heading). Regresses if
    /// `method_name` grows a second, divergent per-language table instead of delegating.
    #[test]
    fn test_method_name_non_c_delegates_to_func_name() {
        assert_eq!(method_name("Type", "new", Language::Java, TEST_PREFIX), "create");
        assert_eq!(
            method_name("Type", "new", Language::Java, TEST_PREFIX),
            func_name("new", Language::Java, TEST_PREFIX)
        );
        for lang in [
            Language::Python,
            Language::Node,
            Language::Go,
            Language::Csharp,
            Language::Ruby,
            Language::Zig,
        ] {
            assert_eq!(
                method_name("Document", "parse_document", lang, TEST_PREFIX),
                func_name("parse_document", lang, TEST_PREFIX),
                "{lang} must delegate to func_name unchanged"
            );
        }
    }

    #[test]
    fn test_csharp_async_member_name_appends_async_suffix() {
        assert_eq!(csharp_async_member_name("ParseDocument", true), "ParseDocumentAsync");
        assert_eq!(csharp_async_member_name("ParseDocument", false), "ParseDocument");
        assert_eq!(csharp_async_member_name("ExtractAsync", true), "ExtractAsync");
    }

    #[test]
    fn test_enum_variant_name_ffi_uses_prefix() {
        assert_eq!(
            enum_variant_name("Atx", Language::Ffi, "SampleCrate"),
            "SAMPLE_CRATE_ATX"
        );
    }

    #[test]
    fn test_field_name_go_pascal_case() {
        assert_eq!(field_name("heading_style", Language::Go), "HeadingStyle");
        assert_eq!(field_name("list_indent_type", Language::Go), "ListIndentType");
    }

    #[test]
    fn test_func_name_conventions() {
        assert_eq!(func_name("convert", Language::Python, TEST_PREFIX), "convert");
        assert_eq!(
            func_name("parse_document", Language::Node, TEST_PREFIX),
            "parseDocument"
        );
        assert_eq!(func_name("parse_document", Language::Go, TEST_PREFIX), "ParseDocument");
        assert_eq!(func_name("convert", Language::Ffi, TEST_PREFIX), "htm_convert");
    }

    #[test]
    fn test_type_name_ffi_prefix() {
        assert_eq!(type_name("ParseOptions", Language::Ffi, TEST_PREFIX), "HTMParseOptions");
        assert_eq!(type_name("ParseOutput", Language::Ffi, TEST_PREFIX), "HTMParseOutput");
    }

    #[test]
    fn test_lang_slug_kotlin_vs_kotlin_android() {
        assert_eq!(lang_slug(Language::Kotlin), "kotlin");
        assert_eq!(lang_slug(Language::KotlinAndroid), "kotlin-android");
    }

    #[test]
    fn test_lang_display_name_kotlin_vs_kotlin_android() {
        assert_eq!(lang_display_name(Language::Kotlin), "Kotlin");
        assert_eq!(lang_display_name(Language::KotlinAndroid), "Kotlin (Android)");
    }

    #[test]
    fn test_lang_code_fence_kotlin_android_uses_kotlin() {
        assert_eq!(lang_code_fence(Language::Kotlin), "kotlin");
        assert_eq!(lang_code_fence(Language::KotlinAndroid), "kotlin");
    }

    #[test]
    fn test_func_name_zig_uses_snake_case() {
        assert_eq!(func_name("create_engine", Language::Zig, TEST_PREFIX), "create_engine");
        assert_eq!(func_name("map_urls", Language::Zig, TEST_PREFIX), "map_urls");
        assert_eq!(func_name("batch_scrape", Language::Zig, TEST_PREFIX), "batch_scrape");
    }

    #[test]
    fn test_field_name_zig_uses_snake_case() {
        assert_eq!(field_name("max_depth", Language::Zig), "max_depth");
        assert_eq!(field_name("user_agent", Language::Zig), "user_agent");
    }

    #[test]
    fn test_enum_variant_name_zig_uses_snake_case() {
        assert_eq!(enum_variant_name("Auto", Language::Zig, TEST_PREFIX), "auto");
        assert_eq!(enum_variant_name("Stealth", Language::Zig, TEST_PREFIX), "stealth");
        assert_eq!(
            enum_variant_name("NetworkIdle", Language::Zig, TEST_PREFIX),
            "network_idle"
        );
    }

    /// Regression pin for the deleted `if name == "RDFa"` hardcode: the acronym-run heuristic
    /// in `crate::codegen::naming::case::pascal_to_snake` now gets this right generically, with
    /// no per-name table.
    #[test]
    fn test_enum_variant_name_zig_rdfa_uses_acronym_aware_snake_case() {
        assert_eq!(enum_variant_name("RDFa", Language::Zig, TEST_PREFIX), "rdfa");
    }

    #[test]
    fn test_type_name_zig_preserves_pascal_case() {
        assert_eq!(type_name("BrowserMode", Language::Zig, TEST_PREFIX), "BrowserMode");
        assert_eq!(type_name("CrawlConfig", Language::Zig, TEST_PREFIX), "CrawlConfig");
    }

    /// The default opaque constructor is a static `pub fn new` carried in `TypeDef::methods`.
    /// The Java backend renames it to `create`; documenting it as `new` names a Java reserved
    /// word, which the identifier gate reports as a violation (and used to panic on, aborting
    /// the whole docs run).
    #[test]
    fn test_func_name_java_renames_reserved_new_to_create() {
        assert_eq!(func_name("new", Language::Java, TEST_PREFIX), "create");
    }

    /// `defaultOptions` was a docs-only invention; the backend emits `defaultInstance`.
    #[test]
    fn test_func_name_java_renames_default_to_default_instance() {
        assert_eq!(func_name("default", Language::Java, TEST_PREFIX), "defaultInstance");
    }

    #[test]
    fn test_func_name_java_suffixes_other_reserved_words() {
        assert_eq!(func_name("class", Language::Java, TEST_PREFIX), "class_");
        assert_eq!(func_name("static", Language::Java, TEST_PREFIX), "static_");
    }

    #[test]
    fn test_func_name_java_leaves_ordinary_names_alone() {
        assert_eq!(
            func_name("parse_document", Language::Java, TEST_PREFIX),
            "parseDocument"
        );
        assert_eq!(func_name("create", Language::Java, TEST_PREFIX), "create");
    }

    /// Pin the docs table against the Java backend's own, so the two cannot drift apart
    /// silently the way `defaultOptions` did.
    ///
    /// Exhaustive over `JAVA_KEYWORDS`, not a sample: `safe_java_method_name`'s third arm is a
    /// membership test against that whole constant, so a sampled cross-check would pass while
    /// leaving 40-odd keywords free to diverge. The non-keyword names cover the fallthrough
    /// arm, where the two sides use different (but equivalent) camel-case helpers -- docs'
    /// `to_camel_case` vs heck's `to_lower_camel_case`.
    #[test]
    fn test_func_name_java_matches_backend_safe_java_method_name() {
        let ordinary = ["parse_document", "to_json", "create", "with_options", "from_str", "id"];
        for name in crate::core::keywords::JAVA_KEYWORDS.iter().copied().chain(ordinary) {
            assert_eq!(
                func_name(name, Language::Java, TEST_PREFIX),
                crate::backends::java::gen_bindings::helpers::safe_java_method_name(name),
                "docs must name `{name}` exactly as the Java backend does"
            );
        }
    }

    /// A Java identifier the docs emit must survive the identifier gate; before the rename
    /// table was corrected, `new` reached it verbatim and aborted the run. Exhaustive over
    /// the keyword table so no reserved word can reach the gate unrenamed.
    ///
    /// Asserts on the gate's `Result` rather than relying on it to panic: a rejected name is
    /// no longer fatal, so a test that only *called* the gate would pass no matter what the
    /// rename table produced.
    #[test]
    fn test_func_name_java_output_passes_the_identifier_gate() {
        use crate::docs::formatting::{IdentifierPosition, check_identifier};

        let ordinary = ["parse_document", "to_json", "create"];
        for name in crate::core::keywords::JAVA_KEYWORDS.iter().copied().chain(ordinary) {
            let rendered = func_name(name, Language::Java, TEST_PREFIX);
            assert_eq!(
                check_identifier(&rendered, Language::Java, IdentifierPosition::Member, "a naming test"),
                Ok(()),
                "`{name}` renders as `{rendered}`, which Java rejects as a member name"
            );
        }
    }

    /// Resolution for the "Java enum variants" disagreement: Java idiom is SCREAMING_SNAKE
    /// (`INLINE`), matching Python/Ruby/Rust, not `to_pascal_case` (`Inline`). Picked docs'
    /// pre-existing answer as correct; `host.rs` was the one that had it wrong (see
    /// `crate::codegen::naming::host::public_enum_variant_name`).
    #[test]
    fn test_enum_variant_name_java_uses_screaming_snake_case_for_multi_word_names() {
        assert_eq!(
            enum_variant_name("SnakeCase", Language::Java, TEST_PREFIX),
            "SNAKE_CASE"
        );
    }

    /// Resolution for the "Go/C# fields, types, functions" disagreement: Go's own initialism
    /// list (`crate::codegen::naming::languages::INITIALISMS`) uppercases `URL`; a bare
    /// `heck::to_pascal_case` (the old `field_name`/`type_name`/`func_name` behavior) would
    /// stop at `Url`. Picked `host.rs`'s answer (`to_go_name`/`go_type_name`) as correct across
    /// all three surfaces, since that is what the real Go backend
    /// (`crate::backends::go::gen_bindings::types::enums`, `type_map.rs`) actually emits.
    ///
    /// `type_name` takes a snake_case input here for `field_name`/`func_name` (real Rust
    /// field/function names), but a real Rust *type* name is already PascalCase -- issue #449
    /// removed `public_type_name`'s Go/C# `to_pascal_case()` pre-step because every real Go/C#
    /// backend call site hands it an already-PascalCase IR name, never a snake_case one. `"BaseUrl"`
    /// is the type-shaped input that pre-step's removal actually needs to be exercised against. ~keep
    #[test]
    fn test_go_field_type_and_func_names_apply_initialisms() {
        assert_eq!(field_name("base_url", Language::Go), "BaseURL");
        assert_eq!(type_name("BaseUrl", Language::Go, TEST_PREFIX), "BaseURL");
        assert_eq!(func_name("base_url", Language::Go, TEST_PREFIX), "BaseURL");
    }

    /// Same disagreement, C# side: `GraphQL` is one of the two initialisms C# preserves in full
    /// caps (`crate::codegen::naming::languages::CSHARP_INITIALISMS`) -- everything else in C#
    /// deliberately stays `Url`/`Id`, never `URL`/`ID` (.NET framework design guidelines), which
    /// is why this disagreement is *not* "make C# match Go": a bare `heck::to_pascal_case`
    /// happens to already be correct for the common case and must stay that way.
    #[test]
    fn test_csharp_type_name_preserves_graphql_initialism_but_not_url() {
        assert_eq!(
            type_name("GraphQLRouteConfig", Language::Csharp, TEST_PREFIX),
            "GraphQLRouteConfig"
        );
        assert_eq!(field_name("base_url", Language::Csharp), "BaseUrl");
    }

    /// Resolution for the "Gleam fields" disagreement: Gleam is snake_case throughout for
    /// values (fields, functions, parameters), the same family as Rust/Elixir/Erlang -- unlike
    /// Gleam *constructors* (enum variants), which are PascalCase (see the enum-variant test
    /// below). Picked `host.rs`'s answer (snake_case) as correct; the old `field_name`/
    /// `func_name` grouped Gleam with the camelCase languages instead.
    #[test]
    fn test_gleam_field_and_func_names_use_snake_case() {
        assert_eq!(field_name("base_url", Language::Gleam), "base_url");
        assert_eq!(func_name("base_url", Language::Gleam, TEST_PREFIX), "base_url");
    }

    /// Gleam's other half: custom-type constructors are PascalCase (`Circle`, `Square`), which
    /// the old `enum_variant_name` got right (it grouped Gleam with the PascalCase languages)
    /// while `host.rs` had it wrong (grouped with the snake_case languages) -- the inverse of
    /// the fields disagreement above. Both were fixed to agree on: fields snake_case, variants
    /// PascalCase.
    #[test]
    fn test_gleam_enum_variant_name_uses_pascal_case() {
        assert_eq!(
            enum_variant_name("SnakeCase", Language::Gleam, TEST_PREFIX),
            "SnakeCase"
        );
    }

    /// The regression guard the task asks for: `docs::naming` and
    /// `crate::codegen::naming::public_host_identifier` must agree, for every `Language`
    /// variant, on the casing of an ordinary (non-keyword, non-FFI-prefixed) field, type and
    /// enum variant name. Both now route through the same `public_casing` authority, so this
    /// currently holds by construction -- its job is to fail loudly the day a future change
    /// gives either side its own special-cased table again, the way `docs::naming` and
    /// `host.rs` independently drifted apart five separate times before this authority existed.
    ///
    /// FFI/C/Jni are excluded: `docs::naming` prefixes those with `[ffi] prefix` for the
    /// cbindgen-header spelling (a docs-only concern `public_host_identifier` does not share,
    /// since it has no `ffi_prefix` parameter), so a direct comparison does not apply there --
    /// those three are covered separately by `type_name_ffi_matches_cbindgen_export_prefix` and
    /// `test_enum_variant_name_ffi_uses_prefix`.
    ///
    /// The sample names are deliberately ordinary: not a keyword in any of the 20 languages'
    /// tables, not a Dart core type, not digit-leading -- so `public_host_identifier`'s
    /// escaping step is a no-op and the comparison isolates casing, not escaping policy (which
    /// deliberately stays split: `host.rs` auto-escapes for real codegen, `docs::naming` relies
    /// on `crate::docs::formatting::identifier_violation` to report a collision instead of
    /// silently renaming it -- see this module's top-of-function docs for why that split is
    /// intentional, not an oversight).
    #[test]
    fn test_field_type_and_enum_variant_casing_agrees_with_public_host_identifier() {
        use crate::codegen::naming::public_host_identifier;

        for lang in Language::ALL {
            if matches!(lang, Language::Ffi | Language::C | Language::Jni) {
                continue;
            }
            assert_eq!(
                field_name("widget_count", lang),
                public_host_identifier(lang, PublicIdentifierKind::Field, "widget_count"),
                "{lang:?} field casing must agree with the shared authority"
            );
            assert_eq!(
                type_name("WidgetSettings", lang, TEST_PREFIX),
                public_host_identifier(lang, PublicIdentifierKind::Type, "WidgetSettings"),
                "{lang:?} type casing must agree with the shared authority"
            );
            assert_eq!(
                enum_variant_name("WidgetVariant", lang, TEST_PREFIX),
                public_host_identifier(lang, PublicIdentifierKind::EnumVariant, "WidgetVariant"),
                "{lang:?} enum variant casing must agree with the shared authority"
            );
        }
    }
}
