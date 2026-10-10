//! Getter, setter and clear-method emission for a tagged-data-enum payload struct.
//!
//! The second of the two emitters that mint `#[wasm_bindgen]` field accessors; the other is
//! `types_accessors.rs`, for ordinary structs. They share the class-backed borrow rule and its
//! two templates but not their surroundings: a payload struct stores *every* field as an
//! `Option`, whatever the variant declared, and its public method name is not derivable from the
//! struct field name (a positional `_0` becomes `field_0`). Those two differences are why this
//! is a sibling module rather than another branch inside `types_accessors`. ~keep

use ahash::AHashSet;

use super::escape_rust_keyword;
use crate::codegen::naming::to_node_name;
use crate::core::ir::FieldDef;

/// One payload field, resolved to what the payload struct actually stores it as.
pub(super) struct PayloadField {
    /// The IR field, for its name and `TypeRef`.
    pub(super) field: FieldDef,
    /// The `Option<...>` type the payload struct declares.
    pub(super) binding_type: String,
    /// The generated wasm class this field is stored as, when it is stored as one. Resolved by
    /// the caller because a field the payload struct degraded to `Option<JsValue>` is not
    /// class-backed however its own `TypeRef` reads. ~keep
    pub(super) class_type: Option<String>,
}

/// Rust identifier for the payload field itself.
fn field_ident(field: &FieldDef) -> String {
    escape_rust_keyword(&field.name)
}

/// Public method stem for a payload field.
///
/// A positional field arrives from the extractor as `_0`, which is a valid struct field but an
/// unusable method name, so the accessors expose it as `field_0`. ~keep
fn method_stem(field: &FieldDef) -> String {
    let name = field.name.as_str();
    if name.starts_with('_') && name.len() > 1 && name[1..].chars().all(|c| c.is_ascii_digit()) {
        format!("field_{}", &name[1..])
    } else {
        escape_rust_keyword(name)
    }
}

/// Every identifier the payload `impl` block already mints, so a generated `clear{Field}()`
/// stands down rather than collide with one as `E0592`.
///
/// Built from what this emitter itself writes -- the constructor, the `default()` factory, the
/// tag accessors and every field accessor -- not from `enum_def`'s IR methods: a payload struct
/// is synthesised, so the consumer's own enum methods are not on it and the identifiers that
/// *are* come from here. ~keep
pub(super) fn reserved_payload_idents(
    tag_ident: &str,
    tag_setter_ident: &str,
    fields: &[PayloadField],
) -> AHashSet<String> {
    let mut reserved: AHashSet<String> = ["new".to_string(), "default".to_string()].into_iter().collect();
    reserved.insert(tag_ident.to_string());
    reserved.insert(tag_setter_ident.to_string());
    for payload in fields {
        let stem = method_stem(&payload.field);
        reserved.insert(format!("set_{stem}"));
        reserved.insert(stem);
    }
    reserved
}

/// Emit the accessor block for every payload field, already indented into the `impl`.
///
/// `flattened_only` names the fields serde folds into the tag object, which therefore get no JS
/// property at all -- the struct field stays, because both `From` impls read it.
pub(super) fn gen_payload_accessors(
    fields: &[PayloadField],
    flattened_only: &std::collections::BTreeSet<&str>,
    reserved_idents: &AHashSet<String>,
) -> Vec<String> {
    let mut lines = Vec::new();
    for payload in fields {
        if flattened_only.contains(payload.field.name.as_str()) {
            continue;
        }
        lines.push(String::new());
        lines.extend(indented(&gen_payload_getter(payload)));
        lines.extend(indented(&gen_payload_setter(payload)));
        if let Some(clear) = gen_payload_clear(payload, reserved_idents) {
            lines.extend(indented(&clear));
        }
    }
    lines
}

fn gen_payload_getter(payload: &PayloadField) -> String {
    crate::backends::wasm::template_env::render(
        "gen_payload_field_getter",
        crate::alef_context! {
            js_name => to_node_name(&payload.field.name),
            getter_ident => method_stem(&payload.field),
            field_ident => field_ident(&payload.field),
            binding_type => payload.binding_type,
        },
    )
}

/// The payload setter, borrowed when the field is stored as a generated class.
///
/// `optional` is unconditionally true here: a payload struct holds the union of every variant's
/// fields, so each one is an `Option` whether or not the variant declared it that way. The
/// struct path's `stores_option` predicate is therefore the wrong question to ask. ~keep
fn gen_payload_setter(payload: &PayloadField) -> String {
    let Some(class_type) = payload.class_type.as_deref() else {
        return crate::backends::wasm::template_env::render(
            "gen_payload_field_setter",
            crate::alef_context! {
                js_name => to_node_name(&payload.field.name),
                setter_ident => format!("set_{}", method_stem(&payload.field)),
                field_ident => field_ident(&payload.field),
                binding_type => payload.binding_type,
            },
        );
    };
    crate::backends::wasm::template_env::render(
        "gen_class_field_setter",
        crate::alef_context! {
            js_name_attr => format!(", js_name = \"{}\"", to_node_name(&payload.field.name)),
            setter_ident => format!("set_{}", method_stem(&payload.field)),
            field_ident => field_ident(&payload.field),
            class_type => class_type,
            optional => true,
        },
    )
}

/// The `clear{Field}()` companion, for a class-backed payload field.
///
/// Every class-backed payload field needs one, unlike the struct path where only the optional
/// subset does: the borrowed setter cannot accept `null` (`Option<&T>` has no
/// `OptionFromWasmAbi` impl) and the payload field is always an `Option`, so without this it
/// could be set but never unset. ~keep
fn gen_payload_clear(payload: &PayloadField, reserved_idents: &AHashSet<String>) -> Option<String> {
    payload.class_type.as_ref()?;
    let clear_ident = format!("clear_{}", method_stem(&payload.field));
    if reserved_idents.contains(&clear_ident) {
        return None;
    }
    Some(crate::backends::wasm::template_env::render(
        "gen_class_field_clear",
        crate::alef_context! {
            js_name => to_node_name(&clear_ident),
            clear_ident => clear_ident,
            field_ident => field_ident(&payload.field),
        },
    ))
}

/// Indent a rendered block into the `impl` body, the way `ImplBuilder` does for the struct path.
fn indented(block: &str) -> Vec<String> {
    block
        .trim_end()
        .lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                crate::codegen::template_env::render(
                    "builders/indented_line.jinja",
                    crate::alef_context! { line => line },
                )
                .trim_end()
                .to_string()
            }
        })
        .collect()
}
