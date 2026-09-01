use super::renderers::{kotlin_getter, quoted_key_literal};
use super::types::{PathSegment, PhpGetterMap};
use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};
use std::collections::{HashMap, HashSet};

/// Appends the bare field/array/map name for `segment` onto `path_so_far`,
/// with no bracket suffix. The result is the config-lookup key to check for
/// `segment`'s OWN optionality (e.g. is the array itself an `Option<Vec<T>>`).
///
/// This is the single source of truth for how every `_with_optionals`
/// renderer builds its `optional_fields` / `array_fields` lookup key as it
/// walks a path segment by segment. Previously each renderer duplicated this
/// string-building logic and several omitted the index-normalisation step in
/// [`push_key_index_suffix`], so a renderer's local check silently stopped
/// matching config entries (like `"results[0].metadata.format"`) as soon as
/// the path crossed an indexed segment. Route every renderer through this
/// function (and `push_key_index_suffix`) so the two conventions cannot
/// drift apart again.
pub(super) fn push_key_field_name(path_so_far: &mut String, segment: &PathSegment) {
    let name = match segment {
        PathSegment::Field(name) | PathSegment::ArrayField { name, .. } => name.as_str(),
        PathSegment::MapAccess { field, .. } => field.as_str(),
        PathSegment::Length => return,
    };
    if !path_so_far.is_empty() {
        path_so_far.push('.');
    }
    path_so_far.push_str(name);
}

/// After checking `segment`'s own optionality against `path_so_far` (via
/// [`push_key_field_name`]), normalises the tracked key for CHILD lookups.
///
/// Config entries in `fields_optional` / `fields_array` always record array
/// indices and numeric map keys as a literal `[0]` suffix — see
/// `parse::normalize_numeric_indices`, which `FieldResolver::is_optional`
/// uses to match config entries regardless of the *actual* index requested.
/// Every renderer must extend its tracked path the same way after an indexed
/// segment, or nested optional keys like `"results[0].metadata.format"`
/// never match once the path has crossed `results[0]`.
///
/// String-keyed map access (`foo["bar"]`) does not extend the tracked path —
/// only the field name preceding the bracket is significant, matching the
/// convention used throughout `fields_optional` / `fields_array` entries.
pub(super) fn push_key_index_suffix(path_so_far: &mut String, segment: &PathSegment) {
    match segment {
        PathSegment::ArrayField { .. } => path_so_far.push_str("[0]"),
        PathSegment::MapAccess { key, .. } if key.chars().all(|c| c.is_ascii_digit()) => {
            path_so_far.push_str("[0]");
        }
        _ => {}
    }
}

/// How one TypeScript-family target spells a map lookup.
///
/// ~keep The only thing that has ever differed between the `node` and `wasm` accessor
/// renderers. NAPI lowers a `HashMap<String, V>` to a plain TypeScript object, so a key is an
/// index; wasm-bindgen lowers it to a JS `Map`, so a key is a `get` call. Everything else about
/// the two — segment casing, array indexing, `.length`, and which links need `?.` — is one
/// answer about one language, and encoding the difference as a parameter is what stops a second
/// renderer from re-deriving the rest of it. `wasm` used to have that second renderer and it had
/// no optionality at all, so every `Option<T>` field reached a published wasm snippet unguarded
/// (`TS18048`) while the same field in the same fixture's node snippet was chained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TypescriptMapAccess {
    /// `value["key"]`, and `value[0]` for a digit-only key. The NAPI/node lowering.
    Index,
    /// `value.get("key")`, for every key. The wasm-bindgen lowering.
    MapGet,
}

