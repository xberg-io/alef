use serde::{Deserialize, Serialize};

use super::type_ref::TypeRef;

/// One container crossed on the path from a binding field to a resolved transparent newtype. ~keep
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NewtypeContainer {
    /// `Option<T>`. ~keep
    Optional,
    /// `Vec<T>`. ~keep
    Vec,
    /// The key in `Map<K, V>`. ~keep
    MapKey,
    /// The value in `Map<K, V>`. ~keep
    MapValue,
}

/// How generated Rust converts a resolved newtype at the binding boundary. ~keep
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum NewtypeConversion {
    /// A conventional public tuple struct: construct with `Type(value)` and consume with `.0`. ~keep
    #[default]
    TupleField,
    /// An explicitly marked private string wrapper. Both names come from source metadata. ~keep
    TransparentString { from: String, into: String },
}

/// Decoded metadata stored in the legacy string-valued newtype IR fields. ~keep
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewtypeWrapper {
    Tuple(String),
    Explicit(Vec<NewtypeWrapperMetadata>),
}

/// Structured metadata for a source-annotated newtype. ~keep
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewtypeWrapperMetadata {
    /// Fully qualified Rust path of the wrapper. ~keep
    pub rust_path: String,
    /// Explicit conversion operations for the wrapper. ~keep
    pub conversion: NewtypeConversion,
    /// Container path from the field root to the wrapped string. ~keep
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub containers: Vec<NewtypeContainer>,
}

impl NewtypeWrapperMetadata {
    /// Build one source-annotated transparent string conversion path. ~keep
    pub fn transparent_string(
        rust_path: impl Into<String>,
        from: impl Into<String>,
        into: impl Into<String>,
        containers: Vec<NewtypeContainer>,
    ) -> Self {
        Self {
            rust_path: rust_path.into(),
            conversion: NewtypeConversion::TransparentString {
                from: from.into(),
                into: into.into(),
            },
            containers,
        }
    }
}

impl NewtypeWrapper {
    const EXPLICIT_PREFIX: &'static str = "alef:newtype-conversions:v1:";
    const LEGACY_EXPLICIT_PREFIX: &'static str = "alef:newtype-conversions:";

    /// Decode a legacy path or an explicit conversion-path set. ~keep
    pub fn decode(value: &str) -> Result<Self, String> {
        if let Some(json) = value.strip_prefix(Self::EXPLICIT_PREFIX) {
            return Self::decode_explicit(json);
        }
        if let Some(json) = value.strip_prefix(Self::LEGACY_EXPLICIT_PREFIX) {
            if json.starts_with('v') {
                return Err("unsupported transparent newtype metadata version".to_string());
            }
            return Self::decode_explicit(json);
        }
        Ok(Self::Tuple(value.to_string()))
    }

    /// Encode explicit paths inside the unchanged `Option<String>` IR field type. ~keep
    pub fn encode_explicit(paths: &[NewtypeWrapperMetadata]) -> String {
        format!(
            "{}{}",
            Self::EXPLICIT_PREFIX,
            serde_json::to_string(paths).expect("newtype conversion metadata must serialize")
        )
    }

    /// Return the legacy tuple-wrapper path, when this is legacy metadata. ~keep
    pub fn tuple_path(&self) -> Option<&str> {
        match self {
            Self::Tuple(path) => Some(path),
            Self::Explicit(_) => None,
        }
    }

    /// Return all explicit conversion paths. ~keep
    pub fn explicit_paths(&self) -> &[NewtypeWrapperMetadata] {
        match self {
            Self::Tuple(_) => &[],
            Self::Explicit(paths) => paths,
        }
    }

    /// Validate that each explicit path can traverse the resolved binding type. ~keep
    pub fn validate_for_type(&self, ty: &TypeRef, outer_optional: bool) -> Result<(), String> {
        let Self::Explicit(paths) = self else {
            return Ok(());
        };
        for metadata in paths {
            validate_newtype_path(ty, outer_optional, &metadata.containers)?;
        }
        Ok(())
    }

    fn decode_explicit(json: &str) -> Result<Self, String> {
        let paths: Vec<NewtypeWrapperMetadata> =
            serde_json::from_str(json).map_err(|error| format!("invalid transparent newtype metadata: {error}"))?;
        if paths.is_empty() {
            return Err("transparent newtype metadata must contain at least one conversion path".to_string());
        }
        for (index, path) in paths.iter().enumerate() {
            if path.rust_path.is_empty() {
                return Err("transparent newtype metadata contains an empty Rust path".to_string());
            }
            let NewtypeConversion::TransparentString { from, into } = &path.conversion else {
                continue;
            };
            if from.is_empty() || into.is_empty() {
                return Err("transparent newtype metadata contains an empty conversion method".to_string());
            }
            if paths[..index]
                .iter()
                .any(|previous| previous.containers == path.containers)
            {
                return Err(format!(
                    "transparent newtype metadata contains duplicate container path {:?}",
                    path.containers
                ));
            }
        }
        validate_newtype_path_shape(&paths)?;
        Ok(Self::Explicit(paths))
    }
}

