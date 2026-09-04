use super::super::ir_enum::enum_type_at_path_from;
use super::super::optional_renderers::{
    TypescriptMapAccess, push_key_field_name, push_key_index_suffix, render_csharp_with_optionals,
    render_dart_with_optionals, render_java_with_optionals, render_kotlin_android_with_optionals,
    render_kotlin_with_optionals, render_php_with_getters, render_rust_with_optionals,
    render_typescript_with_optionals, render_zig_with_optionals,
};
use super::super::parse::parse_path;
use super::super::python_renderer::{
    python_element_owner_type, render_python_element_with_optionals, render_python_with_optionals,
};
use super::super::renderers::{render_accessor, render_swift_with_first_class_map};
use super::super::types::{FieldResolver, PathSegment};
use heck::ToUpperCamelCase;
use std::collections::HashMap;

impl FieldResolver {
    /// Generate a language-specific accessor expression.
    ///
    /// The path is taken from [`FieldResolver::result_relative_path`]: the alias applied and any
    /// virtual namespace prefix removed, so `"browser.browser_used"` produces `result.browser_used`
    /// (Python) / `result.BrowserUsed` (C#) rather than the raw `result.browser.browser_used`,
    /// which addresses a member no result declares.
    ///
    /// ~keep Asking rather than re-deriving is the point, and it is the same correction
    /// `is_array` took. This method used to carry its own copy of the strip decision, gated on
    /// `result_fields.contains(..)` where `result_relative_path` asks the broader
    /// `is_valid_for_result(..)`. That was the un-updated original, not a policy: it predates the
    /// IR oracle `is_valid_for_result` grew, `result_relative_path`'s doc already claims to apply
    /// "the same policy `accessor()` applies", and the two C generator sites that inline this very
    /// block (`c/call_patterns.rs`, `c/test_function.rs`) describe themselves as "matching the
    /// same logic as FieldResolver::accessor" while using the broad predicate. An accessor that
    /// lands somewhere no classifier agrees the value lives is what emitted
    /// `string(result.ActionResults)` — `is_array` consulted the shared answer, this method did
    /// not, and the generated Go package stopped compiling.
    pub fn accessor(&self, fixture_field: &str, language: &str, result_var: &str) -> String {
        let effective = self.result_relative_path(fixture_field);
        self.render_relative_to(&effective, language, result_var)
    }

    /// Generate a language-specific accessor for a path that is ALREADY relative to a bound
    /// collection element — the closure/loop variable a wildcard (`foo[].bar`) fixture path
    /// expands to — rather than to the call's result variable.
    ///
    /// ~keep The only difference from [`Self::accessor`] is the anchor, so it renders through the
    /// same private `render_relative_to` rather than mirroring the language match. Anchoring is
    /// exactly the fact that must differ: [`Self::result_relative_path`] re-anchors a path against
    /// the call's RESULT type, via `envelope_projected_path` (which prefixes a leaf the root does
    /// not declare with the `result_fields` entry that reaches it) and `namespace_stripped_path`.
    /// Both are correct for a result-anchored path and both are wrong for an element-anchored one:
    /// given `structure[].kind`, the element half `kind` is not declared on the root, so the
    /// envelope rescue prefixed it back to `structure[0].kind` and the closure body came out as
    /// `e.structure[0].kind` — the container path applied a second time against a binding that is
    /// already an element, which is `E0609: no field 'structure'` on the element type. An element
    /// path is taken literally; only the alias map still applies.
    pub fn element_accessor(&self, element_path: &str, language: &str, element_var: &str) -> String {
        let effective = self.resolve(element_path);
        self.render_relative_to(effective, language, element_var)
    }

