//! Validates `[e2e.call(s).*.overrides.<lang>] class` against the classes a backend
//! actually emits.
//!
//! `class` names the host-language type generated tests and snippets call methods on
//! (a crate facade, a struct/enum wrapper, or a trait bridge). Nothing checked this
//! value against the backend's own naming before it reached the emitter, so a typo or
//! a stale rename silently produced calls against a class that does not exist —
//! surfacing only as a wall of compile errors in generated code, never at config time.
//! See `crate::e2e::validate` for the sibling checks this mirrors (unknown call
//! references, field-classification-vs-IR mismatches).

use super::diagnostic_log::{DiagnosticLog, unreported};
use super::validate::{Severity, ValidationError};
use crate::codegen::naming::{self, PublicIdentifierKind};
use crate::core::config::e2e::{CallConfig, E2eConfig};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{EnumDef, TypeDef};

const CONFIG_FILE_LABEL: &str = "alef.toml";

/// Languages whose e2e generators read `CallOverride::class`. Kept as an explicit list
/// rather than derived from `Language::ALL` because most languages (python, node, go,
/// rust, csharp, ...) never consult this field — see the backend generators under
/// `src/e2e/codegen/`.
const CLASS_CONSUMING_LANGUAGES: &[(&str, Language)] = &[
    ("java", Language::Java),
    ("kotlin", Language::Kotlin),
    ("kotlin_android", Language::KotlinAndroid),
    ("php", Language::Php),
    ("ruby", Language::Ruby),
    ("dart", Language::Dart),
];

fn naming_language_for(lang: &str) -> Option<Language> {
    CLASS_CONSUMING_LANGUAGES
        .iter()
        .find(|(name, _)| *name == lang)
        .map(|(_, language)| *language)
}

/// Run [`validate_call_class_overrides`], log every diagnostic `log` has not already reported,
/// and turn any
/// `Severity::Error` into a generation-aborting error naming every offending config key.
///
/// Kept here rather than inlined at the `generate_e2e` call site for the same reason
/// `validate_call_result_type::enforce_call_result_type_overrides` is: `src/e2e/mod.rs`
/// sits right at this repo's 1,000-line file cap, and this log-filter-bail glue is
/// boilerplate this module owns anyway.
pub fn enforce_call_class_overrides(
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    languages: &[String],
    log: &DiagnosticLog,
) -> anyhow::Result<()> {
    let diagnostics = validate_call_class_overrides(e2e_config, config, type_defs, enums, languages);
    for diag in unreported(&diagnostics, log) {
        tracing::warn!("{}: {}", diag.file, diag.message);
    }
    let errors: Vec<_> = diagnostics
        .iter()
        .filter(|diag| diag.severity == Severity::Error)
        .collect();
    if errors.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "e2e call class override validation failed: {}",
        errors
            .iter()
            .map(|diag| format!("{}: {}", diag.file, diag.message))
            .collect::<Vec<_>>()
            .join("; ")
    );
}

/// Validate every `class` override in `e2e_config` (the default `[e2e.call]` and every
/// named `[e2e.calls.*]`) against the set of host-language class names the target
/// backend will actually emit for this crate.
///
/// Skipped entirely when both `type_defs` and `enums` are empty: several legitimate
/// callers (unit tests, snippet entry points, generation paths that fall back to
/// explicit call-override mappings — see `crate::e2e::generate_e2e`'s doc comment) pass
/// an empty IR, and validating against a deliberately incomplete candidate set would
/// manufacture false positives rather than catch a real typo. Mirrors the same
/// "absent IR licenses no claim" rule `validate::validate_field_classifications` uses.
pub fn validate_call_class_overrides(
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    languages: &[String],
) -> Vec<ValidationError> {
    if type_defs.is_empty() && enums.is_empty() {
        return Vec::new();
    }

    let mut errors = Vec::new();
    let mut sources: Vec<(String, &CallConfig)> = vec![("[e2e.call]".to_string(), &e2e_config.call)];
    let mut named_calls: Vec<(&String, &CallConfig)> = e2e_config.calls.iter().collect();
    named_calls.sort_by_key(|(name, _)| (*name).clone());
    for (name, call) in named_calls {
        sources.push((format!("[e2e.calls.{name}]"), call));
    }

    for (config_key, call) in sources {
        let mut override_langs: Vec<&String> = call.overrides.keys().collect();
        override_langs.sort();
        for lang in override_langs {
            if !languages.iter().any(|resolved| resolved == lang) {
                continue;
            }
            let Some(naming_lang) = naming_language_for(lang) else {
                continue;
            };
            let Some(class_value) = call.overrides.get(lang).and_then(|o| o.class.as_ref()) else {
                continue;
            };
            check_class_override(
                &config_key,
                lang,
                naming_lang,
                class_value,
                config,
                type_defs,
                enums,
                &mut errors,
            );
        }
    }
    errors
}

