//! C e2e test runner file rendering.

use crate::core::hash::{self, CommentStyle};
use crate::e2e::escape::sanitize_ident;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use std::fmt::Write as FmtWrite;

pub(super) fn render_test_runner_header(
    active_groups: &[(&FixtureGroup, Vec<&Fixture>)],
    visitor_fixtures: &[&Fixture],
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Block));
    let _ = writeln!(out, "#ifndef TEST_RUNNER_H");
    let _ = writeln!(out, "#define TEST_RUNNER_H");
    let _ = writeln!(out);
    let _ = writeln!(out, "#include <string.h>");
    let _ = writeln!(out, "#include <stdlib.h>");
    let _ = writeln!(out);
    // Trim helper for comparing strings that may have trailing whitespace/newlines.
    let _ = writeln!(out, "/**");
    let _ = writeln!(
        out,
        " * Compare a string against an expected value, trimming trailing whitespace."
    );
    let _ = writeln!(
        out,
        " * Returns 0 if the trimmed actual string equals the expected string."
    );
    let _ = writeln!(out, " */");
    let _ = writeln!(
        out,
        "static inline int str_trim_eq(const char *actual, const char *expected) {{"
    );
    let _ = writeln!(
        out,
        "    if (actual == NULL || expected == NULL) return actual != expected;"
    );
    let _ = writeln!(out, "    size_t alen = strlen(actual);");
    let _ = writeln!(
        out,
        "    while (alen > 0 && (actual[alen-1] == ' ' || actual[alen-1] == '\\n' || actual[alen-1] == '\\r' || actual[alen-1] == '\\t')) alen--;"
    );
    let _ = writeln!(out, "    size_t elen = strlen(expected);");
    let _ = writeln!(out, "    if (alen != elen) return 1;");
    let _ = writeln!(out, "    return memcmp(actual, expected, elen);");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);

    // Forward declaration so alef_json_get_string can fall through to the
    // object/array extractor for non-string values without reordering the helpers.
    let _ = writeln!(
        out,
        "static inline char *alef_json_get_object(const char *json, const char *key);"
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "/**");
    let _ = writeln!(
        out,
        " * Extract a string value for a given key from a JSON object string."
    );
    let _ = writeln!(
        out,
        " * Returns a heap-allocated copy of the value, or NULL if not found."
    );
    let _ = writeln!(out, " * Caller must free() the returned string.");
    let _ = writeln!(out, " */");
    let _ = writeln!(
        out,
        "static inline char *alef_json_get_string(const char *json, const char *key) {{"
    );
    let _ = writeln!(out, "    if (json == NULL || key == NULL) return NULL;");
    let _ = writeln!(out, "    /* Build search pattern: \"key\":  */");
    let _ = writeln!(out, "    size_t key_len = strlen(key);");
    let _ = writeln!(out, "    char *pattern = (char *)malloc(key_len + 5);");
    let _ = writeln!(out, "    if (!pattern) return NULL;");
    let _ = writeln!(out, "    pattern[0] = '\"';");
    let _ = writeln!(out, "    memcpy(pattern + 1, key, key_len);");
    let _ = writeln!(out, "    pattern[key_len + 1] = '\"';");
    let _ = writeln!(out, "    pattern[key_len + 2] = ':';");
    let _ = writeln!(out, "    pattern[key_len + 3] = '\\0';");
    let _ = writeln!(out, "    const char *found = strstr(json, pattern);");
    let _ = writeln!(out, "    free(pattern);");
    let _ = writeln!(out, "    if (!found) return NULL;");
    let _ = writeln!(out, "    found += key_len + 3; /* skip past \"key\": */");
    let _ = writeln!(out, "    while (*found == ' ' || *found == '\\t') found++;");
    let _ = writeln!(
        out,
        "    /* Non-string values (arrays/objects) — fall through to alef_json_get_object so"
    );
    let _ = writeln!(
        out,
        "       leaf accessors over collection-typed fields (Vec<T>, Option<Vec<T>>) work for"
    );
    let _ = writeln!(
        out,
        "       not_empty / count_equals assertions without needing per-field type metadata. */"
    );
    let _ = writeln!(out, "    if (*found == '{{' || *found == '[') {{");
    let _ = writeln!(out, "        return alef_json_get_object(json, key);");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(
        out,
        "    /* Primitive non-string value: extract its raw token (numeric / true / false / null)"
    );
    let _ = writeln!(
        out,
        "       so callers asserting on numeric fields can `atoll`/`atof` the result. */"
    );
    let _ = writeln!(out, "    if (*found != '\"') {{");
    let _ = writeln!(out, "        const char *p = found;");
    let _ = writeln!(
        out,
        "        while (*p && *p != ',' && *p != '}}' && *p != ']' && *p != ' ' && *p != '\\t' && *p != '\\n' && *p != '\\r') p++;"
    );
    let _ = writeln!(out, "        size_t plen = (size_t)(p - found);");
    let _ = writeln!(out, "        if (plen == 0) return NULL;");
    let _ = writeln!(out, "        char *prim = (char *)malloc(plen + 1);");
    let _ = writeln!(out, "        if (!prim) return NULL;");
    let _ = writeln!(out, "        memcpy(prim, found, plen);");
    let _ = writeln!(out, "        prim[plen] = '\\0';");
    let _ = writeln!(out, "        return prim;");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    found++; /* skip opening quote */");
    let _ = writeln!(out, "    const char *end = found;");
    let _ = writeln!(out, "    while (*end && *end != '\"') {{");
    let _ = writeln!(out, "        if (*end == '\\\\') {{ end++; if (*end) end++; }}");
    let _ = writeln!(out, "        else end++;");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    size_t val_len = (size_t)(end - found);");
    let _ = writeln!(out, "    char *result_str = (char *)malloc(val_len + 1);");
    let _ = writeln!(out, "    if (!result_str) return NULL;");
    let _ = writeln!(out, "    memcpy(result_str, found, val_len);");
    let _ = writeln!(out, "    result_str[val_len] = '\\0';");
    let _ = writeln!(out, "    return result_str;");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "/**");
    let _ = writeln!(
        out,
        " * Extract a JSON object/array value `{{...}}` or `[...]` for a given key from"
    );
    let _ = writeln!(
        out,
        " * a JSON object string. Returns a heap-allocated copy of the value INCLUDING"
    );
    let _ = writeln!(
        out,
        " * its surrounding braces, or NULL if the key is missing or its value is a"
    );
    let _ = writeln!(out, " * primitive. Caller must free() the returned string.");
    let _ = writeln!(out, " *");
    let _ = writeln!(
        out,
        " * Used by chained-accessor codegen for intermediate object extraction:"
    );
    let _ = writeln!(
        out,
        " * `choices[0].message.content` first peels off `message` (an object), then"
    );
    let _ = writeln!(out, " * looks up `content` (a string) within the extracted substring.");
    let _ = writeln!(out, " */");
    let _ = writeln!(
        out,
        "static inline char *alef_json_get_object(const char *json, const char *key) {{"
    );
    let _ = writeln!(out, "    if (json == NULL || key == NULL) return NULL;");
    let _ = writeln!(out, "    size_t key_len = strlen(key);");
    let _ = writeln!(out, "    char *pattern = (char *)malloc(key_len + 4);");
    let _ = writeln!(out, "    if (!pattern) return NULL;");
    let _ = writeln!(out, "    pattern[0] = '\"';");
    let _ = writeln!(out, "    memcpy(pattern + 1, key, key_len);");
    let _ = writeln!(out, "    pattern[key_len + 1] = '\"';");
    let _ = writeln!(out, "    pattern[key_len + 2] = ':';");
    let _ = writeln!(out, "    pattern[key_len + 3] = '\\0';");
    let _ = writeln!(out, "    const char *found = strstr(json, pattern);");
    let _ = writeln!(out, "    free(pattern);");
    let _ = writeln!(out, "    if (!found) return NULL;");
    let _ = writeln!(out, "    found += key_len + 3;");
    let _ = writeln!(out, "    while (*found == ' ' || *found == '\\t') found++;");
    let _ = writeln!(out, "    char open_ch = *found;");
    let _ = writeln!(out, "    char close_ch;");
    let _ = writeln!(out, "    if (open_ch == '{{') close_ch = '}}';");
    let _ = writeln!(out, "    else if (open_ch == '[') close_ch = ']';");
    let _ = writeln!(
        out,
        "    else return NULL; /* primitive — caller should use alef_json_get_string */"
    );
    let _ = writeln!(out, "    int depth = 0;");
    let _ = writeln!(out, "    int in_string = 0;");
    let _ = writeln!(out, "    const char *end = found;");
    let _ = writeln!(out, "    for (; *end; end++) {{");
    let _ = writeln!(out, "        if (in_string) {{");
    let _ = writeln!(
        out,
        "            if (*end == '\\\\' && *(end + 1)) {{ end++; continue; }}"
    );
    let _ = writeln!(out, "            if (*end == '\"') in_string = 0;");
    let _ = writeln!(out, "            continue;");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "        if (*end == '\"') {{ in_string = 1; continue; }}");
    let _ = writeln!(out, "        if (*end == open_ch) depth++;");
    let _ = writeln!(out, "        else if (*end == close_ch) {{");
    let _ = writeln!(out, "            depth--;");
    let _ = writeln!(out, "            if (depth == 0) {{ end++; break; }}");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    if (depth != 0) return NULL;");
    let _ = writeln!(out, "    size_t val_len = (size_t)(end - found);");
    let _ = writeln!(out, "    char *result_str = (char *)malloc(val_len + 1);");
    let _ = writeln!(out, "    if (!result_str) return NULL;");
    let _ = writeln!(out, "    memcpy(result_str, found, val_len);");
    let _ = writeln!(out, "    result_str[val_len] = '\\0';");
    let _ = writeln!(out, "    return result_str;");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "/**");
    let _ = writeln!(
        out,
        " * Extract the Nth top-level element of a JSON array as a heap string."
    );
    let _ = writeln!(
        out,
        " * Returns NULL if the input is not an array, the index is out of bounds, or"
    );
    let _ = writeln!(out, " * allocation fails. Caller must free() the returned string.");
    let _ = writeln!(out, " */");
    let _ = writeln!(
        out,
        "static inline char *alef_json_array_get_index(const char *json, int index) {{"
    );
    let _ = writeln!(out, "    if (json == NULL || index < 0) return NULL;");
    let _ = writeln!(
        out,
        "    while (*json == ' ' || *json == '\\t' || *json == '\\n') json++;"
    );
    let _ = writeln!(out, "    if (*json != '[') return NULL;");
    let _ = writeln!(out, "    json++;");
    let _ = writeln!(out, "    int current = 0;");
    let _ = writeln!(out, "    while (*json) {{");
    let _ = writeln!(
        out,
        "        while (*json == ' ' || *json == '\\t' || *json == '\\n') json++;"
    );
    let _ = writeln!(out, "        if (*json == ']') return NULL;");
    let _ = writeln!(out, "        const char *elem_start = json;");
    let _ = writeln!(out, "        int depth = 0;");
    let _ = writeln!(out, "        int in_string = 0;");
    let _ = writeln!(out, "        for (; *json; json++) {{");
    let _ = writeln!(out, "            if (in_string) {{");
    let _ = writeln!(
        out,
        "                if (*json == '\\\\' && *(json + 1)) {{ json++; continue; }}"
    );
    let _ = writeln!(out, "                if (*json == '\"') in_string = 0;");
    let _ = writeln!(out, "                continue;");
    let _ = writeln!(out, "            }}");
    let _ = writeln!(out, "            if (*json == '\"') {{ in_string = 1; continue; }}");
    let _ = writeln!(out, "            if (*json == '{{' || *json == '[') depth++;");
    let _ = writeln!(out, "            else if (*json == '}}' || *json == ']') {{");
    let _ = writeln!(out, "                if (depth == 0) break;");
    let _ = writeln!(out, "                depth--;");
    let _ = writeln!(out, "            }}");
    let _ = writeln!(out, "            else if (*json == ',' && depth == 0) break;");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "        if (current == index) {{");
    let _ = writeln!(out, "            const char *elem_end = json;");
    let _ = writeln!(
        out,
        "            while (elem_end > elem_start && (*(elem_end - 1) == ' ' || *(elem_end - 1) == '\\t' || *(elem_end - 1) == '\\n')) elem_end--;"
    );
    let _ = writeln!(out, "            size_t elem_len = (size_t)(elem_end - elem_start);");
    let _ = writeln!(out, "            char *out_buf = (char *)malloc(elem_len + 1);");
    let _ = writeln!(out, "            if (!out_buf) return NULL;");
    let _ = writeln!(out, "            memcpy(out_buf, elem_start, elem_len);");
    let _ = writeln!(out, "            out_buf[elem_len] = '\\0';");
    let _ = writeln!(out, "            return out_buf;");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "        current++;");
    let _ = writeln!(out, "        if (*json == ']') return NULL;");
    let _ = writeln!(out, "        if (*json == ',') json++;");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    return NULL;");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "/**");
    let _ = writeln!(out, " * Count top-level elements in a JSON array string.");
    let _ = writeln!(out, " * Returns 0 for empty arrays (\"[]\") or NULL input.");
    let _ = writeln!(out, " */");
    let _ = writeln!(out, "static inline int alef_json_array_count(const char *json) {{");
    let _ = writeln!(out, "    if (json == NULL) return 0;");
    let _ = writeln!(out, "    /* Skip leading whitespace */");
    let _ = writeln!(
        out,
        "    while (*json == ' ' || *json == '\\t' || *json == '\\n') json++;"
    );
    let _ = writeln!(out, "    if (*json != '[') return 0;");
    let _ = writeln!(out, "    json++;");
    let _ = writeln!(out, "    /* Skip whitespace after '[' */");
    let _ = writeln!(
        out,
        "    while (*json == ' ' || *json == '\\t' || *json == '\\n') json++;"
    );
    let _ = writeln!(out, "    if (*json == ']') return 0;");
    let _ = writeln!(out, "    int count = 1;");
    let _ = writeln!(out, "    int depth = 0;");
    let _ = writeln!(out, "    int in_string = 0;");
    let _ = writeln!(
        out,
        "    for (; *json && !(*json == ']' && depth == 0 && !in_string); json++) {{"
    );
    let _ = writeln!(out, "        if (*json == '\\\\' && in_string) {{ json++; continue; }}");
    let _ = writeln!(
        out,
        "        if (*json == '\"') {{ in_string = !in_string; continue; }}"
    );
    let _ = writeln!(out, "        if (in_string) continue;");
    let _ = writeln!(out, "        if (*json == '[' || *json == '{{') depth++;");
    let _ = writeln!(out, "        else if (*json == ']' || *json == '}}') depth--;");
    let _ = writeln!(out, "        else if (*json == ',' && depth == 0) count++;");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    return count;");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);

    for (group, fixtures) in active_groups {
        let _ = writeln!(out, "/* Tests for category: {} */", group.category);
        for fixture in fixtures {
            let fn_name = sanitize_ident(&fixture.id);
            let _ = writeln!(out, "void test_{fn_name}(void);");
        }
        let _ = writeln!(out);
    }

    if !visitor_fixtures.is_empty() {
        let _ = writeln!(out, "/* Tests for category: visitor */");
        for fixture in visitor_fixtures {
            let fn_name = sanitize_ident(&fixture.id);
            let _ = writeln!(out, "void test_{fn_name}(void);");
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "#endif /* TEST_RUNNER_H */");
    out
}

