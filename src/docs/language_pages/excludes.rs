use crate::core::config::{Language, ResolvedCrateConfig};
use std::collections::HashSet;

/// The function and type names excluded from `lang`'s bindings by `alef.toml`.
///
/// Every arm folds in the crate-wide `[crates.exclude]` list plus zero or more per-language
/// `exclude_functions`/`exclude_types` lists, unioned rather than replaced. The FFI-derived
/// language families (`go`, `java`, `kotlin`, `kotlin_android`, `csharp`) additionally fold in
/// `[crates.ffi]`, since their generated code calls through the C ABI that backend emits.
/// `kotlin_android` and `jni` additionally fold in `[crates.jni].exclude_functions`: the JNI
/// shim crate (`alef-backend-jni`) is what `kotlin_android` actually calls through, so a
/// function excluded only at the JNI level has no shim for `kotlin_android` to reach either.
/// `[crates.jni]` has no `exclude_types` field, so only the function half is folded for it.
///
/// This is the ONLY exclusion surface this function reads. It does not consult
/// `[opaque_types]` (a type-remapping declaration, not an exclusion list), `#[alef::skip]` /
/// `#[doc(hidden)]` (the extraction-time `binding_excluded` IR flag, honored separately and
/// uniformly across every language by each consumer -- e.g. `generate_lang_doc`'s own
/// `!f.binding_excluded` filter), or any crate-level override -- `[[crates]]` is a plain array
/// keyed by `name`, not a `[workspace.crates."<name>"]` map, and there is no such override
/// surface to fold in. ~keep
pub(crate) fn language_excludes(config: &ResolvedCrateConfig, lang: Language) -> (HashSet<String>, HashSet<String>) {
    let mut functions: HashSet<String> = config.exclude.functions.iter().cloned().collect();
    let mut types: HashSet<String> = config.exclude.types.iter().cloned().collect();

    match lang {
        Language::Python => {
            if let Some(c) = &config.python {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Node => {
            if let Some(c) = &config.node {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Ruby => {
            if let Some(c) = &config.ruby {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Php => {
            if let Some(c) = &config.php {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
            types.extend(
                crate::backends::php::excluded_visitor_type_names(config)
                    .into_iter()
                    .map(str::to_string),
            );
        }
        Language::Elixir => {
            if let Some(c) = &config.elixir {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Wasm => {
            if let Some(c) = &config.wasm {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Ffi | Language::C => {
            if let Some(c) = &config.ffi {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Go => {
            if let Some(c) = &config.go {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
            if let Some(c) = &config.ffi {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Java => {
            if let Some(c) = &config.java {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
            if let Some(c) = &config.ffi {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Kotlin => {
            if let Some(c) = &config.kotlin {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
            if let Some(c) = &config.ffi {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::KotlinAndroid => {
            if let Some(c) = &config.kotlin_android {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
            if let Some(c) = &config.ffi {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
            // KotlinAndroid is JNI's consumer: the paired `[crates.kotlin_android]` section
            // configures the Kotlin surface, but the JNI shim crate itself
            // (`alef-backend-jni`) is the one that actually honors `[crates.jni]`. A function
            // excluded only there still has no JNI shim to call through, so it must drop out
            // of the KotlinAndroid docs/ledger surface too, not just the JNI one. ~keep
            if let Some(c) = &config.jni {
                functions.extend(c.exclude_functions.iter().cloned());
            }
        }
        Language::Jni => {
            if let Some(c) = &config.jni {
                functions.extend(c.exclude_functions.iter().cloned());
            }
            if let Some(c) = &config.ffi {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Swift => {
            if let Some(c) = &config.swift {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Dart => {
            if let Some(c) = &config.dart {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Gleam => {
            if let Some(c) = &config.gleam {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Csharp => {
            if let Some(c) = &config.csharp {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
            if let Some(c) = &config.ffi {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::Zig => {
            if let Some(c) = &config.zig {
                extend_excludes(&mut functions, &mut types, &c.exclude_functions, &c.exclude_types);
            }
        }
        Language::R | Language::Rust => {}
    }

    (functions, types)
}

pub(super) fn extend_excludes(
    functions: &mut HashSet<String>,
    types: &mut HashSet<String>,
    exclude_functions: &[String],
    exclude_types: &[String],
) {
    functions.extend(exclude_functions.iter().cloned());
    types.extend(exclude_types.iter().cloned());
}
