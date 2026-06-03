//! Field path resolution for nested struct/map access in e2e assertions.
//!
//! The `FieldResolver` maps fixture field paths (e.g., "metadata.title") to
//! actual API struct paths (e.g., "metadata.document.title") and generates
//! language-specific accessor expressions.

use crate::codegen::naming::to_go_name;
use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};
use std::collections::{HashMap, HashSet};

/// Resolves fixture field paths to language-specific accessor expressions.
#[derive(Clone)]
pub struct FieldResolver {
    aliases: HashMap<String, String>,
    optional_fields: HashSet<String>,
    result_fields: HashSet<String>,
    array_fields: HashSet<String>,
    method_calls: HashSet<String>,
    /// Aliases for error-path field access (used when assertion_type == "error").
    /// Maps fixture sub-field names (the part after "error.") to actual field names
    /// on the error type. E.g., `"status_code" -> "status_code"`.
    error_field_aliases: HashMap<String, String>,
    /// Per-type PHP getter classification: maps an owner type's snake_case field
    /// name to whether THAT field on THAT type requires `->getCamelCase()` syntax
    /// (because the field's mapped PHP type is non-scalar and ext-php-rs emits a
    /// `#[php(getter)]` method) rather than `->camelCase` property access.
    /// Populated by `new_with_php_getters`; empty by default.
    ///
    /// Keying by (type, field) — not bare field name — is required because two
    /// different types can declare the same field name with different scalarness
    /// (e.g. `CrawlConfig.content: ContentConfig` is non-scalar while
    /// `MarkdownResult.content: String` is scalar).
    php_getter_map: PhpGetterMap,
    /// Per-type Swift first-class/opaque classification, populated by the
    /// Swift e2e codegen. When non-empty, `accessor` uses
    /// `render_swift_with_first_class_map` instead of the legacy property-only
    /// `render_swift_with_optionals`, so paths that traverse from first-class
    /// types (property access) into opaque typealias types (method-call access)
    /// pick the correct syntax at each segment.
    swift_first_class_map: SwiftFirstClassMap,
    /// Per-type Dart stringy field classification, populated by the Dart e2e
    /// codegen. Used to aggregate every readable text accessor on a `Vec<T>`
    /// element type for `contains` assertions.
    dart_first_class_map: DartFirstClassMap,
}

/// Per-type PHP getter classification + chain-resolution metadata.
///
/// Holds enough information to resolve a multi-segment field path through the
/// IR's nested type graph and pick the correct accessor style at each segment:
///
/// * `getters[type_name]` — set of field names on `type_name` whose PHP binding
///   uses a `#[php(getter)]` method (caller must emit `->getCamelCase()`).
/// * `field_types[type_name][field_name]` — the IR-resolved `Named` type that
///   `field_name` traverses into, used to advance the "current type" cursor
///   for the next path segment. Absent for terminal/scalar fields.
/// * `root_type` — the IR type name backing the result variable at the start of
///   any chain. When `None`, chain traversal degrades to per-segment lookup
///   using a flattened union across all types (legacy bare-name behaviour),
///   which produces false positives when field names collide across types.
#[derive(Debug, Clone, Default)]
pub struct PhpGetterMap {
    pub getters: HashMap<String, HashSet<String>>,
    pub field_types: HashMap<String, HashMap<String, String>>,
    pub root_type: Option<String>,
    /// All field names per type — used to detect when the recorded `root_type`
    /// is a misclassification (a workspace-global root_type may not match the
    /// actual return type of a per-fixture call). When `owner_type` is set but
    /// `all_fields[owner_type]` doesn't contain `field_name`, the renderer
    /// falls back to the bare-name union instead of trusting the (wrong) owner.
    pub all_fields: HashMap<String, HashSet<String>>,
}

/// Swift first-class struct classification + chain-resolution metadata.
///
/// alef-backend-swift emits two flavors of binding types:
///
/// * **First-class Codable structs** — `public struct Foo: Codable { public let id: String }`.
///   Fields are Swift properties; access with `.id` (no parens).
/// * **Opaque typealiases** — `public typealias Foo = RustBridge.Foo` where the
///   RustBridge class exposes swift-bridge methods. Fields are methods;
///   access with `.id()` (parens).
///
/// The renderer needs per-segment dispatch because a path can traverse both:
/// e.g. `BatchListResponse` (first-class Codable, with `data: [BatchObject]`) →
/// indexed `[0]` → `BatchObject` (opaque typealias). At the `BatchObject` cursor
/// the renderer must switch to method-call access for `.id`, `.status`, etc.
///
/// * `first_class_types` — set of TypeDef names whose binding is a first-class
///   Codable struct. Membership = "use property access for fields on this type".
/// * `field_types[type_name][field_name]` — the IR-resolved `Named` type that
///   `field_name` traverses into.
/// * `vec_field_names` — flat set of field names whose IR type is `Vec<T>` on
///   any owner. Used by swift_count_target to keep `.count` straight on
///   RustVec-typed method-call accessors (don't inject `.toString()`).
/// * `root_type` — the IR type name backing the result variable.
///
/// Kind of a "stringy" field on an opaque DTO element type — used by the swift
/// e2e `contains` assertion to aggregate every readable text accessor on a
/// `Vec<T>` element instead of relying on a single primary accessor (which
/// often guesses wrong: e.g. `ImportInfo.source` is the module path but
/// `ImportInfo.items` carries the imported names).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringyFieldKind {
    /// `field_name() -> RustString` (or `String`). Convert via `.toString()`.
    Plain,
    /// `field_name() -> Optional<RustString>`. Convert via `?.toString() ?? ""`.
    Optional,
    /// `field_name() -> RustVec<RustString>`. Iterate elements (RustStringRef
    /// → `.asStr().toString()` on each).
    Vec,
}

/// A single readable text accessor on an opaque DTO. The `name` is the Rust
/// field name (snake_case), used to derive the swift-bridge lowerCamelCase
/// method call.
#[derive(Debug, Clone)]
pub struct StringyField {
    pub name: String,
    pub kind: StringyFieldKind,
}

#[derive(Debug, Clone, Default)]
pub struct SwiftFirstClassMap {
    pub first_class_types: HashSet<String>,
    pub field_types: HashMap<String, HashMap<String, String>>,
    pub vec_field_names: HashSet<String>,
    pub root_type: Option<String>,
    /// Per-type readable text accessors. Keyed by IR TypeDef name. Used by the
    /// swift e2e `contains` assertion to aggregate every stringy field on a
    /// `Vec<T>` element type into a `contains(where: { ... })` closure that
    /// does substring matching against every text-bearing accessor. Mirrors
    /// python's `_alef_e2e_item_texts` helper.
    pub stringy_fields_by_type: HashMap<String, Vec<StringyField>>,
}

impl SwiftFirstClassMap {
    /// Returns true when fields on `type_name` should be accessed as properties
    /// (no parens), false when they should be accessed via method-call.
    ///
    /// When `type_name` is `None` the renderer defaults to property syntax
    /// (matching the common case where result types are first-class).
    pub fn is_first_class(&self, type_name: Option<&str>) -> bool {
        match type_name {
            Some(t) => self.first_class_types.contains(t),
            None => true,
        }
    }

    /// Returns the IR `Named` type that `field_name` traverses into for the
    /// next chain segment, or `None` if the field is terminal/scalar/unknown.
    pub fn advance(&self, owner_type: Option<&str>, field_name: &str) -> Option<String> {
        let owner = owner_type?;
        self.field_types.get(owner).and_then(|m| m.get(field_name).cloned())
    }

    /// True when `field_name` appears as a `Vec<T>` (or `Option<Vec<T>>`) on
    /// any IR type. swift codegen consults this when deciding whether `.count`
    /// on a method-call accessor needs `.toString()` injected: RustVec already
    /// supports `.count` directly; RustString does not.
    pub fn is_vec_field_name(&self, field_name: &str) -> bool {
        self.vec_field_names.contains(field_name)
    }

    /// True when no per-type information is recorded.
    pub fn is_empty(&self) -> bool {
        self.first_class_types.is_empty() && self.field_types.is_empty()
    }

    /// Returns the list of stringy accessors recorded for `type_name`, or
    /// `None` if the type has no recorded stringy fields.
    pub fn stringy_fields(&self, type_name: &str) -> Option<&[StringyField]> {
        self.stringy_fields_by_type.get(type_name).map(Vec::as_slice)
    }
}