pub(super) fn render_typescript_with_optionals(
    segments: &[PathSegment],
    result_var: &str,
    optional_fields: &HashSet<String>,
    map_access: TypescriptMapAccess,
) -> String {
    let mut out = result_var.to_string();
    let mut path_so_far = String::new();
    let mut previous_optional = false;
    for segment in segments {
        match segment {
            PathSegment::Field(field) => {
                push_key_field_name(&mut path_so_far, segment);
                out.push_str(if previous_optional { "?." } else { "." });
                out.push_str(&field.to_lower_camel_case());
                previous_optional = optional_fields.contains(&path_so_far);
            }
            PathSegment::ArrayField { name, index } => {
                push_key_field_name(&mut path_so_far, segment);
                out.push_str(if previous_optional { "?." } else { "." });
                out.push_str(&name.to_lower_camel_case());
                let optional = optional_fields.contains(&path_so_far);
                if optional {
                    out.push_str("?.");
                }
                out.push_str(&format!("[{index}]"));
                push_key_index_suffix(&mut path_so_far, segment);
                previous_optional = optional;
            }
            PathSegment::MapAccess { field, key } => {
                push_key_field_name(&mut path_so_far, segment);
                out.push_str(if previous_optional { "?." } else { "." });
                out.push_str(&field.to_lower_camel_case());
                let optional = optional_fields.contains(&path_so_far);
                match map_access {
                    // A `get` is a member access, so an optional receiver takes the plain `?.`
                    // that an element access spells `?.[`. ~keep
                    TypescriptMapAccess::MapGet => {
                        out.push_str(if optional { "?." } else { "." });
                        out.push_str(&format!("get({key:?})"));
                    }
                    TypescriptMapAccess::Index => {
                        if optional {
                            out.push_str("?.");
                        }
                        if !key.is_empty() && key.chars().all(|character| character.is_ascii_digit()) {
                            out.push_str(&format!("[{key}]"));
                        } else {
                            out.push_str(&format!("[{key:?}]"));
                        }
                    }
                }
                push_key_index_suffix(&mut path_so_far, segment);
                previous_optional = optional;
            }
            PathSegment::Length => {
                out.push_str(if previous_optional { "?.length" } else { ".length" });
                previous_optional = false;
            }
        }
    }
    out
}

pub(super) fn render_java_with_optionals(
    segments: &[PathSegment],
    result_var: &str,
    optional_fields: &HashSet<String>,
) -> String {
    let mut out = result_var.to_string();
    let mut path_so_far = String::new();
    for (i, seg) in segments.iter().enumerate() {
        let is_leaf = i == segments.len() - 1;
        match seg {
            PathSegment::Field(f) => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(f);
                out.push('.');
                out.push_str(&f.to_lower_camel_case());
                out.push_str("()");
                let _ = is_leaf;
                let _ = optional_fields;
            }
            PathSegment::ArrayField { name, index } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(name);
                out.push('.');
                out.push_str(&name.to_lower_camel_case());
                out.push_str(&format!("().get({index})"));
            }
            PathSegment::MapAccess { field, key } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(field);
                out.push('.');
                out.push_str(&field.to_lower_camel_case());
                // Numeric keys index into List<T> (.get(int)); string keys index into Map<String, V>.
                let is_numeric = !key.is_empty() && key.chars().all(|c| c.is_ascii_digit());
                if is_numeric {
                    out.push_str(&format!("().get({key})"));
                } else {
                    out.push_str(&format!("().get({})", quoted_key_literal(key)));
                }
            }
            PathSegment::Length => {
                out.push_str(".size()");
            }
        }
    }
    out
}

