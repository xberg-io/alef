//! Typed-default normalisation and docs-file setup helpers, split out of args.rs.

use super::*;

/// A type-appropriate non-null literal for a declared-required Kotlin parameter with no fixture
/// value, keyed on `arg.arg_type`. Shared by the "genuinely required, no value" arm and the
/// "fixture claims optional, but the target requires it" arm so the two can never emit different
/// placeholders for the same situation — a bare `null` is only ever safe for a nullable Kotlin
/// type. ~keep
pub(super) fn typed_zero_default(arg_type: &str) -> String {
    match arg_type {
        "string" => "\"\"".to_string(),
        "int" | "integer" => "0".to_string(),
        "float" | "number" => "0.0".to_string(),
        "bool" | "boolean" => "false".to_string(),
        _ => "null".to_string(),
    }
}

/// Whether `type_name`'s Kotlin data class is known to have a zero-arg constructor. `None`
/// means UNKNOWN — `ctx.type_defs` has no `TypeDef` named `type_name` at all, which is the
/// normal state when no core IR is in scope (`type_defs` is empty) or the type is external to
/// this crate — and must NEVER be read as "not constructible". `default_constructible_types`
/// is a positive claim about types the fixpoint actually examined; its absence carries no
/// information, only ignorance. Regression pinned by
/// `android_snippet_keeps_the_constructor_fallback_when_no_ir_is_in_scope`,
/// `android_snippet_still_refuses_null_for_a_declared_required_argument`, and
/// `kotlin_android_optional_config_arg_emits_default_constructor_not_null`: reading an empty
/// set as "nothing is constructible" made every IR-less call site fall into the JSON-stub
/// branch below and emit `MAPPER.readValue("{}", ...)` from zero field knowledge — the `"{}"`
/// with nothing filled in was itself the tell that the set was empty for the wrong reason,
/// not that the type had been checked and found lacking. ~keep
pub(super) fn type_constructibility(type_name: &str, ctx: &KotlinFillContext<'_>) -> Option<bool> {
    if !ctx.type_defs.iter().any(|candidate| candidate.name == type_name) {
        return None;
    }
    Some(ctx.default_constructible_types.contains(type_name))
}

/// The `kotlin_android_style` half of the `json_object`-arg fallback when no fixture value
/// exists for a target-required parameter: a bare `TypeName()` when every constructor
/// parameter the real generated data class emits carries a Kotlin default
/// (`default_constructible_type_names`), or — when it is positively known NOT to — a
/// `MAPPER.readValue(...)` binding seeded from the same recursive JSON stub
/// `fill_missing_required_kotlin_fields` already builds for the `handle` arg path, pushed onto
/// `setup_lines` and returned by variable name instead of as a literal constructor call. See
/// `type_constructibility` for the "positively known" half of that sentence — an unknown type
/// (no IR in scope, or the type is external) must fall through to the bare constructor exactly
/// as before, not into the stub branch.
///
/// Issue #309, third instance this session: a capability existed on the `handle` path
/// (`KotlinFillContext` → `fill_missing_required_kotlin_fields`) and was simply absent here,
/// on the sibling `json_object` path, so the two diverged — this one kept splicing a bare
/// `TypeName()` with no constructibility check at all. A bare `TypeName()` for a type positively
/// known to be outside `default_constructible_types` does not compile (`No value passed for
/// parameter 'x'`) — the exact defect this closes for `ExtractionConfig.url:
/// UrlExtractionConfig`, itself bare only because `UrlExtractionConfig.crawl`'s real default is
/// a `PublicFunctionCall` alef cannot spell in Kotlin. See `fill_missing_required_kotlin_fields`'s
/// own doc comment for the parallel `crawl.ssrf` case this recurses through identically.
/// Point-fixing this call site alone would very likely leave a fourth #309 instance somewhere
/// else in this file; if another bare `TypeName()`/`.builder().build()` fallback turns up, route
/// it through this same helper (or its JVM counterpart) rather than adding a fourth hand-rolled
/// check. ~keep
///
/// `UrlExtractionConfig.crawl` having no Kotlin default is permanent, not a gap to close: its
/// Rust default rest-spreads a foreign crate's `impl Default`, which
/// alef's constant-folder cannot read across a crate boundary, so the value is genuinely unknown
/// to alef. Synthesizing a Kotlin literal default for it (or for `ExtractionConfig.url` in turn)
/// would be a guess that can silently disagree with the real Rust value once it crosses the JNI
/// boundary — the JSON-stub fallback below is the honest alternative for a type positively
/// known to need it, not a workaround for something that should instead grow a default. ~keep
pub(super) fn kotlin_android_json_object_default(
    type_name: &str,
    ctx: &KotlinFillContext<'_>,
    setup_lines: &mut Vec<String>,
    arg: &ArgMapping,
) -> String {
    // Only a POSITIVE "not constructible" finding may justify the JSON-stub substitution —
    // `None` (unknown) and `Some(true)` (known constructible) both keep the pre-existing bare
    // constructor, for opposite reasons: one has no evidence to act on, the other does not need
    // to. ~keep
    if type_constructibility(type_name, ctx) != Some(false) {
        return format!("{}()", type_name);
    }
    let stub = normalize_typed_json(&serde_json::Value::Object(serde_json::Map::new()), type_name, ctx);
    let json_str = serde_json::to_string(&stub).unwrap_or_default();
    let var_name = format!("{}Default", arg.name);
    setup_lines.push(format!(
        "val {var_name} = MAPPER.readValue({}, {type_name}::class.java)",
        super::super::values::kotlin_string_literal(&json_str),
    ));
    var_name
}

