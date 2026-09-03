use super::super::ir_collection::{build_ir_collection_map, build_non_string_scalar_element_fields};
use super::super::ir_enum::build_ir_enum_map;
use super::super::ir_result_fields::{OptionalityRule, build_ir_result_field_map, is_optional_path};
use super::super::python_typeddict::{build_python_typeddict_facts, build_python_typeddict_map};
use super::super::types::{
    DartFirstClassMap, FieldResolver, IrCollectionMap, IrEnumMap, IrResultFieldMap, PhpGetterMap, PythonTypedDictFacts,
    PythonTypedDictMap, SwiftFirstClassMap, VariantAccessorMap,
};
use std::collections::{HashMap, HashSet};

/// Every key an accessor renderer will look up in `optional_fields` while walking `path`, in
/// order, one per segment.
///
/// ~keep Must stay byte-identical to what the renderers build, or an inserted entry silently
/// never matches: they track the path with [`push_key_field_name`] (the segment's bare name,
/// no index) to ask about the segment itself, then [`push_key_index_suffix`] normalises an
/// indexed segment to a literal `[0]` before the next one — so the key for
/// `results[3].metadata` is `results` and then `results[0].metadata`, never `results[3]`.
///
/// [`push_key_field_name`]: super::super::optional_renderers::push_key_field_name
/// [`push_key_index_suffix`]: super::super::optional_renderers::push_key_index_suffix
fn optional_lookup_keys(path: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut path_so_far = String::new();
    for segment in super::super::parse::parse_path(path) {
        // A `.length`/`.count` pseudo-segment names no field, leaves the tracked key untouched,
        // and is never looked up — pushing here would only re-test the previous key. ~keep
        if super::super::parse::segment_name(&segment).is_none() {
            continue;
        }
        super::super::optional_renderers::push_key_field_name(&mut path_so_far, &segment);
        keys.push(path_so_far.clone());
        super::super::optional_renderers::push_key_index_suffix(&mut path_so_far, &segment);
    }
    keys
}

thread_local! {
    /// ~keep Fields already warned about by [`FieldResolver::warn_on_result_fields_contradicting_ir`]
    /// on THIS thread this run. `result_fields` and the IR's `binding_excluded` set are both
    /// static per crate -- the contradiction a `field` entry represents does not change across
    /// the fixtures and languages a resolver gets rebuilt for -- but `with_ir_fields` runs once
    /// per (fixture, language, reachable/excluded pass), so without this a single bad config
    /// entry (e.g. a `#[serde(skip)]` field still listed in `result_fields`) produced the
    /// identical WARN line thousands of times in one run (2600+ in one downstream consumer's
    /// `adopt` run for a single field). Repeating the same finding that many times is the same
    /// failure mode as never emitting it: nobody reads past the first screenful, so the config
    /// bug it is trying to surface stays unfixed.
    ///
    /// Thread-local, not a global set, for the same reason `e2e::codegen`'s `SKIP_LEDGER` and
    /// its inert-example counterpart are: matches the existing convention in this codebase
    /// rather than introducing a new synchronization primitive. This bounds repeats to "at most
    /// once per worker thread" rather than "exactly once per run" under `alef`'s `-j` job
    /// parallelism (verified: `-j1` produces exactly one warning per field; the default parallel
    /// job count produced two for that same downstream consumer's one contradicting field, one
    /// per thread that happened to build a resolver for it) -- still a ~1300x reduction against
    /// the un-deduped
    /// count, and a bound proportional to core count rather than to fixture count.
    static WARNED_CONTRADICTING_FIELDS: std::cell::RefCell<HashSet<String>> =
        std::cell::RefCell::new(HashSet::new());
}