#[allow(clippy::too_many_arguments)]
fn check_class_override(
    config_key: &str,
    lang: &str,
    naming_lang: Language,
    class_value: &str,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    errors: &mut Vec<ValidationError>,
) {
    let (candidates, facade_known) = emitted_class_names(lang, naming_lang, config, type_defs, enums);
    let simple_name = simple_class_name(class_value);
    if candidates.iter().any(|candidate| candidate == simple_name) {
        return;
    }

    // A backend whose crate-facade name we cannot derive gives us an incomplete candidate
    // set: the override may well name that undiscoverable facade correctly. Claiming it is
    // wrong on the strength of a set we know is missing an entry would be a false positive
    // wearing an "Error" severity, so this downgrades to a warning instead of bailing — see
    // `crate_facade_class_names`'s doc comment for which backends this applies to (none of
    // the six currently wired ones; this is a guard against a future backend being added to
    // `CLASS_CONSUMING_LANGUAGES` without its facade derivation).
    let severity = if facade_known {
        Severity::Error
    } else {
        Severity::Warning
    };

    let suggestion = closest_candidates(simple_name, &candidates);
    let suggestion_text = if suggestion.is_empty() {
        String::new()
    } else {
        format!(" (did you mean {}?)", suggestion.join(" or "))
    };
    let incomplete_text = if facade_known {
        ""
    } else {
        " (this backend's crate-facade name could not be derived, so this candidate set may be incomplete)"
    };
    errors.push(ValidationError {
        file: CONFIG_FILE_LABEL.to_string(),
        message: format!(
            "{config_key}.overrides.{lang}.class = \"{class_value}\" does not match any class the {lang} backend \
             emits for crate '{}'{suggestion_text}{incomplete_text}",
            config.name
        ),
        severity,
    });
}

