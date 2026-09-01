use crate::core::ir::{ApiSurface, DefaultValue, FieldDef, PrimitiveType, TypeDef, TypeRef};
use heck::{ToPascalCase, ToShoutySnakeCase, ToSnakeCase};

/// Returns true if a field is a tuple struct positional field (e.g., `_0`, `_1`, `0`, `1`).
/// These fields have no meaningful name and must be skipped in languages requiring named fields.
pub(super) fn is_tuple_field(field: &FieldDef) -> bool {
    (field.name.starts_with('_') && field.name[1..].chars().all(|c| c.is_ascii_digit()))
        || field.name.chars().next().is_none_or(|c| c.is_ascii_digit())
}

/// Returns true if the Rust default value for a field is its type's inherent default,
/// meaning `.unwrap_or_default()` can be used instead of `.unwrap_or(value)`.
/// This avoids clippy::unwrap_or_default warnings.
pub(super) fn use_unwrap_or_default(field: &FieldDef) -> bool {
    if let Some(typed_default) = &field.typed_default {
        return matches!(typed_default, DefaultValue::Empty | DefaultValue::None);
    }
    field.default.is_none() && !matches!(&field.ty, TypeRef::Named(_))
}

pub(super) fn constructor_fields(typ: &TypeDef) -> impl Iterator<Item = &FieldDef> {
    typ.fields.iter().filter(|field| !field.binding_excluded)
}

/// Fails when a `#[serde(default = "path")]` function cannot be reproduced in the generated
/// bindings: alef cannot call a private or feature-gated default, and
/// `rust_default_via_source_deserialize` could not recover the value through the owning type's
/// `Deserialize` impl either. ~keep
pub(crate) fn validate_rust_default_functions(api: &ApiSurface) -> anyhow::Result<()> {
    let failures: Vec<_> = api
        .types
        .iter()
        .filter(|typ| !typ.binding_excluded)
        .flat_map(|typ| {
            typ.fields
                .iter()
                .filter(|field| !field.binding_excluded)
                .filter_map(move |field| {
                    let DefaultValue::FunctionCall(path) = field.typed_default.as_ref()? else {
                        return None;
                    };
                    rust_default_via_source_deserialize(field, typ).is_none().then(|| {
                        format!(
                            "- `{}::{}` uses `#[serde(default = \"{}\")]`",
                            typ.rust_path, field.name, path
                        )
                    })
                })
        })
        .collect();

    if failures.is_empty() {
        return Ok(());
    }

    anyhow::bail!(
        "cannot preserve {} serde default function(s) in generated Rust bindings:\n{}\n\
         For each field, expose a public, unconditional, zero-argument static method and reference it by its \
         fully qualified owner path (`Settings::default_retry_limit`, not `Self::default_retry_limit`), or \
         replace the function default with a literal.",
        failures.len(),
        failures.join("\n")
    )
}

