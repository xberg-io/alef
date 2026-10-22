//! Finds the segments of a fixture path that step through a data-carrying enum, from the IR's
//! own type graph.
//!
//! A path such as `choices[0].message.content.parts[0].text` names two enum hops. `content` is
//! an enum with a `Parts(Vec<Piece>)` variant, and `text` is a field of the `Piece::Text`
//! variant. Neither hop is a struct field in the bindings, so a renderer that spells each
//! segment as `.name` cannot work: Rust needs a `match` on the variant (E0609 otherwise), and
//! Python reads the value below a dict-shaped variant getter as `json.loads` output, so every
//! segment after the getter is a dict lookup.
//!
//! The walk is type-driven, never name-driven. It starts at the call's result type, follows
//! struct fields through the IR's field graph, and at an enum asks which variant the segment
//! names (by the variant's own name) or declares (by a field of a struct variant). A segment
//! that names no single variant is left alone, and the renderer keeps its previous output.
use std::collections::{HashMap, HashSet};

use heck::{ToSnakeCase, ToUpperCamelCase};

use super::optional_renderers::{push_key_field_name, push_key_index_suffix};
use super::parse::segment_name;
use super::types::{FieldResolver, PathSegment, VariantShape};

/// What a variant segment binds in a `match`.
enum VariantBind {
    /// `Enum::Variant(v)`: the single field of a tuple variant.
    Tuple,
    /// `Enum::Variant { field: v, .. }`: one named field of a struct variant.
    Field(String),
}

/// One path segment that steps through an enum.
pub(super) struct VariantCrossing {
    /// The tracked path key at this segment, built the way every renderer builds it.
    pub(super) key: String,
    enum_name: String,
    variant: String,
    bind: VariantBind,
    only_variant: bool,
    /// The segment names a variant that the pyo3 binding exposes through its dict-shaped getter.
    pub(super) python_json: bool,
    /// Index of the segment in the path.
    segment_index: usize,
    /// The segment names a newtype variant of an untagged enum, whose napi and wasm value is the
    /// enum value itself.
    bare_value: bool,
}

impl VariantCrossing {
    /// The arms of the `match` that reads this segment, without the surrounding braces.
    fn rust_arms(&self, module: Option<&str>) -> String {
        let variant = &self.variant;
        let enum_name = match module {
            Some(module) => format!("{module}::{}", self.enum_name),
            None => self.enum_name.clone(),
        };
        let arm = match &self.bind {
            VariantBind::Tuple => format!("{enum_name}::{variant}(v) => v"),
            VariantBind::Field(field) => format!(
                "{enum_name}::{variant} {{ {}: v, .. }} => v",
                crate::core::keywords::rust_raw_ident(field)
            ),
        };
        if self.only_variant {
            arm
        } else {
            format!("{arm}, _ => panic!(\"expected {enum_name}::{variant}\")")
        }
    }
}

/// A variant that a segment reads, how to bind it, and the declared type of what it binds.
struct FoundVariant<'a> {
    shape: &'a VariantShape,
    bind: VariantBind,
    next: Option<String>,
    /// The segment names the variant itself, as opposed to a field the variant declares.
    by_name: bool,
}

/// The variant of `shapes` that `name` reads, with how to bind it and the declared type of what
/// it binds. A variant named like the segment wins; otherwise the one struct variant that
/// declares a field of that name. Two variants declaring the same field name is ambiguous and
/// answers `None`, because one `match` arm cannot read both.
fn find_variant<'a>(shapes: &'a [VariantShape], name: &str) -> Option<FoundVariant<'a>> {
    let pascal = name.to_upper_camel_case();
    if let Some(shape) = shapes.iter().find(|shape| shape.name == pascal)
        && let [(field, next)] = shape.fields.as_slice()
    {
        let bind = if shape.is_tuple {
            VariantBind::Tuple
        } else {
            VariantBind::Field(field.clone())
        };
        return Some(FoundVariant {
            shape,
            bind,
            next: next.clone(),
            by_name: true,
        });
    }
    let snake = name.to_snake_case();
    let declares =
        |shape: &&VariantShape| !shape.is_tuple && shape.fields.iter().any(|(field, _)| field.to_snake_case() == snake);
    let mut declaring = shapes.iter().filter(declares);
    let shape = declaring.next()?;
    if declaring.next().is_some() {
        return None;
    }
    let (field, next) = shape.fields.iter().find(|(field, _)| field.to_snake_case() == snake)?;
    Some(FoundVariant {
        shape,
        bind: VariantBind::Field(field.clone()),
        next: next.clone(),
        by_name: false,
    })
}