fn validate_newtype_path_shape(paths: &[NewtypeWrapperMetadata]) -> Result<(), String> {
    if paths.len() == 1 && paths[0].containers.is_empty() {
        return Ok(());
    }
    if paths.iter().any(|path| path.containers.is_empty()) {
        return Err("transparent newtype metadata overlaps a leaf path with a nested path".to_string());
    }
    let first = paths[0].containers[0];
    let is_uniform = matches!(first, NewtypeContainer::Optional | NewtypeContainer::Vec)
        && paths.iter().all(|path| path.containers[0] == first);
    let is_map = matches!(first, NewtypeContainer::MapKey | NewtypeContainer::MapValue)
        && paths.iter().all(|path| {
            matches!(
                path.containers[0],
                NewtypeContainer::MapKey | NewtypeContainer::MapValue
            )
        });
    if !is_uniform && !is_map {
        return Err("transparent newtype metadata contains incompatible container paths".to_string());
    }
    if is_uniform {
        let nested: Vec<_> = paths
            .iter()
            .map(|path| NewtypeWrapperMetadata {
                rust_path: path.rust_path.clone(),
                conversion: path.conversion.clone(),
                containers: path.containers[1..].to_vec(),
            })
            .collect();
        return validate_newtype_path_shape(&nested);
    }
    for branch in [NewtypeContainer::MapKey, NewtypeContainer::MapValue] {
        let nested: Vec<_> = paths
            .iter()
            .filter(|path| path.containers[0] == branch)
            .map(|path| NewtypeWrapperMetadata {
                rust_path: path.rust_path.clone(),
                conversion: path.conversion.clone(),
                containers: path.containers[1..].to_vec(),
            })
            .collect();
        if !nested.is_empty() {
            validate_newtype_path_shape(&nested)?;
        }
    }
    Ok(())
}

fn validate_newtype_path(
    ty: &TypeRef,
    mut outer_optional: bool,
    containers: &[NewtypeContainer],
) -> Result<(), String> {
    let mut current = ty;
    for container in containers {
        if *container == NewtypeContainer::Optional && outer_optional {
            outer_optional = false;
            continue;
        }
        current = match (container, current) {
            (NewtypeContainer::Optional, TypeRef::Optional(inner)) | (NewtypeContainer::Vec, TypeRef::Vec(inner)) => {
                inner
            }
            (NewtypeContainer::MapKey, TypeRef::Map(key, _)) => key,
            (NewtypeContainer::MapValue, TypeRef::Map(_, value)) => value,
            (container, ty) => {
                return Err(format!(
                    "transparent newtype path segment {} cannot traverse {}",
                    newtype_container_name(*container),
                    type_ref_name(ty)
                ));
            }
        };
    }
    if outer_optional {
        return Err("transparent newtype path does not traverse the outer optional value".to_string());
    }
    if current != &TypeRef::String {
        return Err(format!(
            "transparent newtype path must terminate at String, not {}",
            type_ref_name(current)
        ));
    }
    Ok(())
}

fn newtype_container_name(container: NewtypeContainer) -> &'static str {
    match container {
        NewtypeContainer::Optional => "optional",
        NewtypeContainer::Vec => "vec",
        NewtypeContainer::MapKey => "map_key",
        NewtypeContainer::MapValue => "map_value",
    }
}

fn type_ref_name(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::String => "String",
        TypeRef::Optional(_) => "Option",
        TypeRef::Vec(_) => "Vec",
        TypeRef::Map(_, _) => "Map",
        _ => "non-string type",
    }
}

/// Indicates the core Rust type wraps the resolved type in a smart pointer or cow.
/// Used by codegen to generate correct From/Into conversions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum CoreWrapper {
    #[default]
    None,
    /// `Cow<'static, str>` — binding uses String, core needs `.into()` ~keep
    Cow,
    /// `Arc<T>` — binding unwraps, core wraps with `Arc::new()` ~keep
    Arc,
    /// `bytes::Bytes` — binding uses `Vec<u8>`, core needs `Bytes::from()` ~keep
    Bytes,
    /// `Arc<Mutex<T>>` — binding wraps with `Arc::new(Mutex::new())`, methods call `.lock()` ~keep
    ArcMutex,
    /// `Box<str>` — binding uses String, core needs `.into()` (same shape as Cow
    /// but distinct so backends can keep wrapper-specific behavior addressable). ~keep
    Box,
}