pub(super) fn render_main_c(active_groups: &[(&FixtureGroup, Vec<&Fixture>)], visitor_fixtures: &[&Fixture]) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Block));
    let _ = writeln!(out, "#include <stdio.h>");
    let _ = writeln!(out, "#include \"test_runner.h\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "int main(void) {{");
    let _ = writeln!(out, "    int passed = 0;");
    let _ = writeln!(out);

    for (group, fixtures) in active_groups {
        let _ = writeln!(out, "    /* Category: {} */", group.category);
        for fixture in fixtures {
            let fn_name = sanitize_ident(&fixture.id);
            let _ = writeln!(out, "    printf(\"  Running test_{fn_name}...\");");
            let _ = writeln!(out, "    test_{fn_name}();");
            let _ = writeln!(out, "    printf(\" PASSED\\n\");");
            let _ = writeln!(out, "    passed++;");
        }
        let _ = writeln!(out);
    }

    if !visitor_fixtures.is_empty() {
        let _ = writeln!(out, "    /* Category: visitor */");
        for fixture in visitor_fixtures {
            let fn_name = sanitize_ident(&fixture.id);
            let _ = writeln!(out, "    printf(\"  Running test_{fn_name}...\");");
            let _ = writeln!(out, "    test_{fn_name}();");
            let _ = writeln!(out, "    printf(\" PASSED\\n\");");
            let _ = writeln!(out, "    passed++;");
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "    printf(\"\\nResults: %d passed, 0 failed\\n\", passed);");
    let _ = writeln!(out, "    return 0;");
    let _ = writeln!(out, "}}");
    out
}