/// Clear the dedup set. Test-only: production runs are one process per invocation, so the
/// thread-local's implicit reset on process exit is enough there; `cargo test` reuses threads
/// across tests in the same binary, and dedup state from one test would otherwise silence the
/// next.
#[cfg(test)]
pub(crate) fn reset_contradicting_field_warnings() {
    WARNED_CONTRADICTING_FIELDS.with(|warned| warned.borrow_mut().clear());
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
            config_declared_optional_fields: optional.clone(),
            result_fields: result_fields.clone(),
            array_fields: array_fields.clone(),
            enum_fields: HashSet::new(),
            method_calls: method_calls.clone(),
            error_field_aliases: HashMap::new(),
            php_getter_map: PhpGetterMap::default(),
            variant_accessors: VariantAccessorMap::default(),
            swift_first_class_map: SwiftFirstClassMap::default(),
            dart_first_class_map: DartFirstClassMap::default(),
            display_as_text_fields: HashSet::new(),
            ir_reachable_fields: HashSet::new(),
            ir_known_excluded_fields: HashSet::new(),
            wire_optional_fields: HashSet::new(),
            ir_enum_map: IrEnumMap::default(),
            wasm_untagged_enum_names: HashSet::new(),
            napi_tagged_object_enums: HashMap::new(),
            java_wrapper_enum_names: HashSet::new(),
            ruby_hash_serialized_enum_names: HashSet::new(),
            ir_collection_map: IrCollectionMap::default(),
            non_string_scalar_collection_fields: HashMap::new(),
            ir_result_field_map: IrResultFieldMap::default(),
            result_is_byte_payload: false,
            python_typeddict_map: PythonTypedDictMap::default(),
            python_map_value_edges: HashMap::new(),
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
            config_declared_optional_fields: optional.clone(),
            result_fields: result_fields.clone(),
            array_fields: array_fields.clone(),
            enum_fields: HashSet::new(),
            method_calls: method_calls.clone(),
            error_field_aliases: error_field_aliases.clone(),
            php_getter_map: PhpGetterMap::default(),
            variant_accessors: VariantAccessorMap::default(),
            swift_first_class_map: SwiftFirstClassMap::default(),
            dart_first_class_map: DartFirstClassMap::default(),
            display_as_text_fields: HashSet::new(),
            ir_reachable_fields: HashSet::new(),
            ir_known_excluded_fields: HashSet::new(),
            wire_optional_fields: HashSet::new(),
            ir_enum_map: IrEnumMap::default(),
            wasm_untagged_enum_names: HashSet::new(),
            napi_tagged_object_enums: HashMap::new(),
            java_wrapper_enum_names: HashSet::new(),
            ruby_hash_serialized_enum_names: HashSet::new(),
            ir_collection_map: IrCollectionMap::default(),
            non_string_scalar_collection_fields: HashMap::new(),
            ir_result_field_map: IrResultFieldMap::default(),
            result_is_byte_payload: false,
            python_typeddict_map: PythonTypedDictMap::default(),
            python_map_value_edges: HashMap::new(),
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
            config_declared_optional_fields: optional.clone(),
            result_fields: result_fields.clone(),
            array_fields: array_fields.clone(),
            enum_fields: HashSet::new(),
            method_calls: method_calls.clone(),
            error_field_aliases: error_field_aliases.clone(),
            php_getter_map,
            variant_accessors: VariantAccessorMap::default(),
            swift_first_class_map: SwiftFirstClassMap::default(),
            dart_first_class_map: DartFirstClassMap::default(),
            display_as_text_fields: HashSet::new(),
            ir_reachable_fields: HashSet::new(),
            ir_known_excluded_fields: HashSet::new(),
            wire_optional_fields: HashSet::new(),
            ir_enum_map: IrEnumMap::default(),
            wasm_untagged_enum_names: HashSet::new(),
            napi_tagged_object_enums: HashMap::new(),
            java_wrapper_enum_names: HashSet::new(),
            ruby_hash_serialized_enum_names: HashSet::new(),
            ir_collection_map: IrCollectionMap::default(),
            non_string_scalar_collection_fields: HashMap::new(),
            ir_result_field_map: IrResultFieldMap::default(),
            result_is_byte_payload: false,
            python_typeddict_map: PythonTypedDictMap::default(),
            python_map_value_edges: HashMap::new(),
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
            config_declared_optional_fields: optional.clone(),
            result_fields: result_fields.clone(),
            array_fields: array_fields.clone(),
            enum_fields: HashSet::new(),
            method_calls: method_calls.clone(),
            error_field_aliases: error_field_aliases.clone(),
            php_getter_map: PhpGetterMap::default(),
            variant_accessors: VariantAccessorMap::default(),
            swift_first_class_map,
            dart_first_class_map: DartFirstClassMap::default(),
            display_as_text_fields: HashSet::new(),
            ir_reachable_fields: HashSet::new(),
            ir_known_excluded_fields: HashSet::new(),
            wire_optional_fields: HashSet::new(),
            ir_enum_map: IrEnumMap::default(),
            wasm_untagged_enum_names: HashSet::new(),
            napi_tagged_object_enums: HashMap::new(),
            java_wrapper_enum_names: HashSet::new(),
            ruby_hash_serialized_enum_names: HashSet::new(),
            ir_collection_map: IrCollectionMap::default(),
            non_string_scalar_collection_fields: HashMap::new(),
            ir_result_field_map: IrResultFieldMap::default(),
            result_is_byte_payload: false,
            python_typeddict_map: PythonTypedDictMap::default(),
            python_map_value_edges: HashMap::new(),
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
            config_declared_optional_fields: optional.clone(),
            result_fields: result_fields.clone(),
            array_fields: array_fields.clone(),
            enum_fields: HashSet::new(),
            method_calls: method_calls.clone(),
            error_field_aliases: error_field_aliases.clone(),
            php_getter_map: PhpGetterMap::default(),
            variant_accessors: VariantAccessorMap::default(),
            swift_first_class_map: SwiftFirstClassMap::default(),
            dart_first_class_map,
            display_as_text_fields: HashSet::new(),
            ir_reachable_fields: HashSet::new(),
            ir_known_excluded_fields: HashSet::new(),
            wire_optional_fields: HashSet::new(),
            ir_enum_map: IrEnumMap::default(),
            wasm_untagged_enum_names: HashSet::new(),
            napi_tagged_object_enums: HashMap::new(),
            java_wrapper_enum_names: HashSet::new(),
            ruby_hash_serialized_enum_names: HashSet::new(),
            ir_collection_map: IrCollectionMap::default(),
            non_string_scalar_collection_fields: HashMap::new(),
            ir_result_field_map: IrResultFieldMap::default(),
            result_is_byte_payload: false,
            python_typeddict_map: PythonTypedDictMap::default(),
            python_map_value_edges: HashMap::new(),
        }
    }

    /// Return a clone of this resolver with the Dart first-class map's
    /// `root_type` replaced.
    pub fn with_dart_root_type(&self, root_type: Option<String>) -> Self {
        let mut clone = self.clone();
        clone.dart_first_class_map.root_type = root_type;
        clone
    }

    /// Return a clone of this resolver with `display_as_text_fields` set.
    ///
    /// Fields in this set have an `Option<T>` inner type (e.g. `RichTextContent`)
    /// that is NOT a plain `String`. Language generators will call the language-idiomatic
    /// text accessor (`.Text()` in Go/Java/C#, `.text()` in PHP) instead of generic
    /// object stringification (`string(*ptr)`, `Objects::toString()`, `.ToString()`).
    pub fn with_display_as_text_fields(mut self, fields: HashSet<String>) -> Self {
        self.display_as_text_fields = fields;
        self
    }

    pub fn with_enum_fields(mut self, fields: HashSet<String>) -> Self {
        self.enum_fields = fields;
        self
    }

    /// Compute the IR-derived enum-field classification for [`Self::with_ir_enum_map`],
    /// mirroring [`Self::ir_field_sets`]'s "compute once from the crate's IR" shape. The
    /// returned map has no `root_type` set yet — `with_ir_enum_map` anchors it to the
    /// specific call being rendered.
    pub fn ir_enum_fields(type_defs: &[crate::core::ir::TypeDef], enums: &[crate::core::ir::EnumDef]) -> IrEnumMap {
        build_ir_enum_map(type_defs, enums)
    }

    /// Attach IR-derived enum classification to this resolver, anchored at `root_type` — the
    /// IR type name backing the current call's result variable, if resolved (e.g. via the
    /// call's declared Rust return type, unwrapped through `Option`/`Vec`).
    ///
    /// `map` should come from [`Self::ir_enum_fields`], computed once per crate IR and reused
    /// across calls; only `root_type` varies per call. `is_enum` consults this AFTER the
    /// hand-maintained `fields_enum` config, so an explicit config entry always wins and this
    /// only rescues fields the config never mentioned — the same precedence `with_ir_fields`
    /// already established for `result_fields`. ~keep
    pub fn with_ir_enum_map(mut self, mut map: IrEnumMap, root_type: Option<String>) -> Self {
        map.root_type = root_type;
        self.ir_enum_map = map;
        self
    }

    /// Attach wasm-only serde representation facts without changing the public [`IrEnumMap`]
    /// construction surface.
    pub(crate) fn with_wasm_enum_representations(mut self, enums: &[crate::core::ir::EnumDef]) -> Self {
        self.wasm_untagged_enum_names = enums
            .iter()
            .filter(|enum_def| enum_def.serde_untagged)
            .map(|enum_def| enum_def.name.clone())
            .collect();
        self
    }

    /// Attach, for each IR enum the napi backend lowers to an internally-tagged
    /// `#[napi(object)]` struct, the JavaScript discriminant property it puts on the wire.
    ///
    /// ~keep Both facts come from the napi backend itself rather than being re-derived here:
    /// `is_tagged_data_enum` decides tagged-object vs `#[napi(string_enum)]` and its own doc
    /// names itself "the single authority for that verdict", listing the runtime struct emitter,
    /// the conversion emitters and `errors::gen_dts` as the callers that consult it so those can
    /// never disagree. The e2e generator was the one caller that did not, which is precisely why
    /// its assertion disagreed with both. Same single-authority relationship as
    /// `with_java_wrapper_enum_names` and `with_ruby_hash_serialized_enum_names`.
    pub(crate) fn with_napi_tagged_object_enums(mut self, enums: &[crate::core::ir::EnumDef]) -> Self {
        use crate::backends::napi::{is_tagged_data_enum, tagged_enum_discriminant_js_name};
        self.napi_tagged_object_enums = enums
            .iter()
            .filter(|enum_def| is_tagged_data_enum(enum_def))
            .map(|enum_def| {
                (
                    enum_def.name.clone(),
                    tagged_enum_discriminant_js_name(enum_def).to_string(),
                )
            })
            .collect();
        self
    }

    /// Attach the set of IR enum type names for which the Java binding backend does NOT emit a
    /// plain `enum` with a `getValue()` accessor (tagged/untagged-union wrapper classes
    /// instead). Java e2e codegen is the only caller; every other backend's resolver leaves
    /// this empty. See `java_wrapper_enum_names`'s field doc for the source of truth.
    pub fn with_java_wrapper_enum_names(mut self, names: HashSet<String>) -> Self {
        self.java_wrapper_enum_names = names;
        self
    }

    /// Attach how this language's binding spells a tagged-union variant narrowing. C# and Dart
    /// e2e codegen are the only callers; every other resolver leaves this empty, which renders
    /// exactly as it did before the map existed. See `variant_accessors`' field doc.
    pub fn with_variant_accessors(mut self, accessors: VariantAccessorMap) -> Self {
        self.variant_accessors = accessors;
        self
    }

    /// Attach the set of IR enum type names Ruby's Magnus binding backend lowers to a plain
    /// `Hash` rather than a `Symbol`. Ruby e2e codegen is the only caller; every other backend's
    /// resolver leaves this empty. See `ruby_hash_serialized_enum_names`'s field doc for the
    /// source of truth.
    pub fn with_ruby_hash_serialized_enum_names(mut self, names: HashSet<String>) -> Self {
        self.ruby_hash_serialized_enum_names = names;
        self
    }

    /// Compute the IR-derived collection-field classification for
    /// [`Self::with_ir_collection_map`], mirroring [`Self::ir_enum_fields`]'s "compute once
    /// from the crate's IR" shape. The returned map has no `root_type` set yet —
    /// `with_ir_collection_map` anchors it to the specific call being rendered.
    pub fn ir_collection_fields(type_defs: &[crate::core::ir::TypeDef]) -> IrCollectionMap {
        build_ir_collection_map(type_defs)
    }

    /// Attach IR-derived collection classification to this resolver, anchored at `root_type` —
    /// the IR type name backing the current call's result variable, if resolved.
    ///
    /// `map` should come from [`Self::ir_collection_fields`], computed once per crate IR and
    /// reused across calls; only `root_type` varies per call. `is_collection_root` consults
    /// this AFTER the hand-maintained `fields_array`/`fields_optional` config, so an explicit
    /// config entry always wins and this only rescues fields the config never mentioned — the
    /// same precedence [`Self::with_ir_enum_map`] already established for `is_enum`. ~keep
    pub fn with_ir_collection_map(mut self, mut map: IrCollectionMap, root_type: Option<String>) -> Self {
        map.root_type = root_type;
        self.ir_collection_map = map;
        self
    }

    /// Attach TypeScript-only collection element facts outside public [`IrCollectionMap`].
    pub(crate) fn with_collection_element_metadata(mut self, type_defs: &[crate::core::ir::TypeDef]) -> Self {
        self.non_string_scalar_collection_fields = build_non_string_scalar_element_fields(type_defs);
        self
    }

    /// Compute the Python `TypedDict`-membership classification for
    /// [`Self::with_python_typeddict_map`], mirroring [`Self::ir_collection_fields`]'s "compute
    /// once from the crate's IR" shape. The returned map has no `root_type` set yet —
    /// `with_python_typeddict_map` anchors it to the specific call being rendered.
    ///
    /// This asks the same predicate
    /// (`crate::backends::pyo3::gen_bindings::errors::is_dataclass_backed_config`) the pyo3
    /// backend itself consults, so this can only ever agree with what it actually emits. That
    /// predicate no longer varies by DTO output style or by `reexported_types` for a
    /// return-position type (a real downstream crate's issue #183: such a type is never redefined
    /// as a `TypedDict`), so this classification takes no config input anymore either.
    pub fn python_typeddict_fields(type_defs: &[crate::core::ir::TypeDef]) -> PythonTypedDictMap {
        build_python_typeddict_map(type_defs)
    }

    /// Compute the complete internal Python accessor facts, including map-value traversal edges
    /// that are intentionally absent from the public [`PythonTypedDictMap`] representation.
    pub(crate) fn python_typeddict_facts(type_defs: &[crate::core::ir::TypeDef]) -> PythonTypedDictFacts {
        build_python_typeddict_facts(type_defs)
    }

    /// Attach the Python `TypedDict` classification to this resolver, anchored at `root_type` —
    /// the IR type name backing the current call's result variable, if resolved.
    ///
    /// `map` should come from [`Self::python_typeddict_fields`], computed once per crate IR and
    /// reused across calls; only `root_type` varies per call. `render_python_with_optionals`
    /// consults this to decide subscript (`result["field"]`) vs. attribute (`result.field`)
    /// access at each link of the chain — see [`super::super::types::PythonTypedDictMap`] for
    /// why the classification has to be anchored per-type rather than answered from a bare field
    /// name. A `None`/empty map leaves every path on attribute access, exactly the behaviour
    /// before this map existed. ~keep
    pub fn with_python_typeddict_map(mut self, mut map: PythonTypedDictMap, root_type: Option<String>) -> Self {
        map.root_type = root_type;
        self.python_typeddict_map = map;
        self.python_map_value_edges.clear();
        self
    }

    /// Attach complete internally generated Python accessor facts and anchor them to one call's
    /// declared result type. Existing callers that attach only a public [`PythonTypedDictMap`]
    /// retain their previous behavior and cannot inject or observe private map-value edges.
    pub(crate) fn with_python_typeddict_facts(
        mut self,
        mut facts: PythonTypedDictFacts,
        root_type: Option<String>,
    ) -> Self {
        facts.typeddict_map.root_type = root_type;
        self.python_typeddict_map = facts.typeddict_map;
        self.python_map_value_edges = facts.map_value_edges;
        self
    }

    /// The Python `TypedDict` classification this resolver was built with (see
    /// [`Self::with_python_typeddict_map`]).
    ///
    /// Exposed so callers that render Python accessor expressions OUTSIDE this resolver's own
    /// `PathSegment`-based rendering — namely the streaming-virtual-field accessors in
    /// `e2e::codegen::streaming_assertions`, which build their expressions as hand-rolled
    /// `format!` strings over the stream item type rather than the call's declared result type —
    /// can classify their own field owners against the SAME map this resolver would consult,
    /// instead of re-deriving the rule (see the `two-generators-disagree` skill). ~keep
    pub fn python_typeddict_map(&self) -> &PythonTypedDictMap {
        &self.python_typeddict_map
    }

    /// Compute the per-owner-type field facts for [`Self::with_ir_result_fields`], under the
    /// optionality rule the binding for `language` applies. The returned map has no `root_type`
    /// yet — `with_ir_result_fields` anchors it to the specific call being rendered. Mirrors
    /// [`Self::ir_collection_fields`]'s "compute once from the crate's IR" shape.
    pub(crate) fn ir_result_field_facts(type_defs: &[crate::core::ir::TypeDef], language: &str) -> IrResultFieldMap {
        build_ir_result_field_map(type_defs, OptionalityRule::for_language(language))
    }

    #[cfg(test)]
    pub(crate) fn ir_result_field_facts_with_enums(
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
        language: &str,
    ) -> IrResultFieldMap {
        super::super::ir_result_fields::build_ir_result_field_map_with_enums(
            type_defs,
            enums,
            OptionalityRule::for_language(language),
        )
    }

    #[cfg(test)]
    pub(crate) fn go_ir_result_field_facts(
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
        excluded_names: &HashSet<String>,
    ) -> IrResultFieldMap {
        let emitted = crate::backends::go::emission_facts::GoEmissionFacts::new(
            type_defs,
            enums,
            excluded_names.clone(),
            HashSet::new(),
        );
        super::super::ir_result_fields::build_go_ir_result_field_map(
            type_defs,
            enums,
            OptionalityRule::DeclaredType,
            &emitted,
        )
    }

    pub(crate) fn go_ir_result_field_facts_from_emission(
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
        emitted: &crate::backends::go::emission_facts::GoEmissionFacts<'_>,
    ) -> IrResultFieldMap {
        super::super::ir_result_fields::build_go_ir_result_field_map(
            type_defs,
            enums,
            OptionalityRule::DeclaredType,
            emitted,
        )
    }

    /// Attach IR field facts anchored at `root_type` — the IR type name the call's declared
    /// return type resolves to, per `codegen::call_ir::resolve_declared_result_type`.
    ///
    /// `map` should come from [`Self::ir_result_field_facts`]; only `root_type` varies per call.
    /// A `None` root leaves every anchored answer disabled, which is exactly the behaviour
    /// before this map existed — so wiring it in can never, on its own, change a verdict. ~keep
    pub(crate) fn with_ir_result_fields(mut self, mut map: IrResultFieldMap, root_type: Option<String>) -> Self {
        map.root_type = root_type;
        self.ir_result_field_map = map;
        self
    }

    /// Declare that the call's own declared Rust return type resolves to a raw byte payload
    /// (`bytes::Bytes`, `Vec<u8>`, `[u8]`, `[u8; N]`) rather than a struct — the call-specific
    /// counterpart to [`crate::core::config::e2e::CallConfig::effective_result_is_bytes`], fed
    /// either from that config flag or from the call's own resolved signature.
    ///
    /// Once set, [`Self::is_valid_for_result`] and [`Self::result_field_oracle_knows`] reject
    /// EVERY non-empty field path unconditionally — a byte payload has no fields, so no per-field
    /// distinction is possible or needed. `false` (the default every resolver already had before
    /// this flag existed) leaves every other oracle's behaviour completely unchanged, so wiring
    /// this in at a construction site can only turn an already-wrong accepted path into a
    /// rejection, never the reverse. ~keep
    pub fn with_result_is_byte_payload(mut self, is_byte_payload: bool) -> Self {
        self.result_is_byte_payload = is_byte_payload;
        self
    }

    /// Record, in `optional_fields`, every prefix of `paths` the anchored map proves optional.
    ///
    /// ~keep The accessor renderers do not call [`Self::is_optional`]; they consult the
    /// `optional_fields` path set directly, one dotted prefix at a time, because that is the only
    /// form in which "the value BEFORE this segment may be absent" is expressible while walking a
    /// chain (`render_typescript_with_optionals` and its eight siblings all share this shape).
    /// Anchored optionality therefore has to be materialised into that set for the exact paths a
    /// caller is about to render, rather than answered on demand. Doing it per-path — instead of
    /// enumerating the whole reachable type graph — keeps recursive types finite and adds nothing
    /// for fields nobody accesses.
    ///
    /// Purely additive: an entry is only ever inserted, so no path that already guarded can stop
    /// guarding.
    pub(crate) fn with_anchored_optional_paths<'a>(mut self, paths: impl IntoIterator<Item = &'a str>) -> Self {
        if self.ir_result_field_map.root_type.is_none() {
            return self;
        }
        for path in paths {
            for key in optional_lookup_keys(self.resolve(path)) {
                if is_optional_path(&self.ir_result_field_map, &key) {
                    self.optional_fields.insert(key);
                }
            }
        }
        self
    }

    /// Return a clone of this resolver with IR-derived field-reachability data set.
    ///
    /// `reachable`/`excluded` come from [`Self::ir_field_sets`]. Once set, they become
    /// the primary source of truth for [`Self::is_valid_for_result`]: the hand-maintained
    /// `result_fields` config only gets the final say on field names the IR has never
    /// heard of (virtual namespace prefixes, synthetic/derived assertion fields, and the
    /// like) — see that method's doc comment for why config alone cannot be trusted.
    ///
    /// `optional` (also from [`Self::ir_field_sets`]) is merged into the config-declared
    /// `fields_optional` set rather than replacing it, so an `Option<T>` field is detected
    /// even when a consumer's `alef.toml` never lists it under `fields_optional` at all —
    /// see [`Self::ir_field_sets`] for why this merge is safe to do unconditionally. The merge
    /// only ever touches `optional_fields` (the classification answer); `config_declared_optional_fields`
    /// is fixed at construction and stays exactly what the consumer wrote, so provenance survives
    /// the merge undisturbed. ~keep
    pub fn with_ir_fields(
        mut self,
        reachable: HashSet<String>,
        excluded: HashSet<String>,
        optional: HashSet<String>,
    ) -> Self {
        self.ir_reachable_fields = reachable;
        self.ir_known_excluded_fields = excluded;
        self.optional_fields.extend(optional);
        self.warn_on_result_fields_contradicting_ir();
        self
    }

    /// Return a clone of this resolver with IR-derived wire-optionality data set.
    ///
    /// `fields` comes from [`Self::ir_wire_optional_fields`]. Consulted by
    /// [`Self::is_wire_optional_key`], which JSON-tree-walking accessor generators (currently
    /// only the Zig e2e generator) use to guard a `.get(key)` lookup instead of assuming presence.
    /// Deliberately a separate set from `optional_fields` / [`Self::with_ir_fields`] — see
    /// [`Self::ir_wire_optional_fields`] for why the two must not be merged.
    pub fn with_wire_optional_fields(mut self, fields: HashSet<String>) -> Self {
        self.wire_optional_fields = fields;
        self
    }

    /// Emit a `WARN` event for every `result_fields` entry the IR marks
    /// `binding_excluded` — every case where `is_valid_for_result` now rejects the field
    /// despite the config claiming it's available.
    ///
    /// `result_fields` is meant to *select* which available fields a call asserts on, not
    /// to *declare* availability (the IR does that); an entry landing here is always a
    /// config bug, not a legitimate declaration. This must be loud, not silent — a
    /// shipped config was found with exactly this shape (a `#[serde(skip)]`, no-getter
    /// field still listed in `result_fields`) sitting undetected because nothing surfaced
    /// the contradiction. ~keep
    ///
    /// ~keep Deduplicated via [`WARNED_CONTRADICTING_FIELDS`]: the contradiction is a static
    /// fact about the crate's config and IR, but this method runs once per (fixture, language,
    /// reachable/excluded pass) resolver build, so without the dedup the same field warned
    /// thousands of times in one run and buried the one thing worth reading.
    fn warn_on_result_fields_contradicting_ir(&self) {
        for field in &self.result_fields {
            if self.ir_known_excluded_fields.contains(field)
                && WARNED_CONTRADICTING_FIELDS.with(|warned| warned.borrow_mut().insert(field.clone()))
            {
                tracing::warn!(
                    field = %field,
                    "e2e config result_fields lists a field the IR marks binding_excluded (no \
                     accessor is emitted in any generated binding); the IR now overrides \
                     result_fields for this field and it will be treated as unavailable — fix \
                     or remove this result_fields entry"
                );
            }
        }
    }

    /// Compute the reachable/excluded/optional field-name sets from a crate's IR type
    /// definitions, for use with [`Self::with_ir_fields`].
    ///
    /// A field name is "reachable" if it is present, and not `binding_excluded`, on ANY
    /// type in `type_defs` — the exact predicate `crate::codegen::shared::binding_fields`
    /// uses to decide which struct fields a backend (pyo3, napi, go, …) actually attaches
    /// a real accessor to (e.g. `#[pyo3(get)]`). A field name is "known excluded" if it
    /// appears on some type but IS `binding_excluded` there, and is not reachable on any
    /// other type — reachable-on-any-type wins, since a bare field name can't be pinned to
    /// one exact result type here (callers only reach for this data when they can't
    /// already do that resolution themselves). ~keep
    ///
    /// A field name is "optional" only when EVERY declaration of it across `type_defs` is
    /// `Option<T>` (unanimous, not "any type wins" like `reachable`/`excluded` above). The
    /// direction has to flip here: `optional_fields` membership changes what code an
    /// accessor emits (`.as_ref().unwrap()` in Rust, `!` in C#, …), so a false positive is a
    /// compile error in a caller's generated test, while a false negative merely reproduces
    /// today's behavior (the field falls back to requiring an explicit `fields_optional`
    /// entry, exactly as before this method existed). ~keep
    pub fn ir_field_sets(
        type_defs: &[crate::core::ir::TypeDef],
    ) -> (HashSet<String>, HashSet<String>, HashSet<String>) {
        let mut reachable = HashSet::new();
        let mut excluded = HashSet::new();
        let mut optionality: HashMap<String, (bool, bool)> = HashMap::new();
        for type_def in type_defs {
            for field in &type_def.fields {
                if field.binding_excluded {
                    excluded.insert(field.name.clone());
                } else {
                    reachable.insert(field.name.clone());
                }
                let seen = optionality.entry(field.name.clone()).or_insert((false, false));
                if field.optional {
                    seen.0 = true;
                } else {
                    seen.1 = true;
                }
            }
        }
        excluded.retain(|f| !reachable.contains(f));
        let optional = optionality
            .into_iter()
            .filter_map(|(name, (seen_optional, seen_required))| (seen_optional && !seen_required).then_some(name))
            .collect();
        (reachable, excluded, optional)
    }

    /// Compute the wire-optional field-name set from a crate's IR type definitions, for use
    /// with [`Self::with_wire_optional_fields`].
    ///
    /// A field name lands here when it carries `#[serde(skip_serializing_if = "...")]` on AT
    /// LEAST ONE type — unlike [`Self::ir_field_sets`]'s `optional` set, this does not require
    /// unanimity across every declaration of the name. The asymmetry is intentional: merging a
    /// wrong name into `wire_optional_fields` only makes a JSON-tree-walking accessor guard a
    /// `.get(key)` lookup that would have succeeded anyway (defensive, always safe), whereas
    /// `ir_field_sets`'s `optional` set changes the *shape* of emitted code
    /// (`.as_ref().unwrap()`, `!`, …) and a false positive there is a compile error in a
    /// caller's generated test. No such risk exists here, so "any type wins" is the right,
    /// simpler rule. ~keep
    pub fn ir_wire_optional_fields(type_defs: &[crate::core::ir::TypeDef]) -> HashSet<String> {
        type_defs
            .iter()
            .flat_map(|type_def| &type_def.fields)
            .filter(|field| field.serde_skip_serializing_if)
            .map(|field| field.name.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A downstream consumer's `adopt` run hit this for real: one bad `result_fields` entry
    /// (`screenshot`, IR-excluded) produced the identical WARN line 2600+ times in a
    /// single run because `with_ir_fields` runs once per (fixture, language,
    /// reachable/excluded pass) resolver build. The second `with_ir_fields` call below
    /// reconstructs exactly that -- a fresh `FieldResolver` for the same contradicting
    /// field, as a second fixture/language would produce -- and must not warn again. ~keep
    #[test]
    #[tracing_test::traced_test]
    fn contradicting_result_fields_entry_warns_once_across_repeated_resolver_builds() {
        reset_contradicting_field_warnings();
        let result_fields: HashSet<String> = ["screenshot".to_owned()].into_iter().collect();
        let excluded: HashSet<String> = ["screenshot".to_owned()].into_iter().collect();

        for _ in 0..5 {
            FieldResolver::new(
                &HashMap::new(),
                &HashSet::new(),
                &result_fields,
                &HashSet::new(),
                &HashSet::new(),
            )
            .with_ir_fields(HashSet::new(), excluded.clone(), HashSet::new());
        }

        logs_assert(|lines| {
            let hits = lines.iter().filter(|line| line.contains("screenshot")).count();
            if hits == 1 {
                Ok(())
            } else {
                Err(format!(
                    "expected exactly 1 warning for `screenshot`, got {hits}: {lines:?}"
                ))
            }
        });
    }

    /// Two DIFFERENT contradicting fields must each still be named -- the dedup keys on
    /// the field name, not on "have we warned at all this run".
    #[test]
    #[tracing_test::traced_test]
    fn contradicting_result_fields_entries_are_deduplicated_per_field_not_globally() {
        reset_contradicting_field_warnings();
        let result_fields: HashSet<String> = ["screenshot".to_owned(), "raw_headers".to_owned()]
            .into_iter()
            .collect();
        let excluded: HashSet<String> = ["screenshot".to_owned(), "raw_headers".to_owned()]
            .into_iter()
            .collect();

        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_fields(HashSet::new(), excluded, HashSet::new());

        assert!(logs_contain("screenshot"));
        assert!(logs_contain("raw_headers"));
    }
}