    /// Python-only counterpart to [`Self::element_accessor`], carrying one extra fact
    /// `render_relative_to`'s python branch has no way to reach: `array_path`, the container
    /// field a wildcard (`container[].field`) fixture path iterates.
    ///
    /// `element_accessor`'s shared `render_relative_to` always renders Python through
    /// `render_python_with_optionals`, whose `TypedDict`-vs-attribute owner cursor starts at
    /// `self.python_typeddict_map.root_type` — the call's RESULT type. That is correct for a
    /// result-anchored path, but an element-anchored path is owned by the collection's ELEMENT
    /// type, which can classify differently: an envelope result classified as `TypedDict` (so
    /// `result["structure"]` subscripts correctly) commonly holds elements that stay a native
    /// `#[pyclass]` (attribute access), and starting the element cursor at the result root
    /// rendered `_e["kind"]` against those elements — `TypeError: 'SampleItem' object is not
    /// subscriptable`. This resolves the element owner type by walking `array_path` through
    /// `python_typeddict_map.field_types` (via [`python_element_owner_type`]) and starts the
    /// cursor there instead. ~keep
    ///
    /// ~keep The two halves are anchored differently ON PURPOSE, and each half must be walked
    /// from the path its own renderer used. `element_path` stays on [`Self::resolve`] because it
    /// is already element-relative (see [`Self::element_accessor`] for what re-projecting it
    /// does). `array_path` goes through [`Self::result_relative_path`] because that is exactly
    /// what `render_python_wildcard_assertion` passed to [`Self::accessor`] to render the
    /// container half: walking the raw `resolve`d spelling instead made the owner cursor
    /// traverse a path the emitted container does not have. On an envelope root the container
    /// renders as `result["results"][0]["records"]` while `advance("Envelope", "records")` finds
    /// no edge — `python_element_owner_type` returns `None`, `is_typeddict(None)` is `false`, and
    /// the element silently fell back to attribute access, making the element-anchoring fix inert
    /// on precisely the projected shapes it was meant to cover. Deriving both halves from one
    /// path is what keeps them from disagreeing about where the container is.
    pub fn python_element_accessor(&self, element_path: &str, array_path: &str, element_var: &str) -> String {
        let array_effective = self.result_relative_path(array_path);
        let array_segments = parse_path(&array_effective);
        let owner_type = python_element_owner_type(
            &array_segments,
            &self.python_typeddict_map,
            &self.python_map_value_edges,
        );

        let effective = self.resolve(element_path);
        let segments = parse_path(effective);
        let segments = self.inject_array_indexing(segments);
        render_python_element_with_optionals(
            &segments,
            element_var,
            &self.optional_fields,
            &self.python_typeddict_map,
            &self.python_map_value_edges,
            owner_type,
        )
    }

    /// Which segments of `path` step into a tagged-union variant, keyed by the same tracked
    /// path key the accessor renderers build as they walk.
    ///
    /// ~keep Keyed by the renderer's own key (`push_key_field_name` then
    /// `push_key_index_suffix`, so `results[3].metadata` tracks as `results` then
    /// `results[0].metadata`) rather than by `(union, variant)`, because the renderer knows
    /// only where it is in the path, not what type it is standing on. Building the key here
    /// with the renderers' own helpers is what keeps the two from disagreeing -- a key built
    /// any other way silently never matches and the narrowing is inert.
    ///
    /// Empty whenever the language supplied no `variant_accessors`, which is every language
    /// but C# and Dart, so this costs nothing and changes nothing for them.
    fn variant_narrowings(&self, path: &str) -> HashMap<String, String> {
        let mut narrowings = HashMap::new();
        if self.variant_accessors.is_empty() {
            return narrowings;
        }
        let Some(root) = self.ir_enum_map.root_type.clone() else {
            return narrowings;
        };
        let mut key = String::new();
        let mut owner_path: Vec<String> = Vec::new();
        for segment in parse_path(path) {
            let Some(name) = super::super::parse::segment_name(&segment) else {
                continue;
            };
            let name = name.to_string();
            push_key_field_name(&mut key, &segment);
            let owner = owner_path.join(".");
            if !owner.is_empty()
                && let Some(union_type) = enum_type_at_path_from(&self.ir_enum_map, &root, &owner)
            {
                let variant = name.to_upper_camel_case();
                if let Some(accessor) = self.variant_accessors.narrowing_for(&union_type, &variant) {
                    narrowings.insert(key.clone(), accessor.to_string());
                }
            }
            owner_path.push(name);
            push_key_index_suffix(&mut key, &segment);
        }
        narrowings
    }

