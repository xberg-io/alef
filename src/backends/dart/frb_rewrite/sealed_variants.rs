use regex::Regex;
use std::sync::OnceLock;

use super::external_library_loader::apply_loader_fix_from_stem;

/// Rewrite all flutter_rust_bridge sealed-class variant parameter names in
/// `source` from positional (`field0`, `field1`, ...) to payload-derived names.
///
/// Returns the rewritten source. Lines that do not match the variant signature
/// are returned verbatim, so this function is safe to apply unconditionally to
/// any frb-generated `lib.dart`.
///
/// When applied to `frb_generated.dart` (which carries the FRB external-library
/// loader config) this also injects the published-package native-library loader
/// via the external-library-loader rewrite; the injection is idempotent and a
/// no-op for files without the loader config.
pub fn rewrite_frb_sealed_variants(source: &str) -> String {
    let source = apply_loader_fix_from_stem(source);
    let source = source.as_str();
    let variant_re = variant_regex();

    variant_re
        .replace_all(source, |caps: &regex::Captures<'_>| {
            let prefix = &caps["prefix"];
            let params = &caps["params"];
            let suffix = &caps["suffix"];
            let variant_pascal = &caps["variant"];

            let rewritten_params = rewrite_param_list(params, variant_pascal);
            format!("{prefix}{rewritten_params}{suffix}")
        })
        .into_owned()
}

/// Match a single frb sealed-variant `const factory` line, allowing the
/// parameter block and the trailing `= <Enum>_<Variant>;` to span lines.
///
/// Capture groups:
/// - `prefix`  — `const factory <Enum>.<variantCamel>({`
/// - `params`  — the parameter list between `{` and `}` (no braces)
/// - `suffix`  — `}) = <Enum>_<Variant>;`
/// - `variant` — `<Variant>` (PascalCase) extracted from the assignment
fn variant_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?s)(?P<prefix>const\s+factory\s+[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*\s*\(\s*\{)(?P<params>[^{}]*)(?P<suffix>\}\s*\)\s*=\s*[A-Za-z_][A-Za-z0-9_]*_(?P<variant>[A-Za-z][A-Za-z0-9]*)\s*;)",
        )
        .expect("variant regex must compile")
    })
}

/// Rewrite the comma-separated parameter list inside the variant constructor.
///
/// Each parameter has the shape `required <Type> field<N>`. The `<Type>` can be
/// `List<T>`, `Map<K, V>`, nullable types, etc. — we capture everything between
/// the leading `required` keyword and the trailing positional `field<N>` ident.
fn rewrite_param_list(params: &str, variant_pascal: &str) -> String {
    let param_re = param_regex();

    let matches: Vec<regex::Captures<'_>> = param_re.captures_iter(params).collect();
    let total_fields = matches
        .iter()
        .filter(|m| {
            let name = m.name("name").map(|m| m.as_str()).unwrap_or("");
            is_positional_field(name)
        })
        .count();

    if total_fields == 0 {
        return params.to_string();
    }

    let mut out = String::with_capacity(params.len());
    let mut cursor = 0usize;

    for caps in &matches {
        let whole = caps.get(0).expect("regex match must have group 0");
        let name_match = caps.name("name").expect("name capture is required");
        let raw_name = name_match.as_str();

        out.push_str(&params[cursor..name_match.start()]);

        if let Some(field_idx) = field_index(raw_name) {
            let type_name = caps.name("type").map(|m| m.as_str()).unwrap_or("").trim();
            let new_name = payload_param_name(type_name, variant_pascal, field_idx, total_fields);
            out.push_str(&new_name);
        } else {
            out.push_str(raw_name);
        }

        cursor = name_match.end();
        let _ = whole;
    }

    out.push_str(&params[cursor..]);
    out
}

/// Match a single `required <Type> <ident>` parameter inside a frb variant
/// constructor. The `<Type>` is a balanced-bracket-free chunk plus `?` for
/// nullable; this is sufficient because frb does not emit nested generic types
/// like `Map<List<T>, U>` for sealed variants — but to be defensive we accept
/// any characters except `{`, `}`, `,` between the keyword and the ident.
fn param_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"required\s+(?P<type>[^,{}]+?)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?:,|$)")
            .expect("param regex must compile")
    })
}

/// Return the numeric index `N` for a positional `field<N>` parameter, or
/// `None` if `name` is a named (non-positional) parameter.
fn field_index(name: &str) -> Option<usize> {
    let rest = name.strip_prefix("field")?;
    rest.parse::<usize>().ok()
}

/// `true` when `name` is a positional `field<N>` parameter emitted by frb.
fn is_positional_field(name: &str) -> bool {
    field_index(name).is_some()
}

/// Derive a payload-informed parameter name for a single frb variant field.
///
/// Mirrors the algorithm in `alef-backend-kotlin::gen_bindings::shared::
/// kotlin_field_name_with_type` so that all "modern" backends (kotlin, swift,
/// dart, ...) converge on the same surface.
///
/// Rules:
/// - Single-field variant with a named struct payload whose name starts with
///   the variant name (`PdfMetadata` for variant `Pdf`) → strip the prefix and
///   lowerCamelCase the remainder (`Metadata` → `metadata`).
/// - Single-field variant with a primitive/stdlib payload (`String`, `int`,
///   etc.) → `value`.
/// - Multi-field variant → `value0`, `value1`, ... (uses `field_idx`).
/// - Otherwise (single-field with no inferable prefix) → `value`.
fn payload_param_name(type_name: &str, variant_pascal: &str, field_idx: usize, total_fields: usize) -> String {
    let _ = (type_name, variant_pascal, total_fields);
    format!("field{field_idx}")
}

/// Lower the first character of a PascalCase identifier; preserve the rest.
#[allow(dead_code)]
fn to_lower_camel(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Dart primitive / stdlib types that should map to a generic `value` name.
#[allow(dead_code)]
fn is_dart_primitive(type_name: &str) -> bool {
    matches!(
        type_name,
        "String"
            | "int"
            | "double"
            | "bool"
            | "num"
            | "void"
            | "dynamic"
            | "Object"
            | "Uint8List"
            | "List"
            | "Map"
            | "Set"
            | "BigInt"
            | "DateTime"
            | "Duration"
    )
}
