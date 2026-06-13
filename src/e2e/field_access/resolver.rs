use super::optional_renderers::{
    render_csharp_with_optionals, render_dart_with_optionals, render_java_with_optionals,
    render_kotlin_android_with_optionals, render_kotlin_with_optionals, render_php_with_getters,
    render_rust_with_optionals, render_zig_with_optionals,
};
use super::parse::{normalize_numeric_indices, parse_path, strip_numeric_indices};
use super::renderers::{render_accessor, render_swift_with_first_class_map};
use super::types::{DartFirstClassMap, FieldResolver, PathSegment, PhpGetterMap, StringyField, SwiftFirstClassMap};
use std::collections::{HashMap, HashSet};

impl FieldResolver {
    /// Create a new resolver from the e2e config's `fields` aliases,
    /// `fields_optional` set, `result_fields` set, `fields_array` set,
    /// and `fields_method_calls` set.
    pub fn new(
        fields: &HashMap<String, String>,
        optional: &HashSet<String>,
        result_fields: &HashSet<String>,
        array_fields: &HashSet<String>,
        method_calls: &HashSet<String>,
    ) -> Self {
        Self {
            aliases: fields.clone(),
            optional_fields: optional.clone(),
            result_fields: result_fields.clone(),
            array_fields: array_fields.clone(),
            method_calls: method_calls.clone(),
            error_field_aliases: HashMap::new(),
            php_getter_map: PhpGetterMap::default(),
            swift_first_class_map: SwiftFirstClassMap::default(),
            dart_first_class_map: DartFirstClassMap::default(),
        }
    }

    /// Create a new resolver that also includes error-path field aliases.
    ///
    /// `error_field_aliases` maps fixture sub-field names (the part after `"error."`)
    /// to the actual field names on the error type, enabling `accessor_for_error` to
    /// resolve fields like `"status_code"` against the error value.
    pub fn new_with_error_aliases(
        fields: &HashMap<String, String>,
        optional: &HashSet<String>,
        result_fields: &HashSet<String>,
        array_fields: &HashSet<String>,
        method_calls: &HashSet<String>,
        error_field_aliases: &HashMap<String, String>,
    ) -> Self {
        Self {
            aliases: fields.clone(),
            optional_fields: optional.clone(),
            result_fields: result_fields.clone(),
            array_fields: array_fields.clone(),
            method_calls: method_calls.clone(),
            error_field_aliases: error_field_aliases.clone(),
            php_getter_map: PhpGetterMap::default(),
            swift_first_class_map: SwiftFirstClassMap::default(),
            dart_first_class_map: DartFirstClassMap::default(),
        }
    }

    /// Create a new resolver that also knows which PHP fields need getter-method syntax.
    ///
    /// `php_getter_map` carries a per-`(type_name, field_name)` classification: the PHP
    /// accessor renderer emits `->getCamelCase()` when `(owner_type, field)` is
    /// recorded as needing a getter, and `->camelCase` property syntax otherwise.
    /// This matches the ext-php-rs 0.15.x behaviour where `#[php(getter)]` is used for
    /// non-scalar fields (Named structs, `Vec<Named>`, Map, etc.) while `#[php(prop)]` is
    /// used for scalar-compatible fields.
    ///
    /// Keying by (type, field) — not bare field name — is essential because the same
    /// field name can have different scalarness on different types. The map also carries
    /// per-type field→nested-type mappings so the renderer can walk a path like
    /// `outer.inner.content` through the IR, advancing the current-type cursor at each
    /// segment.
    pub fn new_with_php_getters(
        fields: &HashMap<String, String>,
        optional: &HashSet<String>,
        result_fields: &HashSet<String>,
        array_fields: &HashSet<String>,
        method_calls: &HashSet<String>,
        error_field_aliases: &HashMap<String, String>,
        php_getter_map: PhpGetterMap,
    ) -> Self {
        Self {
            aliases: fields.clone(),
            optional_fields: optional.clone(),
            result_fields: result_fields.clone(),
            array_fields: array_fields.clone(),
            method_calls: method_calls.clone(),
            error_field_aliases: error_field_aliases.clone(),
            php_getter_map,
            swift_first_class_map: SwiftFirstClassMap::default(),
            dart_first_class_map: DartFirstClassMap::default(),
        }
    }