    /// Render `path` for TypeScript (node/wasm) when it crosses an internally-tagged union
    /// variant boundary whose payload is single-field/Named-type-resolvable
    /// ([`Self::union_variant_payload`]) -- the one shape `field_refusal::refusal_line` used to
    /// treat as an unconditional TypeScript dead end regardless of what either binding actually
    /// exposes. `None` when the path does not cross a declared tagged union at all, or when the
    /// crossing's variant is not this shape (several fields, or fields inlined by name rather
    /// than wrapped in one Named type) -- the caller's ordinary refusal still applies to those.
    ///
    /// node (NAPI): `backends::napi::gen_bindings::enums::gen_tagged_enum_as_object` gives this
    /// shape a REAL optional field on the flattened binding struct, named after the variant
    /// itself (`html`, `excel`, ...) via `tagged_enum_binding_field_js_name` -- the crossing has
    /// a member to spell, it was simply never asked for one. `field_skip.rs`'s "no `excel`
    /// property at all" note describes the OTHER shape napi also flattens (inline named fields,
    /// exposed under their OWN names with no variant-named member at all), not this one.
    ///
    /// wasm: an internally-tagged enum (no `#[serde(tag = .., content = ..)]`) is bridged as
    /// `JsValue` at the FIELD site regardless of shape (`mod.rs`'s `jsvalue_bridged_enum_names`
    /// covers every `is_tagged_data_enum`), straight off the CORE value via
    /// `serde_wasm_bindgen` -- and serde's internal tagging FLATTENS a struct-wrapping variant's
    /// fields onto the SAME JS object as the discriminant. The crossing segment therefore has no
    /// accessor of its own; the suffix reads directly off the container, untyped (`any`), so
    /// nothing needs narrowing to compile. `gen_tagged_enum_as_struct`'s standalone class (a
    /// discriminant plus a positionally-named payload slot shared by every tuple variant) is
    /// dead code for this purpose -- nothing ever types an actual struct field as that class. An
    /// adjacently-tagged enum (`tagged_enum_content_key` answers `Some`) does not flatten this
    /// way, so this arm declines rather than guess a shape it was never shown to produce. ~keep
    pub fn typescript_tagged_union_accessor(&self, path: &str, language: &str, result_var: &str) -> Option<String> {
        let (prefix, union_type, variant, suffix) = self.ir_tagged_union_split(path)?;
        self.union_variant_payload(&union_type, &variant)?;
        let container = if prefix.is_empty() {
            result_var.to_string()
        } else {
            self.accessor(&prefix, language, result_var)
        };
        match language {
            "node" => {
                let js_field = crate::codegen::naming::to_node_name(&variant);
                if suffix.is_empty() {
                    return Some(format!("{container}.{js_field}"));
                }
                let suffix_chain: Vec<String> =
                    suffix.split('.').map(crate::codegen::naming::to_node_name).collect();
                Some(format!("{container}.{js_field}?.{}", suffix_chain.join("?.")))
            }
            "wasm" if self.tagged_enum_content_key(&union_type).is_none() => {
                if suffix.is_empty() {
                    Some(container)
                } else {
                    Some(format!("{container}.{suffix}"))
                }
            }
            _ => None,
        }
    }

