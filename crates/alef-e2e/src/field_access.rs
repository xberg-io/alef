//! Field path resolution for nested struct/map access in e2e assertions.
//!
//! The `FieldResolver` maps fixture field paths (e.g., "metadata.title") to
//! actual API struct paths (e.g., "metadata.document.title") and generates
//! language-specific accessor expressions.

use alef_codegen::naming::to_go_name;
use heck::{ToLowerCamelCase, ToPascalCase, ToSnakeCase};
use std::collections::{HashMap, HashSet};

/// Resolves fixture field paths to language-specific accessor expressions.
pub struct FieldResolver {
    aliases: HashMap<String, String>,
    optional_fields: HashSet<String>,
    result_fields: HashSet<String>,
    array_fields: HashSet<String>,
}

/// A parsed segment of a field path.
#[derive(Debug, Clone)]
enum PathSegment {
    /// Struct field access: `foo`
    Field(String),
    /// Array field access with index: `foo[0]`
    ArrayField(String),
    /// Map/dict key access: `foo[key]`
    MapAccess { field: String, key: String },
    /// Length/count of the preceding collection: `.length`
    Length,
}

impl FieldResolver {
    /// Create a new resolver from the e2e config's `fields` aliases,
    /// `fields_optional` set, `result_fields` set, and `fields_array` set.
    pub fn new(
        fields: &HashMap<String, String>,
        optional: &HashSet<String>,
        result_fields: &HashSet<String>,
        array_fields: &HashSet<String>,
    ) -> Self {
        Self {
            aliases: fields.clone(),
            optional_fields: optional.clone(),
            result_fields: result_fields.clone(),
            array_fields: array_fields.clone(),
        }
    }

    /// Resolve a fixture field path to the actual struct path.
    /// Falls back to the field itself if no alias exists.
    pub fn resolve<'a>(&'a self, fixture_field: &'a str) -> &'a str {
        self.aliases
            .get(fixture_field)
            .map(String::as_str)
            .unwrap_or(fixture_field)
    }