pub(super) fn normalize_typed_json(
    value: &serde_json::Value,
    type_name: &str,
    ctx: &KotlinFillContext<'_>,
) -> serde_json::Value {
    let Some(type_def) = ctx.type_defs.iter().find(|candidate| candidate.name == type_name) else {
        return crate::e2e::codegen::transform_json_keys_for_language(value, "snake_case");
    };
    let Some(object) = value.as_object() else {
        return value.clone();
    };
    let mut normalized = serde_json::Map::new();
    for (key, field_value) in object {
        let field = type_def.fields.iter().find(|field| {
            field.name == *key
                || crate::codegen::naming::wire_field_name(
                    &field.name,
                    field.serde_rename.as_deref(),
                    type_def.serde_rename_all.as_deref(),
                ) == *key
        });
        let Some(field) = field else {
            normalized.insert(key.clone(), field_value.clone());
            continue;
        };
        let wire_name = crate::codegen::naming::wire_field_name(
            &field.name,
            field.serde_rename.as_deref(),
            type_def.serde_rename_all.as_deref(),
        );
        normalized.insert(wire_name, normalize_typed_value(field_value, &field.ty, ctx));
    }
    let mut memo = std::collections::HashMap::new();
    let mut visiting = std::collections::HashSet::new();
    fill_missing_required_kotlin_fields(&mut normalized, type_def, ctx, &mut memo, &mut visiting);
    serde_json::Value::Object(normalized)
}

/// Materialise a JSON stub for every constructor-required field a `kotlin_android_style`
/// fixture literal left out, when doing so is provably safe — recursing through nested
/// required types rather than requiring the whole immediate field type to have a compilable
/// zero-arg Kotlin constructor.
///
/// `CrawlConfig.ssrf: SsrfPolicy` (no Kotlin default; `SsrfPolicy::from_env` is env-dependent
/// and genuinely unresolvable, see `kotlin_field_default`'s doc comment) makes `SsrfPolicy`
/// itself default-constructible once every one of *its* fields has a real Kotlin default, so
/// `"ssrf": {}` is enough there. But a field whose type is bare *because one of its own nested
/// fields* is one of these (`UrlExtractionConfig.crawl: CrawlConfig`, itself bare
/// only because of `crawl.ssrf`) is not in `default_constructible_types` as a whole — Jackson
/// cannot synthesise `UrlExtractionConfig()` from `{}` alone, since `crawl` has no Kotlin
/// default either. `required_field_stub` recurses one field at a time instead: reuse
/// `kotlin_field_default` to ask, per field, "does the real binding already give this a
/// default" (skip it — Jackson gets there on its own) or "is it bare" (recurse into a `Named`
/// type's own stub, or refuse the whole containing type when it is a bare scalar/collection —
/// there is no honest JSON literal alef can spell for those, the same "no default" the Kotlin
/// binding itself renders). The net effect for `url: UrlExtractionConfig` is
/// `{"crawl": {"ssrf": {}}}`, not a blind `{}`. ~keep
pub(super) fn fill_missing_required_kotlin_fields(
    object: &mut serde_json::Map<String, serde_json::Value>,
    type_def: &crate::core::ir::TypeDef,
    ctx: &KotlinFillContext<'_>,
    memo: &mut std::collections::HashMap<String, Option<serde_json::Map<String, serde_json::Value>>>,
    visiting: &mut std::collections::HashSet<String>,
) {
    for field in &type_def.fields {
        if field.binding_excluded || field.serde_skip || field.serde_flatten || field.optional {
            continue;
        }
        let crate::core::ir::TypeRef::Named(nested_type_name) = &field.ty else {
            continue;
        };
        let wire_name = crate::codegen::naming::wire_field_name(
            &field.name,
            field.serde_rename.as_deref(),
            type_def.serde_rename_all.as_deref(),
        );
        if object.contains_key(&wire_name) {
            continue;
        }
        let has_kotlin_default = !crate::backends::kotlin::kotlin_field_default(
            &field.ty,
            field.optional,
            field.typed_default.as_ref(),
            &ctx.enum_defaults,
            &ctx.default_constructible_types,
        )
        .is_empty();
        if has_kotlin_default {
            continue;
        }
        if let Some(stub) = required_field_stub(nested_type_name, ctx, memo, visiting) {
            object.insert(wire_name, serde_json::Value::Object(stub));
        }
    }
}