/// Dart opaque type classification + chain-resolution metadata, mirroring
/// Swift's needs to track stringy field accessors on element types for
/// `Vec<T>` contains assertions. Unlike Swift, Dart doesn't distinguish
/// first-class vs opaque; we just track stringy fields per type.
#[derive(Debug, Clone, Default)]
pub struct DartFirstClassMap {
    pub field_types: HashMap<String, HashMap<String, String>>,
    pub root_type: Option<String>,
    /// Per-type readable text accessors. Used by the dart e2e `contains`
    /// assertion to aggregate every stringy field on a `Vec<T>` element type.
    pub stringy_fields_by_type: HashMap<String, Vec<StringyField>>,
}

impl DartFirstClassMap {
    /// Returns the IR `Named` type that `field_name` traverses into for the
    /// next chain segment, or `None` if the field is terminal/scalar/unknown.
    pub fn advance(&self, owner_type: Option<&str>, field_name: &str) -> Option<String> {
        let owner = owner_type?;
        self.field_types.get(owner).and_then(|m| m.get(field_name).cloned())
    }

    /// Returns the list of stringy accessors recorded for `type_name`, or
    /// `None` if the type has no recorded stringy fields.
    pub fn stringy_fields(&self, type_name: &str) -> Option<&[StringyField]> {
        self.stringy_fields_by_type.get(type_name).map(Vec::as_slice)
    }

    /// True when no per-type information is recorded.
    pub fn is_empty(&self) -> bool {
        self.field_types.is_empty() && self.stringy_fields_by_type.is_empty()
    }
}

impl PhpGetterMap {
    /// Returns true if `(owner_type, field_name)` requires getter-method syntax.
    ///
    /// When `owner_type` is `None` (root type unknown, or chain advanced into an
    /// unmapped type), falls back to the union across all types: any type
    /// declaring `field_name` as non-scalar marks it as needing a getter. This
    /// is the legacy behaviour and is unsafe when field names collide.
    pub fn needs_getter(&self, owner_type: Option<&str>, field_name: &str) -> bool {
        if let Some(t) = owner_type {
            // Only trust the owner-type classification if the type actually declares
            // this field. A misclassified root_type (workspace-global guess that
            // doesn't match the per-fixture call's actual return type) shouldn't
            // shadow the bare-name fallback.
            let owner_has_field = self.all_fields.get(t).is_some_and(|s| s.contains(field_name));
            if owner_has_field {
                // The owner declares this field — the per-type `getters` map is
                // the authoritative answer. Returning early here prevents the
                // global bare-name union (below) from flipping a scalar field
                // (e.g. `ExtractionResult.content: String`) into a getter call
                // just because some unrelated type declares a same-named field
                // as non-scalar (e.g. `Chunk.content: Vec<Span>`).
                return self.getters.get(t).is_some_and(|fields| fields.contains(field_name));
            }
        }
        self.getters.values().any(|set| set.contains(field_name))
    }

    /// Returns the IR `Named` type that `field_name` traverses into for the
    /// next chain segment, or `None` if the field is terminal/scalar/unknown.
    pub fn advance(&self, owner_type: Option<&str>, field_name: &str) -> Option<String> {
        let owner = owner_type?;
        self.field_types.get(owner).and_then(|m| m.get(field_name).cloned())
    }

    /// True when no per-type information is recorded — equivalent to the legacy
    /// "no PHP getter resolution" code path.
    pub fn is_empty(&self) -> bool {
        self.getters.is_empty()
    }
}

/// A parsed segment of a field path.
#[derive(Debug, Clone)]
enum PathSegment {
    /// Struct field access: `foo`
    Field(String),
    /// Array field access with explicit numeric index: `foo[N]`
    ///
    /// The `index` is the integer parsed from the bracket (e.g. `choices[2]` → index 2).
    /// When synthesised by `inject_array_indexing` the index defaults to `0`.
    ArrayField { name: String, index: usize },
    /// Map/dict key access: `foo[key]`
    MapAccess { field: String, key: String },
    /// Length/count of the preceding collection: `.length`
    Length,
}

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
    /// non-scalar fields (Named structs, Vec<Named>, Map, etc.) while `#[php(prop)]` is
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
    /// assertions on Vec<T> fields).
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
    /// prefix pattern is common in sample-crawler-style fixtures where authors group
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
            "swift" if !self.swift_first_class_map.is_empty() => render_swift_with_first_class_map(
                &segments,
                result_var,
                &self.optional_fields,
                &self.swift_first_class_map,
            ),
            "swift" => render_swift_with_optionals(&segments, result_var, &self.optional_fields),
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
        let accessor = render_accessor(&segments, "rust", result_var);
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

/// Strip all numeric indices from a path so `"choices[0].message.tool_calls"` →
/// `"choices.message.tool_calls"`. Used by `is_optional` to match entries like
/// `"choices.message.tool_calls"` in `optional_fields` when the caller supplies a
/// path that includes a concrete index.
fn strip_numeric_indices(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '[' {
            let mut key = String::new();
            let mut closed = false;
            for inner in chars.by_ref() {
                if inner == ']' {
                    closed = true;
                    break;
                }
                key.push(inner);
            }
            if closed && !key.is_empty() && key.chars().all(|k| k.is_ascii_digit()) {
                // Numeric index — drop it entirely (including any trailing dot).
            } else {
                result.push('[');
                result.push_str(&key);
                if closed {
                    result.push(']');
                }
            }
        } else {
            result.push(c);
        }
    }
    // Collapse any double-dots introduced by dropping `[N].` sequences.
    while result.contains("..") {
        result = result.replace("..", ".");
    }
    if result.starts_with('.') {
        result.remove(0);
    }
    result
}

fn normalize_numeric_indices(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '[' {
            let mut key = String::new();
            let mut closed = false;
            for inner in chars.by_ref() {
                if inner == ']' {
                    closed = true;
                    break;
                }
                key.push(inner);
            }
            if closed && !key.is_empty() && key.chars().all(|k| k.is_ascii_digit()) {
                result.push_str("[0]");
            } else {
                result.push('[');
                result.push_str(&key);
                if closed {
                    result.push(']');
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn parse_path(path: &str) -> Vec<PathSegment> {
    let mut segments = Vec::new();
    for part in path.split('.') {
        if part == "length" || part == "count" || part == "size" {
            segments.push(PathSegment::Length);
        } else if let Some(bracket_pos) = part.find('[') {
            let name = part[..bracket_pos].to_string();
            let key = part[bracket_pos + 1..].trim_end_matches(']').to_string();
            if key.is_empty() {
                // `foo[]` — bare array bracket, index defaults to 0 (upgraded by inject_array_indexing).
                segments.push(PathSegment::ArrayField { name, index: 0 });
            } else if !key.is_empty() && key.chars().all(|c| c.is_ascii_digit()) {
                // `foo[N]` — user-typed explicit numeric index.
                let index: usize = key.parse().unwrap_or(0);
                segments.push(PathSegment::ArrayField { name, index });
            } else {
                // `foo[key]` — string-keyed map access.
                segments.push(PathSegment::MapAccess { field: name, key });
            }
        } else {
            segments.push(PathSegment::Field(part.to_string()));
        }
    }
    segments
}

fn render_accessor(segments: &[PathSegment], language: &str, result_var: &str) -> String {
    match language {
        "rust" => render_rust(segments, result_var),
        "python" => render_dot_access(segments, result_var, "python"),
        "typescript" | "node" => render_typescript(segments, result_var),
        "wasm" => render_wasm(segments, result_var),
        "go" => render_go(segments, result_var),
        "java" => render_java(segments, result_var),
        "kotlin" => render_kotlin(segments, result_var),
        "kotlin_android" => render_kotlin_android(segments, result_var),
        "csharp" => render_pascal_dot(segments, result_var),
        "ruby" => render_dot_access(segments, result_var, "ruby"),
        "php" => render_php(segments, result_var),
        "elixir" => render_dot_access(segments, result_var, "elixir"),
        "r" => render_r(segments, result_var),
        "c" => render_c(segments, result_var),
        "swift" => render_swift(segments, result_var),
        "dart" => render_dart(segments, result_var),
        _ => render_dot_access(segments, result_var, language),
    }
}

/// Generate a Swift accessor expression.
///
/// Alef now emits first-class Swift structs (`public struct Foo: Codable { public let
/// id: String }`) for most DTO types, where fields are properties — property access
/// uses `.id` (no parens). The remaining typealias-to-opaque types (e.g. request
/// types with Vec/Map/Named fields that aren't first-class candidates) are accessed
/// via the swift-bridge-generated method-call syntax `.id()`, but in e2e tests these
/// typealias types are method inputs / streaming outputs rather than parents for
/// field-access chains, so property syntax works in practice. If a future e2e test
/// asserts on a field-access chain rooted in an opaque type, a per-type
/// `SwiftFirstClassMap` (analogous to `PhpGetterMap`) would be needed.
fn render_swift(segments: &[PathSegment], result_var: &str) -> String {
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
                    out.push_str(&format!("[\"{key}\"]"));
                }
            }
            PathSegment::Length => {
                out.push_str(".count");
            }
        }
    }
    out
}