/// Typed default value for a field, enabling backends to emit language-native defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DefaultValue {
    BoolLiteral(bool),
    StringLiteral(String),
    IntLiteral(i64),
    FloatLiteral(f64),
    EnumVariant(String),
    /// A tuple-variant enum default (`Mode::Custom(5)`), each positional argument folded
    /// independently and kept in source order. Distinct from [`DefaultValue::EnumVariant`],
    /// which names a bare unit-variant path with no arguments of its own.
    ///
    /// Produced only when every argument itself folds to a value-carrying `DefaultValue` (see
    /// `extract::extractor::defaults::carries_value`); a call with even one unfoldable argument
    /// keeps the whole field [`DefaultValue::Unresolved`] rather than a partially-known payload
    /// — rendering some arguments as literals and silently dropping the rest would be a subtler
    /// instance of the fabrication `Unresolved` exists to prevent. ~keep
    TupleVariant(String, Vec<DefaultValue>),
    /// A struct-variant enum default (`Kind::Curated { label: "balanced".to_string() }`), each
    /// named field folded independently and kept in source order. Same all-or-nothing rule and
    /// rationale as [`DefaultValue::TupleVariant`]. ~keep
    StructVariant(String, Vec<(String, DefaultValue)>),
    /// A zero-argument Rust function that supplies the value at runtime. ~keep
    FunctionCall(String),
    /// A public zero-argument Rust function callable from generated binding crates. ~keep
    PublicFunctionCall(String),
    /// A non-empty collection literal, holding its elements in source order.
    ///
    /// A genuinely empty `vec![]`/`Vec::new()` stays [`DefaultValue::Empty`]: the two are
    /// distinct because every backend already renders "the empty collection" natively, whereas
    /// this variant carries elements that have to be rendered individually. Only produced when
    /// every element is itself representable — anything else falls back to `Empty`, so a
    /// backend never emits a default that silently differs from the Rust one. ~keep
    ListLiteral(Vec<DefaultValue>),
    /// Empty collection or `Default::default()` — the type's own zero, and known to be exactly
    /// what the Rust default is. Contrast [`DefaultValue::Unresolved`]. ~keep
    Empty,
    /// The extractor found the type's `Default` implementation but could not read a value out
    /// of it: the body is neither a struct literal nor a delegation alef can constant-fold
    /// (`Self::builder().build()`, a `match`, a computed constructor).
    ///
    /// Distinct from [`DefaultValue::Empty`], and the distinction is the entire point of the
    /// variant. `Empty` asserts *"the default is exactly this type's zero"* — true for
    /// `#[derive(Default)]`, for `Vec::new()`, for `Default::default()` — so a backend
    /// substituting its target language's zero is exact. `Unresolved` asserts the opposite:
    /// alef does **not** know the value, and a zero would be a guess.
    ///
    /// Before this variant existed both wrote `Empty`, so one enum value carried "exact" and
    /// "guess" at once and nothing could tell them apart. Every per-field-literal backend
    /// (C#, Java, Kotlin, Swift, Python, Go) then shipped its type-zero directly underneath a
    /// generated doc comment quoting the real Rust default — the value the extractor had
    /// already read out of the same doc prose.
    ///
    /// The payload is the source text of the `fn default()` body that could not be read, so a
    /// diagnostic can name it. ~keep
    Unresolved(String),
    /// None / null
    None,
}

/// Stable identity metadata for one error variant. ~keep
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorTaxonomy {
    #[serde(default)]
    pub code: u32,
    #[serde(default)]
    pub error_type: String,
    #[serde(default)]
    pub variant: String,
}

impl ErrorTaxonomy {
    pub fn for_variant(code: u32, error_type: &str, variant: &str) -> Self {
        Self {
            code,
            error_type: error_type.to_string(),
            variant: variant.to_string(),
        }
    }
}

/// Deprecation metadata extracted from `#[deprecated(...)]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DeprecationInfo {
    /// Version when the item was deprecated (from `#[deprecated(since = "...")]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Deprecation note (from `#[deprecated(note = "...")]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Version annotation on an IR item.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct VersionAnnotation {
    /// Version when this item was introduced (from `#[alef(since = "...")]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Deprecation info (from `#[deprecated(...)]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<DeprecationInfo>,
}