    /// Check if a resolved field path is optional.
    pub fn is_optional(&self, field: &str) -> bool {
        if self.optional_fields.contains(field) {
            return true;
        }
        // Normalize all numeric array indices to [0] so `data[1].url` matches
        // `data[0].url` in `optional_fields`. Uses a simple regex-free approach:
        // replace `[<digits>]` with `[0]`.
        let index_normalized = normalize_numeric_indices(field);
        if index_normalized != field && self.optional_fields.contains(index_normalized.as_str()) {
            return true;
        }
        // Also check with/without bracket notation: `json_ld.name` ↔ `json_ld[].name`
        // Strip `[]` from each segment and retry.
        let normalized = field.replace("[].", ".");
        if normalized != field && self.optional_fields.contains(normalized.as_str()) {
            return true;
        }
        // Try adding `[]` after known array fields.
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

    /// Check whether a fixture field path is valid for the configured result type.
    ///
    /// When `result_fields` is non-empty, this returns `true` only if the
    /// first segment of the *resolved* field path appears in that set.
    /// When `result_fields` is empty (not configured), all fields are
    /// considered valid (backwards-compatible).
    pub fn is_valid_for_result(&self, fixture_field: &str) -> bool {
        if self.result_fields.is_empty() {
            return true;
        }
        let resolved = self.resolve(fixture_field);
        let first_segment = resolved.split('.').next().unwrap_or(resolved);
        // Strip any map-access bracket suffix (e.g., "foo[key]" -> "foo").
        let first_segment = first_segment.split('[').next().unwrap_or(first_segment);
        self.result_fields.contains(first_segment)
    }

    /// Check if a resolved field is an array/Vec type.
    pub fn is_array(&self, field: &str) -> bool {
        self.array_fields.contains(field)
    }

    /// Check if a resolved field path contains a non-numeric map access (e.g., `foo["key"]`).
    /// This is needed because Go map access returns a value type (not a pointer),
    /// so nil checks and pointer dereferences don't apply.
    /// Numeric keys (e.g., `choices[0]`) are array/slice indices, not map keys,
    /// and do NOT qualify as map access.
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
    /// `result_var` is the variable holding the function return value.
    pub fn accessor(&self, fixture_field: &str, language: &str, result_var: &str) -> String {
        let resolved = self.resolve(fixture_field);
        let segments = parse_path(resolved);

        // When a segment is an array field and has child segments following it,
        // replace Field with ArrayField so renderers emit `[0]` indexing.
        let segments = self.inject_array_indexing(segments);

        match language {
            "java" => render_java_with_optionals(&segments, result_var, &self.optional_fields),
            "rust" => render_rust_with_optionals(&segments, result_var, &self.optional_fields),
            "csharp" => render_csharp_with_optionals(&segments, result_var, &self.optional_fields),
            _ => render_accessor(&segments, language, result_var),
        }
    }

    /// Replace `Field` segments with `ArrayField` when the field is in `fields_array`
    /// and is followed by further child property segments (i.e., we're accessing a
    /// property on an element, not the array itself).
    ///
    /// Does NOT convert when the next segment is `Length` — `links.length` should
    /// produce `len(result.links)`, not `len(result.links[0])`.
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
                    // Convert to ArrayField only if:
                    // 1. There are more segments after this one
                    // 2. The field is in fields_array
                    // 3. The next segment is NOT Length (we want array size, not element size)
                    let next_is_length = i + 1 < len && matches!(segments[i + 1], PathSegment::Length);
                    if i + 1 < len && self.array_fields.contains(&path_so_far) && !next_is_length {
                        result.push(PathSegment::ArrayField(f.clone()));
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
    /// Returns `(binding_line, local_var_name)` or `None` if the field is not optional.
    pub fn rust_unwrap_binding(&self, fixture_field: &str, result_var: &str) -> Option<(String, String)> {
        let resolved = self.resolve(fixture_field);
        if !self.is_optional(resolved) {
            return None;
        }
        let segments = parse_path(resolved);
        let segments = self.inject_array_indexing(segments);
        let local_var = resolved.replace(['.', '['], "_").replace(']', "");
        let accessor = render_accessor(&segments, "rust", result_var);
        // Non-numeric MapAccess (.get("key").map(|s| s.as_str())) already returns Option<&str>,
        // so skip .as_deref() to avoid borrowing from a temporary.
        // Numeric MapAccess (e.g. choices[0]) is array indexing and does NOT return Option,
        // so it does NOT qualify as a map access for this purpose.
        let has_map_access = segments.iter().any(|s| {
            if let PathSegment::MapAccess { key, .. } = s {
                !key.chars().all(|c| c.is_ascii_digit())
            } else {
                false
            }
        });
        // Array fields (Option<Vec<T>>) dereference to Option<&[T]>, so unwrap_or needs &[].
        let is_array = self.is_array(resolved);
        let binding = if has_map_access {
            format!("let {local_var} = {accessor}.unwrap_or(\"\");")
        } else if is_array {
            format!("let {local_var} = {accessor}.as_deref().unwrap_or(&[]);")
        } else {
            // Use `.as_ref().map(|v| v.to_string()).unwrap_or_default()` so that:
            // - `.as_ref()` avoids moving out of a Vec index (required when accessing
            //   fields like `result.choices[0].finish_reason`).
            // - `.map(|v| v.to_string())` converts enum types (e.g. `FinishReason`)
            //   that implement `Display` to an owned `String`.
            // - `.unwrap_or_default()` gives `String::new()` when `None`, avoiding the
            //   temporary lifetime issue that `.as_deref().unwrap_or("")` would cause.
            // - The binding is `String`, not `&str`, which is fine for test assertions.
            format!("let {local_var} = {accessor}.as_ref().map(|v| v.to_string()).unwrap_or_default();")
        };
        Some((binding, local_var))
    }
}

/// Normalize all numeric array indices in a field path to `[0]`.
///
/// E.g. `"data[2].url"` → `"data[0].url"` so that `is_optional` lookups
/// using `data[0].url` as the canonical key also match `data[1].url`, `data[2].url`, etc.
fn normalize_numeric_indices(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '[' {
            // Collect characters until the matching `]`.
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
                // Replace numeric index with [0].
                result.push_str("[0]");
            } else {
                // Non-numeric key or unclosed bracket — emit as-is.
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

/// Parse a dotted field path into segments, handling map access `foo[key]`
/// and the special `.length` pseudo-property for collection sizes.
fn parse_path(path: &str) -> Vec<PathSegment> {
    let mut segments = Vec::new();
    for part in path.split('.') {
        if part == "length" || part == "count" || part == "size" {
            segments.push(PathSegment::Length);
        } else if let Some(bracket_pos) = part.find('[') {
            let field = part[..bracket_pos].to_string();
            let key = part[bracket_pos + 1..].trim_end_matches(']').to_string();
            if key.is_empty() {
                // `field[]` means "first element" — treat as ArrayField
                segments.push(PathSegment::ArrayField(field));
            } else {
                segments.push(PathSegment::MapAccess { field, key });
            }
        } else {
            segments.push(PathSegment::Field(part.to_string()));
        }
    }
    segments
}

/// Render an accessor expression for the given language.
fn render_accessor(segments: &[PathSegment], language: &str, result_var: &str) -> String {
    match language {
        "rust" => render_rust(segments, result_var),
        "python" => render_dot_access(segments, result_var, "python"),
        "typescript" | "node" => render_typescript(segments, result_var),
        "wasm" => render_wasm(segments, result_var),
        "go" => render_go(segments, result_var),
        "java" => render_java(segments, result_var),
        "csharp" => render_pascal_dot(segments, result_var),
        "ruby" => render_dot_access(segments, result_var, "ruby"),
        "php" => render_php(segments, result_var),
        "elixir" => render_dot_access(segments, result_var, "elixir"),
        "r" => render_r(segments, result_var),
        "c" => render_c(segments, result_var),
        _ => render_dot_access(segments, result_var, language),
    }
}

// ---------------------------------------------------------------------------
// Per-language renderers
// ---------------------------------------------------------------------------

/// Rust: `result.foo.bar.baz` or `result.foo.bar[0]` or `result.foo.bar.get("key").map(|s| s.as_str())`
fn render_rust(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&f.to_snake_case());
            }
            PathSegment::ArrayField(f) => {
                out.push('.');
                out.push_str(&f.to_snake_case());
                out.push_str("[0]");
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&field.to_snake_case());
                // Numeric keys are array indices (`choices[0]`), not hash-map keys.
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

/// Simple dot access (Python, Ruby, Elixir): `result.foo.bar.baz`
fn render_dot_access(segments: &[PathSegment], result_var: &str, language: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(f);
            }
            PathSegment::ArrayField(f) => {
                if language == "elixir" {
                    let current = std::mem::take(&mut out);
                    out = format!("Enum.at({current}.{f}, 0)");
                } else {
                    out.push('.');
                    out.push_str(f);
                    out.push_str("[0]");
                }
            }
            PathSegment::MapAccess { field, key } => {
                let is_numeric = key.chars().all(|c| c.is_ascii_digit());
                if is_numeric && language == "elixir" {
                    // Elixir: Enum.at(prefix.field, index)
                    let current = std::mem::take(&mut out);
                    out = format!("Enum.at({current}.{field}, {key})");
                } else {
                    out.push('.');
                    out.push_str(field);
                    if is_numeric {
                        // Python, Ruby: list[index]
                        let idx: usize = key.parse().unwrap_or(0);
                        out.push_str(&format!("[{idx}]"));
                    } else if language == "elixir" {
                        // Elixir maps use bracket access (map["key"]), not method calls.
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
                // Python and default: len()
                _ => {
                    let current = std::mem::take(&mut out);
                    out = format!("len({current})");
                }
            },
        }
    }
    out
}

/// TypeScript/Node: `result.foo.bar.baz` or `result.foo.bar["key"]`
/// NAPI-RS generates camelCase field names, so snake_case segments are converted.
fn render_typescript(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&f.to_lower_camel_case());
            }
            PathSegment::ArrayField(f) => {
                out.push('.');
                out.push_str(&f.to_lower_camel_case());
                out.push_str("[0]");
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&field.to_lower_camel_case());
                out.push_str(&format!("[\"{key}\"]"));
            }
            PathSegment::Length => {
                out.push_str(".length");
            }
        }
    }
    out
}