/// Build the JSON stub `fill_missing_required_kotlin_fields` inserts for one bare `Named`
/// field, recursing into `type_name`'s own bare-but-`Named` fields. `None` when some field
/// along the way is bare and not `Named` (a scalar/`Vec`/`Map` with no honest literal alef can
/// spell) or `type_name` is unknown (opaque/external type this pass cannot see into) — the
/// caller then leaves the parent field exactly as the fixture wrote it. `visiting` guards a
/// recursive type (`type_name` reachable from itself) the same way, since there is no
/// terminating stub for a cycle. `memo` avoids re-walking a type reached from multiple fields.
pub(super) fn required_field_stub(
    type_name: &str,
    ctx: &KotlinFillContext<'_>,
    memo: &mut std::collections::HashMap<String, Option<serde_json::Map<String, serde_json::Value>>>,
    visiting: &mut std::collections::HashSet<String>,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    if let Some(cached) = memo.get(type_name) {
        return cached.clone();
    }
    if !visiting.insert(type_name.to_string()) {
        return None;
    }
    let result = (|| {
        let type_def = ctx.type_defs.iter().find(|candidate| candidate.name == type_name)?;
        let mut stub = serde_json::Map::new();
        for field in &type_def.fields {
            if field.binding_excluded || field.serde_skip || field.serde_flatten || field.optional {
                continue;
            }
            let has_kotlin_default = !crate::backends::kotlin::kotlin_field_default(
                &field.ty,
                field.optional,
                field.typed_default.as_ref(),
                &ctx.enum_defaults,
                &ctx.default_constructible_types,
            )
            .is_empty();
            if has_kotlin_default {
                continue;
            }
            let crate::core::ir::TypeRef::Named(nested_type_name) = &field.ty else {
                return None;
            };
            let nested_stub = required_field_stub(nested_type_name, ctx, memo, visiting)?;
            let wire_name = crate::codegen::naming::wire_field_name(
                &field.name,
                field.serde_rename.as_deref(),
                type_def.serde_rename_all.as_deref(),
            );
            stub.insert(wire_name, serde_json::Value::Object(nested_stub));
        }
        Some(stub)
    })();
    visiting.remove(type_name);
    memo.insert(type_name.to_string(), result.clone());
    result
}

pub(super) fn normalize_typed_value(
    value: &serde_json::Value,
    field_type: &crate::core::ir::TypeRef,
    ctx: &KotlinFillContext<'_>,
) -> serde_json::Value {
    match field_type {
        crate::core::ir::TypeRef::Named(name) => normalize_typed_json(value, name, ctx),
        crate::core::ir::TypeRef::Optional(inner) => normalize_typed_value(value, inner, ctx),
        crate::core::ir::TypeRef::Vec(inner) => serde_json::Value::Array(
            value
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|item| normalize_typed_value(item, inner, ctx))
                        .collect()
                })
                .unwrap_or_default(),
        ),
        _ => value.clone(),
    }
}

pub(super) fn prepare_docs_file_reads(
    value: &mut serde_json::Value,
    files: &[crate::e2e::fixture::FixtureDocsFileInput],
) -> Vec<(usize, String, String)> {
    files
        .iter()
        .enumerate()
        .filter_map(|(index, file)| {
            let marker = format!("__ALEF_DOC_FILE_{index}__");
            let target = if file.field.is_empty() {
                Some(&mut *value)
            } else {
                value.pointer_mut(&file.field)
            }?;
            *target = serde_json::Value::String(marker.clone());
            Some((index, marker, file.path.clone()))
        })
        .collect()
}

pub(super) fn append_docs_file_setup(
    setup_lines: &mut Vec<String>,
    variable: &str,
    value: &serde_json::Value,
    type_name: &str,
    file_reads: &[(usize, String, String)],
) {
    let replacements = file_reads
        .iter()
        .map(|(index, marker, _)| format!(".replace(\"{marker}\", {variable}File{index})"))
        .collect::<String>();
    for (index, _, path) in file_reads {
        setup_lines.push(
            crate::e2e::template_env::render(
                "kotlin/docs_file_read.jinja",
                crate::alef_context! { variable => variable, index => index, path => escape_kotlin(path) },
            )
            .trim_end()
            .to_string(),
        );
    }
    let json = serde_json::to_string(value).unwrap_or_default();
    setup_lines.push(
        crate::e2e::template_env::render(
            "kotlin/snippet_json_object_setup.jinja",
            crate::alef_context! {
                variable => variable,
                json_literal => super::super::values::kotlin_string_literal(&json),
                replacements => replacements,
                type_name => type_name,
            },
        )
        .trim_end()
        .to_string(),
    );
}

/// Wrap a resolved mock-server URL argument in the streaming adapter's declared request DTO,
/// returning the expression the call should receive. With no `request_type` declared the bare
/// argument name is returned unchanged, so every non-adapter call site renders exactly as before.
/// Mirrors `java/args.rs`'s `{name}Req` wrapper. ~keep
pub(super) fn wrap_in_request_type(
    setup_lines: &mut Vec<String>,
    name: &str,
    adapter_request_type: Option<&str>,
) -> String {
    match adapter_request_type {
        Some(request_type) => {
            let req_var = format!("{name}Req");
            setup_lines.push(format!("val {req_var} = {request_type}({name})"));
            req_var
        }
        None => name.to_string(),
    }
}