/// A struct's container-level `#[serde(from/into/try_from/transparent)]`, when present.
///
/// One cohesive fact -- "how does this container convert for serde" -- kept as one `TypeDef`
/// field instead of four, so every exhaustive `TypeDef` literal in the tree pays one line of
/// churn per addition to this concept instead of four. `from`/`into`/`try_from` carry the type
/// path serde converts through and are independent of each other (a type may declare `into`
/// without `from`); `transparent` is a bare flag needing no companion type. All four are
/// `false`/`None` by default, matching every other TypeDef field this struct replaces. ~keep
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SerdeContainerConversion {
    /// Type path from `#[serde(from = "...")]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Type path from `#[serde(into = "...")]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub into: Option<String>,
    /// Type path from `#[serde(try_from = "...")]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub try_from: Option<String>,
    /// True when the struct carries `#[serde(transparent)]`.
    #[serde(default)]
    pub transparent: bool,
}

impl SerdeContainerConversion {
    /// True when any of the four attributes are present -- the condition every caller actually
    /// wants, so `TypeDef::default()`'s all-absent `SerdeContainerConversion` reads the same as
    /// "no container conversion" without each caller re-deriving that from four field checks.
    pub fn is_present(&self) -> bool {
        self.from.is_some() || self.into.is_some() || self.try_from.is_some() || self.transparent
    }
}

#[cfg(test)]
mod metadata_tests {
    use super::{ErrorTaxonomy, NewtypeContainer, NewtypeConversion, NewtypeWrapper, NewtypeWrapperMetadata, TypeRef};

    #[test]
    fn explicit_variant_code_is_preserved() {
        let taxonomy = ErrorTaxonomy::for_variant(101, "sample::RequestError", "InvalidInput");
        assert_eq!(taxonomy.code, 101);
        assert_eq!(taxonomy.error_type, "sample::RequestError");
        assert_eq!(taxonomy.variant, "InvalidInput");
    }

    #[test]
    fn legacy_serialized_taxonomy_defaults_compatibly() {
        let taxonomy: ErrorTaxonomy = serde_json::from_str("{}").expect("legacy metadata deserializes");

        assert_eq!(taxonomy, ErrorTaxonomy::default());
    }

    #[test]
    fn tuple_newtype_wrapper_preserves_legacy_string_wire_format() {
        let wrapper = NewtypeWrapper::decode("sample::Index").expect("legacy metadata decodes");

        assert_eq!(wrapper.tuple_path(), Some("sample::Index"));
        assert!(wrapper.explicit_paths().is_empty());
    }

    #[test]
    fn transparent_string_wrapper_round_trips_conversion_metadata() {
        let metadata = NewtypeWrapperMetadata::transparent_string(
            "sample::SecretString",
            "from",
            "into_inner",
            vec![NewtypeContainer::Optional, NewtypeContainer::MapValue],
        );
        let encoded = NewtypeWrapper::encode_explicit(std::slice::from_ref(&metadata));
        let decoded = NewtypeWrapper::decode(&encoded).expect("versioned metadata decodes");

        assert!(encoded.starts_with("alef:newtype-conversions:v1:"));
        assert_eq!(decoded.explicit_paths(), &[metadata]);
        assert_eq!(
            &decoded.explicit_paths()[0].conversion,
            &NewtypeConversion::TransparentString {
                from: "from".to_string(),
                into: "into_inner".to_string(),
            }
        );
    }

    #[test]
    fn malformed_versioned_newtype_metadata_is_rejected() {
        let error = NewtypeWrapper::decode("alef:newtype-conversions:v1:not-json")
            .expect_err("malformed metadata must not become a legacy tuple path");

        assert!(error.contains("invalid transparent newtype metadata"), "{error}");
    }

    #[test]
    fn empty_and_duplicate_newtype_paths_are_rejected() {
        let empty = "alef:newtype-conversions:v1:[]";
        assert!(
            NewtypeWrapper::decode(empty)
                .expect_err("empty path set")
                .contains("at least one")
        );

        let metadata = NewtypeWrapperMetadata::transparent_string("sample::Secret", "from", "into_inner", vec![]);
        let duplicate_json = serde_json::to_string(&vec![metadata.clone(), metadata]).expect("metadata serializes");
        let duplicate = format!("alef:newtype-conversions:v1:{duplicate_json}");
        assert!(
            NewtypeWrapper::decode(&duplicate)
                .expect_err("duplicate paths")
                .contains("duplicate container path")
        );
    }

    #[test]
    fn impossible_newtype_path_is_rejected_for_resolved_type() {
        let metadata = NewtypeWrapperMetadata::transparent_string(
            "sample::Secret",
            "from",
            "into_inner",
            vec![NewtypeContainer::MapKey],
        );
        let wrapper = NewtypeWrapper::decode(&NewtypeWrapper::encode_explicit(&[metadata])).expect("metadata decodes");

        let error = wrapper
            .validate_for_type(&TypeRef::String, false)
            .expect_err("map path cannot address a string");
        assert!(error.contains("map_key cannot traverse String"), "{error}");
    }
}