pub fn default_value_for_field(field: &FieldDef, language: &str) -> String {
    if let Some(typed_default) = &field.typed_default {
        return match typed_default {
            DefaultValue::BoolLiteral(b) => match language {
                "python" => {
                    if *b {
                        "True".to_string()
                    } else {
                        "False".to_string()
                    }
                }
                "ruby" => {
                    if *b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
                "go" => {
                    if *b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
                "java" => {
                    if *b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
                "csharp" => {
                    if *b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
                "php" => {
                    if *b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
                "r" => {
                    if *b {
                        "TRUE".to_string()
                    } else {
                        "FALSE".to_string()
                    }
                }
                "rust" => {
                    if *b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
                _ => {
                    if *b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                }
            },
            DefaultValue::StringLiteral(s) => match language {
                "rust" => format!("\"{}\".to_string()", s.replace('"', "\\\"")),
                _ => format!("\"{}\"", s.replace('"', "\\\"")),
            },
            DefaultValue::IntLiteral(n) => n.to_string(),
            DefaultValue::FloatLiteral(f) => {
                let s = f.to_string();
                if !s.contains('.') { format!("{}.0", s) } else { s }
            }
            DefaultValue::EnumVariant(v) => {
                if matches!(field.ty, TypeRef::String) {
                    let snake = v.to_snake_case();
                    return match language {
                        "rust" => format!("\"{}\".to_string()", snake),
                        _ => format!("\"{}\"", snake),
                    };
                }
                match language {
                    "python" => format!("{}.{}", field.ty.type_name(), v.to_shouty_snake_case()),
                    "ruby" => format!("{}::{}", field.ty.type_name(), v.to_pascal_case()),
                    "go" => format!("{}{}", field.ty.type_name(), v.to_pascal_case()),
                    "java" => format!("{}.{}", field.ty.type_name(), v.to_shouty_snake_case()),
                    "csharp" => format!("{}.{}", field.ty.type_name(), v.to_pascal_case()),
                    "php" => format!("{}::{}", field.ty.type_name(), v.to_pascal_case()),
                    "r" => format!("{}${}", field.ty.type_name(), v.to_pascal_case()),
                    "rust" => format!("{}::{}", field.ty.type_name(), v.to_pascal_case()),
                    _ => v.clone(),
                }
            }
            // Rendered per language wherever the syntax is a plain bracketed list. Go needs the
            // element type spelled out (`[]string{…}`) and R's `c()` carries vector-coercion
            // semantics of its own, so both fall through to the empty-collection rendering
            // rather than being guessed at here — a wrong default is worse than a missing one. ~keep
            DefaultValue::ListLiteral(items) => {
                let rendered: Option<Vec<String>> =
                    items.iter().map(|item| config_scalar_default(item, language)).collect();
                match (rendered, language) {
                    (Some(values), "python" | "ruby" | "csharp" | "php") => format!("[{}]", values.join(", ")),
                    (Some(values), "java") => format!("List.of({})", values.join(", ")),
                    (Some(values), "rust") => format!("vec![{}]", values.join(", ")),
                    _ => match language {
                        "python" | "ruby" | "csharp" | "php" => "[]".to_string(),
                        "go" => "nil".to_string(),
                        "java" => "List.of()".to_string(),
                        "r" => "c()".to_string(),
                        "rust" => "vec![]".to_string(),
                        _ => "null".to_string(),
                    },
                }
            }
            DefaultValue::Empty => match &field.ty {
                TypeRef::Vec(_) => match language {
                    "python" | "ruby" | "csharp" => "[]".to_string(),
                    "go" => "nil".to_string(),
                    "java" => "List.of()".to_string(),
                    "php" => "[]".to_string(),
                    "r" => "c()".to_string(),
                    "rust" => "vec![]".to_string(),
                    _ => "null".to_string(),
                },
                TypeRef::Map(_, _) => match language {
                    "python" => "{}".to_string(),
                    "go" => "nil".to_string(),
                    "java" => "Map.of()".to_string(),
                    "rust" => "Default::default()".to_string(),
                    _ => "null".to_string(),
                },
                TypeRef::Primitive(p) => match p {
                    PrimitiveType::Bool => match language {
                        "python" => "False".to_string(),
                        "ruby" => "false".to_string(),
                        _ => "false".to_string(),
                    },
                    PrimitiveType::F32 | PrimitiveType::F64 => "0.0".to_string(),
                    _ => "0".to_string(),
                },
                TypeRef::String | TypeRef::Char | TypeRef::Path => match language {
                    "rust" => "String::new()".to_string(),
                    _ => "\"\"".to_string(),
                },
                TypeRef::Json => match language {
                    "python" | "ruby" => "{}".to_string(),
                    "go" => "json.RawMessage(nil)".to_string(),
                    "java" => "new com.fasterxml.jackson.databind.node.ObjectNode(null)".to_string(),
                    "csharp" => "JObject.Parse(\"{}\")".to_string(),
                    "php" => "[]".to_string(),
                    "r" => "list()".to_string(),
                    "rust" => "serde_json::json!({})".to_string(),
                    _ => "{}".to_string(),
                },
                TypeRef::Duration => "0".to_string(),
                TypeRef::Bytes => match language {
                    "python" => "b\"\"".to_string(),
                    "go" => "[]byte{}".to_string(),
                    "rust" => "vec![]".to_string(),
                    _ => "\"\"".to_string(),
                },
                _ => match language {
                    "python" => "None".to_string(),
                    "ruby" => "nil".to_string(),
                    "go" => "nil".to_string(),
                    "rust" => "Default::default()".to_string(),
                    _ => "null".to_string(),
                },
            },
            // Distinct from `Empty` on purpose: `Empty` asserts "the default IS the type's own
            // zero", which is exactly what the type-zero table above renders. `Unresolved`
            // asserts alef could not read the value at all — rendering the same table here would
            // ship a value the source crate never actually specified, underneath a doc comment
            // quoting the real (unreadable) default. This has no type context to attempt the
            // `core_default_field_access` recovery `default_value_for_field_in_type` does for
            // `"rust"`, so it fails loudly there and — like `FunctionCall`/`TupleVariant` below —
            // answers every other language with "no value" rather than a guess. ~keep
            DefaultValue::Unresolved(_) => match language {
                "python" => "None".to_string(),
                "ruby" => "nil".to_string(),
                "go" => "nil".to_string(),
                "rust" => format!(
                    "compile_error!(r#\"cannot render an unresolved `impl Default` for field `{field_name}` \
                     without its owning type context\"#)",
                    field_name = field.name,
                ),
                _ => "null".to_string(),
            },
            DefaultValue::None => match language {
                "python" => "None".to_string(),
                "ruby" => "nil".to_string(),
                "go" => "nil".to_string(),
                "java" => "null".to_string(),
                "csharp" => "null".to_string(),
                "php" => "null".to_string(),
                "r" => "NULL".to_string(),
                "rust" => "None".to_string(),
                _ => "null".to_string(),
            },
            // ~keep `path` names a zero-arg function in the *source* crate, reached from
            // `#[serde(default = "path")]`. Emitting `path()` into a generated binding
            // crate cannot compile: the function is not `pub`, is frequently
            // `#[cfg(feature = "serde")]`-gated, and is never imported. Callers that know
            // the owning `TypeDef` MUST go through [`default_value_for_field_in_type`]
            // instead, which recovers the *real* value by deserializing an empty-field
            // JSON stub through the source type's own `Deserialize` impl (the same
            // mechanism `#[serde(default = "path")]` itself relies on) rather than
            // guessing. This arm has no type context and so cannot attempt that recovery.
            // It fails loudly rather than substituting `Default::default()`: the substitute
            // compiles and looks right while silently shipping the field type's zero value,
            // which is a different number than the source crate's (one consumer's
            // `default_span()` is 1, `u32::default()` is 0). A generated binding must never
            // disagree with its own source crate about a default.
            DefaultValue::FunctionCall(path) => match language {
                "python" => "None".to_string(),
                "ruby" => "nil".to_string(),
                "go" => "nil".to_string(),
                "rust" => format!(
                    "compile_error!(r#\"cannot preserve serde default function `{path}` for field \
                     `{field_name}` without its owning type context\"#)",
                    field_name = field.name,
                ),
                _ => "null".to_string(),
            },
            DefaultValue::PublicFunctionCall(path) => match language {
                "python" => "None".to_string(),
                "ruby" => "nil".to_string(),
                "go" => "nil".to_string(),
                "rust" => format!("{path}()"),
                _ => "null".to_string(),
            },
            // Neither payload is a spellable literal in any target language on its own — the
            // enclosing enum's path is not part of either variant, only the bare variant/field
            // names are. Callers that know the owning `TypeDef` MUST go through
            // [`default_value_for_field_in_type`] instead, which recovers the real value by
            // reading it back off `<CoreType as Default>::default().field` when the owning type
            // has one. This arm has no type context and so cannot attempt that recovery; it
            // fails loudly for "rust" (the one language this ever compiles) rather than
            // substituting `Default::default()`, for the same reason `FunctionCall` does above. ~keep
            DefaultValue::TupleVariant(variant, _) | DefaultValue::StructVariant(variant, _) => match language {
                "python" => "None".to_string(),
                "ruby" => "nil".to_string(),
                "go" => "nil".to_string(),
                "rust" => format!(
                    "compile_error!(r#\"cannot render tuple/struct-variant default `{variant}` for field \
                     `{field_name}` without its owning type context\"#)",
                    field_name = field.name,
                ),
                _ => "null".to_string(),
            },
        };
    }

    // `#[serde(default)]` as a "/* serde(default) */" placeholder and
    // `#[serde(default = "path")]` as a `serde(default = "path")` marker. Both are
    if let Some(default_str) = &field.default
        && default_str != "/* serde(default) */"
        && !default_str.starts_with("serde(default = \"")
    {
        return default_str.clone();
    }

    match &field.ty {
        TypeRef::Primitive(p) => match p {
            crate::core::ir::PrimitiveType::Bool => match language {
                "python" => "False".to_string(),
                "ruby" => "false".to_string(),
                "csharp" => "false".to_string(),
                "java" => "false".to_string(),
                "php" => "false".to_string(),
                "r" => "FALSE".to_string(),
                _ => "false".to_string(),
            },
            crate::core::ir::PrimitiveType::U8
            | crate::core::ir::PrimitiveType::U16
            | crate::core::ir::PrimitiveType::U32
            | crate::core::ir::PrimitiveType::U64
            | crate::core::ir::PrimitiveType::I8
            | crate::core::ir::PrimitiveType::I16
            | crate::core::ir::PrimitiveType::I32
            | crate::core::ir::PrimitiveType::I64
            | crate::core::ir::PrimitiveType::Usize
            | crate::core::ir::PrimitiveType::Isize => "0".to_string(),
            crate::core::ir::PrimitiveType::F32 | crate::core::ir::PrimitiveType::F64 => "0.0".to_string(),
        },
        TypeRef::String | TypeRef::Char => match language {
            "python" => "\"\"".to_string(),
            "ruby" => "\"\"".to_string(),
            "go" => "\"\"".to_string(),
            "java" => "\"\"".to_string(),
            "csharp" => "\"\"".to_string(),
            "php" => "\"\"".to_string(),
            "r" => "\"\"".to_string(),
            "rust" => "String::new()".to_string(),
            _ => "\"\"".to_string(),
        },
        TypeRef::Bytes => match language {
            "python" => "b\"\"".to_string(),
            "ruby" => "\"\"".to_string(),
            "go" => "[]byte{}".to_string(),
            "java" => "new byte[]{}".to_string(),
            "csharp" => "new byte[]{}".to_string(),
            "php" => "\"\"".to_string(),
            "r" => "raw()".to_string(),
            "rust" => "vec![]".to_string(),
            _ => "[]".to_string(),
        },
        TypeRef::Optional(_) => match language {
            "python" => "None".to_string(),
            "ruby" => "nil".to_string(),
            "go" => "nil".to_string(),
            "java" => "null".to_string(),
            "csharp" => "null".to_string(),
            "php" => "null".to_string(),
            "r" => "NULL".to_string(),
            "rust" => "None".to_string(),
            _ => "null".to_string(),
        },
        TypeRef::Vec(_) => match language {
            "python" => "[]".to_string(),
            "ruby" => "[]".to_string(),
            "go" => "[]interface{}{}".to_string(),
            "java" => "new java.util.ArrayList<>()".to_string(),
            "csharp" => "[]".to_string(),
            "php" => "[]".to_string(),
            "r" => "c()".to_string(),
            "rust" => "vec![]".to_string(),
            _ => "[]".to_string(),
        },
        TypeRef::Map(_, _) => match language {
            "python" => "{}".to_string(),
            "ruby" => "{}".to_string(),
            "go" => "make(map[string]interface{})".to_string(),
            "java" => "new java.util.HashMap<>()".to_string(),
            "csharp" => "new Dictionary<string, object>()".to_string(),
            "php" => "[]".to_string(),
            "r" => "list()".to_string(),
            "rust" => "std::collections::HashMap::new()".to_string(),
            _ => "{}".to_string(),
        },
        TypeRef::Json => match language {
            "python" => "{}".to_string(),
            "ruby" => "{}".to_string(),
            "go" => "json.RawMessage(nil)".to_string(),
            "java" => "new com.fasterxml.jackson.databind.JsonNode()".to_string(),
            "csharp" => "JObject.Parse(\"{}\")".to_string(),
            "php" => "[]".to_string(),
            "r" => "list()".to_string(),
            "rust" => "serde_json::json!({})".to_string(),
            _ => "{}".to_string(),
        },
        TypeRef::Named(name) => match language {
            "rust" => format!("{name}::default()"),
            "python" => "None".to_string(),
            "ruby" => "nil".to_string(),
            "go" => "nil".to_string(),
            "java" => "null".to_string(),
            "csharp" => "null".to_string(),
            "php" => "null".to_string(),
            "r" => "NULL".to_string(),
            _ => "null".to_string(),
        },
        _ => match language {
            "python" => "None".to_string(),
            "ruby" => "nil".to_string(),
            "go" => "nil".to_string(),
            "java" => "null".to_string(),
            "csharp" => "null".to_string(),
            "php" => "null".to_string(),
            "r" => "NULL".to_string(),
            "rust" => "Default::default()".to_string(),
            _ => "null".to_string(),
        },
    }
}

/// Like [`default_value_for_field`], but for callers that know the `TypeDef` the field
/// belongs to. This is the entry point every "rust"-emitting caller (Magnus, PHP, NAPI,
/// Rustler) must use: it is the only place that can recover a `#[serde(default = "path")]`
/// field's *true* value instead of silently substituting the field type's own default.
///
/// For every other `DefaultValue` variant this simply forwards to
/// [`default_value_for_field`]. For `DefaultValue::FunctionCall` on `"rust"`, see
/// [`rust_default_via_source_deserialize`] for the recovery strategy. When recovery is not
/// possible, generation fails with a message naming the crate, type, field, and the
/// uncallable function, per the constraint that a binding must never silently ship a value
/// that differs from the source crate's.
pub fn default_value_for_field_in_type(field: &FieldDef, language: &str, typ: &TypeDef) -> String {
    if language == "rust"
        && let Some(DefaultValue::FunctionCall(path)) = &field.typed_default
    {
        if let Some(expr) = rust_default_via_source_deserialize(field, typ) {
            return expr;
        }
        let crate_name = typ.rust_path.split("::").next().unwrap_or(typ.rust_path.as_str());
        return format!(
            "compile_error!(r#\"cannot preserve serde default function `{path}` for \
             `{crate_name}::{type_name}.{field_name}`; expose a public unconditional static method with a fully \
             qualified owner path or use an Alef-visible literal\"#)",
            type_name = typ.name,
            field_name = field.name,
        );
    }
    // Neither payload is a spellable Rust literal on its own — the enclosing enum's path is
    // not part of either variant, only the bare variant/field names are — but the owning
    // struct's own `Default` impl, when it has one, already computed the real value; reading
    // it back off `<CoreType as Default>::default().field` (the same recovery
    // `codegen::shared::format_default_value` uses for `EnumVariant`) asks the source crate
    // instead of guessing at a spelling. ~keep
    if language == "rust"
        && let Some(DefaultValue::TupleVariant(variant, _) | DefaultValue::StructVariant(variant, _)) =
            &field.typed_default
    {
        if let Some(expr) = crate::codegen::shared::core_default_field_access(field, typ) {
            return expr;
        }
        return format!(
            "compile_error!(r#\"cannot render tuple/struct-variant default `{variant}` for \
             `{type_path}.{field_name}` without a `Default` impl on the owning type to read it back \
             from\"#)",
            type_path = owning_type_path(typ),
            field_name = field.name,
        );
    }
    // `Unresolved` means alef could not constant-fold `impl Default`'s body, not that no such
    // impl exists — `extract_default_values` only marks a field `Unresolved` after finding a
    // real `fn default()` it could not read through. That impl is real, compiled Rust the
    // generated crate can call directly: `<CoreType as Default>::default().field` (the same
    // recovery `TupleVariant`/`StructVariant` use above) asks the source crate for the actual
    // value instead of guessing at a spelling — sidestepping alef's static-analysis limit
    // entirely rather than merely refusing to guess. ~keep
    if language == "rust"
        && let Some(DefaultValue::Unresolved(_)) = &field.typed_default
    {
        if let Some(expr) = crate::codegen::shared::core_default_field_access(field, typ) {
            return expr;
        }
        return format!(
            "compile_error!(r#\"cannot render an unresolved `impl Default` for \
             `{type_path}.{field_name}` without a `Default` impl on the owning type to read it back \
             from\"#)",
            type_path = owning_type_path(typ),
            field_name = field.name,
        );
    }
    default_value_for_field(field, language)
}

/// How a `compile_error!` diagnostic names the type that owns an unrenderable field.
///
/// Uses `rust_path` whole rather than splicing its first segment onto `typ.name`. The splice
/// produced a path that need not exist: it dropped every intermediate module, rendering
/// `demo::inner::Settings` as `demo::Settings`, and it silently disagreed with itself whenever
/// `rust_path` and `name` named different types. A diagnostic that misnames the type it is
/// pointing at sends the reader to the wrong definition, which is worse than no diagnostic. ~keep
fn owning_type_path(typ: &TypeDef) -> &str {
    if typ.rust_path.is_empty() {
        &typ.name
    } else {
        &typ.rust_path
    }
}

/// Recover a `#[serde(default = "path")]` field's true value by deserializing a minimal
/// JSON object through the owning type's own `Deserialize` impl, mirroring the exact
/// mechanism serde itself uses `path()` for: every sibling field serde will genuinely fill
/// when its wire key is absent — one with its own field-level serde default, one covered by
/// a container-level `#[serde(default)]`, or `Option<T>` — is omitted from the JSON, so
/// serde fills it with its real, source-crate-computed value. `field` itself — the one this
/// call is solving for — is omitted unconditionally, never by falling through to the same
/// `has_own_default`/`is_optional` test the other siblings use (see the loop's own comment):
/// that test is not a reliable signal for `field` in particular. Every other (truly required)
/// sibling gets a placeholder value, since its presence is needed only to make the object
/// deserialize at all and does not affect the field being solved for.
///
/// Returns `None` when this cannot be done with confidence, in which case the caller must
/// fail generation rather than guess:
/// - the type does not derive both `Serialize` and `Deserialize`,
/// - the type has `#[cfg]`-gated fields alef cannot know are present in the compiled crate,
/// - any field is a tuple-struct positional field (tuple structs serialize as JSON arrays,
///   not objects),
/// - any field carries `#[serde(flatten)]` (its wire shape is not a single known key), or
/// - a required sibling's type has no safe placeholder value (see
///   [`json_placeholder_literal`]).
fn rust_default_via_source_deserialize(field: &FieldDef, typ: &TypeDef) -> Option<String> {
    if !typ.has_serde || typ.has_stripped_cfg_fields || typ.rust_path.is_empty() {
        return None;
    }
    if typ.fields.iter().any(is_tuple_field) {
        return None;
    }

    let mut placeholders = Vec::new();
    for sibling in &typ.fields {
        if sibling.serde_flatten || sibling.cfg.is_some() {
            return None;
        }
        // The field being solved for must never get a placeholder in its own probe JSON: the
        // whole mechanism depends on its wire key being *absent*, so serde fills it with the
        // real value `path()`/`impl Default` computes — the value this function exists to
        // recover. Before this check, `field` fell through to the same `has_own_default`/
        // `is_optional` test as any other sibling, and neither one reliably holds for it: a
        // `FunctionCall` read from a manual `impl Default`'s call expression (rather than a
        // field-level `#[serde(default = "path")]` attribute) leaves `field.default` at `None`,
        // so the field looked exactly like a genuinely required sibling and got a
        // `json_placeholder_literal` type-zero (`0`, `""`, `false`) forced into its own key —
        // well-formed Rust that silently deserialized back to that placeholder instead of the
        // real default. Skipping the field unconditionally, ahead of that test, is what actually
        // omits its key from the probe body. ~keep
        if sibling.name == field.name {
            continue;
        }
        // `sibling.typed_default.is_some()` is NOT a substitute for "serde will fill this
        // field if its wire key is absent": `#[derive(Default)]` seeds `typed_default =
        // Some(Empty)` on every field of the container (`extract::extractor::types::
        // extract_struct`), including ones with no serde attribute at all, so it is
        // essentially always `Some` for any field on a `derive(Default)` type. Using it here
        // omitted every required sibling from the probe JSON, leaving `placeholders` empty
        // and the caller emitting `serde_json::from_str(r#"{}"#)`, which panics at runtime
        // the moment a required field is genuinely missing. `sibling.default` is the durable,
        // narrower signal: `extract::extractor::helpers::fields::extract_field` sets it only
        // from a real field-level `#[serde(default)]`/`#[serde(default = "path")]` attribute,
        // and nothing downstream overwrites it — unlike `typed_default`, which a later
        // `#[derive(Default)]` or manual `impl Default` unconditionally clobbers. A
        // container-level `#[serde(default)]` (`typ.serde_container_default`) makes every
        // field on the type absent-tolerant regardless of its own attributes. This mirrors
        // `backends::go::gen_bindings::types::helpers::needs_omitempty_pointer`, which gates
        // the same decision on `field.default.is_none() && !typ.serde_container_default` and
        // documents `has_default` itself as inadmissible for exactly this reason. ~keep
        let has_own_default = sibling.default.is_some() || typ.serde_container_default;
        let is_optional = sibling.optional || matches!(&sibling.ty, TypeRef::Optional(_));
        if has_own_default || is_optional {
            continue;
        }
        let value = json_placeholder_literal(&sibling.ty)?;
        let key = crate::codegen::naming::wire_field_name(
            &sibling.name,
            sibling.serde_rename.as_deref(),
            typ.serde_rename_all.as_deref(),
        );
        placeholders.push(format!("{key:?}:{value}"));
    }

    let core_path = typ.rust_path.replace('-', "_");
    let json_body = placeholders.join(",");
    Some(format!(
        "serde_json::from_str::<{core_path}>(r#\"{{{json_body}}}\"#).expect(\"alef-generated default JSON for \
         `{type_name}` failed to deserialize\").{field_name}",
        type_name = typ.name,
        field_name = field.name,
    ))
}

/// A safe placeholder JSON literal for a field that is genuinely required (no serde
/// default, not `Option<T>`) during the empty-field deserialization in
/// [`rust_default_via_source_deserialize`]. The value is never observed by the field
/// actually being solved for — it exists only so the containing object deserializes.
/// Returns `None` for shapes that are not safely representable without deeper, per-type
/// knowledge (nested named types, `Duration`, raw byte buffers), which makes the caller
/// fail generation instead of fabricating a value that might not round-trip.
fn json_placeholder_literal(ty: &TypeRef) -> Option<String> {
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => Some("false".to_string()),
        TypeRef::Primitive(PrimitiveType::F32 | PrimitiveType::F64) => Some("0.0".to_string()),
        TypeRef::Primitive(_) => Some("0".to_string()),
        TypeRef::String | TypeRef::Path => Some("\"\"".to_string()),
        // A one-character JSON string: serde's `char` deserializer requires exactly one
        // Unicode scalar value, so an empty string would fail.
        TypeRef::Char => Some("\"\\u0000\"".to_string()),
        TypeRef::Vec(_) => Some("[]".to_string()),
        TypeRef::Map(_, _) => Some("{}".to_string()),
        TypeRef::Json => Some("null".to_string()),
        TypeRef::Bytes | TypeRef::Duration | TypeRef::Unit | TypeRef::Optional(_) | TypeRef::Named(_) => None,
    }
}

trait TypeRefExt {
    fn type_name(&self) -> String;
}

impl TypeRefExt for TypeRef {
    fn type_name(&self) -> String {
        match self {
            TypeRef::Named(n) => n.clone(),
            TypeRef::Primitive(p) => format!("{:?}", p),
            TypeRef::String | TypeRef::Char => "String".to_string(),
            TypeRef::Bytes => "Bytes".to_string(),
            TypeRef::Optional(inner) => format!("Option<{}>", inner.type_name()),
            TypeRef::Vec(inner) => format!("Vec<{}>", inner.type_name()),
            TypeRef::Map(k, v) => format!("Map<{}, {}>", k.type_name(), v.type_name()),
            TypeRef::Path => "Path".to_string(),
            TypeRef::Unit => "()".to_string(),
            TypeRef::Json => "Json".to_string(),
            TypeRef::Duration => "Duration".to_string(),
        }
    }
}

/// Render one element of a collection-literal default for `language`.
///
/// Scalar-only: a nested list, an empty marker and a function-call default all need context this
/// element position does not carry, so they return `None` and the caller falls back to the empty
/// collection for the whole field. ~keep
fn config_scalar_default(item: &DefaultValue, language: &str) -> Option<String> {
    match item {
        DefaultValue::BoolLiteral(b) => Some(match language {
            "python" => {
                if *b {
                    "True".to_string()
                } else {
                    "False".to_string()
                }
            }
            _ => b.to_string(),
        }),
        DefaultValue::StringLiteral(s) => Some(match language {
            // A `Vec<String>` element must be an owned `String`, not the `&'static str` a bare
            // string literal produces — `vec!["noscript"]` fails to type-check against
            // `Vec<String>` (E0308). Every other language's collection literal accepts a bare
            // string literal for its elements, so only "rust" needs the conversion. ~keep
            "rust" => format!("\"{}\".to_string()", s.escape_default()),
            _ => format!("\"{}\"", s.escape_default()),
        }),
        DefaultValue::IntLiteral(i) => Some(i.to_string()),
        DefaultValue::FloatLiteral(f) => Some(f.to_string()),
        DefaultValue::ListLiteral(_)
        | DefaultValue::EnumVariant(_)
        | DefaultValue::TupleVariant(_, _)
        | DefaultValue::StructVariant(_, _)
        | DefaultValue::Empty
        | DefaultValue::Unresolved(_)
        | DefaultValue::None
        | DefaultValue::FunctionCall(_)
        | DefaultValue::PublicFunctionCall(_) => None,
    }
}
