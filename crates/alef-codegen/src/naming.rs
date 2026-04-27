use heck::{ToLowerCamelCase, ToPascalCase, ToShoutySnakeCase, ToSnakeCase};

/// Convert a Rust snake_case name to the target language convention.
pub fn to_python_name(name: &str) -> String {
    name.to_snake_case()
}

/// Convert a Rust snake_case name to Node.js/TypeScript lowerCamelCase convention.
pub fn to_node_name(name: &str) -> String {
    name.to_lower_camel_case()
}

/// Convert a Rust snake_case name to Ruby snake_case convention.
pub fn to_ruby_name(name: &str) -> String {
    name.to_snake_case()
}

/// Convert a Rust snake_case name to PHP lowerCamelCase convention.
pub fn to_php_name(name: &str) -> String {
    name.to_lower_camel_case()
}

/// Convert a Rust snake_case name to Elixir snake_case convention.
pub fn to_elixir_name(name: &str) -> String {
    name.to_snake_case()
}

/// Well-known Go acronyms that must be fully uppercased per Go naming conventions.
/// See: https://go.dev/wiki/CodeReviewComments#initialisms
const GO_ACRONYMS: &[&str] = &[
    "API", "ASCII", "CPU", "CSS", "DNS", "EOF", "FTP", "GID", "GUI", "HTML", "HTTP", "HTTPS", "ID", "IMAP", "IP",
    "JSON", "LHS", "MFA", "POP", "QPS", "RAM", "RHS", "RPC", "SLA", "SMTP", "SQL", "SSH", "SSL", "TCP", "TLS", "TTL",
    "UDP", "UI", "UID", "UUID", "URI", "URL", "UTF8", "VM", "XML", "XMPP", "XSRF", "XSS",
];

/// Apply Go acronym uppercasing to a PascalCase name.
///
/// Scans word boundaries in the PascalCase string and replaces any run of
/// characters that matches a known Go acronym (case-insensitively) with the
/// all-caps form. For example `ImageUrl` becomes `ImageURL` and `UserId`
/// becomes `UserID`.
fn apply_go_acronyms(name: &str) -> String {
    if name.is_empty() {
        return name.to_string();
    }

    // Split the PascalCase string into words at uppercase letter boundaries.
    // Each "word" is a contiguous sequence starting with an uppercase letter.
    let mut words: Vec<&str> = Vec::new();
    let mut word_start = 0;
    let bytes = name.as_bytes();
    for i in 1..bytes.len() {
        if bytes[i].is_ascii_uppercase() {
            words.push(&name[word_start..i]);
            word_start = i;
        }
    }
    words.push(&name[word_start..]);

    // For each word, check if it matches a known acronym (case-insensitive).
    let mut result = String::with_capacity(name.len());
    let mut i = 0;
    while i < words.len() {
        // Try to match as many consecutive words as possible to a single acronym
        // (handles acronyms like "UTF8" which span one word but look like two parts).
        let word = words[i];
        let upper = word.to_ascii_uppercase();
        if GO_ACRONYMS.contains(&upper.as_str()) {
            result.push_str(&upper);
        } else {
            result.push_str(word);
        }
        i += 1;
    }
    result
}

/// Convert a Rust snake_case name to Go PascalCase convention with acronym uppercasing.
pub fn to_go_name(name: &str) -> String {
    apply_go_acronyms(&name.to_pascal_case())
}

/// Apply Go acronym uppercasing to a name that is already in PascalCase (e.g. an IR type name).
///
/// IR type names come directly from Rust PascalCase (e.g. `ImageUrl`, `JsonSchemaFormat`).
/// This function uppercases known acronym segments so they conform to Go naming conventions
/// (e.g. `ImageUrl` → `ImageURL`, `JsonSchemaFormat` → `JSONSchemaFormat`).
pub fn go_type_name(name: &str) -> String {
    apply_go_acronyms(name)
}