    /// Render `path` for Dart when it steps into a tagged-union variant, or `None` when it does
    /// not and the ordinary chain renderer applies.
    ///
    /// ~keep Dart narrows by CASTING, so unlike C#'s `As<Variant>` this cannot be a segment
    /// rename — the prefix has to be wrapped: `(<prefix> as <Union>_<Variant>).field0.<rest>`.
    /// That is why the two languages are handled at different levels; the shared decision (does
    /// this path cross a variant) is the resolver's either way, and only the rendered form
    /// differs. The subclass spelling and the payload accessor are both supplied by the Dart
    /// e2e codegen, because flutter_rust_bridge owns that naming, not alef.
    fn dart_narrowed_accessor(&self, path: &str, result_var: &str) -> Option<String> {
        if self.variant_accessors.is_empty() {
            return None;
        }
        let (prefix, union_type, variant, suffix) = self.ir_tagged_union_split(path)?;
        let subclass = self.variant_accessors.narrowing_for(&union_type, &variant)?;
        let payload = self.variant_accessors.payload_for(&union_type, &variant)?;

        let container = render_dart_with_optionals(
            &self.inject_array_indexing(parse_path(&prefix)),
            result_var,
            &self.optional_fields,
        );
        let narrowed = format!("({container} as {subclass}).{payload}");
        if suffix.is_empty() {
            return Some(narrowed);
        }
        // The suffix is owned by the payload type, not by the result anchor, so it is rendered
        // against the narrowed expression as its own root. Optionality keys are result-relative
        // and deliberately do not match here: emitting no `?.` on a payload field is the
        // conservative reading, and it is what the assertion emitter already produces. ~keep
        Some(render_dart_with_optionals(
            &parse_path(&suffix),
            &narrowed,
            &self.optional_fields,
        ))
    }

    /// Render an already-anchored path as a language-specific accessor rooted at `result_var`,
    /// which is the result variable for [`Self::accessor`] and the element binding for
    /// [`Self::element_accessor`].
    fn render_relative_to(&self, effective: &str, language: &str, result_var: &str) -> String {
        let segments = parse_path(effective);
        let segments = self.inject_array_indexing(segments);
        match language {
            // `node` and `wasm` are one language and must answer "does this link need `?.`"
            // once, from the same renderer. `wasm` used to fall through to the catch-all's
            // `render_wasm`, which knows nothing about optionality, so a fixture with an
            // `Option<T>` field got `result.document?.nodes` for node and the `TS18048`
            // `result.document.nodes` for wasm. Only the map lowering differs. ~keep
            "typescript" | "node" => render_typescript_with_optionals(
                &segments,
                result_var,
                &self.optional_fields,
                TypescriptMapAccess::Index,
            ),
            "wasm" => render_typescript_with_optionals(
                &segments,
                result_var,
                &self.optional_fields,
                TypescriptMapAccess::MapGet,
            ),
            "java" => render_java_with_optionals(&segments, result_var, &self.optional_fields),
            "kotlin" => render_kotlin_with_optionals(&segments, result_var, &self.optional_fields),
            // kotlin_android data classes expose fields as Kotlin properties (no parens),
            // not as Java-style getter methods. Use the dedicated renderer.
            "kotlin_android" => render_kotlin_android_with_optionals(&segments, result_var, &self.optional_fields),
            "rust" => render_rust_with_optionals(
                &segments,
                result_var,
                &self.optional_fields,
                &self.method_calls,
                &self.result_fields,
            ),
            "csharp" => render_csharp_with_optionals(
                &segments,
                result_var,
                &self.optional_fields,
                &self.variant_narrowings(effective),
            ),
            "zig" => render_zig_with_optionals(
                &segments,
                result_var,
                &self.optional_fields,
                &self.method_calls,
                &self.result_fields,
            ),
            // Always use `render_swift_with_first_class_map` for Swift. The map
            // correctly handles both first-class (property syntax) and opaque
            // (method-call syntax) types. When no type info is available (empty map,
            // unknown root type), `is_first_class(None)` returns `false` so
            // method-call syntax is the safe default — opaque swift-bridge types
            // expose fields as methods, not properties.
            "swift" => render_swift_with_first_class_map(
                &segments,
                result_var,
                &self.optional_fields,
                &self.swift_first_class_map,
            ),
            "dart" => self
                .dart_narrowed_accessor(effective, result_var)
                .unwrap_or_else(|| render_dart_with_optionals(&segments, result_var, &self.optional_fields)),
            "php" if !self.php_getter_map.is_empty() => {
                render_php_with_getters(&segments, result_var, &self.php_getter_map, &self.optional_fields)
            }
            "python" => render_python_with_optionals(
                &segments,
                result_var,
                &self.optional_fields,
                &self.python_typeddict_map,
                &self.python_map_value_edges,
            ),
            _ => render_accessor(&segments, language, result_var),
        }
    }