/// Kotlin variant of `render_java_with_optionals` using Kotlin idioms.
///
/// When the previous field in the chain is optional (nullable), uses `?.`
/// safe-call navigation for the next segment so the Kotlin compiler is
/// satisfied by the nullable receiver.
///
/// Nullability is **sticky**: once a `?.` safe-call has been emitted for any
/// segment, all subsequent segments also use `?.` because they operate on a
/// nullable receiver. A non-optional field after a `?.` call still returns
/// `T?` (because the whole chain can be null if any prefix was null).
///
/// Example: for `toolCalls[0].function.name` where `toolCalls` is optional:
/// `result.toolCalls()?.first()?.function()?.name()` — even though `function`
/// and `name` are themselves non-optional, they follow a `?.` chain.
pub(super) fn render_kotlin_with_optionals(
    segments: &[PathSegment],
    result_var: &str,
    optional_fields: &HashSet<String>,
) -> String {
    let mut out = result_var.to_string();
    let mut path_so_far = String::new();
    // Track whether the previous segment returned a nullable type. Starts
    // false because `result_var` is always non-null.
    //
    // This flag is sticky: once set to true it stays true for the rest of
    // the chain because a `?.` call returns `T?` regardless of whether the
    // subsequent field itself is declared optional. All accesses on a
    // nullable receiver must also use `?.`.
    let mut prev_was_nullable = false;
    for seg in segments {
        let nav = if prev_was_nullable { "?." } else { "." };
        match seg {
            PathSegment::Field(f) => {
                push_key_field_name(&mut path_so_far, seg);
                // After this call, the receiver is nullable if the field is in
                // optional_fields (the Java @Nullable annotation makes the
                // return type T? in Kotlin) OR if the incoming receiver was
                // already nullable (sticky: `?.` call yields `T?`).
                let is_optional = optional_fields.contains(&path_so_far);
                out.push_str(nav);
                out.push_str(&kotlin_getter(f));
                out.push_str("()");
                prev_was_nullable = prev_was_nullable || is_optional;
            }
            PathSegment::ArrayField { name, index } => {
                push_key_field_name(&mut path_so_far, seg);
                let is_optional = optional_fields.contains(&path_so_far);
                out.push_str(nav);
                out.push_str(&kotlin_getter(name));
                let safe = if prev_was_nullable || is_optional { "?" } else { "" };
                if *index == 0 {
                    out.push_str(&format!("(){safe}.first()"));
                } else {
                    out.push_str(&format!("(){safe}.get({index})"));
                }
                // Record the "[0]" suffix so subsequent optional-field checks against
                // paths like "choices[0].message.tool_calls" continue to match when the
                // optional_fields set uses indexed keys (mirrors the Rust renderer).
                push_key_index_suffix(&mut path_so_far, seg);
                prev_was_nullable = prev_was_nullable || is_optional;
            }
            PathSegment::MapAccess { field, key } => {
                push_key_field_name(&mut path_so_far, seg);
                let is_optional = optional_fields.contains(&path_so_far);
                out.push_str(nav);
                out.push_str(&kotlin_getter(field));
                let is_numeric = !key.is_empty() && key.chars().all(|c| c.is_ascii_digit());
                if is_numeric {
                    if prev_was_nullable || is_optional {
                        out.push_str(&format!("()?.get({key})"));
                    } else {
                        out.push_str(&format!("().get({key})"));
                    }
                } else if prev_was_nullable || is_optional {
                    out.push_str(&format!("()?.get({})", quoted_key_literal(key)));
                } else {
                    out.push_str(&format!("().get({})", quoted_key_literal(key)));
                }
                push_key_index_suffix(&mut path_so_far, seg);
                prev_was_nullable = prev_was_nullable || is_optional;
            }
            PathSegment::Length => {
                // .size is a Kotlin property, no () needed.
                // If the previous field was nullable, use ?.size
                let size_nav = if prev_was_nullable { "?" } else { "" };
                out.push_str(&format!("{size_nav}.size"));
                prev_was_nullable = false;
            }
        }
    }
    out
}

/// kotlin_android variant of `render_kotlin_with_optionals`.
///
/// kotlin_android generates Kotlin data classes whose fields are Kotlin
/// **properties** (not Java-style getter methods). Every field segment must
/// therefore be accessed without parentheses: `result.choices.first().message.content`
/// rather than `result.choices().first().message().content()`.
///
/// The nullable-chain rules are identical to `render_kotlin_with_optionals`:
/// once a segment in the path is optional (`T?`) the remainder of the chain
/// uses `?.` safe-call syntax.
pub(super) fn render_kotlin_android_with_optionals(
    segments: &[PathSegment],
    result_var: &str,
    optional_fields: &HashSet<String>,
) -> String {
    let mut out = result_var.to_string();
    let mut path_so_far = String::new();
    let mut prev_was_nullable = false;
    for seg in segments {
        let nav = if prev_was_nullable { "?." } else { "." };
        match seg {
            PathSegment::Field(f) => {
                push_key_field_name(&mut path_so_far, seg);
                let is_optional = optional_fields.contains(&path_so_far);
                out.push_str(nav);
                // Property access — no () suffix.
                out.push_str(&kotlin_getter(f));
                prev_was_nullable = prev_was_nullable || is_optional;
            }
            PathSegment::ArrayField { name, index } => {
                push_key_field_name(&mut path_so_far, seg);
                let is_optional = optional_fields.contains(&path_so_far);
                out.push_str(nav);
                // Property access — no () suffix on the collection itself.
                out.push_str(&kotlin_getter(name));
                let safe = if prev_was_nullable || is_optional { "?" } else { "" };
                if *index == 0 {
                    out.push_str(&format!("{safe}.first()"));
                } else {
                    out.push_str(&format!("{safe}.get({index})"));
                }
                push_key_index_suffix(&mut path_so_far, seg);
                prev_was_nullable = prev_was_nullable || is_optional;
            }
            PathSegment::MapAccess { field, key } => {
                push_key_field_name(&mut path_so_far, seg);
                let is_optional = optional_fields.contains(&path_so_far);
                out.push_str(nav);
                // Property access — no () suffix on the map field.
                out.push_str(&kotlin_getter(field));
                let is_numeric = !key.is_empty() && key.chars().all(|c| c.is_ascii_digit());
                if is_numeric {
                    if prev_was_nullable || is_optional {
                        out.push_str(&format!("?.get({key})"));
                    } else {
                        out.push_str(&format!(".get({key})"));
                    }
                } else if prev_was_nullable || is_optional {
                    out.push_str(&format!("?.get({})", quoted_key_literal(key)));
                } else {
                    out.push_str(&format!(".get({})", quoted_key_literal(key)));
                }
                push_key_index_suffix(&mut path_so_far, seg);
                prev_was_nullable = prev_was_nullable || is_optional;
            }
            PathSegment::Length => {
                let size_nav = if prev_was_nullable { "?" } else { "" };
                out.push_str(&format!("{size_nav}.size"));
                prev_was_nullable = false;
            }
        }
    }
    out
}

