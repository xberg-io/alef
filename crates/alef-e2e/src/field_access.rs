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
    method_calls: HashSet<String>,
    /// Aliases for error-path field access (used when assertion_type == "error").
    /// Maps fixture sub-field names (the part after "error.") to actual field names
    /// on the error type. E.g., `"status_code" -> "status_code"`.
    error_field_aliases: HashMap<String, String>,
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
        let index_normalized = normalize_numeric_indices(field);
        if index_normalized != field && self.optional_fields.contains(index_normalized.as_str()) {
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

    /// Check whether a fixture field path is valid for the configured result type.
    pub fn is_valid_for_result(&self, fixture_field: &str) -> bool {
        if self.result_fields.is_empty() {
            return true;
        }
        let resolved = self.resolve(fixture_field);
        let first_segment = resolved.split('.').next().unwrap_or(resolved);
        let first_segment = first_segment.split('[').next().unwrap_or(first_segment);
        self.result_fields.contains(first_segment)
    }

    /// Check if a resolved field is an array/Vec type.
    pub fn is_array(&self, field: &str) -> bool {
        self.array_fields.contains(field)
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
    pub fn accessor(&self, fixture_field: &str, language: &str, result_var: &str) -> String {
        let resolved = self.resolve(fixture_field);
        let segments = parse_path(resolved);
        let segments = self.inject_array_indexing(segments);
        match language {
            "java" => render_java_with_optionals(&segments, result_var, &self.optional_fields),
            "kotlin" => render_kotlin_with_optionals(&segments, result_var, &self.optional_fields),
            "rust" => render_rust_with_optionals(&segments, result_var, &self.optional_fields, &self.method_calls),
            "csharp" => render_csharp_with_optionals(&segments, result_var, &self.optional_fields),
            "zig" => render_zig_with_optionals(&segments, result_var, &self.optional_fields, &self.method_calls),
            "swift" => render_swift_with_optionals(&segments, result_var, &self.optional_fields),
            "dart" => render_dart_with_optionals(&segments, result_var, &self.optional_fields),
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
            "rust" => render_rust_with_optionals(&segments, err_var, &self.optional_fields, &self.method_calls),
            _ => render_accessor(&segments, language, err_var),
        }
    }

    /// Check whether a sub-field (the part after `"error."`) has an entry in
    /// `error_field_aliases` or if there are any error aliases at all.
    ///
    /// When there are no error aliases configured, callers fall back to
    /// direct field access, which is the safe default for known public fields
    /// like `status_code` on `LiterLlmError`.
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
        let segments = parse_path(resolved);
        let segments = self.inject_array_indexing(segments);
        let local_var = resolved.replace(['.', '['], "_").replace(']', "");
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
            format!("let {local_var} = {accessor}.as_ref().map(|v| v.to_string()).unwrap_or_default();")
        };
        Some((binding, local_var))
    }
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
/// Swift-bridge exposes all Rust struct fields as methods with `()`, so every
/// field segment must be followed by `()`. Array fields (e.g. `nodes` inside
/// an array parent) also need `()`.
fn render_swift(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push('.');
                out.push_str(f);
                out.push_str("()");
            }
            PathSegment::ArrayField { name, index } => {
                out.push('.');
                out.push_str(name);
                out.push_str(&format!("()[{index}]"));
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(field);
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("()[{key}]"));
                } else {
                    out.push_str(&format!("()[\"{key}\"]"));
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
                out.push_str(f);
                out.push_str("()");
                // Insert `?` after `()` for non-leaf optional fields so the next
                // member access becomes `?.`.
                if !is_leaf && optional_fields.contains(&path_so_far) {
                    out.push('?');
                }
            }
            PathSegment::ArrayField { name, index } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(name);
                out.push('.');
                out.push_str(name);
                out.push_str(&format!("()[{index}]"));
            }
            PathSegment::MapAccess { field, key } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(field);
                out.push('.');
                out.push_str(field);
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("()[{key}]"));
                } else {
                    out.push_str(&format!("()[\"{key}\"]"));
                }
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
                    } else if language == "elixir" {
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

/// Kotlin accessor: same camelCase method calls as Java but uses Kotlin idioms.
///
/// Differences from Java:
/// - Array index-0: `.field().first()` instead of `.field().getFirst()`
/// - Array index-N: `.field().get(N)` (explicit index)
/// - Collection size: `.size` (property) instead of `.size()` (method)
fn render_kotlin(segments: &[PathSegment], result_var: &str) -> String {
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
                if *index == 0 {
                    out.push_str("().first()");
                } else {
                    out.push_str(&format!("().get({index})"));
                }
            }
            PathSegment::MapAccess { field, key } => {
                out.push('.');
                out.push_str(&field.to_lower_camel_case());
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
fn render_kotlin_with_optionals(
    segments: &[PathSegment],
    result_var: &str,
    optional_fields: &HashSet<String>,
) -> String {
    let mut out = result_var.to_string();
    let mut path_so_far = String::new();
    // Track whether the previous segment returned a nullable type. Starts
    // false because `result_var` is always non-null.
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
                // return type T? in Kotlin).
                let is_optional = optional_fields.contains(&path_so_far);
                out.push_str(nav);
                out.push_str(&f.to_lower_camel_case());
                out.push_str("()");
                prev_was_nullable = is_optional;
            }
            PathSegment::ArrayField { name, index } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(name);
                let is_optional = optional_fields.contains(&path_so_far);
                out.push_str(nav);
                out.push_str(&name.to_lower_camel_case());
                let safe = if prev_was_nullable || is_optional { "?" } else { "" };
                if *index == 0 {
                    out.push_str(&format!("(){safe}.first()"));
                } else {
                    out.push_str(&format!("(){safe}.get({index})"));
                }
                prev_was_nullable = is_optional;
            }
            PathSegment::MapAccess { field, key } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(field);
                let is_optional = optional_fields.contains(&path_so_far);
                out.push_str(nav);
                out.push_str(&field.to_lower_camel_case());
                let is_numeric = !key.is_empty() && key.chars().all(|c| c.is_ascii_digit());
                if is_numeric {
                    if is_optional {
                        out.push_str(&format!("()?.get({key})"));
                    } else {
                        out.push_str(&format!("().get({key})"));
                    }
                } else if is_optional {
                    out.push_str(&format!("()?.get(\"{key}\")"));
                } else {
                    out.push_str(&format!("().get(\"{key}\")"));
                }
                prev_was_nullable = is_optional;
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

/// Rust accessor with Option unwrapping for intermediate fields.
///
/// When an intermediate field is in the `optional_fields` set, `.as_ref().unwrap()`
/// is appended after the field access to unwrap the `Option<T>`.
/// When a path is in `method_calls`, `()` is appended to make it a method call.
fn render_rust_with_optionals(
    segments: &[PathSegment],
    result_var: &str,
    optional_fields: &HashSet<String>,
    method_calls: &HashSet<String>,
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
                let is_method = method_calls.contains(&path_so_far);
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
    let mut parts = Vec::new();
    let mut trailing_length = false;
    for seg in segments {
        match seg {
            PathSegment::Field(f) => parts.push(f.to_snake_case()),
            PathSegment::ArrayField { name, .. } => parts.push(name.to_snake_case()),
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
                out.push_str(&f.to_lower_camel_case());
                prev_was_nullable = is_optional;
            }
            PathSegment::ArrayField { name, index } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(name);
                out.push_str(nav);
                out.push_str(&name.to_lower_camel_case());
                out.push_str(&format!("[{index}]"));
                prev_was_nullable = false;
            }
            PathSegment::MapAccess { field, key } => {
                if !path_so_far.is_empty() {
                    path_so_far.push('.');
                }
                path_so_far.push_str(field);
                let is_optional = optional_fields.contains(&path_so_far);
                out.push_str(nav);
                out.push_str(&field.to_lower_camel_case());
                if key.chars().all(|c| c.is_ascii_digit()) {
                    out.push_str(&format!("[{key}]"));
                } else {
                    out.push_str(&format!("[\"{key}\"]"));
                }
                prev_was_nullable = is_optional;
            }
            PathSegment::Length => {
                out.push_str(".length");
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
}