    /// Generate a language-specific accessor expression for an error-path field.
    ///
    /// Used when `assertion_type == "error"` and the fixture declares a `field`
    /// like `"error.status_code"`. The caller strips the `"error."` prefix and
    /// passes the sub-field name (e.g. `"status_code"`) here.
    ///
    /// Resolves against `error_field_aliases` (instead of the success-path
    /// `aliases`). Falls back to direct field access (i.e. `err_var.status_code`)
    /// when no alias exists.
    ///
    /// For Rust, uses `render_rust_with_optionals` so that fields in
    /// `method_calls` emit parentheses (e.g. `err.status_code()` when
    /// `"status_code"` is in `fields_method_calls`).
    pub fn accessor_for_error(&self, sub_field: &str, language: &str, err_var: &str) -> String {
        let resolved = self
            .error_field_aliases
            .get(sub_field)
            .map(String::as_str)
            .unwrap_or(sub_field);
        let segments = parse_path(resolved);
        // Error fields are simple scalar fields — no array injection needed.
        // For Rust, delegate to render_rust_with_optionals so method_calls are honoured.
        match language {
            "rust" => render_rust_with_optionals(
                &segments,
                err_var,
                &self.optional_fields,
                &self.method_calls,
                &self.result_fields,
            ),
            _ => render_accessor(&segments, language, err_var),
        }
    }

    /// Check whether a sub-field (the part after `"error."`) has an entry in
    /// `error_field_aliases` or if there are any error aliases at all.
    ///
    /// When there are no error aliases configured, callers fall back to
    /// direct field access, which is the safe default for known public fields
    /// like `status_code` on `SampleLlmError`.
    pub fn has_error_aliases(&self) -> bool {
        !self.error_field_aliases.is_empty()
    }

    fn inject_array_indexing(&self, segments: Vec<PathSegment>) -> Vec<PathSegment> {
        if self.array_fields.is_empty() {
            return segments;
        }
        let len = segments.len();
        let mut result = Vec::with_capacity(len);
        let mut path_so_far = String::new();
        for i in 0..len {
            let seg = &segments[i];
            match seg {
                PathSegment::Field(f) => {
                    if !path_so_far.is_empty() {
                        path_so_far.push('.');
                    }
                    path_so_far.push_str(f);
                    let next_is_length = i + 1 < len && matches!(segments[i + 1], PathSegment::Length);
                    if i + 1 < len && self.array_fields.contains(&path_so_far) && !next_is_length {
                        // Config-registered array field without explicit index — default to 0.
                        result.push(PathSegment::ArrayField {
                            name: f.clone(),
                            index: 0,
                        });
                    } else {
                        result.push(seg.clone());
                    }
                }
                // Explicit ArrayField from parse_path — pass through unchanged; the user's
                // explicit index takes precedence over any config default.
                PathSegment::ArrayField { .. } => {
                    result.push(seg.clone());
                }
                PathSegment::MapAccess { field, key } => {
                    if !path_so_far.is_empty() {
                        path_so_far.push('.');
                    }
                    path_so_far.push_str(field);
                    let is_numeric = !key.is_empty() && key.chars().all(|c| c.is_ascii_digit());
                    if is_numeric && self.array_fields.contains(&path_so_far) {
                        // Numeric map-access on a registered array field — upgrade to ArrayField.
                        let index: usize = key.parse().unwrap_or(0);
                        result.push(PathSegment::ArrayField {
                            name: field.clone(),
                            index,
                        });
                    } else {
                        result.push(seg.clone());
                    }
                }
                _ => {
                    result.push(seg.clone());
                }
            }
        }
        result
    }