impl FieldResolver {
    /// Every segment of `segments` that steps through an enum, in path order.
    pub(super) fn variant_crossings(&self, segments: &[PathSegment]) -> Vec<VariantCrossing> {
        let map = &self.ir_enum_map;
        let mut crossings = Vec::new();
        let mut cursor = map.root_type.clone();
        let mut key = String::new();
        for (segment_index, segment) in segments.iter().enumerate() {
            let Some(name) = segment_name(segment) else {
                continue;
            };
            push_key_field_name(&mut key, segment);
            let Some(owner) = cursor.take() else {
                break;
            };
            cursor = if let Some(shapes) = map.variant_shapes.get(&owner) {
                let Some(FoundVariant {
                    shape,
                    bind,
                    next,
                    by_name,
                }) = find_variant(shapes, name)
                else {
                    break;
                };
                crossings.push(VariantCrossing {
                    key: key.clone(),
                    enum_name: owner.clone(),
                    variant: shape.name.clone(),
                    bind,
                    only_variant: shapes.len() == 1,
                    python_json: by_name && shape.python_json_getter,
                    segment_index,
                    bare_value: by_name && shape.untagged_newtype,
                });
                next
            } else {
                map.enum_field_types
                    .get(&owner)
                    .and_then(|fields| fields.get(name))
                    .or_else(|| map.field_types.get(&owner).and_then(|fields| fields.get(name)))
                    .cloned()
            };
            push_key_index_suffix(&mut key, segment);
        }
        crossings
    }

    /// The `match` arms that read each enum segment of `segments` in Rust, keyed by the tracked path
    /// key. A segment the renderer would emit as a method call (`parts()`) is left out: the
    /// consumer's binding has a real method there.
    pub(super) fn rust_variant_matches(&self, segments: &[PathSegment]) -> HashMap<String, String> {
        self.variant_crossings(segments)
            .into_iter()
            .filter(|crossing| {
                !(self.method_calls.contains(&crossing.key) && !self.result_fields.contains(&crossing.key))
            })
            .map(|crossing| {
                let arms = crossing.rust_arms(self.rust_enum_module.as_deref());
                (crossing.key, arms)
            })
            .collect()
    }

    /// The tracked path keys of the segments below which Python reads `json.loads` output.
    pub(super) fn python_json_crossings(&self, segments: &[PathSegment]) -> HashSet<String> {
        self.variant_crossings(segments)
            .into_iter()
            .filter(|crossing| crossing.python_json)
            .map(|crossing| crossing.key)
            .collect()
    }

    /// `segments` with each newtype-variant segment of an untagged enum folded into the segment
    /// before it, for the napi and wasm bindings, where that variant's value is the enum value
    /// itself: `content.parts[0].text` reads as `content[0].text`. A segment with no previous
    /// field segment to fold into is kept.
    pub(super) fn bare_variant_segments(&self, segments: &[PathSegment]) -> Vec<PathSegment> {
        let bare: HashSet<usize> = self
            .variant_crossings(segments)
            .into_iter()
            .filter(|crossing| crossing.bare_value)
            .map(|crossing| crossing.segment_index)
            .collect();
        let mut out: Vec<PathSegment> = Vec::with_capacity(segments.len());
        for (index, segment) in segments.iter().enumerate() {
            if bare.contains(&index)
                && let Some(PathSegment::Field(owner)) = out.last()
            {
                let folded = match segment {
                    PathSegment::Field(_) => PathSegment::Field(owner.clone()),
                    PathSegment::ArrayField { index, .. } => PathSegment::ArrayField {
                        name: owner.clone(),
                        index: *index,
                    },
                    other => {
                        out.push(other.clone());
                        continue;
                    }
                };
                out.pop();
                out.push(folded);
                continue;
            }
            out.push(segment.clone());
        }
        out
    }
}