/// Non-optional variant of `render_kotlin_android_with_optionals`.
///
/// Used by `render_accessor` (the path without per-field optionality tracking).
pub(super) fn render_kotlin_android(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&kotlin_getter(f));
                // No () — property access.
            }
            PathSegment::ArrayField { name, index } => {
                out.push('.');
                out.push_str(&kotlin_getter(name));
                if *index == 0 {
                    out.push_str(".first()");
                } else {
                    out.push_str(&format!(".get({index})"));
                }
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&kotlin_getter(field));
                let is_numeric = !key.is_empty() && key.chars().all(|c| c.is_ascii_digit());
                if is_numeric {
                    out.push_str(&format!(".get({key})"));
                } else {
                    out.push_str(&format!(".get({})", quoted_key_literal(key)));
                }
            }
            PathSegment::Length => {
                out.push_str(".size");
            }
        }
    }
    out
}

/// Rust accessor with Option unwrapping for intermediate fields.
///
/// When an intermediate field is in the `optional_fields` set, `.as_ref().unwrap()`
/// is appended after the field access to unwrap the `Option<T>`.
/// When a path is in `method_calls` AND is not in `result_fields`, `()` is appended
/// to make it a method call. The `result_fields` check prevents the global
/// `method_calls` set from leaking method-call syntax into accessors that the
/// per-fixture `[fields_method_calls = []]` config has classified as struct
/// field access (e.g. a fixture DTO's `DocumentResult.content: String`).
pub(super) fn render_rust_with_optionals(
    segments: &[PathSegment],
    result_var: &str,
    optional_fields: &HashSet<String>,
    method_calls: &HashSet<String>,
    result_fields: &HashSet<String>,
) -> String {
    let mut out = result_var.to_string();
    let mut path_so_far = String::new();
    for (i, seg) in segments.iter().enumerate() {
        let is_leaf = i == segments.len() - 1;
        match seg {
            PathSegment::Field(f) => {
                push_key_field_name(&mut path_so_far, seg);
                out.push('.');
                out.push_str(&f.to_snake_case());
                let is_method = method_calls.contains(&path_so_far) && !result_fields.contains(&path_so_far);
                if is_method {
                    out.push_str("()");
                    if !is_leaf && optional_fields.contains(&path_so_far) {
                        out.push_str(".as_ref().unwrap()");
                    }
                } else if !is_leaf && optional_fields.contains(&path_so_far) {
                    out.push_str(".as_ref().unwrap()");
                }
            }
            PathSegment::ArrayField { name, index } => {
                push_key_field_name(&mut path_so_far, seg);
                out.push('.');
                out.push_str(&name.to_snake_case());
                // Option<Vec<T>>: must unwrap the Option before indexing.
                // Check both "name" (bare) and "name[0]" (indexed) forms since the
                // optional_fields registry may use either convention.
                let path_with_idx = format!("{path_so_far}[0]");
                let is_opt = optional_fields.contains(&path_so_far) || optional_fields.contains(path_with_idx.as_str());
                if is_opt {
                    out.push_str(&format!(".as_ref().unwrap()[{index}]"));
                } else {
                    out.push_str(&format!("[{index}]"));
                }
                // Record the normalised "[0]" suffix in path_so_far so that deeper
                // optional-field keys which include explicit indices (e.g.
                // "choices[0].message.tool_calls") continue to match when we check
                // subsequent segments.
                push_key_index_suffix(&mut path_so_far, seg);
            }
            PathSegment::MapAccess { field, key } => {
                push_key_field_name(&mut path_so_far, seg);
                out.push('.');
                out.push_str(&field.to_snake_case());
                if key.chars().all(|c| c.is_ascii_digit()) {
                    // Check optional both with and without the numeric index suffix.
                    let path_with_idx = format!("{path_so_far}[0]");
                    let is_opt =
                        optional_fields.contains(&path_so_far) || optional_fields.contains(path_with_idx.as_str());
                    if is_opt {
                        out.push_str(&format!(".as_ref().unwrap()[{key}]"));
                    } else {
                        out.push_str(&format!("[{key}]"));
                    }
                    push_key_index_suffix(&mut path_so_far, seg);
                } else {
                    out.push_str(&format!(".get({}).map(|s| s.as_str())", quoted_key_literal(key)));
                }
            }
            PathSegment::Length => {
                out.push_str(".len()");
            }
        }
    }
    out
}