    /// Generate a Rust variable binding that unwraps an Optional string field.
    pub fn rust_unwrap_binding(&self, fixture_field: &str, result_var: &str) -> Option<(String, String)> {
        let resolved = self.resolve(fixture_field);
        if !self.is_optional(resolved) {
            return None;
        }
        // ~keep Same shared answer `accessor()` renders from, not a mirror of it. The local's
        // name is derived from this path, so a copy that drifted would name the binding after one
        // path while the assertion that must reference it was rendered from another.
        let effective = self.result_relative_path(fixture_field);
        let segments = parse_path(&effective);
        let segments = self.inject_array_indexing(segments);
        // Sanitize the resolved path into a snake_case Rust identifier:
        // 1. `.` and `[` become `_` separators, `]` is dropped.
        // 2. Collapse runs of `_` so `foo[].bar` → `foo__bar` → `foo_bar`
        //    and strip any leading/trailing underscores.
        let local_var = {
            let raw = effective.replace(['.', '['], "_").replace(']', "");
            let mut collapsed = String::with_capacity(raw.len());
            let mut prev_underscore = false;
            for ch in raw.chars() {
                if ch == '_' {
                    if !prev_underscore {
                        collapsed.push('_');
                    }
                    prev_underscore = true;
                } else {
                    collapsed.push(ch);
                    prev_underscore = false;
                }
            }
            // Prefix with `_` so the binding declaration suppresses `-D unused_variables`
            // when no assertion actually references the local.  The variable remains fully
            // accessible under the `_`-prefixed name if an assertion does use it.
            format!("_{}", collapsed.trim_matches('_'))
        };
        // Use the optional-aware Rust renderer so intermediate `Option<T>`
        // segments produce `.as_ref().unwrap()` instead of bare field access.
        // For e.g. `summary.strategy` with `summary` in `optional_fields`, the
        // basic `render_accessor` would emit `result.summary.strategy`, which
        // is a compile error because `Option<Summary>` has no `strategy` field.
        let accessor = render_rust_with_optionals(
            &segments,
            result_var,
            &self.optional_fields,
            &self.method_calls,
            &self.result_fields,
        );
        let has_map_access = segments.iter().any(|s| {
            if let PathSegment::MapAccess { key, .. } = s {
                !key.chars().all(|c| c.is_ascii_digit())
            } else {
                false
            }
        });
        // `is_array` asks whether the *field* is Vec-typed, via `segment_name`, which returns the
        // bare name for a `PathSegment::ArrayField` and so cannot tell `detected_languages` from
        // `detected_languages[0]`. A trailing indexed segment has already reduced the accessor to
        // one element, so the array branch below would emit `.as_deref().unwrap_or(&[])` on a
        // concrete `String`. (A `Vec<Vec<T>>` leaf would still be an array after one index; no
        // fixture path indexes into a nested collection, and IR-walking the element type here
        // would duplicate `ir_collection`'s cursor.) ~keep
        let leaf_is_indexed_element = matches!(segments.last(), Some(PathSegment::ArrayField { .. }));
        let is_array = self.is_array(resolved) && !leaf_is_indexed_element;
        let binding = if has_map_access {
            format!("let {local_var} = {accessor}.unwrap_or(\"\");")
        } else if leaf_is_indexed_element {
            // The trailing index already consumed the `Option<Vec<T>>` wrapper -- `render_rust_with_optionals`
            // emitted `.as_ref().unwrap()[0]`, so the accessor is a concrete element, not an `Option`.
            // The optional-scalar branch below would call `.as_ref().map(..)` on it, which for a `String`
            // leaf is an ambiguous `AsRef` and does not even infer a type. ~keep
            format!("let {local_var} = {accessor}.to_string();")
        } else if is_array {
            format!("let {local_var} = {accessor}.as_deref().unwrap_or(&[]);")
        } else {
            // Use Display (via `.to_string()`) so types that intentionally implement Display
            // with a serde-style representation (e.g. `FinishReason` rendering as
            // `"content_filter"`) match the wire-format strings asserted in fixtures.
            // Types without Display would need to be excluded from string-equals assertions
            // or have a Display impl added to the core library.
            format!("let {local_var} = {accessor}.as_ref().map(|v| v.to_string()).unwrap_or_default();")
        };
        Some((binding, local_var))
    }
}