/// Generate a Swift accessor expression with optional chaining.
///
/// When an intermediate field is in `optional_fields`, a `?` is inserted after the
/// `()` call on that segment so the next access uses `?.`. This prevents compile
/// errors when accessing members through an `Optional<T>` in Swift.
///
/// Example: for `metadata.format.excel.sheet_count` where `metadata.format` and
/// `metadata.format.excel` are optional, the result is:
/// `result.metadata().format()?.excel()?.sheet_count()`
fn render_swift_with_optionals(
    segments: &[PathSegment],
    result_var: &str,
    optional_fields: &HashSet<String>,
) -> String {
    let mut out = result_var.to_string();
    let mut path_so_far = String::new();
    let total = segments.len();
    for (i, seg) in segments.iter().enumerate() {
        let is_leaf = i == total - 1;
        match seg {
            PathSegment::Field(f) => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(f);
                out.push('.');
                // Swift bindings always use lowerCamelCase for both first-class
                // `public let` properties and swift-bridge accessor methods.
                out.push_str(&f.to_lower_camel_case());
                // First-class Swift struct fields are properties (no parens).
                // Insert `?` after the property name for non-leaf optional fields so the
                // next member access becomes `?.`.
                if !is_leaf && optional_fields.contains(&path_so_far) {
                    out.push('?');
                }
            }
            PathSegment::ArrayField { name, index } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(name);
                let is_optional = optional_fields.contains(&path_so_far);
                out.push('.');
                out.push_str(&name.to_lower_camel_case());
                if is_optional {
                    // Optional<[T]>: unwrap before indexing.
                    out.push_str(&format!("?[{index}]"));
                } else {
                    out.push_str(&format!("[{index}]"));
                }
                path_so_far.push_str("[0]");
                let _ = is_leaf;
            }
            PathSegment::MapAccess { field, key } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(field);
                out.push('.');
                out.push_str(&field.to_lower_camel_case());
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("[{key}]"));
                } else {
                    out.push_str(&format!("[\"{key}\"]"));
                }
            }
            PathSegment::Length => {
                out.push_str(".count");
            }
        }
    }
    out
}

/// Like `render_swift_with_optionals` but dispatches per-segment between
/// property access (first-class Codable struct) and method-call access
/// (typealias-to-opaque RustBridge class). Uses the `SwiftFirstClassMap` to
/// track the current type as the path advances.
fn render_swift_with_first_class_map(
    segments: &[PathSegment],
    result_var: &str,
    optional_fields: &HashSet<String>,
    map: &SwiftFirstClassMap,
) -> String {
    let mut out = result_var.to_string();
    let mut path_so_far = String::new();
    let mut current_type: Option<String> = map.root_type.clone();
    // Once a chain crosses an `ArrayField` segment, every subsequent segment
    // operates on an element pulled from a `RustVec<T>` — and `RustVec[i]`
    // yields the OPAQUE `RustBridge.T` (whose fields are swift-bridge methods),
    // never the first-class Codable Swift struct `T`. swift-bridge generates
    // `RustVec` as a thin wrapper around the Rust vector, not as a converter
    // to the binding's first-class struct. Pin opaque (method-call) syntax
    // after the first index step so paths like `data[0].id` emit `.id()` even
    // when the Codable `Model` first-class struct also exists.
    let mut via_rust_vec = false;
    // Once a chain crosses an opaque (typealias-to-`RustBridge.X`) segment,
    // every subsequent accessor must also be opaque (method-call syntax). The
    // backend emits `public typealias X = RustBridge.X` when `X` fails the
    // `can_emit_first_class_struct` check (e.g. contains a non-unit enum, or a
    // field of a still-opaque type). Calling a method on `RustBridge.X` returns
    // the OPAQUE wrapper of the next type, never the first-class Codable
    // struct — even when that next type IS independently eligible for
    // first-class emission. Pin method-call syntax after the first opaque step
    // so paths like `metrics.total_lines` on opaque `ProcessResult` emit
    // `.metrics().totalLines()` (not `.metrics().totalLines`).
    let mut via_opaque = false;
    let total = segments.len();
    for (i, seg) in segments.iter().enumerate() {
        let is_leaf = i == total - 1;
        let property_syntax = !via_rust_vec && !via_opaque && map.is_first_class(current_type.as_deref());
        if !property_syntax {
            via_opaque = true;
        }
        match seg {
            PathSegment::Field(f) => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(f);
                out.push('.');
                // Swift bindings (both first-class `public let` props and
                // swift-bridge method names) always use lowerCamelCase.
                out.push_str(&f.to_lower_camel_case());
                if !property_syntax {
                    out.push_str("()");
                }
                if !is_leaf && optional_fields.contains(&path_so_far) {
                    out.push('?');
                }
                current_type = map.advance(current_type.as_deref(), f);
            }
            PathSegment::ArrayField { name, index } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(name);
                let is_optional = optional_fields.contains(&path_so_far);
                out.push('.');
                out.push_str(&name.to_lower_camel_case());
                let access = if property_syntax { "" } else { "()" };
                if is_optional {
                    out.push_str(&format!("{access}?[{index}]"));
                } else {
                    out.push_str(&format!("{access}[{index}]"));
                }
                path_so_far.push_str("[0]");
                // Indexing into a Vec<Named> yields a Named element — advance current_type.
                // Only pin opaque syntax when the array field was itself emitted in
                // method-call mode (i.e. it's a RustVec accessor). When the owning
                // type is first-class, the array IS a Swift `[T]` and indexing yields
                // the first-class `T` directly (also a Codable struct → property access).
                current_type = map.advance(current_type.as_deref(), name);
                if !property_syntax {
                    via_rust_vec = true;
                }
            }
            PathSegment::MapAccess { field, key } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(field);
                out.push('.');
                out.push_str(&field.to_lower_camel_case());
                let access = if property_syntax { "" } else { "()" };
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("{access}[{key}]"));
                } else {
                    out.push_str(&format!("{access}[\"{key}\"]"));
                }
                current_type = map.advance(current_type.as_deref(), field);
            }
            PathSegment::Length => {
                out.push_str(".count");
            }
        }
    }
    out
}

fn render_rust(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&f.to_snake_case());
            }
            PathSegment::ArrayField { name, index } => {
                out.push('.');
                out.push_str(&name.to_snake_case());
                out.push_str(&format!("[{index}]"));
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&field.to_snake_case());
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("[{key}]"));
                } else {
                    out.push_str(&format!(".get(\"{key}\").map(|s| s.as_str())"));
                }
            }
            PathSegment::Length => {
                out.push_str(".len()");
            }
        }
    }
    out
}

fn render_dot_access(segments: &[PathSegment], result_var: &str, language: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(f);
            }
            PathSegment::ArrayField { name, index } => {
                if language == "elixir" {
                    let current = std::mem::take(&mut out);
                    out = format!("Enum.at({current}.{name}, {index})");
                } else {
                    out.push('.');
                    out.push_str(name);
                    out.push_str(&format!("[{index}]"));
                }
            }
            PathSegment::MapAccess { field, key } => {
                let is_numeric = key.chars().all(|c| c.is_ascii_digit());
                if is_numeric && language == "elixir" {
                    let current = std::mem::take(&mut out);
                    out = format!("Enum.at({current}.{field}, {key})");
                } else {
                    out.push('.');
                    out.push_str(field);
                    if is_numeric {
                        let idx: usize = key.parse().unwrap_or(0);
                        out.push_str(&format!("[{idx}]"));
                    } else if language == "elixir" || language == "ruby" {
                        // Ruby/Elixir hashes use `["key"]` bracket access (Ruby's Hash has
                        // no `get` method; Elixir maps use bracket access too).
                        out.push_str(&format!("[\"{key}\"]"));
                    } else {
                        out.push_str(&format!(".get(\"{key}\")"));
                    }
                }
            }
            PathSegment::Length => match language {
                "ruby" => out.push_str(".length"),
                "elixir" => {
                    let current = std::mem::take(&mut out);
                    out = format!("length({current})");
                }
                "gleam" => {
                    let current = std::mem::take(&mut out);
                    out = format!("list.length({current})");
                }
                _ => {
                    let current = std::mem::take(&mut out);
                    out = format!("len({current})");
                }
            },
        }
    }
    out
}

