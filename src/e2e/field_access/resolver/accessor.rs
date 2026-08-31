use super::super::optional_renderers::{
    TypescriptMapAccess, render_csharp_with_optionals, render_dart_with_optionals, render_java_with_optionals,
    render_kotlin_android_with_optionals, render_kotlin_with_optionals, render_php_with_getters,
    render_rust_with_optionals, render_typescript_with_optionals, render_zig_with_optionals,
};
use super::super::parse::parse_path;
use super::super::python_renderer::{
    python_element_owner_type, render_python_element_with_optionals, render_python_with_optionals,
};
use super::super::renderers::{render_accessor, render_swift_with_first_class_map};
use super::super::types::{FieldResolver, PathSegment};

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
            "csharp" => render_csharp_with_optionals(&segments, result_var, &self.optional_fields),
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
            "dart" => render_dart_with_optionals(&segments, result_var, &self.optional_fields),
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
