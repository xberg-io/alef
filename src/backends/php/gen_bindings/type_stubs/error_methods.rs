use crate::core::ir::ApiSurface;

/// Stub declarations for the errors' `Duration` methods: whole milliseconds, `?int` when optional,
/// named the way ext-php-rs camel-cases the `#[php_method]`.
pub(super) fn duration_error_method_stubs(api: &ApiSurface) -> String {
    use crate::codegen::error_gen::{DurationShape, duration_shape};
    let mut seen: Vec<&str> = Vec::new();
    let mut out = String::new();
    for method in api.errors.iter().flat_map(|e| &e.methods) {
        let Some(shape) = duration_shape(&method.return_type) else {
            continue;
        };
        if seen.contains(&method.name.as_str()) {
            continue;
        }
        seen.push(&method.name);
        let (nullable, note) = match shape {
            DurationShape::Optional => ("?", " (null when absent)"),
            DurationShape::Bare => ("", ""),
        };
        out.push_str(&format!(
            "    /** `{}` in whole milliseconds{note}. */\n    public function {}(): {nullable}int {{ throw new \\RuntimeException('Not implemented.'); }}\n",
            method.name,
            heck::ToLowerCamelCase::to_lower_camel_case(method.name.as_str()),
        ));
    }
    out
}