/// The host-language class name(s) the `naming_lang` backend's crate facade is emitted
/// under, i.e. what `[crates.<lang>]` config resolves to once the backend's own
/// class-naming convention is applied — not a generic PascalCase of the Rust crate name.
///
/// Each arm calls the exact function the corresponding backend's codegen uses for this name
/// (not a re-derivation), so a rename on either side breaks a test rather than silently
/// drifting:
/// - `Kotlin` emits the crate module object via `to_pascal_case(crate_name)` with no
///   suffix stripping (`src/backends/kotlin/gen_bindings/mod.rs`'s `generate_jvm`, `module_name`
///   binding). That `to_pascal_case` is `shared::kotlin_pascal_case`, i.e.
///   `naming::public_host_identifier(Language::Kotlin, PublicIdentifierKind::Type, crate_name)`
///   — not bare `heck::ToPascalCase` — because it also backtick-escapes Kotlin keywords, so
///   this arm calls that same function rather than the generic `to_class_name` helper other
///   arms use for languages with no keyword-escaping step. ~keep
/// - `Java` emits *two* real classes an override can legitimately name: the raw FFI wrapper
///   via `crate::backends::java::naming::main_class_name` (PascalCased crate name, trailing
///   `Rs` kept/added — `src/backends/java/gen_bindings/mod.rs`'s `main_class`), and the
///   public facade that delegates to it via `..::public_class_name` (the same name with `Rs`
///   stripped — that file's `public_class`). Both are genuine, alef-emitted classes; an e2e
///   call override is free to target either. ~keep
/// - `KotlinAndroid` emits its wrapper object via
///   `crate::codegen::naming::kotlin_android_wrapper_object_name`, the same PascalCase-then-
///   strip-`Rs` algorithm as Java's public facade, under its own name.
/// - `Php` emits *two* real classes an override can legitimately name, the same raw/public
///   split as Java and Ruby: the hand-facing wrapper class via
///   `crate::backends::php::naming::php_public_class_name` (extension_name PascalCased, no
///   suffix — `src/backends/php/gen_bindings/public_api.rs`'s `class_name`), and the
///   `#[php_class]` extension class that actually holds the generated static methods via
///   `..::php_ext_api_class_name` (the same name with an `Api` suffix appended — the
///   `#[php(name = "...")]` struct emitted into `lib.rs`). The wrapper forwards to the `Api`
///   class, so both are genuine, independently callable entry points; an e2e call override is
///   free to target either. ~keep
/// - `Ruby` emits *two* real modules an override can legitimately name, the same
///   raw/public split as Java: the compiled native extension registers itself as
///   `to_pascal_case(crate_name)` (`src/backends/magnus/gen_bindings/mod.rs`'s private
///   `get_module_name(&api.crate_name)`), and a wrapper module re-exports its types and
///   delegates its functions under `to_pascal_case([crates.ruby] gem_name)` (that file's
///   `get_module_name(&config.ruby_gem_name())`). `get_module_name` is `crate_name.to_upper_camel_case()`
///   and this arm uses `naming::to_class_name`, i.e. `heck::ToPascalCase`; unlike the Kotlin
///   drift above this is not a coincidence to fix but a provable identity — heck's
///   `impl ToPascalCase for T { fn to_pascal_case(&self) { self.to_upper_camel_case() } }`
///   (`heck::upper_camel`) makes `ToPascalCase` a compile-time alias of `ToUpperCamelCase`, so
///   the two calls can never diverge without a heck major-version change this crate would pin
///   against anyway. For a crate named `sample-widget-rs`, the crate name PascalCases to `SampleWidgetRs` (the
///   native module `lib/sample_widget/native.rb` loads and `const_get`s from), while
///   its `[crates.ruby] gem_name = "sample-widget"` PascalCases to `SampleWidget`,
///   the wrapper `lib/sample_widget.rb` declares and e2e specs call `.convert` on
///   directly — `native.rb` wires `SampleWidget.convert` to forward to
///   `SampleWidgetRs.convert`, so both names are genuine, independently callable
///   entry points. ~keep
/// - `Dart` emits its FRB bridge class as `ResolvedCrateConfig::dart_bridge_class_name`,
///   which is already the exact name the `dart` e2e generator's default receiver uses.
///
/// Returns `None` for any language not covered above. Every language actually reachable
/// through `CLASS_CONSUMING_LANGUAGES` today is covered; `None` exists so a future addition
/// to that list is forced to either wire up a real arm here or accept the warning-only
/// fallback in `check_class_override`, rather than inheriting a guessed candidate silently.
fn crate_facade_class_names(naming_lang: Language, config: &ResolvedCrateConfig) -> Option<Vec<String>> {
    match naming_lang {
        Language::Kotlin => Some(vec![naming::public_host_identifier(
            Language::Kotlin,
            PublicIdentifierKind::Type,
            &config.name,
        )]),
        Language::Java => Some(vec![
            crate::backends::java::naming::main_class_name(&config.name),
            crate::backends::java::naming::public_class_name(&config.name),
        ]),
        Language::KotlinAndroid => Some(vec![naming::kotlin_android_wrapper_object_name(&config.name)]),
        Language::Php => Some(vec![
            crate::backends::php::naming::php_public_class_name(&config.php_extension_name()),
            crate::backends::php::naming::php_ext_api_class_name(&config.php_extension_name()),
        ]),
        Language::Ruby => Some(vec![
            naming::to_class_name(&config.name),
            naming::to_class_name(&config.ruby_gem_name()),
        ]),
        Language::Dart => Some(vec![config.dart_bridge_class_name()]),
        _ => None,
    }
}