    /// Return a clone of this resolver with the Swift first-class map's
    /// `root_type` replaced.
    ///
    /// Used by Swift e2e codegen to thread a per-fixture (per-call) root type
    /// into the `render_swift_with_first_class_map` dispatcher. Each fixture's
    /// call returns a different IR type (e.g. `ChatCompletionResponse` vs
    /// `FileObject`), and the first-class/opaque classification of the root
    /// drives whether path segments are emitted with property access or
    /// method-call access. Setting it per-fixture avoids picking a single
    /// workspace-wide default that breaks half the fixtures.
    pub fn with_swift_root_type(&self, root_type: Option<String>) -> Self {
        let mut clone = self.clone();
        clone.swift_first_class_map.root_type = root_type;
        clone
    }

    /// Create a new resolver that also knows the Swift first-class/opaque
    /// classification per IR type. Mirrors `new_with_php_getters` but for the
    /// Swift `render_swift_with_first_class_map` path.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_swift_first_class(
        fields: &HashMap<String, String>,
        optional: &HashSet<String>,
        result_fields: &HashSet<String>,
        array_fields: &HashSet<String>,
        method_calls: &HashSet<String>,
        error_field_aliases: &HashMap<String, String>,
        swift_first_class_map: SwiftFirstClassMap,
    ) -> Self {
        Self {
            aliases: fields.clone(),
            optional_fields: optional.clone(),
            result_fields: result_fields.clone(),
            array_fields: array_fields.clone(),
            method_calls: method_calls.clone(),
            error_field_aliases: error_field_aliases.clone(),
            php_getter_map: PhpGetterMap::default(),
            swift_first_class_map,
            dart_first_class_map: DartFirstClassMap::default(),
        }
    }

    /// Create a new resolver that also knows the Dart stringy field
    /// classification per IR type (for aggregating text accessors in contains
    /// assertions on `Vec<T>` fields).
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_dart_first_class(
        fields: &HashMap<String, String>,
        optional: &HashSet<String>,
        result_fields: &HashSet<String>,
        array_fields: &HashSet<String>,
        method_calls: &HashSet<String>,
        error_field_aliases: &HashMap<String, String>,
        dart_first_class_map: DartFirstClassMap,
    ) -> Self {
        Self {
            aliases: fields.clone(),
            optional_fields: optional.clone(),
            result_fields: result_fields.clone(),
            array_fields: array_fields.clone(),
            method_calls: method_calls.clone(),
            error_field_aliases: error_field_aliases.clone(),
            php_getter_map: PhpGetterMap::default(),
            swift_first_class_map: SwiftFirstClassMap::default(),
            dart_first_class_map,
        }
    }

    /// Return a clone of this resolver with the Dart first-class map's
    /// `root_type` replaced.
    pub fn with_dart_root_type(&self, root_type: Option<String>) -> Self {
        let mut clone = self.clone();
        clone.dart_first_class_map.root_type = root_type;
        clone
    }

    /// Resolve a fixture field path to the actual struct path.
    /// Falls back to the field itself if no alias exists.
    pub fn resolve<'a>(&'a self, fixture_field: &'a str) -> &'a str {
        self.aliases
            .get(fixture_field)
            .map(String::as_str)
            .unwrap_or(fixture_field)
    }

    /// True when the leaf segment of `field` is a `Vec<T>` field on any IR type.
    ///
    /// Used by swift codegen to keep `.count` straight on method-call accessors
    /// (`result.output()` returns RustVec — `.count` works directly, no
    /// `.toString()` needed). The check is on the bare leaf name, so it is best-
    /// effort when distinct types share a field name with different kinds.
    pub fn leaf_is_vec_via_swift_map(&self, field: &str) -> bool {
        let leaf = field.split('.').next_back().unwrap_or(field);
        let leaf = leaf.split('[').next().unwrap_or(leaf);
        self.swift_first_class_map.is_vec_field_name(leaf)
    }

    /// IR type backing the Swift result variable, if known. Used by
    /// `swift_build_accessor` to seed its per-segment type cursor.
    pub fn swift_root_type(&self) -> Option<&String> {
        self.swift_first_class_map.root_type.as_ref()
    }

    /// Whether fields on `type_name` should be accessed as Swift properties
    /// (first-class Codable struct → `public let`) vs swift-bridge method calls
    /// (typealias-to-opaque RustBridge class). Mirrors `SwiftFirstClassMap::is_first_class`.
    pub fn swift_is_first_class(&self, type_name: Option<&str>) -> bool {
        self.swift_first_class_map.is_first_class(type_name)
    }

    /// Advance the per-segment type cursor by one field name. Mirrors
    /// `SwiftFirstClassMap::advance`.
    pub fn swift_advance(&self, owner_type: Option<&str>, field_name: &str) -> Option<String> {
        self.swift_first_class_map.advance(owner_type, field_name)
    }

    /// Stringy field accessors recorded for `type_name` in the Swift
    /// first-class map (used by `contains` assertions on `Vec<T>` element
    /// types).
    pub fn swift_stringy_fields(&self, type_name: &str) -> Option<&[StringyField]> {
        self.swift_first_class_map.stringy_fields(type_name)
    }

    /// IR type backing the Dart result variable, if known.
    pub fn dart_root_type(&self) -> Option<&String> {
        self.dart_first_class_map.root_type.as_ref()
    }

    /// Advance the Dart type cursor through a field, returning the target type name.
    pub fn dart_advance(&self, owner_type: Option<&str>, field_name: &str) -> Option<String> {
        self.dart_first_class_map.advance(owner_type, field_name)
    }

    /// Stringy field accessors recorded for `type_name` in the Dart
    /// first-class map (used by `contains` assertions on `Vec<T>` element
    /// types).
    pub fn dart_stringy_fields(&self, type_name: &str) -> Option<&[StringyField]> {
        self.dart_first_class_map.stringy_fields(type_name)
    }

    /// Check if a resolved field path is optional.
    pub fn is_optional(&self, field: &str) -> bool {
        if self.is_optional_direct(field) {
            return true;
        }
        // Namespace-prefix fallback: paths like `interaction.action_results[0].data`
        // strip the virtual `interaction.` prefix before consulting `optional_fields`,
        // matching the same convention used by `is_valid_for_result`.
        if let Some(suffix) = self.namespace_stripped_path(field) {
            if self.is_optional_direct(suffix) {
                return true;
            }
        }
        false
    }

    fn is_optional_direct(&self, field: &str) -> bool {
        if self.optional_fields.contains(field) {
            return true;
        }
        let index_normalized = normalize_numeric_indices(field);
        if index_normalized != field && self.optional_fields.contains(index_normalized.as_str()) {
            return true;
        }
        // Also check with all numeric indices stripped: "choices[0].message.tool_calls"
        // should match optional_fields entry "choices.message.tool_calls".
        let de_indexed = strip_numeric_indices(field);
        if de_indexed != field && self.optional_fields.contains(de_indexed.as_str()) {
            return true;
        }
        let normalized = field.replace("[].", ".");
        if normalized != field && self.optional_fields.contains(normalized.as_str()) {
            return true;
        }
        for af in &self.array_fields {
            if let Some(rest) = field.strip_prefix(af.as_str()) {
                if let Some(rest) = rest.strip_prefix('.') {
                    let with_bracket = format!("{af}[].{rest}");
                    if self.optional_fields.contains(with_bracket.as_str()) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Check if a fixture field has an explicit alias mapping.
    pub fn has_alias(&self, fixture_field: &str) -> bool {
        self.aliases.contains_key(fixture_field)
    }

    /// Check whether `field_name` is configured as an explicit result field.
    ///
    /// Returns true only when the caller has populated `result_fields` AND the
    /// field name is present. Empty `result_fields` always returns false — use
    /// `is_valid_for_result` for the default-allow semantics.
    pub fn has_explicit_field(&self, field_name: &str) -> bool {
        if self.result_fields.is_empty() {
            return false;
        }
        self.result_fields.contains(field_name)
    }

    /// Check whether a fixture field path is valid for the configured result type.
    ///
    /// Returns `true` when the resolved path's first segment is in `result_fields`,
    /// or when the path uses a single virtual namespace prefix (e.g. `"browser."`,
    /// `"interaction."`) whose second segment IS in `result_fields`.  The namespace
    /// prefix pattern is common in route-array fixtures where authors group
    /// related assertion fields under an organizational prefix that does not
    /// correspond to a real struct field on the return type.
    pub fn is_valid_for_result(&self, fixture_field: &str) -> bool {
        if self.result_fields.is_empty() {
            return true;
        }
        let resolved = self.resolve(fixture_field);
        let first_segment = resolved.split('.').next().unwrap_or(resolved);
        let first_segment = first_segment.split('[').next().unwrap_or(first_segment);
        if self.result_fields.contains(first_segment) {
            return true;
        }
        // Namespace-prefix fallback: if the first segment is NOT a known result field
        // but stripping it yields a path whose own first segment IS a known result
        // field, treat the path as valid.  This supports fixture field paths like
        // `"browser.browser_used"` where `"browser"` is a virtual grouping prefix
        // and the real field is `"browser_used"`.
        if let Some(suffix) = self.namespace_stripped_path(resolved) {
            let suffix_first = suffix.split('.').next().unwrap_or(suffix);
            let suffix_first = suffix_first.split('[').next().unwrap_or(suffix_first);
            return self.result_fields.contains(suffix_first);
        }
        false
    }

    /// If `path`'s first dot-separated segment is NOT in `result_fields` and
    /// contains no `[…]` indexing (i.e. it looks like a pure namespace label),
    /// return the remainder of the path after that first segment.  Returns `None`
    /// when the first segment already matches a result field or when stripping it
    /// would leave an empty string.
    pub fn namespace_stripped_path<'a>(&self, path: &'a str) -> Option<&'a str> {
        // When the consumer hasn't configured `result_fields`, there is no way
        // to tell a virtual namespace prefix (e.g. `interaction.action_results`)
        // from a real nested-struct field path (e.g. `metrics.total_lines`).
        // Defaulting to "strip" was lossy — every dotted field path was reduced
        // to its leaf segment, so backends (notably the C e2e codegen) emitted
        // accessors against the wrong parent type. Opt the stripping in only
        // when the consumer explicitly listed the top-level result fields.
        if self.result_fields.is_empty() {
            return None;
        }
        let dot_pos = path.find('.')?;
        let first = &path[..dot_pos];
        // Only strip if the first segment contains no brackets (i.e. is a bare
        // label, not an array access like `pages[0]`).
        if first.contains('[') {
            return None;
        }
        // Only strip if the first segment is NOT itself a known result field —
        // real fields should never be treated as namespace prefixes.
        if self.result_fields.contains(first) {
            return None;
        }
        let suffix = &path[dot_pos + 1..];
        if suffix.is_empty() { None } else { Some(suffix) }
    }

    /// Check if a resolved field is an array/Vec type.
    pub fn is_array(&self, field: &str) -> bool {
        self.array_fields.contains(field)
    }

    /// Check if a field name is the root of a collection type (i.e., the field
    /// itself returns a `Vec`/array, even though it is not in `fields_array`
    /// directly).
    ///
    /// `fields_array` tracks traversal paths like `choices[0].message.tool_calls`
    /// — the array element paths — not the bare collection accessor (`choices`).
    /// `fields_optional` may also contain paths like `data[0].url` that reveal
    /// `data` is a collection root.
    ///
    /// Returns `true` when any entry in `array_fields` or `optional_fields`
    /// starts with `{field}[`, indicating that `field` is the top-level
    /// collection getter.
    pub fn is_collection_root(&self, field: &str) -> bool {
        let prefix = format!("{field}[");
        self.array_fields.iter().any(|af| af.starts_with(&prefix))
            || self.optional_fields.iter().any(|of| of.starts_with(&prefix))
    }

    /// Check if a resolved field path traverses a tagged-union variant.
    ///
    /// Returns `Some((prefix, variant, suffix))` where:
    /// - `prefix` is the path up to (but not including) the tagged-union field
    ///   (e.g., `"metadata.format"`)
    /// - `variant` is the tagged-union accessor segment
    ///   (e.g., `"excel"`)
    /// - `suffix` is the remaining path after the variant
    ///   (e.g., `"sheet_count"`)
    ///
    /// Returns `None` if no tagged-union segment exists in the path.
    pub fn tagged_union_split(&self, fixture_field: &str) -> Option<(String, String, String)> {
        let resolved = self.resolve(fixture_field);
        let segments: Vec<&str> = resolved.split('.').collect();
        let mut path_so_far = String::new();
        for (i, seg) in segments.iter().enumerate() {
            if !path_so_far.is_empty() {
                path_so_far.push('.');
            }
            path_so_far.push_str(seg);
            if self.method_calls.contains(&path_so_far) {
                // Everything before the last segment of path_so_far is the prefix.
                let prefix = segments[..i].join(".");
                let variant = (*seg).to_string();
                let suffix = segments[i + 1..].join(".");
                return Some((prefix, variant, suffix));
            }
        }
        None
    }

    /// Check if a resolved field path contains a non-numeric map access.
    pub fn has_map_access(&self, fixture_field: &str) -> bool {
        let resolved = self.resolve(fixture_field);
        let segments = parse_path(resolved);
        segments.iter().any(|s| {
            if let PathSegment::MapAccess { key, .. } = s {
                !key.chars().all(|c| c.is_ascii_digit())
            } else {
                false
            }
        })
    }

    /// Generate a language-specific accessor expression.
    ///
    /// When `fixture_field` resolves to a path whose first segment is a virtual
    /// namespace prefix (not a real result field), the prefix is stripped before
    /// generating the accessor.  This matches the behaviour of `is_valid_for_result`
    /// so that paths like `"browser.browser_used"` produce `result.browser_used`
    /// (Python) / `result.BrowserUsed` (C#) / etc. rather than the raw
    /// `result.browser.browser_used` which would fail at runtime.
    pub fn accessor(&self, fixture_field: &str, language: &str, result_var: &str) -> String {
        let resolved = self.resolve(fixture_field);
        // Strip a leading namespace prefix when the first segment is not a known
        // result field but the remainder's first segment is.  This handles fixture
        // paths like `"browser.browser_used"` → actual accessor path `"browser_used"`.
        let effective = if !self.result_fields.is_empty() {
            if let Some(stripped) = self.namespace_stripped_path(resolved) {
                let stripped_first = stripped.split('.').next().unwrap_or(stripped);
                let stripped_first = stripped_first.split('[').next().unwrap_or(stripped_first);
                if self.result_fields.contains(stripped_first) {
                    stripped
                } else {
                    resolved
                }
            } else {
                resolved
            }
        } else {
            resolved
        };
        let segments = parse_path(effective);
        let segments = self.inject_array_indexing(segments);
        match language {
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
            "zig" => render_zig_with_optionals(&segments, result_var, &self.optional_fields, &self.method_calls),
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
                render_php_with_getters(&segments, result_var, &self.php_getter_map)
            }
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
        // Mirror the namespace-prefix stripping done in `accessor()` so paths
        // like `"interaction.action_results[0].data"` resolve against the real
        // result type (`InteractionResult`) rather than the literal namespace.
        let effective = if !self.result_fields.is_empty() {
            if let Some(stripped) = self.namespace_stripped_path(resolved) {
                let stripped_first = stripped.split('.').next().unwrap_or(stripped);
                let stripped_first = stripped_first.split('[').next().unwrap_or(stripped_first);
                if self.result_fields.contains(stripped_first) {
                    stripped
                } else {
                    resolved
                }
            } else {
                resolved
            }
        } else {
            resolved
        };
        let segments = parse_path(effective);
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
        let is_array = self.is_array(resolved);
        let binding = if has_map_access {
            format!("let {local_var} = {accessor}.unwrap_or(\"\");")
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