#[cfg(test)]
mod typescript_tagged_union_accessor_tests {
    use super::*;
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    /// The IR shape `metadata.format.html.title`/`metadata.format.excel.sheet_count` actually
    /// have: an internally-tagged, single-tuple-Named-type-variant enum (`FormatMetadata`) with
    /// no `content` key, reached through an `Option<FormatMetadata>` struct field -- matching
    /// `tagged_union_crossing::collection_tests`'s `nested_headers_resolve_through_batch_and_tagged_union_ownership`.
    fn resolver_over_format_metadata() -> FieldResolver {
        let types = vec![
            TypeDef {
                name: "Metadata".to_string(),
                fields: vec![field(
                    "format",
                    TypeRef::Optional(Box::new(TypeRef::Named("FormatMetadata".to_string()))),
                )],
                ..TypeDef::default()
            },
            TypeDef {
                name: "HtmlMetadata".to_string(),
                fields: vec![field("title", TypeRef::Optional(Box::new(TypeRef::String)))],
                ..TypeDef::default()
            },
        ];
        let enums = vec![EnumDef {
            name: "FormatMetadata".to_string(),
            serde_tag: Some("format_type".to_string()),
            variants: vec![EnumVariant {
                name: "Html".to_string(),
                is_tuple: true,
                fields: vec![field("_0", TypeRef::Named("HtmlMetadata".to_string()))],
                ..EnumVariant::default()
            }],
            ..EnumDef::default()
        }];
        FieldResolver::new(
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        )
        .with_ir_enum_map(FieldResolver::ir_enum_fields(&types, &enums), Some("Metadata".to_string()))
    }

    /// The defect: napi flattens `FormatMetadata` into a REAL optional `html` field
    /// (`backends::napi::gen_bindings::enums::gen_tagged_enum_as_object`), so the crossing has a
    /// member to spell. Node must reach it with optional chaining, not refuse it.
    #[test]
    fn node_reaches_the_napi_flattened_variant_field() {
        let resolver = resolver_over_format_metadata();
        assert_eq!(
            resolver.typescript_tagged_union_accessor("format.html.title", "node", "result"),
            Some("result.format.html?.title".to_string())
        );
    }

    /// wasm bridges an internally-tagged enum as `JsValue` at the field site and serde flattens
    /// the payload onto the same object as the discriminant, so the crossing segment itself has
    /// no accessor -- the suffix reads straight off the container.
    #[test]
    fn wasm_reaches_the_flattened_serde_payload_with_no_variant_segment() {
        let resolver = resolver_over_format_metadata();
        assert_eq!(
            resolver.typescript_tagged_union_accessor("format.html.title", "wasm", "result"),
            Some("result.format.title".to_string())
        );
    }

    /// The control that stops "every crossing is now reachable" from passing: a variant whose
    /// payload is not a single Named type (here, two inline fields) is a shape neither binding
    /// gives a real member for, so [`FieldResolver::union_variant_payload`] has nothing to
    /// resolve and the accessor must still decline.
    #[test]
    fn a_multi_field_variant_crossing_still_declines() {
        let types = vec![TypeDef {
            name: "Metadata".to_string(),
            fields: vec![field("format", TypeRef::Named("FormatMetadata".to_string()))],
            ..TypeDef::default()
        }];
        let enums = vec![EnumDef {
            name: "FormatMetadata".to_string(),
            serde_tag: Some("format_type".to_string()),
            variants: vec![EnumVariant {
                name: "Basic".to_string(),
                fields: vec![
                    field("username", TypeRef::String),
                    field("password", TypeRef::String),
                ],
                ..EnumVariant::default()
            }],
            ..EnumDef::default()
        }];
        let resolver = FieldResolver::new(
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        )
        .with_ir_enum_map(FieldResolver::ir_enum_fields(&types, &enums), Some("Metadata".to_string()));

        assert_eq!(
            resolver.typescript_tagged_union_accessor("format.basic.username", "node", "result"),
            None
        );
        assert_eq!(
            resolver.typescript_tagged_union_accessor("format.basic.username", "wasm", "result"),
            None
        );
    }
}