/// Zig accessor that unwraps optional fields with `.?`, and adds `()` for opaque-handle
/// getter fields.
///
/// Zig does not allow field access, indexing, or comparisons through `?T`;
/// the value must be unwrapped first. Each segment whose path appears in the
/// optional-field set is followed by `.?` so the resulting expression is a
/// concrete value usable in assertions.
///
/// `method_calls` is overloaded the same way `render_rust_with_optionals` disambiguates it
/// (see that function's doc comment): a path in `method_calls` AND NOT in `result_fields` is a
/// real Zig method call on an opaque-handle type (`tree.language()`, an FFI getter) and gets a
/// trailing `()`. A path in `method_calls` that the per-fixture `[fields_method_calls]`/
/// `result_fields` config has ALSO classified as `result_fields` keeps the pre-existing
/// tagged-union-variant shape: plain `.{name}` with no `()` and no forced `.?`, because Zig
/// tagged-union variants are accessed via ordinary dot syntax and this path is the one
/// `render_rust_with_optionals`'s own doc names as the reason `result_fields` overrides the
/// global `method_calls` set. ~keep Before this, `method_calls` could only ever suppress `.?`;
/// there was no way for this function to emit `()` at all, so every opaque-handle getter field
/// path it built was unreachable-shaped: `.field` where the generated Zig needs `.field()`.
pub(super) fn render_zig_with_optionals(
    segments: &[PathSegment],
    result_var: &str,
    optional_fields: &HashSet<String>,
    method_calls: &HashSet<String>,
    result_fields: &HashSet<String>,
) -> String {
    let mut out = result_var.to_string();
    let mut path_so_far = String::new();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                push_key_field_name(&mut path_so_far, seg);
                out.push('.');
                out.push_str(f);
                zig_append_call_and_unwrap(&mut out, &path_so_far, optional_fields, method_calls, result_fields);
            }
            PathSegment::ArrayField { name, index } => {
                push_key_field_name(&mut path_so_far, seg);
                out.push('.');
                out.push_str(name);
                zig_append_call_and_unwrap(&mut out, &path_so_far, optional_fields, method_calls, result_fields);
                out.push_str(&format!("[{index}]"));
                push_key_index_suffix(&mut path_so_far, seg);
            }
            PathSegment::MapAccess { field, key } => {
                push_key_field_name(&mut path_so_far, seg);
                out.push('.');
                out.push_str(field);
                zig_append_call_and_unwrap(&mut out, &path_so_far, optional_fields, method_calls, result_fields);
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("[{key}]"));
                } else {
                    out.push_str(&format!(".get({})", quoted_key_literal(key)));
                }
                push_key_index_suffix(&mut path_so_far, seg);
            }
            PathSegment::Length => {
                out.push_str(".len");
            }
        }
    }
    out
}

/// Append `()` for an opaque-handle method-call segment, then `.?` when the segment is
/// registered optional — shared by every segment kind in [`render_zig_with_optionals`] so the
/// method-call/tagged-union disambiguation is written once. See that function's doc comment for
/// what distinguishes the two `method_calls` meanings.
fn zig_append_call_and_unwrap(
    out: &mut String,
    path_so_far: &str,
    optional_fields: &HashSet<String>,
    method_calls: &HashSet<String>,
    result_fields: &HashSet<String>,
) {
    let is_opaque_method = method_calls.contains(path_so_far) && !result_fields.contains(path_so_far);
    if is_opaque_method {
        out.push_str("()");
        if optional_fields.contains(path_so_far) {
            out.push_str(".?");
        }
    } else if !method_calls.contains(path_so_far) && optional_fields.contains(path_so_far) {
        out.push_str(".?");
    }
}

pub(super) fn render_pascal_dot(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&f.to_pascal_case());
            }
            PathSegment::ArrayField { name, index } => {
                out.push('.');
                out.push_str(&name.to_pascal_case());
                out.push_str(&format!("[{index}]"));
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&field.to_pascal_case());
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("[{key}]"));
                } else {
                    out.push_str(&format!("[{}]", quoted_key_literal(key)));
                }
            }
            PathSegment::Length => {
                out.push_str(".Count");
            }
        }
    }
    out
}

