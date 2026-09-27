//! Every `native*` symbol the generated Kotlin calls must be both declared as an `external fun`
//! on the `*Bridge` object AND implemented as a `Java_..._native*` shim in the paired `jni`
//! Rust crate. Nothing else in this repo drives the kotlin_android emitter and the jni emitter
//! over the same IR and cross-checks the two sides: kotlin_android decides Kotlin-side
//! reachability (`crate::backends::kotlin::handle_only_type_names`, fed by
//! `kotlin_visible_functions`/`kotlin_exclude_functions`), while `jni` decides which native
//! shims it actually implements, and each used to compute that reachability independently. A
//! function excluded only via `[crates.jni].exclude_functions` (documented as excluding a
//! function from JNI *shim generation* only — e.g. so a consumer can hand-write that one native
//! symbol in a sibling file of the same `-jni` crate — not from Kotlin's own call surface) used
//! to also narrow the jni backend's notion of which opaque return types were reachable, so the
//! `nativeFree<Type>` destructor Kotlin still calls silently lost its native implementation.
//! `every_native_method_kotlin_calls_is_declared_and_implemented` below is the general,
//! symmetric, no-exceptions cross-check; `jni_only_exclude_functions_does_not_starve_the_destructor`
//! is the exact regression shape (alef task #188 / liter-llm's Android AAR). ~keep