fn render_typescript(segments: &[PathSegment], result_var: &str) -> String {
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
                // Numeric (digit-only) keys index into arrays as integers, not as
                // string-keyed object properties; emit `[0]` not `["0"]`.
                if !key.is_empty() && key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("[{key}]"));
                } else {
                    out.push_str(&format!("[\"{key}\"]"));
                }
            }
            PathSegment::Length => {
                out.push_str(".length");
            }
        }
    }
    out
}

fn render_wasm(segments: &[PathSegment], result_var: &str) -> String {
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
                out.push_str(&format!(".get(\"{key}\")"));
            }
            PathSegment::Length => {
                out.push_str(".length");
            }
        }
    }
    out
}

fn render_go(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&to_go_name(f));
            }
            PathSegment::ArrayField { name, index } => {
                out.push('.');
                out.push_str(&to_go_name(name));
                out.push_str(&format!("[{index}]"));
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&to_go_name(field));
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("[{key}]"));
                } else {
                    out.push_str(&format!("[\"{key}\"]"));
                }
            }
            PathSegment::Length => {
                let current = std::mem::take(&mut out);
                out = format!("len({current})");
            }
        }
    }
    out
}

fn render_java(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&f.to_lower_camel_case());
                out.push_str("()");
            }
            PathSegment::ArrayField { name, index } => {
                out.push('.');
                out.push_str(&name.to_lower_camel_case());
                out.push_str(&format!("().get({index})"));
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&field.to_lower_camel_case());
                // Numeric keys index into List<T> (.get(int)); string keys index into Map<String, V>.
                let is_numeric = !key.is_empty() && key.chars().all(|c| c.is_ascii_digit());
                if is_numeric {
                    out.push_str(&format!("().get({key})"));
                } else {
                    out.push_str(&format!("().get(\"{key}\")"));
                }
            }
            PathSegment::Length => {
                out.push_str(".size()");
            }
        }
    }
    out
}

/// Wrap a Kotlin getter name in backticks when it collides with a Kotlin hard keyword.
///
/// Hard keywords cannot be used as identifiers without escaping, so `result.object()`
/// is a syntax error; `` result.`object`() `` is the legal form.
fn kotlin_getter(name: &str) -> String {
    let camel = name.to_lower_camel_case();
    match camel.as_str() {
        "as" | "break" | "class" | "continue" | "do" | "else" | "false" | "for" | "fun" | "if" | "in" | "interface"
        | "is" | "null" | "object" | "package" | "return" | "super" | "this" | "throw" | "true" | "try"
        | "typealias" | "typeof" | "val" | "var" | "when" | "while" => format!("`{camel}`"),
        _ => camel,
    }
}

fn render_kotlin(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&kotlin_getter(f));
                out.push_str("()");
            }
            PathSegment::ArrayField { name, index } => {
                out.push('.');
                out.push_str(&kotlin_getter(name));
                if *index == 0 {
                    out.push_str("().first()");
                } else {
                    out.push_str(&format!("().get({index})"));
                }
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&kotlin_getter(field));
                let is_numeric = !key.is_empty() && key.chars().all(|c| c.is_ascii_digit());
                if is_numeric {
                    out.push_str(&format!("().get({key})"));
                } else {
                    out.push_str(&format!("().get(\"{key}\")"));
                }
            }
            PathSegment::Length => {
                out.push_str(".size");
            }
        }
    }
    out
}

fn render_java_with_optionals(segments: &[PathSegment], result_var: &str, optional_fields: &HashSet<String>) -> String {
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
                    out.push_str(&format!("().get(\"{key}\")"));
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
fn render_kotlin_with_optionals(
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
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(f);
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
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(name);
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
                path_so_far.push_str("[0]");
                prev_was_nullable = prev_was_nullable || is_optional;
            }
            PathSegment::MapAccess { field, key } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(field);
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
                    out.push_str(&format!("()?.get(\"{key}\")"));
                } else {
                    out.push_str(&format!("().get(\"{key}\")"));
                }
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
fn render_kotlin_android_with_optionals(
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
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(f);
                let is_optional = optional_fields.contains(&path_so_far);
                out.push_str(nav);
                // Property access — no () suffix.
                out.push_str(&kotlin_getter(f));
                prev_was_nullable = prev_was_nullable || is_optional;
            }
            PathSegment::ArrayField { name, index } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(name);
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
                path_so_far.push_str("[0]");
                prev_was_nullable = prev_was_nullable || is_optional;
            }
            PathSegment::MapAccess { field, key } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(field);
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
                    out.push_str(&format!("?.get(\"{key}\")"));
                } else {
                    out.push_str(&format!(".get(\"{key}\")"));
                }
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
fn render_kotlin_android(segments: &[PathSegment], result_var: &str) -> String {
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
                    out.push_str(&format!(".get(\"{key}\")"));
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
fn render_rust_with_optionals(
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
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(f);
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
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(name);
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
                path_so_far.push_str("[0]");
            }
            PathSegment::MapAccess { field, key } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(field);
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
                    path_so_far.push_str("[0]");
                } else {
                    out.push_str(&format!(".get(\"{key}\").map(|s| s.as_str())"));
                }
            }
            PathSegment::Length => {
                out.push_str(".len()");
            }
        }
    }
    out
}

/// Zig accessor that unwraps optional fields with `.?`.
///
/// Zig does not allow field access, indexing, or comparisons through `?T`;
/// the value must be unwrapped first. Each segment whose path appears in the
/// optional-field set is followed by `.?` so the resulting expression is a
/// concrete value usable in assertions.
///
/// Paths in `method_calls` represent tagged-union variant accessors (Rust
/// variant getters such as `FormatMetadata::excel()`). In Zig, tagged-union
/// variants are accessed via the same dot syntax as struct fields, so the
/// segment is emitted as `.{name}` *without* `.?` even if the path also
/// appears in `optional_fields`.
fn render_zig_with_optionals(
    segments: &[PathSegment],
    result_var: &str,
    optional_fields: &HashSet<String>,
    method_calls: &HashSet<String>,
) -> String {
    let mut out = result_var.to_string();
    let mut path_so_far = String::new();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(f);
                out.push('.');
                out.push_str(f);
                if !method_calls.contains(&path_so_far) && optional_fields.contains(&path_so_far) {
                    out.push_str(".?");
                }
            }
            PathSegment::ArrayField { name, index } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(name);
                out.push('.');
                out.push_str(name);
                if !method_calls.contains(&path_so_far) && optional_fields.contains(&path_so_far) {
                    out.push_str(".?");
                }
                out.push_str(&format!("[{index}]"));
            }
            PathSegment::MapAccess { field, key } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(field);
                out.push('.');
                out.push_str(field);
                if !method_calls.contains(&path_so_far) && optional_fields.contains(&path_so_far) {
                    out.push_str(".?");
                }
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("[{key}]"));
                } else {
                    out.push_str(&format!(".get(\"{key}\")"));
                }
            }
            PathSegment::Length => {
                out.push_str(".len");
            }
        }
    }
    out
}

fn render_pascal_dot(segments: &[PathSegment], result_var: &str) -> String {
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
                    out.push_str(&format!("[\"{key}\"]"));
                }
            }
            PathSegment::Length => {
                out.push_str(".Count");
            }
        }
    }
    out
}

fn render_csharp_with_optionals(
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
                out.push_str(&f.to_pascal_case());
                if !is_leaf && optional_fields.contains(&path_so_far) {
                    out.push('!');
                }
            }
            PathSegment::ArrayField { name, index } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(name);
                out.push('.');
                out.push_str(&name.to_pascal_case());
                out.push_str(&format!("[{index}]"));
            }
            PathSegment::MapAccess { field, key } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(field);
                out.push('.');
                out.push_str(&field.to_pascal_case());
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("[{key}]"));
                } else {
                    out.push_str(&format!("[\"{key}\"]"));
                }
            }
            PathSegment::Length => {
                out.push_str(".Count");
            }
        }
    }
    out
}