/// WASM: `result.foo.bar.baz` or `result.foo.bar.get("key")`
/// WASM bindings return Maps (from BTreeMap via serde_wasm_bindgen),
/// which require `.get("key")` instead of bracket notation.
/// Generates camelCase field names, so snake_case segments are converted.
fn render_wasm(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&f.to_lower_camel_case());
            }
            PathSegment::ArrayField(f) => {
                out.push('.');
                out.push_str(&f.to_lower_camel_case());
                out.push_str("[0]");
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

/// Go: `result.Foo.Bar.HTML` (PascalCase with Go initialism uppercasing) or `result.Foo.Bar["key"]`
///
/// Uses `alef_codegen::naming::to_go_name` so that fields like `html`, `url`, `user_id`
/// are rendered as `HTML`, `URL`, `UserID` — matching the Go binding generator.
fn render_go(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&to_go_name(f));
            }
            PathSegment::ArrayField(f) => {
                out.push('.');
                out.push_str(&to_go_name(f));
                out.push_str("[0]");
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&to_go_name(field));
                // Numeric keys index a slice ([]T) — emit as integer index.
                // String keys index a map (map[string]T) — emit as quoted string.
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

/// Java: `result.foo().bar().baz()` or `result.foo().bar().get("key")`
/// Field names are converted to lowerCamelCase (Java convention).
fn render_java(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&f.to_lower_camel_case());
                out.push_str("()");
            }
            PathSegment::ArrayField(f) => {
                out.push('.');
                out.push_str(&f.to_lower_camel_case());
                out.push_str("().getFirst()");
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&field.to_lower_camel_case());
                out.push_str(&format!("().get(\"{key}\")"));
            }
            PathSegment::Length => {
                out.push_str(".size()");
            }
        }
    }
    out
}