pub(super) fn render_csharp_with_optionals(
    segments: &[PathSegment],
    result_var: &str,
    optional_fields: &HashSet<String>,
    variant_narrowings: &HashMap<String, String>,
) -> String {
    let mut out = result_var.to_string();
    let mut path_so_far = String::new();
    for (i, seg) in segments.iter().enumerate() {
        let is_leaf = i == segments.len() - 1;
        match seg {
            PathSegment::Field(f) => {
                push_key_field_name(&mut path_so_far, seg);
                out.push('.');
                // A segment that steps into a tagged-union variant names the generated
                // `As<Variant>` property, not the variant type: `.Format!.Html` is CS0572
                // ("cannot reference a type through an expression"). The accessor name comes
                // from the C# backend's own emitter so it can never be one this binding did
                // not generate -- see `variant_accessors`' field doc. ~keep
                let narrowed = variant_narrowings.get(&path_so_far);
                match narrowed {
                    Some(accessor) => out.push_str(accessor),
                    None => out.push_str(&f.to_pascal_case()),
                }
                // `As<Variant>` is generated as `T? As<Variant>` -- it returns null for every
                // other variant, so it is unconditionally nullable and needs the forgiving
                // operator to be stepped through, whether or not the FIELD was optional. ~keep
                if !is_leaf && (narrowed.is_some() || optional_fields.contains(&path_so_far)) {
                    out.push('!');
                }
            }
            PathSegment::ArrayField { name, index } => {
                push_key_field_name(&mut path_so_far, seg);
                out.push('.');
                out.push_str(&name.to_pascal_case());
                // Indexing a nullable collection dereferences it exactly as reading `.Count`
                // does, so this arm has to ask `optional_fields` the same question the `Field`
                // arm above asks — and without the `!is_leaf` guard, because the dereference
                // happens whether or not another segment follows. Skipping it let one emitted
                // snippet contradict itself: `result.Metadata.Headings!.Count` on one line and
                // `result.Metadata.Headings[0].Level` (a CS8602) on the next. ~keep
                if optional_fields.contains(&path_so_far) {
                    out.push('!');
                }
                out.push_str(&format!("[{index}]"));
                // Normalise the tracked key so a Field segment further down the
                // chain (e.g. "results[0].metadata.format") still matches
                // `optional_fields` entries recorded with an indexed prefix.
                push_key_index_suffix(&mut path_so_far, seg);
            }
            PathSegment::MapAccess { field, key } => {
                push_key_field_name(&mut path_so_far, seg);
                out.push('.');
                out.push_str(&field.to_pascal_case());
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("[{key}]"));
                } else {
                    out.push_str(&format!("[{}]", quoted_key_literal(key)));
                }
                push_key_index_suffix(&mut path_so_far, seg);
            }
            PathSegment::Length => {
                out.push_str(".Count");
            }
        }
    }
    out
}

pub(super) fn render_php(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push_str("->");
                // PHP properties are camelCase (per #[php(prop, name = "...")]),
                // so convert snake_case field names to camelCase.
                out.push_str(&f.to_lower_camel_case());
            }
            PathSegment::ArrayField { name, index } => {
                out.push_str("->");
                out.push_str(&name.to_lower_camel_case());
                out.push_str(&format!("[{index}]"));
            }
            PathSegment::MapAccess { field, key } => {
                out.push_str("->");
                out.push_str(&field.to_lower_camel_case());
                out.push_str(&format!("[{}]", quoted_key_literal(key)));
            }
            PathSegment::Length => {
                let current = std::mem::take(&mut out);
                out = format!("count({current})");
            }
        }
    }
    out
}