fn render_php(segments: &[PathSegment], result_var: &str) -> String {
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
                out.push_str(&format!("[\"{key}\"]"));
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
/// method because `get_method_props` is `todo!()` in ext-php-rs-derive 0.11.7.
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
fn render_php_with_getters(segments: &[PathSegment], result_var: &str, getter_map: &PhpGetterMap) -> String {
    let mut out = result_var.to_string();
    let mut current_type: Option<String> = getter_map.root_type.clone();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                let camel = f.to_lower_camel_case();
                if getter_map.needs_getter(current_type.as_deref(), f.as_str()) {
                    // Non-scalar field: ext-php-rs emits a `get{CamelCase}()` method.
                    // The `get_` prefix is stripped by ext-php-rs when it derives the
                    // PHP property name, but the Rust method ident is `get_{camelCase}`,
                    // so the PHP call is `->get{CamelCase}()`.
                    let getter = format!("get{}", camel.as_str()[..1].to_uppercase() + &camel[1..]);
                    out.push_str("->");
                    out.push_str(&getter);
                    out.push_str("()");
                } else {
                    out.push_str("->");
                    out.push_str(&camel);
                }
                current_type = getter_map.advance(current_type.as_deref(), f.as_str());
            }
            PathSegment::ArrayField { name, index } => {
                let camel = name.to_lower_camel_case();
                if getter_map.needs_getter(current_type.as_deref(), name.as_str()) {
                    let getter = format!("get{}", camel.as_str()[..1].to_uppercase() + &camel[1..]);
                    out.push_str("->");
                    out.push_str(&getter);
                    out.push_str("()");
                } else {
                    out.push_str("->");
                    out.push_str(&camel);
                }
                out.push_str(&format!("[{index}]"));
                current_type = getter_map.advance(current_type.as_deref(), name.as_str());
            }
            PathSegment::MapAccess { field, key } => {
                let camel = field.to_lower_camel_case();
                if getter_map.needs_getter(current_type.as_deref(), field.as_str()) {
                    let getter = format!("get{}", camel.as_str()[..1].to_uppercase() + &camel[1..]);
                    out.push_str("->");
                    out.push_str(&getter);
                    out.push_str("()");
                } else {
                    out.push_str("->");
                    out.push_str(&camel);
                }
                out.push_str(&format!("[\"{key}\"]"));
                current_type = getter_map.advance(current_type.as_deref(), field.as_str());
            }
            PathSegment::Length => {
                let current = std::mem::take(&mut out);
                out = format!("count({current})");
            }
        }
    }
    out
}

fn render_r(segments: &[PathSegment], result_var: &str) -> String {
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
                out.push_str(&format!("[[\"{key}\"]]"));
            }
            PathSegment::Length => {
                let current = std::mem::take(&mut out);
                out = format!("length({current})");
            }
        }
    }
    out
}