/// Java accessor with Optional unwrapping for intermediate fields.
///
/// When an intermediate field is in the `optional_fields` set, `.orElseThrow()`
/// is appended after the accessor call to unwrap the `Optional<T>`.
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
                // Unwrap intermediate Optional fields with .get() so downstream accessors work.
                // Only unwrap non-leaf fields (intermediate steps in the path) that are Optional.
                if !is_leaf && optional_fields.contains(&path_so_far) {
                    out.push_str(".get()");
                }
            }
            PathSegment::ArrayField(f) => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(f);
                out.push('.');
                out.push_str(&f.to_lower_camel_case());
                out.push_str("().getFirst()");
            }
            PathSegment::MapAccess { field, key } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(field);
                out.push('.');
                out.push_str(&field.to_lower_camel_case());
                out.push_str(&format!("().get(\"{key}\")"));
            }
            PathSegment::Length => {
                out.push_str(".size()");
            }
        }
    }
    out
}

/// Rust accessor with Option unwrapping for intermediate fields.
///
/// When an intermediate field is in the `optional_fields` set, `.as_ref().unwrap()`
/// is appended after the field access to unwrap the `Option<T>`.
fn render_rust_with_optionals(segments: &[PathSegment], result_var: &str, optional_fields: &HashSet<String>) -> String {
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
                // Unwrap intermediate Optional fields so downstream accessors work.
                if !is_leaf && optional_fields.contains(&path_so_far) {
                    out.push_str(".as_ref().unwrap()");
                }
            }
            PathSegment::ArrayField(f) => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(f);
                out.push('.');
                out.push_str(&f.to_snake_case());
                out.push_str("[0]");
            }
            PathSegment::MapAccess { field, key } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(field);
                out.push('.');
                out.push_str(&field.to_snake_case());
                // Numeric keys are array indices (`choices[0]`), not hash-map keys.
                if key.chars().all(|c| c.is_ascii_digit()) {
                    // When the array field itself is Optional (e.g. `segments` is
                    // `Option<Vec<T>>`), we must unwrap it before indexing.
                    // Check both bare name (`segments`) and normalized form (`segments[0]`).
                    let is_opt = optional_fields.contains(&path_so_far);
                    if is_opt {
                        out.push_str(&format!(".as_ref().unwrap()[{key}]"));
                    } else {
                        out.push_str(&format!("[{key}]"));
                    }
                    // Update path_so_far to include [0] so subsequent Field lookups
                    // (e.g. ".message" after "choices[0]") build the correct path
                    // for optional_fields lookups (keyed as "choices[0].message.tool_calls").
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

/// C#: `result.Foo.Bar.Baz` (PascalCase properties)
fn render_pascal_dot(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(&f.to_pascal_case());
            }
            PathSegment::ArrayField(f) => {
                out.push('.');
                out.push_str(&f.to_pascal_case());
                out.push_str("[0]");
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&field.to_pascal_case());
                // Numeric keys are List<T> indices in C# — emit as integer, not string.
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

/// C# accessor with nullable unwrapping for intermediate fields.
///
/// When an intermediate field is in the `optional_fields` set, `!` (null-forgiving)
/// is appended after the field access to unwrap the nullable type.
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
                // Unwrap intermediate nullable fields so downstream accessors work.
                if !is_leaf && optional_fields.contains(&path_so_far) {
                    out.push('!');
                }
            }
            PathSegment::ArrayField(f) => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(f);
                out.push('.');
                out.push_str(&f.to_pascal_case());
                out.push_str("[0]");
            }
            PathSegment::MapAccess { field, key } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(field);
                out.push('.');
                out.push_str(&field.to_pascal_case());
                // Numeric keys are List<T> indices in C# — emit as integer, not string.
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