/// Convert a Rust snake_case parameter/variable name to Go lowerCamelCase with acronym uppercasing.
///
/// Go naming conventions require that acronyms in identifiers be fully uppercased.
/// `to_lower_camel_case` alone converts `base_url` → `baseUrl`, but Go wants `baseURL`.
/// This function converts via PascalCase (which applies acronym uppercasing) then lowercases
/// the first "word" (the initial run of uppercase letters treated as a unit) while preserving
/// the case of subsequent words/acronyms:
/// - `base_url`  → `BaseURL`  → `baseURL`
/// - `api_key`   → `APIKey`   → `apiKey`
/// - `user_id`   → `UserID`   → `userID`
/// - `json`      → `JSON`     → `json`
pub fn go_param_name(name: &str) -> String {
    let pascal = apply_go_acronyms(&name.to_pascal_case());
    if pascal.is_empty() {
        return pascal;
    }
    let bytes = pascal.as_bytes();
    // Find the boundary of the first "word":
    // - If the string begins with a multi-char uppercase run followed by a lowercase letter,
    //   the run minus its last char is an acronym prefix (e.g. "APIKey": run="API", next='K')
    //   → lowercase "AP" and keep "IKey" → "apIKey" ... but Go actually wants "apiKey".
    //   The real rule: lowercase the whole leading uppercase run regardless, because the
    //   acronym-prefix IS the first word.
    // - If the string begins with a single uppercase char (e.g. "BaseURL"), lowercase just it.
    //
    // Concretely: find how many leading bytes are uppercase. If that whole run is followed by
    // end-of-string, lowercase everything. If followed by more chars, lowercase the entire run.
    // For "APIKey": upper_len=3, next='K'(uppercase) but that starts the second word.
    // Actually: scan for the first lowercase char to find where the first word ends.
    let first_lower = bytes.iter().position(|b| b.is_ascii_lowercase());
    match first_lower {
        None => {
            // Entire string is uppercase (single acronym like "JSON", "URL") — all lowercase.
            pascal.to_lowercase()
        }
        Some(0) => {
            // Starts with lowercase (already correct)
            pascal
        }
        Some(pos) => {
            // pos is the index of the first lowercase char.
            // The first "word" ends just before pos-1 (the char at pos-1 is the first char of
            // the next PascalCase word that isnds with a lowercase continuation).
            // For "BaseURL": pos=1 ('a'), so uppercase run = ['B'], lowercase just index 0.
            // For "APIKey":  pos=4 ('e' in "Key"), uppercase run = "APIK", next lower = 'e',
            //   so word boundary is at pos-1=3 ('K' is start of "Key").
            //   → lowercase "API" (indices 0..2), keep "Key" → "apiKey" ✓
            // For "UserID":  pos=1 ('s'), uppercase run starts at 'U', lowercase just 'U' → "userID"... wait
            //   "UserID": 'U'(upper),'s'(lower) → pos=1, word="U", lower "U" → "u"+"serID" = "userID" ✓
            let word_end = if pos > 1 { pos - 1 } else { 1 };
            let lower_prefix = pascal[..word_end].to_lowercase();
            format!("{}{}", lower_prefix, &pascal[word_end..])
        }
    }
}

/// Convert a Rust snake_case name to Java lowerCamelCase convention.
pub fn to_java_name(name: &str) -> String {
    name.to_lower_camel_case()
}

/// Convert a Rust snake_case name to C# PascalCase convention.
pub fn to_csharp_name(name: &str) -> String {
    name.to_pascal_case()
}

/// Convert a Rust name to a C-style prefixed snake_case identifier (e.g. `prefix_name`).
pub fn to_c_name(prefix: &str, name: &str) -> String {
    format!("{}_{}", prefix, name.to_snake_case())
}

/// Convert a Rust type name to class name convention for target language.
pub fn to_class_name(name: &str) -> String {
    name.to_pascal_case()
}

/// Convert to SCREAMING_SNAKE for constants.
pub fn to_constant_name(name: &str) -> String {
    name.to_shouty_snake_case()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- to_go_name (snake_case → Go PascalCase with initialism uppercasing) ---

    #[test]
    fn test_to_go_name_html_initialism() {
        assert_eq!(to_go_name("html"), "HTML");
    }

    #[test]
    fn test_to_go_name_url_initialism() {
        assert_eq!(to_go_name("url"), "URL");
    }

    #[test]
    fn test_to_go_name_id_initialism() {
        assert_eq!(to_go_name("id"), "ID");
    }

    #[test]
    fn test_to_go_name_plain_word() {
        assert_eq!(to_go_name("links"), "Links");
    }

    #[test]
    fn test_to_go_name_user_id() {
        assert_eq!(to_go_name("user_id"), "UserID");
    }

    #[test]
    fn test_to_go_name_request_url() {
        assert_eq!(to_go_name("request_url"), "RequestURL");
    }

    // --- Additional cases ---

    #[test]
    fn test_to_go_name_http_status() {
        assert_eq!(to_go_name("http_status"), "HTTPStatus");
    }

    #[test]
    fn test_to_go_name_json_body() {
        assert_eq!(to_go_name("json_body"), "JSONBody");
    }

    // --- go_param_name (snake_case → Go lowerCamelCase with initialism uppercasing) ---

    #[test]
    fn test_go_param_name_base_url() {
        assert_eq!(go_param_name("base_url"), "baseURL");
    }

    #[test]
    fn test_go_param_name_user_id() {
        assert_eq!(go_param_name("user_id"), "userID");
    }

    #[test]
    fn test_go_param_name_api_key() {
        assert_eq!(go_param_name("api_key"), "apiKey");
    }

    #[test]
    fn test_go_param_name_plain() {
        assert_eq!(go_param_name("json"), "json");
    }
}