fn render_c(segments: &[PathSegment], result_var: &str) -> String {
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
                out = format!("result_{snake}({current})[\"{key}\"]");
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
fn render_dart(segments: &[PathSegment], result_var: &str) -> String {
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
                    out.push_str(&format!("[\"{key}\"]"));
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
fn render_dart_with_optionals(segments: &[PathSegment], result_var: &str, optional_fields: &HashSet<String>) -> String {
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
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                    path_with_indices.push('.');
                }
                path_so_far.push_str(f);
                path_with_indices.push_str(f);
                let optional = is_optional(&path_so_far, &path_with_indices);
                out.push_str(nav);
                out.push_str(&f.to_lower_camel_case());
                prev_was_nullable = optional;
            }
            PathSegment::ArrayField { name, index } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                    path_with_indices.push('.');
                }
                path_so_far.push_str(name);
                path_with_indices.push_str(name);
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
                path_with_indices.push_str(&format!("[{index}]"));
                prev_was_nullable = false;
            }
            PathSegment::MapAccess { field, key } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                    path_with_indices.push('.');
                }
                path_so_far.push_str(field);
                path_with_indices.push_str(field);
                let optional = is_optional(&path_so_far, &path_with_indices);
                out.push_str(nav);
                out.push_str(&field.to_lower_camel_case());
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("[{key}]"));
                    path_with_indices.push_str(&format!("[{key}]"));
                } else {
                    out.push_str(&format!("[\"{key}\"]"));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_resolver() -> FieldResolver {
        let mut fields = HashMap::new();
        fields.insert("title".to_string(), "metadata.document.title".to_string());
        fields.insert("tags".to_string(), "metadata.tags[name]".to_string());
        fields.insert("og".to_string(), "metadata.document.open_graph".to_string());
        fields.insert("twitter".to_string(), "metadata.document.twitter_card".to_string());
        fields.insert("canonical".to_string(), "metadata.document.canonical_url".to_string());
        fields.insert("og_tag".to_string(), "metadata.open_graph_tags[og_title]".to_string());
        let mut optional = HashSet::new();
        optional.insert("metadata.document.title".to_string());
        FieldResolver::new(&fields, &optional, &HashSet::new(), &HashSet::new(), &HashSet::new())
    }

    fn make_resolver_with_doc_optional() -> FieldResolver {
        let mut fields = HashMap::new();
        fields.insert("title".to_string(), "metadata.document.title".to_string());
        fields.insert("tags".to_string(), "metadata.tags[name]".to_string());
        let mut optional = HashSet::new();
        optional.insert("document".to_string());
        optional.insert("metadata.document.title".to_string());
        optional.insert("metadata.document".to_string());
        FieldResolver::new(&fields, &optional, &HashSet::new(), &HashSet::new(), &HashSet::new())
    }

    #[test]
    fn test_resolve_alias() {
        let r = make_resolver();
        assert_eq!(r.resolve("title"), "metadata.document.title");
    }

    #[test]
    fn test_resolve_passthrough() {
        let r = make_resolver();
        assert_eq!(r.resolve("content"), "content");
    }

    #[test]
    fn test_is_optional() {
        let r = make_resolver();
        assert!(r.is_optional("metadata.document.title"));
        assert!(!r.is_optional("content"));
    }

    #[test]
    fn is_optional_strips_namespace_prefix() {
        let fields = HashMap::new();
        let mut optional = HashSet::new();
        optional.insert("action_results.data".to_string());
        let result_fields: HashSet<String> = ["action_results".to_string()].into_iter().collect();
        let r = FieldResolver::new(&fields, &optional, &result_fields, &HashSet::new(), &HashSet::new());
        // `interaction.` is a virtual namespace prefix — strip and re-check.
        assert!(r.is_optional("interaction.action_results[0].data"));
        // Still finds it without the prefix.
        assert!(r.is_optional("action_results[0].data"));
    }

    #[test]
    fn test_accessor_rust_struct() {
        let r = make_resolver();
        assert_eq!(r.accessor("title", "rust", "result"), "result.metadata.document.title");
    }

    #[test]
    fn test_accessor_rust_map() {
        let r = make_resolver();
        assert_eq!(
            r.accessor("tags", "rust", "result"),
            "result.metadata.tags.get(\"name\").map(|s| s.as_str())"
        );
    }

    #[test]
    fn test_accessor_python() {
        let r = make_resolver();
        assert_eq!(
            r.accessor("title", "python", "result"),
            "result.metadata.document.title"
        );
    }

    #[test]
    fn test_accessor_go() {
        let r = make_resolver();
        assert_eq!(r.accessor("title", "go", "result"), "result.Metadata.Document.Title");
    }

    #[test]
    fn test_accessor_go_initialism_fields() {
        let mut fields = std::collections::HashMap::new();
        fields.insert("content".to_string(), "html".to_string());
        fields.insert("link_url".to_string(), "links.url".to_string());
        let r = FieldResolver::new(
            &fields,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert_eq!(r.accessor("content", "go", "result"), "result.HTML");
        assert_eq!(r.accessor("link_url", "go", "result"), "result.Links.URL");
        assert_eq!(r.accessor("html", "go", "result"), "result.HTML");
        assert_eq!(r.accessor("url", "go", "result"), "result.URL");
        assert_eq!(r.accessor("id", "go", "result"), "result.ID");
        assert_eq!(r.accessor("user_id", "go", "result"), "result.UserID");
        assert_eq!(r.accessor("request_url", "go", "result"), "result.RequestURL");
        assert_eq!(r.accessor("links", "go", "result"), "result.Links");
    }

    #[test]
    fn test_accessor_typescript() {
        let r = make_resolver();
        assert_eq!(
            r.accessor("title", "typescript", "result"),
            "result.metadata.document.title"
        );
    }

    #[test]
    fn test_accessor_typescript_snake_to_camel() {
        let r = make_resolver();
        assert_eq!(
            r.accessor("og", "typescript", "result"),
            "result.metadata.document.openGraph"
        );
        assert_eq!(
            r.accessor("twitter", "typescript", "result"),
            "result.metadata.document.twitterCard"
        );
        assert_eq!(
            r.accessor("canonical", "typescript", "result"),
            "result.metadata.document.canonicalUrl"
        );
    }

    #[test]
    fn test_accessor_typescript_map_snake_to_camel() {
        let r = make_resolver();
        assert_eq!(
            r.accessor("og_tag", "typescript", "result"),
            "result.metadata.openGraphTags[\"og_title\"]"
        );
    }

    #[test]
    fn test_accessor_typescript_numeric_index_is_unquoted() {
        // Digit-only map-access keys (e.g. JSON pointer segments like `results.0`)
        // must emit numeric bracket access (`[0]`) not string-keyed access
        // (`["0"]`), which would return undefined on arrays.
        let mut fields = HashMap::new();
        fields.insert("first_score".to_string(), "results[0].relevance_score".to_string());
        let r = FieldResolver::new(
            &fields,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert_eq!(
            r.accessor("first_score", "typescript", "result"),
            "result.results[0].relevanceScore"
        );
    }

    #[test]
    fn test_accessor_node_alias() {
        let r = make_resolver();
        assert_eq!(r.accessor("og", "node", "result"), "result.metadata.document.openGraph");
    }

    #[test]
    fn test_accessor_wasm_camel_case() {
        let r = make_resolver();
        assert_eq!(r.accessor("og", "wasm", "result"), "result.metadata.document.openGraph");
        assert_eq!(
            r.accessor("twitter", "wasm", "result"),
            "result.metadata.document.twitterCard"
        );
        assert_eq!(
            r.accessor("canonical", "wasm", "result"),
            "result.metadata.document.canonicalUrl"
        );
    }

    #[test]
    fn test_accessor_wasm_map_access() {
        let r = make_resolver();
        assert_eq!(
            r.accessor("og_tag", "wasm", "result"),
            "result.metadata.openGraphTags.get(\"og_title\")"
        );
    }

    #[test]
    fn test_accessor_java() {
        let r = make_resolver();
        assert_eq!(
            r.accessor("title", "java", "result"),
            "result.metadata().document().title()"
        );
    }

    #[test]
    fn test_accessor_kotlin_uses_kotlin_collection_idioms() {
        let mut fields = HashMap::new();
        fields.insert("first_node_name".to_string(), "nodes[0].name".to_string());
        fields.insert("node_count".to_string(), "nodes.length".to_string());
        let mut arrays = HashSet::new();
        arrays.insert("nodes".to_string());
        let r = FieldResolver::new(&fields, &HashSet::new(), &HashSet::new(), &arrays, &HashSet::new());
        assert_eq!(
            r.accessor("first_node_name", "kotlin", "result"),
            "result.nodes().first().name()"
        );
        assert_eq!(r.accessor("node_count", "kotlin", "result"), "result.nodes().size");
    }

    #[test]
    fn test_accessor_kotlin_uses_safe_calls_for_optional_prefixes() {
        let r = make_resolver_with_doc_optional();
        assert_eq!(
            r.accessor("title", "kotlin", "result"),
            "result.metadata().document()?.title()"
        );
    }

    #[test]
    fn test_accessor_kotlin_uses_safe_calls_for_optional_arrays_and_maps() {
        let mut fields = HashMap::new();
        fields.insert("first_node_name".to_string(), "nodes[0].name".to_string());
        fields.insert("tag".to_string(), "tags[name]".to_string());
        let mut optional = HashSet::new();
        optional.insert("nodes".to_string());
        optional.insert("tags".to_string());
        let mut arrays = HashSet::new();
        arrays.insert("nodes".to_string());
        let r = FieldResolver::new(&fields, &optional, &HashSet::new(), &arrays, &HashSet::new());
        assert_eq!(
            r.accessor("first_node_name", "kotlin", "result"),
            "result.nodes()?.first()?.name()"
        );
        assert_eq!(r.accessor("tag", "kotlin", "result"), "result.tags()?.get(\"name\")");
    }

    /// Regression: optional-field keys with explicit `[0]` indices (e.g.
    /// `"choices[0].message.tool_calls"`) were not matched by
    /// `render_kotlin_with_optionals` because `path_so_far` omitted the `[0]`
    /// suffix after traversing an ArrayField segment. Fix: append `"[0]"` to
    /// `path_so_far` after each ArrayField, mirroring the Rust renderer.
    #[test]
    fn test_accessor_kotlin_optional_field_after_indexed_array() {
        // "choices[0].message.tool_calls" is optional; the path is accessed as
        // choices[0].message.tool_calls[0].function.name.
        let mut fields = HashMap::new();
        fields.insert(
            "tool_call_name".to_string(),
            "choices[0].message.tool_calls[0].function.name".to_string(),
        );
        let mut optional = HashSet::new();
        optional.insert("choices[0].message.tool_calls".to_string());
        let mut arrays = HashSet::new();
        arrays.insert("choices".to_string());
        arrays.insert("choices[0].message.tool_calls".to_string());
        let r = FieldResolver::new(&fields, &optional, &HashSet::new(), &arrays, &HashSet::new());
        let expr = r.accessor("tool_call_name", "kotlin", "result");
        // toolCalls() is optional so it must use `?.` before `.first()`.
        assert!(
            expr.contains("toolCalls()?.first()"),
            "expected toolCalls()?.first() for optional list, got: {expr}"
        );
    }

    #[test]
    fn test_accessor_csharp() {
        let r = make_resolver();
        assert_eq!(
            r.accessor("title", "csharp", "result"),
            "result.Metadata.Document.Title"
        );
    }

    #[test]
    fn test_accessor_php() {
        let r = make_resolver();
        assert_eq!(
            r.accessor("title", "php", "$result"),
            "$result->metadata->document->title"
        );
    }

    #[test]
    fn test_accessor_r() {
        let r = make_resolver();
        assert_eq!(r.accessor("title", "r", "result"), "result$metadata$document$title");
    }

    #[test]
    fn test_accessor_c() {
        let r = make_resolver();
        assert_eq!(
            r.accessor("title", "c", "result"),
            "result_title(result_document(result_metadata(result)))"
        );
    }

    #[test]
    fn test_rust_unwrap_binding() {
        let r = make_resolver();
        let (binding, var) = r.rust_unwrap_binding("title", "result").unwrap();
        // Binding is prefixed with `_` to suppress `-D unused_variables` when no
        // assertion references it; the variable remains accessible under that name.
        assert_eq!(var, "_metadata_document_title");
        assert!(binding.starts_with("let _metadata_document_title ="));
        // Optional scalar fields are unwrapped via Display (`to_string()`) so enum
        // types like `FinishReason` render their serde-style string form.
        assert!(binding.contains("as_ref().map(|v| v.to_string()).unwrap_or_default()"));
    }

    #[test]
    fn test_rust_unwrap_binding_non_optional() {
        let r = make_resolver();
        assert!(r.rust_unwrap_binding("content", "result").is_none());
    }

    #[test]
    fn test_rust_unwrap_binding_collapses_double_underscore() {
        // When an alias resolves to a path with `[]` (e.g. `json_ld.name` →
        // `json_ld[].name`), the naive replace previously yielded `json_ld__name`,
        // which trips Rust's non_snake_case lint under -D warnings. The local
        // binding name must collapse consecutive underscores into one.
        let mut aliases = HashMap::new();
        aliases.insert("json_ld.name".to_string(), "json_ld[].name".to_string());
        let mut optional = HashSet::new();
        optional.insert("json_ld[].name".to_string());
        let mut array = HashSet::new();
        array.insert("json_ld".to_string());
        let result_fields = HashSet::new();
        let method_calls = HashSet::new();
        let r = FieldResolver::new(&aliases, &optional, &result_fields, &array, &method_calls);
        let (_binding, var) = r.rust_unwrap_binding("json_ld.name", "result").unwrap();
        assert_eq!(var, "_json_ld_name");
    }

    #[test]
    fn test_direct_field_no_alias() {
        let r = make_resolver();
        assert_eq!(r.accessor("content", "rust", "result"), "result.content");
        assert_eq!(r.accessor("content", "go", "result"), "result.Content");
    }

    #[test]
    fn test_accessor_rust_with_optionals() {
        let r = make_resolver_with_doc_optional();
        assert_eq!(
            r.accessor("title", "rust", "result"),
            "result.metadata.document.as_ref().unwrap().title"
        );
    }

    #[test]
    fn test_accessor_csharp_with_optionals() {
        let r = make_resolver_with_doc_optional();
        assert_eq!(
            r.accessor("title", "csharp", "result"),
            "result.Metadata.Document!.Title"
        );
    }

    #[test]
    fn test_accessor_rust_non_optional_field() {
        let r = make_resolver();
        assert_eq!(r.accessor("content", "rust", "result"), "result.content");
    }

    #[test]
    fn test_accessor_csharp_non_optional_field() {
        let r = make_resolver();
        assert_eq!(r.accessor("content", "csharp", "result"), "result.Content");
    }

    #[test]
    fn test_accessor_rust_method_call() {
        // "metadata.format.excel" is in method_calls — should emit `excel()` instead of `excel`
        let mut fields = HashMap::new();
        fields.insert(
            "excel_sheet_count".to_string(),
            "metadata.format.excel.sheet_count".to_string(),
        );
        let mut optional = HashSet::new();
        optional.insert("metadata.format".to_string());
        optional.insert("metadata.format.excel".to_string());
        let mut method_calls = HashSet::new();
        method_calls.insert("metadata.format.excel".to_string());
        let r = FieldResolver::new(&fields, &optional, &HashSet::new(), &HashSet::new(), &method_calls);
        assert_eq!(
            r.accessor("excel_sheet_count", "rust", "result"),
            "result.metadata.format.as_ref().unwrap().excel().as_ref().unwrap().sheet_count"
        );
    }

    // ---------------------------------------------------------------------------
    // PHP getter-method tests (ext-php-rs 0.15.x `#[php(getter)]` vs `#[php(prop)]`)
    // ---------------------------------------------------------------------------

    fn make_php_getter_resolver() -> FieldResolver {
        let mut getters: HashMap<String, HashSet<String>> = HashMap::new();
        getters.insert(
            "Root".to_string(),
            ["metadata".to_string(), "links".to_string()].into_iter().collect(),
        );
        let map = PhpGetterMap {
            getters,
            field_types: HashMap::new(),
            root_type: Some("Root".to_string()),
            all_fields: HashMap::new(),
        };
        FieldResolver::new_with_php_getters(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            map,
        )
    }

    #[test]
    fn render_php_uses_getter_method_for_non_scalar_field() {
        let r = make_php_getter_resolver();
        assert_eq!(r.accessor("metadata", "php", "$result"), "$result->getMetadata()");
    }

    #[test]
    fn render_php_uses_property_for_scalar_field() {
        let r = make_php_getter_resolver();
        assert_eq!(r.accessor("status_code", "php", "$result"), "$result->statusCode");
    }

    #[test]
    fn render_php_nested_non_scalar_uses_getter_then_property() {
        let mut fields = HashMap::new();
        fields.insert("title".to_string(), "metadata.title".to_string());
        let mut getters: HashMap<String, HashSet<String>> = HashMap::new();
        getters.insert("Root".to_string(), ["metadata".to_string()].into_iter().collect());
        // No entry for Metadata.title → scalar by default.
        getters.insert("Metadata".to_string(), HashSet::new());
        let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
        field_types.insert(
            "Root".to_string(),
            [("metadata".to_string(), "Metadata".to_string())].into_iter().collect(),
        );
        let map = PhpGetterMap {
            getters,
            field_types,
            root_type: Some("Root".to_string()),
            all_fields: HashMap::new(),
        };
        let r = FieldResolver::new_with_php_getters(
            &fields,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            map,
        );
        // `metadata` → `->getMetadata()`, then `title` (scalar on returned object) → `->title`
        assert_eq!(r.accessor("title", "php", "$result"), "$result->getMetadata()->title");
    }

    #[test]
    fn render_php_array_field_uses_getter_when_non_scalar() {
        let mut fields = HashMap::new();
        fields.insert("first_link".to_string(), "links[0]".to_string());
        let mut getters: HashMap<String, HashSet<String>> = HashMap::new();
        getters.insert("Root".to_string(), ["links".to_string()].into_iter().collect());
        let map = PhpGetterMap {
            getters,
            field_types: HashMap::new(),
            root_type: Some("Root".to_string()),
            all_fields: HashMap::new(),
        };
        let r = FieldResolver::new_with_php_getters(
            &fields,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            map,
        );
        assert_eq!(r.accessor("first_link", "php", "$result"), "$result->getLinks()[0]");
    }

    #[test]
    fn render_php_falls_back_to_property_when_getter_fields_empty() {
        // With empty php_getter_map the resolver uses the plain `render_php` path,
        // which emits `->camelCase` for every field regardless of scalar-ness.
        let r = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert_eq!(r.accessor("status_code", "php", "$result"), "$result->statusCode");
        assert_eq!(r.accessor("metadata", "php", "$result"), "$result->metadata");
    }

    // Regression: bare-name HashSet classification produced false getters when two
    // types shared a field name with different scalarness (sample-crawler `content`
    // collision between CrawlConfig.content: ContentConfig and MarkdownResult.content: String).
    #[test]
    fn render_php_with_getters_distinguishes_same_field_name_on_different_types() {
        let mut getters: HashMap<String, HashSet<String>> = HashMap::new();
        // A.content is non-scalar.
        getters.insert("A".to_string(), ["content".to_string()].into_iter().collect());
        // B.content is scalar — explicit empty set.
        getters.insert("B".to_string(), HashSet::new());
        // Both A and B declare a "content" field — needed so the per-type
        // classification is consulted (not fallback bare-name union).
        let mut all_fields: HashMap<String, HashSet<String>> = HashMap::new();
        all_fields.insert("A".to_string(), ["content".to_string()].into_iter().collect());
        all_fields.insert("B".to_string(), ["content".to_string()].into_iter().collect());
        let map_a = PhpGetterMap {
            getters: getters.clone(),
            field_types: HashMap::new(),
            root_type: Some("A".to_string()),
            all_fields: all_fields.clone(),
        };
        let map_b = PhpGetterMap {
            getters,
            field_types: HashMap::new(),
            root_type: Some("B".to_string()),
            all_fields,
        };
        let r_a = FieldResolver::new_with_php_getters(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            map_a,
        );
        let r_b = FieldResolver::new_with_php_getters(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            map_b,
        );
        assert_eq!(r_a.accessor("content", "php", "$a"), "$a->getContent()");
        assert_eq!(r_b.accessor("content", "php", "$b"), "$b->content");
    }

    // Regression: the chain renderer must advance current_type through the IR's
    // nested-type graph so a scalar field on a nested type is not falsely
    // classified as needing a getter because some other type uses the same name.
    #[test]
    fn render_php_with_getters_chains_through_correct_type() {
        let mut fields = HashMap::new();
        fields.insert("nested_content".to_string(), "inner.content".to_string());
        let mut getters: HashMap<String, HashSet<String>> = HashMap::new();
        // Outer.inner is non-scalar (struct B).
        getters.insert("Outer".to_string(), ["inner".to_string()].into_iter().collect());
        // B.content is scalar.
        getters.insert("B".to_string(), HashSet::new());
        // Decoy: another type with non-scalar `content` field — used to verify
        // the legacy bare-name union would have produced the wrong answer.
        getters.insert("Decoy".to_string(), ["content".to_string()].into_iter().collect());
        let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
        field_types.insert(
            "Outer".to_string(),
            [("inner".to_string(), "B".to_string())].into_iter().collect(),
        );
        let mut all_fields: HashMap<String, HashSet<String>> = HashMap::new();
        all_fields.insert("Outer".to_string(), ["inner".to_string()].into_iter().collect());
        all_fields.insert("B".to_string(), ["content".to_string()].into_iter().collect());
        all_fields.insert("Decoy".to_string(), ["content".to_string()].into_iter().collect());
        let map = PhpGetterMap {
            getters,
            field_types,
            root_type: Some("Outer".to_string()),
            all_fields,
        };
        let r = FieldResolver::new_with_php_getters(
            &fields,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            map,
        );
        assert_eq!(
            r.accessor("nested_content", "php", "$result"),
            "$result->getInner()->content"
        );
    }

    // ---------------------------------------------------------------------------
    // Namespace-prefix stripping tests
    // ---------------------------------------------------------------------------

    fn make_resolver_with_result_fields(result_fields: &[&str]) -> FieldResolver {
        let rf: HashSet<String> = result_fields.iter().map(|s| s.to_string()).collect();
        FieldResolver::new(&HashMap::new(), &HashSet::new(), &rf, &HashSet::new(), &HashSet::new())
    }

    /// `browser.browser_used` — `browser` is a virtual namespace prefix, actual
    /// field is `browser_used` which IS in result_fields.
    #[test]
    fn is_valid_for_result_accepts_virtual_namespace_prefix() {
        let r = make_resolver_with_result_fields(&["browser_used", "js_render_hint", "status_code"]);
        assert!(
            r.is_valid_for_result("browser.browser_used"),
            "browser.browser_used should be valid via namespace-prefix stripping"
        );
        assert!(
            r.is_valid_for_result("browser.js_render_hint"),
            "browser.js_render_hint should be valid via namespace-prefix stripping"
        );
    }

    /// `interaction.action_results[0].action_type` — `interaction` is a virtual
    /// namespace prefix, `action_results` IS in result_fields.
    #[test]
    fn is_valid_for_result_accepts_namespace_prefix_before_array_field() {
        let r = make_resolver_with_result_fields(&["action_results", "final_html", "final_url"]);
        assert!(
            r.is_valid_for_result("interaction.action_results[0].action_type"),
            "interaction. prefix should be stripped so action_results is recognised"
        );
    }

    /// Fields that genuinely don't exist should still be rejected.
    #[test]
    fn is_valid_for_result_rejects_unknown_field_even_after_namespace_strip() {
        let r = make_resolver_with_result_fields(&["pages", "final_url"]);
        assert!(
            !r.is_valid_for_result("browser.browser_used"),
            "browser_used is not in result_fields so should be rejected"
        );
        assert!(
            !r.is_valid_for_result("ns.unknown_field"),
            "unknown_field is not in result_fields so should be rejected"
        );
    }

    /// Accessor for `browser.browser_used` should produce the stripped path
    /// (i.e. `result.browser_used` for Python, not `result.browser.browser_used`).
    #[test]
    fn accessor_strips_namespace_prefix_for_python() {
        let r = make_resolver_with_result_fields(&["browser_used", "js_render_hint"]);
        assert_eq!(
            r.accessor("browser.browser_used", "python", "result"),
            "result.browser_used"
        );
        assert_eq!(
            r.accessor("browser.js_render_hint", "python", "result"),
            "result.js_render_hint"
        );
    }

    /// Accessor for `browser.browser_used` should produce PascalCase path for C#.
    #[test]
    fn accessor_strips_namespace_prefix_for_csharp() {
        let r = make_resolver_with_result_fields(&["browser_used"]);
        assert_eq!(
            r.accessor("browser.browser_used", "csharp", "result"),
            "result.BrowserUsed"
        );
    }

    /// Accessor for `interaction.action_results[0].action_type` — strips `interaction.`
    /// prefix and resolves the remaining path.
    #[test]
    fn accessor_strips_namespace_prefix_for_indexed_array_field() {
        let r = make_resolver_with_result_fields(&["action_results", "final_html", "final_url"]);
        // Python: result.action_results[0].action_type
        assert_eq!(
            r.accessor("interaction.action_results[0].action_type", "python", "result"),
            "result.action_results[0].action_type"
        );
        // TypeScript: result.actionResults[0].actionType
        assert_eq!(
            r.accessor("interaction.action_results[0].action_type", "typescript", "result"),
            "result.actionResults[0].actionType"
        );
    }

    /// When `result_fields` is empty, namespace stripping is disabled and every
    /// path is accepted (the permissive default).
    #[test]
    fn is_valid_for_result_is_permissive_when_result_fields_empty() {
        let r = make_resolver_with_result_fields(&[]);
        assert!(r.is_valid_for_result("browser.browser_used"));
        assert!(r.is_valid_for_result("anything.at.all"));
    }

    /// A real two-segment path like `metadata.title` where `metadata` IS a
    /// known result field must NOT be stripped — the full path resolves correctly.
    #[test]
    fn accessor_does_not_strip_real_first_segment() {
        let r = make_resolver_with_result_fields(&["metadata", "status_code"]);
        // `metadata` is a real result field; should not be stripped.
        assert_eq!(
            r.accessor("metadata.title", "python", "result"),
            "result.metadata.title"
        );
    }

    /// When `result_fields` is empty, `namespace_stripped_path` must return
    /// `None` so dotted field paths like `metrics.total_lines` survive intact
    /// for backends that navigate the path segment-by-segment (e.g. C). Prior
    /// behaviour stripped unconditionally, collapsing every dotted path to its
    /// leaf — the C codegen then emitted `<root>_<leaf>` accessors against the
    /// wrong parent type (e.g. `sample_pack_process_result_total_lines(result)`
    /// instead of `sample_pack_file_metrics_total_lines(metrics)`).
    #[test]
    fn namespace_stripped_path_returns_none_when_result_fields_empty() {
        let r = make_resolver_with_result_fields(&[]);
        assert_eq!(r.namespace_stripped_path("metrics.total_lines"), None);
        assert_eq!(r.namespace_stripped_path("anything.deeply.nested.path"), None);
    }

    // ---------------------------------------------------------------------------
    // Rust + PHP accessor regression: result_fields and per-type getter lookups
    // must override the global "method_calls" / "any-type" leakage.
    // ---------------------------------------------------------------------------

    /// Rust accessor: when a field is in both `method_calls` (workspace-global
    /// from `[crates.e2e.fields_method_calls]`) AND `result_fields` (the
    /// fixture's root-type field list), it must render as field access
    /// (`result.content`), not a method call (`result.content()`).
    ///
    /// Regression: a fixture DTO's `DocumentResult.content: String` is a struct
    /// field, but other types in the workspace declare `content` as a method,
    /// so the global `method_calls` set carries it. Without consulting
    /// `result_fields`, the Rust e2e renderer emitted `result.content()` and
    /// produced E0599 against `pub content: String`.
    #[test]
    fn render_rust_with_result_fields_overrides_method_calls() {
        let result_fields: HashSet<String> = ["content".to_string(), "mime_type".to_string()].into_iter().collect();
        let method_calls: HashSet<String> = [
            "content".to_string(),
            "mime_type".to_string(),
            "other_accessor".to_string(),
        ]
        .into_iter()
        .collect();
        let r = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &HashSet::new(),
            &method_calls,
        );
        assert_eq!(r.accessor("content", "rust", "result"), "result.content");
        assert_eq!(r.accessor("mime_type", "rust", "result"), "result.mime_type");
        // A path that's in method_calls but NOT in result_fields still renders
        // as a method call — the override is targeted at result-root fields.
        assert_eq!(
            r.accessor("other_accessor", "rust", "result"),
            "result.other_accessor()"
        );
    }

    /// PHP `needs_getter`: when the owner type declares the field but has no
    /// entry in `getters` (i.e. all its fields are scalar), the answer must be
    /// `false` — without falling back to the global bare-name union.
    ///
    /// Regression: a fixture DTO `DocumentResult` declares `content: String`
    /// (scalar) and has no entry in `getters`. Some other type in the workspace
    /// declares `content` as non-scalar (e.g. a chunk struct). The legacy
    /// fallback would flip `$result->content` to `$result->getContent()` based
    /// on the bare-name union — producing a "method does not exist" error
    /// against the actual ExtractionResult class. The fix: when owner is known
    /// and declares the field, trust the per-type getters map exclusively.
    #[test]
    fn render_php_needs_getter_returns_false_when_owner_has_no_getter_entry() {
        let getters: HashMap<String, HashSet<String>> = {
            let mut m = HashMap::new();
            // Chunk.content is non-scalar (some Vec<Span>); only Chunk has a
            // getters entry.
            m.insert("Chunk".to_string(), ["content".to_string()].into_iter().collect());
            m
        };
        let all_fields: HashMap<String, HashSet<String>> = {
            let mut m = HashMap::new();
            m.insert(
                "ExtractionResult".to_string(),
                ["content".to_string()].into_iter().collect(),
            );
            m.insert("Chunk".to_string(), ["content".to_string()].into_iter().collect());
            m
        };
        let map = PhpGetterMap {
            getters,
            field_types: HashMap::new(),
            root_type: Some("ExtractionResult".to_string()),
            all_fields,
        };
        // ExtractionResult declares `content` but has no getters entry — must be
        // treated as scalar, NOT flipped to getter syntax via bare-name union.
        assert!(!map.needs_getter(Some("ExtractionResult"), "content"));
        // Chunk DOES need getter syntax (entry exists).
        assert!(map.needs_getter(Some("Chunk"), "content"));
        // Unknown owner still uses the bare-name fallback (legacy behaviour).
        assert!(map.needs_getter(None, "content"));

        // Confirm end-to-end accessor rendering matches.
        let r = FieldResolver::new_with_php_getters(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            map,
        );
        assert_eq!(r.accessor("content", "php", "$result"), "$result->content");
    }
}
