use super::renderers::{TailSeg, parse_tail};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{TypeDef, TypeRef};
use crate::e2e::field_access::SwiftFirstClassMap;

pub(crate) struct UsageAccessContext<'a> {
    pub language: Language,
    pub item_type: &'a str,
    pub types: &'a [TypeDef],
    pub config: &'a ResolvedCrateConfig,
    pub swift_map: Option<&'a SwiftFirstClassMap>,
}

impl UsageAccessContext<'_> {
    pub fn accessor(&self, path: &str, chunks: &str) -> Option<String> {
        self.typed_accessor(path, chunks).map(|(expression, _)| expression)
    }

    /// [`Self::accessor`] plus the declared type of the value it reads, so a backend whose
    /// binding widens a primitive (Dart's `u64` → `BigInt`) can type the expected literal.
    pub fn typed_accessor(&self, path: &str, chunks: &str) -> Option<(String, TypeRef)> {
        if !super::is_stream_scoped_field(path) {
            return None;
        }
        let mut current = TypeRef::Named(self.item_type.into());
        let mut optional = true;
        let last = if self.language == Language::Swift {
            "last"
        } else {
            "lastOrNull"
        };
        let mut expression = format!("{chunks}.{last}");
        for segment in parse_tail(&format!(".{path}")) {
            match segment {
                TailSeg::Field(name) => {
                    let owner = self.owner(&current)?;
                    let field = crate::codegen::shared::binding_fields(&owner.fields).find(|field| {
                        field.name == name
                            || crate::codegen::naming::wire_field_name(
                                &field.name,
                                field.serde_rename.as_deref(),
                                owner.serde_rename_all.as_deref(),
                            ) == name
                    })?;
                    let (property, next_optional) = self.field_access(owner, field)?;
                    expression.push_str(if optional { "?." } else { "." });
                    expression.push_str(&property);
                    optional = next_optional;
                    current = field.ty.clone();
                }
                TailSeg::Index(index) => {
                    let TypeRef::Vec(inner) = unwrapped(&current) else {
                        return None;
                    };
                    if optional {
                        expression.push('?');
                    }
                    expression.push_str(&format!("[{index}]"));
                    current = *inner.clone();
                    optional = matches!(current, TypeRef::Optional(_));
                }
            }
        }
        Some((expression, current))
    }

    fn field_access(&self, owner: &TypeDef, field: &crate::core::ir::FieldDef) -> Option<(String, bool)> {
        let declared_optional = field.optional || matches!(field.ty, TypeRef::Optional(_));
        if self.language == Language::Swift
            && let Some(map) = self.swift_map.filter(|map| !map.is_first_class(Some(&owner.name)))
        {
            if map.json_bridged_getter(&owner.name, &field.name) == Some(true) {
                return None;
            }
            let method = crate::codegen::naming::escape_identifier_for(
                Language::Swift,
                &field.name,
                crate::codegen::naming::IdentifierContext::SwiftRustShim,
            );
            return Some((
                format!("{method}()"),
                map.getter_is_optional(&owner.name, &field.name)
                    .unwrap_or(declared_optional),
            ));
        }
        let renamed = self.config.resolve_field_name(self.language, &owner.name, &field.name);
        Some((
            crate::codegen::naming::public_field_name(self.language, &field.name, renamed.as_deref()),
            declared_optional,
        ))
    }

    fn owner(&self, ty: &TypeRef) -> Option<&TypeDef> {
        let TypeRef::Named(name) = unwrapped(ty) else {
            return None;
        };
        let name = name.rsplit("::").next()?;
        self.types
            .iter()
            .find(|owner| owner.name == name && !owner.binding_excluded)
    }
}

fn unwrapped(ty: &TypeRef) -> &TypeRef {
    match ty {
        TypeRef::Optional(inner) => unwrapped(inner),
        _ => ty,
    }
}

pub(crate) struct UsagePathOverrides {
    pub expressions: std::collections::HashMap<String, String>,
    unavailable: std::collections::HashSet<String>,
}
impl UsagePathOverrides {
    pub fn prepare(
        initial: &std::collections::HashMap<String, String>,
        assertions: &[crate::e2e::fixture::Assertion],
        context: Option<&UsageAccessContext<'_>>,
        chunks: &str,
    ) -> Self {
        let mut result = Self {
            expressions: initial.clone(),
            unavailable: Default::default(),
        };
        let Some(context) = context.filter(|context| context.has_declared_item()) else {
            return result;
        };
        for field in assertions
            .iter()
            .filter_map(|assertion| assertion.field.as_deref())
            .filter(|field| super::is_stream_scoped_field(field))
        {
            if result.expressions.contains_key(field) {
                continue;
            }
            if let Some(expression) = context.accessor(field, chunks) {
                result.expressions.insert(field.into(), expression);
            } else {
                result.unavailable.insert(field.into());
            }
        }
        result
    }
    pub fn render_unavailable(
        &self,
        out: &mut String,
        assertion: &crate::e2e::fixture::Assertion,
        indent: &str,
    ) -> bool {
        let Some(field) = assertion
            .field
            .as_deref()
            .filter(|field| self.unavailable.contains(*field))
        else {
            return false;
        };
        render_unavailable(out, field, indent);
        true
    }
}

impl UsageAccessContext<'_> {
    pub fn has_declared_item(&self) -> bool {
        let name = self.item_type.rsplit("::").next().unwrap_or(self.item_type);
        self.types.iter().any(|ty| ty.name == name)
    }
}
pub(crate) fn render_unavailable(out: &mut String, field: &str, indent: &str) {
    out.push_str(&crate::e2e::template_env::render("streaming/usage_unavailable.jinja",crate::alef_context! {
        indent => indent, message => crate::e2e::codegen::field_skip::FieldSkip::StreamingAssertionOnUnsupportedField.message(field),
    }));
}

#[cfg(test)]
mod tests;