/// PHP accessor that distinguishes between scalar fields (property access: `->camelCase`)
/// and non-scalar fields (getter-method access: `->getCamelCase()`).
///
/// ext-php-rs 0.15.x exposes scalar fields via `#[php(prop)]` as PHP properties, but
/// non-scalar fields (Named structs, `Vec<Named>`, `Map`, etc.) require a `#[php(getter)]`
/// method because `get_method_props` is unimplemented in ext-php-rs-derive 0.11.7.
/// The generated getter method name is `get{CamelCase}` (stripping the `get_` prefix and
/// converting the camelCase remainder to a PHP property name), so e2e assertions must call
/// `->getCamelCase()` for those fields.
///
/// `getter_map` carries the per-`(owner_type, field_name)` classification along with the
/// chain-resolution metadata required to walk multi-segment paths through the IR's nested
/// type graph. Each path segment is classified using the *current* owner type, then the
/// owner cursor advances to the field's referenced `Named` type (if any) for the next
/// segment. When `root_type` is unset the renderer falls back to the legacy bare-name
/// union, which is unsafe but preserves backwards compatibility for callers that have
/// not wired type resolution.
pub(super) fn render_php_with_getters(
    segments: &[PathSegment],
    result_var: &str,
    getter_map: &PhpGetterMap,
    optional_fields: &HashSet<String>,
) -> String {
    let mut out = result_var.to_string();
    let mut current_type: Option<String> = getter_map.root_type.clone();
    let mut path_so_far = String::new();
    // Sticky, same convention as `render_php`: once any segment in the chain is optional,
    // every following access must null-safe-navigate off it.
    let mut prev_was_nullable = false;
    for seg in segments {
        let arrow = if prev_was_nullable { "?->" } else { "->" };
        match seg {
            PathSegment::Field(f) => {
                push_key_field_name(&mut path_so_far, seg);
                let camel = f.to_lower_camel_case();
                if getter_map.needs_getter(current_type.as_deref(), f.as_str()) {
                    // Non-scalar field: ext-php-rs emits a `get{CamelCase}()` method.
                    // The `get_` prefix is stripped by ext-php-rs when it derives the
                    // PHP property name, but the Rust method ident is `get_{camelCase}`,
                    // so the PHP call is `->get{CamelCase}()`.
                    let getter = format!("get{}", camel.as_str()[..1].to_uppercase() + &camel[1..]);
                    out.push_str(arrow);
                    out.push_str(&getter);
                    out.push_str("()");
                } else {
                    out.push_str(arrow);
                    out.push_str(&camel);
                }
                current_type = getter_map.advance(current_type.as_deref(), f.as_str());
                prev_was_nullable = prev_was_nullable || optional_fields.contains(&path_so_far);
            }
            PathSegment::ArrayField { name, index } => {
                push_key_field_name(&mut path_so_far, seg);
                let camel = name.to_lower_camel_case();
                if getter_map.needs_getter(current_type.as_deref(), name.as_str()) {
                    let getter = format!("get{}", camel.as_str()[..1].to_uppercase() + &camel[1..]);
                    out.push_str(arrow);
                    out.push_str(&getter);
                    out.push_str("()");
                } else {
                    out.push_str(arrow);
                    out.push_str(&camel);
                }
                out.push_str(&format!("[{index}]"));
                current_type = getter_map.advance(current_type.as_deref(), name.as_str());
                push_key_index_suffix(&mut path_so_far, seg);
                prev_was_nullable = prev_was_nullable || optional_fields.contains(&path_so_far);
            }
            PathSegment::MapAccess { field, key } => {
                push_key_field_name(&mut path_so_far, seg);
                let camel = field.to_lower_camel_case();
                if getter_map.needs_getter(current_type.as_deref(), field.as_str()) {
                    let getter = format!("get{}", camel.as_str()[..1].to_uppercase() + &camel[1..]);
                    out.push_str(arrow);
                    out.push_str(&getter);
                    out.push_str("()");
                } else {
                    out.push_str(arrow);
                    out.push_str(&camel);
                }
                out.push_str(&format!("[{}]", quoted_key_literal(key)));
                current_type = getter_map.advance(current_type.as_deref(), field.as_str());
                push_key_index_suffix(&mut path_so_far, seg);
                prev_was_nullable = prev_was_nullable || optional_fields.contains(&path_so_far);
            }
            PathSegment::Length => {
                let current = std::mem::take(&mut out);
                out = format!("count({current})");
            }
        }
    }
    out
}

pub(super) fn render_r(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('$');
                out.push_str(f);
            }
            PathSegment::ArrayField { name, index } => {
                out.push('$');
                out.push_str(name);
                // R uses 1-based indexing.
                out.push_str(&format!("[[{}]]", index + 1));
            }
            PathSegment::MapAccess { field, key } => {
                out.push('$');
                out.push_str(field);
                out.push_str(&format!("[[{}]]", quoted_key_literal(key)));
            }
            PathSegment::Length => {
                let current = std::mem::take(&mut out);
                out = format!("length({current})");
            }
        }
    }
    out
}

pub(super) fn render_c(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                let snake = f.to_snake_case();
                let current = std::mem::take(&mut out);
                // Emit nested accessor calls with result_<field_name> pattern
                out = format!("result_{snake}({current})");
            }
            PathSegment::ArrayField { name, index } => {
                let snake = name.to_snake_case();
                let current = std::mem::take(&mut out);
                out = format!("result_{snake}({current})[{index}]");
            }
            PathSegment::MapAccess { field, key } => {
                let snake = field.to_snake_case();
                let current = std::mem::take(&mut out);
                out = format!("result_{snake}({current})[{}]", quoted_key_literal(key));
            }
            PathSegment::Length => {
                let current = std::mem::take(&mut out);
                out = format!("result_{current}_count()");
            }
        }
    }
    out
}

