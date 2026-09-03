//! Splices a `$mock_url` placeholder into typed-builder-generated TypeScript source.
//!
//! `args.rs`'s `mock_url` handling for a `json_object` array/object argument routes BOTH node and
//! wasm through this splice. node once used a `JSON.stringify(...).replaceAll(...)` /
//! `JSON.parse(...) as T` round trip on the theory that napi's structural types accept a plain
//! object -- true only for values JSON can represent. A `bytes` field emitted as
//! `Uint8Array.from([...])` serialised to `{"0":66,"1":97,...}` and napi rejected it, so node now
//! splices too. wasm never could take the JSON path at all: `ts_builder_expression` already emits real `T.default()` /
//! `new T()` construction with field setters, and JSON.parse-ing that construction's stringified
//! output throws away the class instance, so wasm-bindgen's `instanceof` check in `_assertClass`
//! rejects the plain object the round trip produces at runtime. Splicing the runtime mock-url
//! value directly into the builder's own source text -- as a template-literal interpolation in
//! place of the literal `$mock_url` text -- keeps the class instance the builder already built and
//! never re-parses it. ~keep

/// Rewrite every double-quoted JS string literal in `code` that embeds
/// [`crate::e2e::codegen::MOCK_URL_PLACEHOLDER`] into a template literal interpolating
/// `base_var_expr`, leaving every other part of `code` -- including its `new T()`/`.default()`
/// construction and field-setter statements -- untouched.
pub(in crate::e2e::codegen::typescript::test_file) fn splice_mock_url_into_builder_code(
    code: &str,
    base_var_expr: &str,
) -> String {
    let marker = crate::e2e::codegen::MOCK_URL_PLACEHOLDER;
    let chars: Vec<char> = code.chars().collect();
    let mut out = String::with_capacity(code.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '"' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let mut j = i + 1;
        let mut raw = String::new();
        while j < chars.len() && chars[j] != '"' {
            if chars[j] == '\\' && j + 1 < chars.len() {
                raw.push(chars[j]);
                raw.push(chars[j + 1]);
                j += 2;
            } else {
                raw.push(chars[j]);
                j += 1;
            }
        }
        if raw.contains(marker) {
            // Undo `escape_js`'s specific-sequence escapes, then re-escape for a template
            // literal (backtick/`$` instead of `"`). The doubled-backslash undo runs last, or
            // it would corrupt an original `\n`/`\"` that only became two-char sequences via
            // the specific escapes above. ~keep
            let unescaped = raw
                .replace("\\\"", "\"")
                .replace("\\n", "\n")
                .replace("\\r", "\r")
                .replace("\\t", "\t")
                .replace("\\\\", "\\");
            out.push('`');
            let mut rest = unescaped.as_str();
            while let Some(pos) = rest.find(marker) {
                out.push_str(&crate::e2e::escape::escape_js_template(&rest[..pos]));
                out.push_str("${");
                out.push_str(base_var_expr);
                out.push('}');
                rest = &rest[pos + marker.len()..];
            }
            out.push_str(&crate::e2e::escape::escape_js_template(rest));
            out.push('`');
        } else {
            out.push('"');
            out.push_str(&raw);
            out.push('"');
        }
        i = j + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::splice_mock_url_into_builder_code;

    #[test]
    fn replaces_placeholder_with_template_interpolation() {
        let code =
            r#"(() => { const _u0 = WasmExtractInput.default(); _u0.uri = "$mock_url/pdf/fake.pdf"; return _u0; })()"#;
        let spliced = splice_mock_url_into_builder_code(code, "inputsMockBaseUrl");
        assert_eq!(
            spliced,
            r#"(() => { const _u0 = WasmExtractInput.default(); _u0.uri = `${inputsMockBaseUrl}/pdf/fake.pdf`; return _u0; })()"#
        );
    }

    #[test]
    fn leaves_code_without_the_placeholder_unchanged() {
        let code = r#"WasmExtractInput.default()"#;
        assert_eq!(splice_mock_url_into_builder_code(code, "base"), code);
    }
}