use alef::backends::jni::JniBackend;
use alef::backends::kotlin_android::KotlinAndroidBackend;
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{JniConfig, KotlinAndroidConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
use std::collections::BTreeSet;

const CRATE_NAME: &str = "sample_llm";
const CLIENT_TYPE: &str = "Client";
const RESULT_TYPE: &str = "SingleflightResult";

/// A `Client` with one instance method returning `SingleflightResult`, an opaque type with no
/// instance methods of its own — the minimal shape of "a type whose lifecycle needs an explicit
/// free/dispose" that kotlin_android's `handle_only_type_names` classifies as a handle wrapper
/// (a `.kt` class whose `close()` calls `nativeFree<Type>`) rather than a client class.
fn base_surface() -> ApiSurface {
    let client = TypeDef {
        name: CLIENT_TYPE.to_owned(),
        rust_path: format!("{CRATE_NAME}::{CLIENT_TYPE}"),
        is_opaque: true,
        methods: vec![MethodDef {
            name: "get_or_fetch".to_owned(),
            params: vec![ParamDef {
                name: "key".to_owned(),
                ty: TypeRef::String,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Named(RESULT_TYPE.to_owned()),
            receiver: Some(ReceiverKind::Ref),
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    };
    let result = TypeDef {
        name: RESULT_TYPE.to_owned(),
        rust_path: format!("{CRATE_NAME}::{RESULT_TYPE}"),
        is_opaque: true,
        ..TypeDef::default()
    };
    ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: CRATE_NAME.to_owned(),
        version: "0.1.0".to_owned(),
        types: vec![client, result],
        ..ApiSurface::default()
    }
}

fn base_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: CRATE_NAME.to_owned(),
        kotlin_android: Some(KotlinAndroidConfig {
            package: Some("dev.sample_llm".to_owned()),
            namespace: Some("dev.sample_llm".to_owned()),
            ..KotlinAndroidConfig::default()
        }),
        ..ResolvedCrateConfig::default()
    }
}

fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> (Vec<GeneratedFile>, String) {
    let kotlin_files = KotlinAndroidBackend
        .generate_bindings(api, config)
        .expect("kotlin_android generation should succeed");
    let jni_files = JniBackend
        .generate_bindings(api, config)
        .expect("jni generation should succeed");
    let jni_source = jni_files
        .into_iter()
        .find(|file| file.path.file_name().is_some_and(|name| name == "lib.rs"))
        .expect("jni backend must emit lib.rs")
        .content;
    (kotlin_files, jni_source)
}

fn kt_files(files: &[GeneratedFile]) -> Vec<(&str, &str)> {
    files
        .iter()
        .filter_map(|generated| {
            let name = generated.path.file_name()?.to_str()?;
            name.ends_with(".kt").then_some((name, generated.content.as_str()))
        })
        .collect()
}

/// Identifier starting at `text`, reading left to right.
fn leading_identifier(text: &str) -> Option<&str> {
    let end = text
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(text.len());
    (end > 0).then(|| &text[..end])
}

/// Every `native*` name declared as `external fun native*(` anywhere in the generated Kotlin.
fn declared_native_funs(files: &[(&str, &str)]) -> BTreeSet<String> {
    let mut declared = BTreeSet::new();
    for (_, content) in files {
        for line in content.lines() {
            let Some(rest) = line.trim_start().strip_prefix("external fun ") else {
                continue;
            };
            if let Some(name) = leading_identifier(rest)
                && name.starts_with("native")
            {
                declared.insert(name.to_owned());
            }
        }
    }
    declared
}

/// Every `<Receiver>.native*(` call site in the generated Kotlin, as `(file, symbol)`.
fn called_native_funs<'a>(files: &[(&'a str, &'a str)]) -> Vec<(&'a str, String)> {
    let mut calls = Vec::new();
    for (name, content) in files {
        for line in content.lines() {
            for (index, _) in line.match_indices('.') {
                let after = &line[index + 1..];
                let Some(ident) = leading_identifier(after) else {
                    continue;
                };
                if ident.starts_with("native") && after[ident.len()..].starts_with('(') {
                    calls.push((*name, ident.to_owned()));
                }
            }
        }
    }
    calls
}

/// Every `native*` symbol the jni backend actually implements, read back out of
/// `pub unsafe extern "system" fn Java_..._native*(`.
fn implemented_native_symbols(jni_source: &str) -> BTreeSet<String> {
    let mut implemented = BTreeSet::new();
    for line in jni_source.lines() {
        let Some(rest) = line.trim_start().strip_prefix("pub unsafe extern \"system\" fn Java_") else {
            continue;
        };
        let Some(open_paren) = rest.find('(') else {
            continue;
        };
        let full_name = &rest[..open_paren];
        // The JNI-encoded prefix is `<package>_<Class>`; the method segment is always the part
        // starting at the last `_native`, because generated method names are pure camelCase
        // (JNI's own `_1` underscore-escaping never produces a literal `_native` substring).
        if let Some(idx) = full_name.rfind("_native") {
            implemented.insert(full_name[idx + 1..].to_owned());
        }
    }
    implemented
}

#[test]
fn every_native_method_kotlin_calls_is_declared_and_implemented() {
    let api = base_surface();
    let config = base_config();
    let (kotlin_files, jni_source) = generate(&api, &config);
    let files = kt_files(&kotlin_files);

    let declared = declared_native_funs(&files);
    let calls = called_native_funs(&files);
    let implemented = implemented_native_symbols(&jni_source);

    // Non-vacuity: a run that parsed no declarations or calls would satisfy an empty-difference
    // assertion while examining nothing.
    assert!(
        declared.len() >= 2,
        "parsed {} `external fun native*` declarations — the declaration scanner stopped \
         recognising the emitted shape: {declared:?}",
        declared.len()
    );
    assert!(
        calls.len() >= 2,
        "found {} `.native*(` call sites — the call scanner stopped recognising the emitted \
         shape: {calls:?}",
        calls.len()
    );
    for required in ["nativeClientGetOrFetch", "nativeFreeSingleflightResult"] {
        assert!(
            calls.iter().any(|(_, symbol)| symbol == required),
            "the fixture must reach the handle-wrapper close(), which calls `{required}`; \
             without it this cross-check never examines the branch it exists for: {calls:?}"
        );
    }

    let undeclared: Vec<&(&str, String)> = calls.iter().filter(|(_, symbol)| !declared.contains(symbol)).collect();
    assert!(
        undeclared.is_empty(),
        "generated Kotlin calls native methods the Bridge object never declares as `external \
         fun` — Kotlin fails to compile with Unresolved reference:\n{undeclared:#?}\ndeclared: \
         {declared:?}"
    );

    let unimplemented: BTreeSet<&String> = declared
        .iter()
        .filter(|symbol| !implemented.contains(*symbol))
        .collect();
    assert!(
        unimplemented.is_empty(),
        "the Bridge object declares native methods the jni backend never implements — the AAR \
         links but throws UnsatisfiedLinkError the first time Kotlin calls one:\n{unimplemented:#?}\n\
         implemented: {implemented:?}"
    );
}

/// The exact regression shape behind alef task #188 (liter-llm's Kotlin Android AAR): a function
/// excluded ONLY via `[crates.jni].exclude_functions` (meant to exclude that one function's own
/// native shim — e.g. so a consumer hand-writes it in a sibling file of the same `-jni` crate —
/// not to hide the function from Kotlin) used to also narrow the jni backend's own notion of
/// which opaque return types were reachable. kotlin_android has no visibility into
/// `[crates.jni].exclude_functions` (correctly — it is a jni-shim-only concern) and keeps
/// declaring and calling both the excluded function's `external fun` and the `nativeFree<Type>`
/// destructor for what it returns. Before the fix, `nativeFreeSingleflightResult` was declared
/// and called but never implemented; the excluded function's OWN native shim
/// (`nativeGetBackupResult`) correctly stays unimplemented in both states — that half is by
/// design, since the consumer is expected to hand-write it elsewhere in the same crate. ~keep
#[test]
fn jni_only_exclude_functions_does_not_starve_the_destructor_it_still_needs() {
    let mut api = base_surface();
    // `base_surface()`'s `Client.get_or_fetch` also returns `SingleflightResult`, which would
    // keep it reachable through an unrelated, non-excluded path and mask the regression this
    // test exists to catch (the old, unfixed jni backend scanned every type's methods for
    // return-type reachability without applying `[crates.jni].exclude_functions` at all — see
    // `emit_opaque_return_destructors`'s history). Dropping the method makes
    // `get_backup_result` the *only* thing that reaches `SingleflightResult`, so the test
    // actually exercises the jni-only exclusion instead of passing regardless of it.
    api.types
        .iter_mut()
        .find(|type_def| type_def.name == CLIENT_TYPE)
        .expect("fixture must declare Client")
        .methods
        .clear();
    api.functions.push(FunctionDef {
        name: "get_backup_result".to_owned(),
        rust_path: format!("{CRATE_NAME}::get_backup_result"),
        return_type: TypeRef::Named(RESULT_TYPE.to_owned()),
        ..FunctionDef::default()
    });
    let mut config = base_config();
    config.jni = Some(JniConfig {
        exclude_functions: vec!["get_backup_result".to_owned()],
        ..JniConfig::default()
    });

    let (kotlin_files, jni_source) = generate(&api, &config);
    let files = kt_files(&kotlin_files);
    let declared = declared_native_funs(&files);
    let calls = called_native_funs(&files);
    let implemented = implemented_native_symbols(&jni_source);

    assert!(
        calls.iter().any(|(_, symbol)| symbol == "nativeGetBackupResult"),
        "the fixture must reach the top-level function call; without it this test examines \
         nothing: {calls:?}"
    );

    // The destructor for the type the excluded function returns must still be declared, called,
    // and implemented — Kotlin still calls it regardless of `[crates.jni].exclude_functions`.
    let destructor = "nativeFreeSingleflightResult";
    assert!(
        declared.contains(destructor),
        "the Bridge object must still declare `{destructor}`: {declared:?}"
    );
    assert!(
        calls.iter().any(|(_, symbol)| symbol == destructor),
        "the handle wrapper's close() must still call `{destructor}`: {calls:?}"
    );
    assert!(
        implemented.contains(destructor),
        "`[crates.jni].exclude_functions` narrowing which function gets its OWN native shim must \
         not also drop the destructor for what that function returns — `{destructor}` is missing \
         from the jni crate's implemented symbols: {implemented:?}\n{jni_source}"
    );

    // The excluded function's own native shim correctly stays unimplemented -- that part is
    // intentional (the consumer hand-writes it in a sibling file of the same `-jni` crate).
    assert!(
        !implemented.contains("nativeGetBackupResult"),
        "`[crates.jni].exclude_functions` should still exclude the function's own native shim; \
         a consumer relying on hand-writing it would now get a duplicate `#[no_mangle]` symbol: \
         {implemented:?}"
    );
}