/// Dart accessor using camelCase field names (FRB v2 convention).
///
/// FRB v2 generates Dart property getters with camelCase names for every
/// snake_case Rust field, so `snake_case_field` becomes `snakeCaseField`.
/// Array fields index with `[N]`; map fields use `["key"]` or `[N]` notation.
/// Length/count segments use `.length` (Dart `List.length`).
pub(super) fn render_dart(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&f.to_lower_camel_case());
            }
            PathSegment::ArrayField { name, index } => {
                out.push('.');
                out.push_str(&name.to_lower_camel_case());
                out.push_str(&format!("[{index}]"));
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&field.to_lower_camel_case());
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("[{key}]"));
                } else {
                    out.push_str(&format!("[{}]", quoted_key_literal(key)));
                }
            }
            PathSegment::Length => {
                out.push_str(".length");
            }
        }
    }
    out
}

/// Dart accessor with optional-safe navigation using `?.` (FRB v2 convention).
///
/// When an intermediate field is in `optional_fields`, the next segment uses
/// `?.` safe-call navigation instead of `.` to avoid a null-dereference on
/// a nullable Dart type.  Field names are camelCase (FRB v2 generation rule).
pub(super) fn render_dart_with_optionals(
    segments: &[PathSegment],
    result_var: &str,
    optional_fields: &HashSet<String>,
) -> String {
    let mut out = result_var.to_string();
    // Two parallel path trackers:
    //   `path_so_far`           — dot-joined field names without array indices
    //                             (e.g. `choices.message.tool_calls`).
    //   `path_with_indices`     — same path but retaining `[N]` segments from
    //                             prior ArrayField segments (e.g.
    //                             `choices[0].message.tool_calls`).
    // `fields_optional` in alef.toml may list either form; we check both.
    let mut path_so_far = String::new();
    let mut path_with_indices = String::new();
    let mut prev_was_nullable = false;
    let is_optional =
        |bare: &str, indexed: &str| -> bool { optional_fields.contains(bare) || optional_fields.contains(indexed) };
    for seg in segments {
        let nav = if prev_was_nullable { "?." } else { "." };
        match seg {
            PathSegment::Field(f) => {
                push_key_field_name(&mut path_so_far, seg);
                push_key_field_name(&mut path_with_indices, seg);
                let optional = is_optional(&path_so_far, &path_with_indices);
                out.push_str(nav);
                out.push_str(&f.to_lower_camel_case());
                prev_was_nullable = optional;
            }
            PathSegment::ArrayField { name, index } => {
                push_key_field_name(&mut path_so_far, seg);
                push_key_field_name(&mut path_with_indices, seg);
                let optional = is_optional(&path_so_far, &path_with_indices);
                out.push_str(nav);
                out.push_str(&name.to_lower_camel_case());
                // FRB models `Option<Vec<T>>` as `List<T>?` — force-unwrap when the field
                // is registered as optional. Adding `!` to a non-nullable receiver is a Dart
                // compile-time error ("unnecessary non-null assertion").
                if optional {
                    out.push('!');
                }
                out.push_str(&format!("[{index}]"));
                // Normalise to a literal "[0]" suffix (matching
                // `push_key_index_suffix` / `normalize_numeric_indices`) rather than the
                // actual index — `fields_optional` entries always record indexed
                // segments as "[0]" regardless of which index a fixture requests.
                push_key_index_suffix(&mut path_with_indices, seg);
                prev_was_nullable = false;
            }
            PathSegment::MapAccess { field, key } => {
                push_key_field_name(&mut path_so_far, seg);
                push_key_field_name(&mut path_with_indices, seg);
                let optional = is_optional(&path_so_far, &path_with_indices);
                out.push_str(nav);
                out.push_str(&field.to_lower_camel_case());
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("[{key}]"));
                    push_key_index_suffix(&mut path_with_indices, seg);
                } else {
                    out.push_str(&format!("[{}]", quoted_key_literal(key)));
                    path_with_indices.push_str(&format!("[\"{key}\"]"));
                }
                prev_was_nullable = optional;
            }
            PathSegment::Length => {
                // Use `?.length` when the receiver is optional — emitting `.length` against
                // a `List<T>?` is a Dart sound-null-safety error.
                out.push_str(nav);
                out.push_str("length");
                prev_was_nullable = false;
            }
        }
    }
    out
}
