use regex::Regex;
use std::sync::OnceLock;

pub(super) fn catch_callback_serialization(source: &str) -> String {
    callback_regex()
        .replace_all(source, |caps: &regex::Captures<'_>| {
            let rendered = crate::backends::dart::template_env::render(
                "dart_callback_serializer_guard.jinja",
                crate::alef_context! {
                    invoke => &caps["invoke"], binding => &caps["binding"], encode => &caps["encode"]
                },
            );
            let indent = &caps["indent"];
            rendered
                .trim_end()
                .lines()
                .map(|line| format!("{indent}{line}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .into_owned()
}

pub(crate) fn validate_callback_serialization(source: &str) -> anyhow::Result<()> {
    if !source.contains("putBigUint64(_alefCheckU64BeforeEncode(self))") {
        return Ok(());
    }
    let callbacks = invocation_regex().find_iter(source).count();
    let guarded = source.matches("rawOutput = null;").count();
    anyhow::ensure!(
        callbacks == guarded,
        "unsupported FRB Dart callback serializer shape: {callbacks} callbacks but {guarded} guarded error paths"
    );
    Ok(())
}

fn invocation_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"rawOutput\s*=\s*Box\(\s*await\s+raw\(").unwrap())
}

fn callback_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
        r#"(?m)^(?P<indent>[ \t]*)try\s*\{\s*rawOutput\s*=\s*Box\((?P<invoke>await\s+raw\([^;{}]*\))\);\s*"#,
        r#"\}\s*catch\s*\(e,\s*s\)\s*\{\s*rawError\s*=\s*Box\(AnyhowException\("\$e\\n\\n\$s"\)\);\s*\}\s*"#,
        r#"final\s+serializer\s*=\s*SseSerializer\((?P<binding>[A-Za-z_][A-Za-z0-9_]*)\);\s*"#,
        r#"assert\(\(rawOutput\s*!=\s*null\)\s*\^\s*\(rawError\s*!=\s*null\)\);\s*"#,
        r#"if\s*\(rawOutput\s*!=\s*null\)\s*\{\s*serializer\.buffer\.putUint8\(0\);\s*"#,
        r#"(?P<encode>sse_encode_[A-Za-z0-9_]+\(rawOutput\.value,\s*serializer\);)\s*\}\s*else\s*\{\s*"#,
        r#"serializer\.buffer\.putUint8\(1\);\s*sse_encode_AnyhowException\(rawError!\.value,\s*serializer\);\s*\}"#,
        r#"\s*final\s+output\s*=\s*serializer\.intoRaw\(\);"#,
    )).expect("pinned FRB callback serializer regex")
    })
}

#[cfg(test)]
#[path = "callback_errors_tests.rs"]
mod tests;