/// The host-language class names the `lang` backend actually emits for this crate: the
/// crate facade, every struct/enum wrapper, and every active trait bridge.
///
/// Returns whether the facade name itself could be derived alongside the candidate list —
/// `check_class_override` uses that flag to decide whether a non-match is a real typo
/// (`Severity::Error`) or an unverifiable guess (`Severity::Warning`).
fn emitted_class_names(
    lang: &str,
    naming_lang: Language,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> (Vec<String>, bool) {
    let facade = crate_facade_class_names(naming_lang, config);
    let facade_known = facade.is_some();
    let mut names: Vec<String> = facade.into_iter().flatten().collect();
    for type_def in type_defs {
        names.push(naming::public_host_identifier(
            naming_lang,
            PublicIdentifierKind::Type,
            &type_def.name,
        ));
    }
    for enum_def in enums {
        names.push(naming::public_host_identifier(
            naming_lang,
            PublicIdentifierKind::Type,
            &enum_def.name,
        ));
    }
    for bridge in &config.trait_bridges {
        if bridge.is_active_for(lang) {
            names.push(format!("{}Bridge", bridge.trait_name));
        }
    }
    names.sort();
    names.dedup();
    (names, facade_known)
}

/// The trailing class name of a possibly-qualified override value. Java/Kotlin/Dart use
/// `.`-separated packages, Ruby uses `::`-nested modules, PHP uses `\`-namespaces —
/// candidates are always bare names, so a qualified override is compared on its last
/// segment, mirroring how `src/e2e/codegen/java/snippet.rs` resolves an FQN override
/// down to a simple name for import handling.
fn simple_class_name(raw: &str) -> &str {
    let after_dot = raw.rsplit('.').next().unwrap_or(raw);
    let after_namespace = after_dot.rsplit("::").next().unwrap_or(after_dot);
    after_namespace.rsplit('\\').next().unwrap_or(after_namespace)
}

/// Up to two candidates closest to `value` by Levenshtein edit distance, ascending.
///
/// `pub(crate)`, not private: `validate_call_result_type` reuses this exact ranking rather
/// than duplicating it, since a second override-value validator wanting the same
/// did-you-mean behavior doesn't justify a whole new shared module for two functions used
/// by exactly two callers.
pub(crate) fn closest_candidates(value: &str, candidates: &[String]) -> Vec<String> {
    let mut scored: Vec<(usize, &String)> = candidates
        .iter()
        .map(|candidate| (levenshtein_distance(value, candidate), candidate))
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(2)
        .map(|(_, candidate)| format!("\"{candidate}\""))
        .collect()
}

/// Classic Wagner-Fischer edit distance. No external dependency, small inputs (class
/// names), so the O(n*m) table is negligible.
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, &character_a) in a.iter().enumerate() {
        let mut previous_diagonal = row[0];
        row[0] = i + 1;
        for (j, &character_b) in b.iter().enumerate() {
            let above = row[j + 1];
            let cost = usize::from(character_a != character_b);
            let substitution = previous_diagonal + cost;
            let insertion = row[j] + 1;
            let deletion = above + 1;
            previous_diagonal = above;
            row[j + 1] = substitution.min(insertion).min(deletion);
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::e2e::CallOverride;
    use crate::core::config::trait_bridge::TraitBridgeConfig;
    use crate::core::ir::FieldDef;

    fn make_config(crate_name: &str) -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            name: crate_name.to_string(),
            ..ResolvedCrateConfig::default()
        }
    }

    fn make_type(name: &str) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            fields: vec![FieldDef::default()],
            ..TypeDef::default()
        }
    }

    fn make_e2e_config(class: &str, lang: &str) -> E2eConfig {
        let mut call = CallConfig::default();
        let override_config = CallOverride {
            class: Some(class.to_string()),
            ..CallOverride::default()
        };
        call.overrides.insert(lang.to_string(), override_config);
        E2eConfig {
            call,
            ..E2eConfig::default()
        }
    }

    #[test]
    fn a_class_override_matching_an_emitted_struct_passes() {
        let config = make_config("sample_crate");
        let type_defs = vec![make_type("DocumentApi")];
        let e2e_config = make_e2e_config("DocumentApi", "java");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["java".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn a_misspelled_class_override_fails_with_the_offending_language_and_value() {
        let config = make_config("sample_crate");
        let type_defs = vec![make_type("DocumentApi")];
        let e2e_config = make_e2e_config("DocumentAppi", "java");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["java".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert_eq!(errors[0].severity, Severity::Error);
        assert_eq!(errors[0].file, "alef.toml");
        assert!(
            errors[0]
                .message
                .contains("[e2e.call].overrides.java.class = \"DocumentAppi\""),
            "got: {}",
            errors[0].message
        );
        assert!(
            errors[0]
                .message
                .contains("java backend emits for crate 'sample_crate'"),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn the_suggestion_names_the_closest_candidate() {
        let config = make_config("sample_crate");
        let type_defs = vec![make_type("DocumentApi")];
        let e2e_config = make_e2e_config("DocumentAppi", "java");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["java".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert!(
            errors[0].message.contains("did you mean \"DocumentApi\""),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn an_absent_override_is_a_no_op() {
        let config = make_config("sample_crate");
        let type_defs = vec![make_type("DocumentApi")];
        let e2e_config = E2eConfig::default();

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["java".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn a_fully_qualified_class_override_is_compared_on_its_simple_name() {
        let config = make_config("sample_crate");
        let type_defs = vec![make_type("SampleService")];
        let e2e_config = make_e2e_config("dev.example.SampleService", "java");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["java".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn a_crate_facade_override_passes_with_no_ir_types() {
        let config = make_config("sample_crate");
        let type_defs = vec![make_type("Placeholder")];
        let e2e_config = make_e2e_config("SampleCrate", "java");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["java".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn an_active_trait_bridge_class_override_passes() {
        let mut config = make_config("sample_crate");
        config.trait_bridges = vec![TraitBridgeConfig {
            trait_name: "Validator".to_string(),
            ..TraitBridgeConfig::default()
        }];
        let type_defs = vec![make_type("Placeholder")];
        let e2e_config = make_e2e_config("ValidatorBridge", "kotlin_android");

        let errors =
            validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["kotlin_android".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn an_empty_ir_skips_validation_entirely() {
        let config = make_config("sample_crate");
        let e2e_config = make_e2e_config("TotallyWrongClassName", "java");

        let errors = validate_call_class_overrides(&e2e_config, &config, &[], &[], &["java".to_string()]);

        assert_eq!(errors.len(), 0, "an absent IR must license no claim: {errors:?}");
    }

    #[test]
    fn a_language_that_does_not_consume_class_is_never_checked() {
        let config = make_config("sample_crate");
        let type_defs = vec![make_type("DocumentApi")];
        let e2e_config = make_e2e_config("TotallyWrongClassName", "python");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["python".to_string()]);

        assert_eq!(errors.len(), 0, "python does not consume `class`: {errors:?}");
    }

    #[test]
    fn a_named_call_override_names_its_own_config_key() {
        let config = make_config("sample_crate");
        let type_defs = vec![make_type("DocumentApi")];
        let mut e2e_config = E2eConfig::default();
        let mut call = CallConfig::default();
        let override_config = CallOverride {
            class: Some("NotARealClass".to_string()),
            ..CallOverride::default()
        };
        call.overrides.insert("java".to_string(), override_config);
        e2e_config.calls.insert("summarize".to_string(), call);

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["java".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert!(
            errors[0]
                .message
                .starts_with("[e2e.calls.summarize].overrides.java.class"),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn levenshtein_distance_matches_known_values() {
        assert_eq!(levenshtein_distance("DocumentApi", "DocumentApi"), 0);
        assert_eq!(levenshtein_distance("DocumentAppi", "DocumentApi"), 1);
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
    }

    fn resolved_one(toml: &str) -> ResolvedCrateConfig {
        use crate::core::config::new_config::NewAlefConfig;
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    /// Reproduces the exact reported regression: a crate whose name PascalCases to
    /// `SampleWidgetRs`, but whose kotlin_android backend strips the trailing `Rs` and
    /// emits the wrapper object as `SampleWidget`. A bare override naming the real,
    /// stripped facade must pass.
    #[test]
    fn a_bare_kotlin_android_override_matching_the_rs_stripped_facade_passes() {
        let config = make_config("sample-widget-rs");
        let type_defs = vec![make_type("Placeholder")];
        let e2e_config = make_e2e_config("SampleWidget", "kotlin_android");

        let errors =
            validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["kotlin_android".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    /// Same regression as above, but with the fully-qualified form the reported override
    /// actually used (`com.example.android.SampleWidget`), proving qualified overrides are
    /// compared against the corrected, Rs-stripped facade name too.
    #[test]
    fn a_fully_qualified_kotlin_android_override_matching_the_rs_stripped_facade_passes() {
        let config = make_config("sample-widget-rs");
        let type_defs = vec![make_type("Placeholder")];
        let e2e_config = make_e2e_config("com.example.android.SampleWidget", "kotlin_android");

        let errors =
            validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["kotlin_android".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    /// The `java` backend's public facade strips the same trailing `Rs` as `kotlin_android`
    /// (`main_class_name` then `.trim_end_matches("Rs")`), so a bare override naming that
    /// stripped facade must pass.
    #[test]
    fn a_bare_java_override_matching_the_rs_stripped_facade_passes() {
        let config = make_config("sample-widget-rs");
        let type_defs = vec![make_type("Placeholder")];
        let e2e_config = make_e2e_config("SampleWidget", "java");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["java".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    /// The `java` backend also emits the raw FFI wrapper class the public facade delegates
    /// to, `main_class_name` (the `Rs`-suffixed name, never stripped) -- this is the exact
    /// shape of a previously-passing fully-qualified override
    /// (`com.example.samplewidget.SampleWidgetRs`). Narrowing the candidate set to only the
    /// stripped public facade would regress this override from passing to failing. ~keep
    #[test]
    fn a_fully_qualified_java_override_matching_the_unstripped_raw_class_passes() {
        let config = make_config("sample-widget-rs");
        let type_defs = vec![make_type("Placeholder")];
        let e2e_config = make_e2e_config("com.example.samplewidget.SampleWidgetRs", "java");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["java".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    /// The other reported regression: `[crates.php] extension_name` diverges from the crate
    /// name, and the php facade is PascalCased from `extension_name`, not from the crate
    /// name. A bare override naming that facade must pass.
    #[test]
    fn a_bare_php_override_matching_the_configured_extension_name_passes() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["php"]

[[crates]]
name = "sample-widget-rs"
sources = ["src/lib.rs"]

[crates.php]
extension_name = "sample_widget"
"#,
        );
        let type_defs = vec![make_type("Placeholder")];
        let e2e_config = make_e2e_config("SampleWidget", "php");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["php".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    /// The ruby backend's compiled native extension registers itself under a module
    /// PascalCased from the crate name (`get_module_name(&api.crate_name)`), independent of
    /// `[crates.ruby] gem_name`. A bare override naming that raw native module must pass even
    /// when `gem_name` is configured to something else entirely.
    #[test]
    fn a_bare_ruby_override_matching_the_crate_name_native_module_passes_regardless_of_gem_name() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "sample-widget-rs"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "sample-widget"
"#,
        );
        let type_defs = vec![make_type("Placeholder")];
        let e2e_config = make_e2e_config("SampleWidgetRs", "ruby");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["ruby".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    /// The ruby backend also emits a wrapper module PascalCased from `[crates.ruby]
    /// gem_name` that re-exports the native module's types and delegates its functions
    /// (`get_module_name(&config.ruby_gem_name())`) — a real, independently callable module
    /// an override is equally free to name, even when `gem_name` diverges from the crate
    /// name (`gem_name` may contain `-`, which Ruby forbids in module names, so this is
    /// never a no-op PascalCase of the crate name itself). ~keep
    #[test]
    fn a_bare_ruby_override_matching_the_gem_name_wrapper_module_passes() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "sample-widget-rs"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "sample-widget"
"#,
        );
        let type_defs = vec![make_type("Placeholder")];
        let e2e_config = make_e2e_config("SampleWidget", "ruby");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["ruby".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    /// The dart backend's FRB bridge class is `ResolvedCrateConfig::dart_bridge_class_name`
    /// (`[crates.dart] lib_name`, PascalCased, plus a `Bridge` suffix), which the crate-name
    /// PascalCase this validator used to compute never produces.
    #[test]
    fn a_bare_dart_override_matching_the_bridge_class_name_passes() {
        let config = resolved_one(
            r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "sample-widget-rs"
sources = ["src/lib.rs"]

[crates.dart]
lib_name = "widget"
"#,
        );
        let type_defs = vec![make_type("Placeholder")];
        let e2e_config = make_e2e_config("WidgetBridge", "dart");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["dart".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    /// A genuinely wrong override — one that matches neither the corrected facade name nor
    /// any IR type — must still be rejected. The fix to the candidate set must not turn this
    /// validator into a no-op.
    #[test]
    fn a_genuinely_absent_kotlin_android_class_is_still_rejected() {
        let config = make_config("sample-widget-rs");
        let type_defs = vec![make_type("HtmlMetadata")];
        let e2e_config = make_e2e_config("com.example.android.SampleWidgetConverter", "kotlin_android");

        let errors =
            validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["kotlin_android".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert_eq!(errors[0].severity, Severity::Error);
        assert!(
            errors[0]
                .message
                .contains("does not match any class the kotlin_android backend emits"),
            "got: {}",
            errors[0].message
        );
    }

    /// A backend whose facade-name derivation is not wired into `crate_facade_class_names`
    /// (simulated here via a `naming_lang` this module does not handle, since every language
    /// actually reachable through `CLASS_CONSUMING_LANGUAGES` today is handled) must not
    /// claim an override is wrong on the strength of a candidate set it knows is incomplete
    /// — it downgrades to a warning instead of an error.
    #[test]
    fn an_unresolvable_facade_backend_downgrades_the_diagnostic_to_a_warning() {
        let config = make_config("sample_crate");
        let type_defs = vec![make_type("Placeholder")];
        let mut errors = Vec::new();

        check_class_override(
            "[e2e.call]",
            "mystery_backend",
            Language::Python,
            "TotallyUnverifiable",
            &config,
            &type_defs,
            &[],
            &mut errors,
        );

        assert_eq!(errors.len(), 1, "expected exactly one diagnostic, got: {errors:?}");
        assert_eq!(errors[0].severity, Severity::Warning);
        assert!(
            errors[0].message.contains("candidate set may be incomplete"),
            "got: {}",
            errors[0].message
        );
    }

    /// Guards against the exact failure mode that let the Php arm ship broken: a test
    /// literally named "matches each backend's own derivation" that in fact never called
    /// any backend's own derivation for Php, and would have kept passing even if a second
    /// arm (or every arm) suffered the same drift, because nothing here counted how many
    /// arms were actually exercised.
    ///
    /// Two things every arm below must do, not just one:
    /// 1. Compare `crate_facade_class_names`'s output against a call to the *real* function
    ///    the corresponding backend's codegen uses (never a hardcoded string, never a
    ///    re-derivation with a generic helper) — a rename on either side must break this
    ///    test rather than silently drift, per `crate_facade_class_names`'s doc comment.
    /// 2. Push its `Language` into `checked` so the final count assertion can catch an arm
    ///    being silently skipped (e.g. by a stray early `continue`/`return`, or simply
    ///    never being added when a language is wired into `CLASS_CONSUMING_LANGUAGES`).
    ///    Without that count check, a loop or a copy-paste that dropped an arm would pass
    ///    having verified nothing for it — indistinguishable, from the test's green result,
    ///    from having checked every arm.
    #[test]
    fn crate_facade_class_names_matches_each_backends_own_derivation() {
        let sample_widget = make_config("sample-widget-rs");
        let mut checked: Vec<Language> = Vec::new();

        checked.push(Language::Kotlin);
        assert_eq!(
            crate_facade_class_names(Language::Kotlin, &sample_widget),
            Some(vec![naming::public_host_identifier(
                Language::Kotlin,
                PublicIdentifierKind::Type,
                &sample_widget.name,
            )]),
            "kotlin must match shared::kotlin_pascal_case (public_host_identifier), the \
             keyword-escaping function the real emitter uses — not bare heck::ToPascalCase"
        );

        checked.push(Language::Java);
        assert_eq!(
            crate_facade_class_names(Language::Java, &sample_widget),
            Some(vec![
                crate::backends::java::naming::main_class_name(&sample_widget.name),
                crate::backends::java::naming::public_class_name(&sample_widget.name),
            ]),
            "java's raw FFI class and its Rs-stripped public facade are both real candidates"
        );

        checked.push(Language::KotlinAndroid);
        assert_eq!(
            crate_facade_class_names(Language::KotlinAndroid, &sample_widget),
            Some(vec![naming::kotlin_android_wrapper_object_name(&sample_widget.name)])
        );

        checked.push(Language::Php);
        assert_eq!(
            crate_facade_class_names(Language::Php, &sample_widget),
            Some(vec![
                crate::backends::php::naming::php_public_class_name(&sample_widget.php_extension_name()),
                crate::backends::php::naming::php_ext_api_class_name(&sample_widget.php_extension_name()),
            ]),
            "php's hand-facing wrapper class and the Api-suffixed #[php_class] extension \
             facade it forwards to are both real, independently callable candidates"
        );

        checked.push(Language::Ruby);
        assert_eq!(
            crate_facade_class_names(Language::Ruby, &sample_widget),
            Some(vec![
                crate::backends::magnus::gen_bindings::get_module_name(&sample_widget.name),
                crate::backends::magnus::gen_bindings::get_module_name(&sample_widget.ruby_gem_name()),
            ]),
            "ruby's native module (crate name) and its gem_name-derived wrapper module are \
             both real candidates; with no `[crates.ruby] gem_name` configured, gem_name \
             defaults to the crate name with hyphens replaced by underscores, which \
             PascalCases identically to the crate name itself"
        );

        checked.push(Language::Dart);
        assert_eq!(
            crate_facade_class_names(Language::Dart, &sample_widget),
            Some(vec![sample_widget.dart_bridge_class_name()])
        );

        assert_eq!(
            checked.len(),
            CLASS_CONSUMING_LANGUAGES.len(),
            "every language wired into CLASS_CONSUMING_LANGUAGES must have its own comparison \
             against that backend's real naming function in this test; a missing comparison \
             would let this test pass having verified nothing for that arm, which is exactly \
             how the Php arm shipped broken"
        );
        for (name, lang) in CLASS_CONSUMING_LANGUAGES {
            assert!(
                checked.contains(lang),
                "CLASS_CONSUMING_LANGUAGES lists \"{name}\" but this test never compared it \
                 against that backend's real derivation"
            );
        }

        assert_eq!(crate_facade_class_names(Language::Python, &sample_widget), None);
    }
}