/// PHP: `$result->foo->bar->baz` or `$result->foo->bar["key"]`
fn render_php(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push_str("->");
                out.push_str(f);
            }
            PathSegment::ArrayField(f) => {
                out.push_str("->");
                out.push_str(f);
                out.push_str("[0]");
            }
            PathSegment::MapAccess { field, key } => {
                out.push_str("->");
                out.push_str(field);
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

/// R: `result$foo$bar$baz` or `result$foo$bar[["key"]]`
fn render_r(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('$');
                out.push_str(f);
            }
            PathSegment::ArrayField(f) => {
                out.push('$');
                out.push_str(f);
                out.push_str("[[1]]");
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

/// C FFI: `{prefix}_result_foo_bar_baz({result})` accessor function style.
fn render_c(segments: &[PathSegment], result_var: &str) -> String {
    let mut parts = Vec::new();
    let mut trailing_length = false;
    for seg in segments {
        match seg {
            PathSegment::Field(f) | PathSegment::ArrayField(f) => parts.push(f.to_snake_case()),
            PathSegment::MapAccess { field, key } => {
                parts.push(field.to_snake_case());
                parts.push(key.clone());
            }
            PathSegment::Length => {
                trailing_length = true;
            }
        }
    }
    let suffix = parts.join("_");
    if trailing_length {
        format!("result_{suffix}_count({result_var})")
    } else {
        format!("result_{suffix}({result_var})")
    }
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

        FieldResolver::new(&fields, &optional, &HashSet::new(), &HashSet::new())
    }

    fn make_resolver_with_doc_optional() -> FieldResolver {
        let mut fields = HashMap::new();
        fields.insert("title".to_string(), "metadata.document.title".to_string());
        fields.insert("tags".to_string(), "metadata.tags[name]".to_string());

        let mut optional = HashSet::new();
        optional.insert("document".to_string());
        optional.insert("metadata.document.title".to_string());
        optional.insert("metadata.document".to_string());

        FieldResolver::new(&fields, &optional, &HashSet::new(), &HashSet::new())
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
        // Verifies that Go initialism uppercasing is applied consistently with the
        // binding generator — `html` → `HTML`, `url` → `URL`, etc.
        let mut fields = std::collections::HashMap::new();
        fields.insert("content".to_string(), "html".to_string());
        fields.insert("link_url".to_string(), "links.url".to_string());
        let r = FieldResolver::new(&fields, &HashSet::new(), &HashSet::new(), &HashSet::new());

        assert_eq!(r.accessor("content", "go", "result"), "result.HTML");
        assert_eq!(r.accessor("link_url", "go", "result"), "result.Links.URL");
        // Direct field access without alias.
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
        // WASM returns Maps, which need .get("key") instead of ["key"]
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
            "result_metadata_document_title(result)"
        );
    }

    #[test]
    fn test_rust_unwrap_binding() {
        let r = make_resolver();
        let (binding, var) = r.rust_unwrap_binding("title", "result").unwrap();
        assert_eq!(var, "metadata_document_title");
        // Non-map, non-array optional fields use as_ref().map(|v| v.to_string()).unwrap_or_default()
        // to handle enum types that implement Display and avoid temporary lifetime issues.
        assert!(binding.contains("as_ref().map(|v| v.to_string()).unwrap_or_default()"));
    }

    #[test]
    fn test_rust_unwrap_binding_non_optional() {
        let r = make_resolver();
        assert!(r.rust_unwrap_binding("content", "result").is_none());
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
        // "metadata.document" is optional, so it should be unwrapped
        assert_eq!(
            r.accessor("title", "rust", "result"),
            "result.metadata.document.as_ref().unwrap().title"
        );
    }

    #[test]
    fn test_accessor_csharp_with_optionals() {
        let r = make_resolver_with_doc_optional();
        // "metadata.document" is optional, so it should be unwrapped
        assert_eq!(
            r.accessor("title", "csharp", "result"),
            "result.Metadata.Document!.Title"
        );
    }

    #[test]
    fn test_accessor_rust_non_optional_field() {
        let r = make_resolver();
        // "content" is not optional, so no unwrapping needed
        assert_eq!(r.accessor("content", "rust", "result"), "result.content");
    }

    #[test]
    fn test_accessor_csharp_non_optional_field() {
        let r = make_resolver();
        // "content" is not optional, so no unwrapping needed
        assert_eq!(r.accessor("content", "csharp", "result"), "result.Content");
    }
}
