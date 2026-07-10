use super::{DocTarget, utf8::advance_char};

/// Replace Rust generic type wrappers in prose.
pub(super) fn replace_type_wrappers(s: &str, target: DocTarget) -> String {
    let mut out = s.to_string();

    let vec_u8_replacement = match target {
        DocTarget::PhpDoc => "string",
        DocTarget::JavaDoc => "byte[]",
        DocTarget::TsDoc | DocTarget::JsDoc => "Uint8Array",
        DocTarget::CSharpDoc => "byte[]",
    };
    out = replace_generic1(&out, "Vec", "u8", vec_u8_replacement);

    let map_replacement_fn = |k: &str, v: &str| match target {
        DocTarget::PhpDoc => format!("array<{k}, {v}>"),
        DocTarget::JavaDoc => format!("Map<{k}, {v}>"),
        DocTarget::TsDoc | DocTarget::JsDoc => format!("Record<{k}, {v}>"),
        DocTarget::CSharpDoc => format!("Dictionary<{k}, {v}>"),
    };
    out = replace_generic2(&out, "HashMap", &map_replacement_fn);

    out = replace_generic1_passthrough(&out, "Vec", |inner| format!("{inner}[]"));

    let option_replacement_fn = |inner: &str| match target {
        DocTarget::PhpDoc => format!("{inner}?"),
        DocTarget::JavaDoc => format!("{inner} | null"),
        DocTarget::TsDoc | DocTarget::JsDoc => format!("{inner} | undefined"),
        DocTarget::CSharpDoc => format!("{inner}?"),
    };
    out = replace_generic1_passthrough(&out, "Option", option_replacement_fn);

    if matches!(target, DocTarget::CSharpDoc) {
        out = replace_generic2(&out, "Result", &|t: &str, _e: &str| t.to_string());
    }

    for wrapper in &["Arc", "Box", "Mutex", "RwLock", "Rc", "Cell", "RefCell"] {
        out = replace_generic1_passthrough(&out, wrapper, |inner| inner.to_string());
    }

    out
}

/// Replace `Name<SingleArg>` where SingleArg is an exact literal (e.g. `Vec<u8>`).
fn replace_generic1(s: &str, name: &str, arg: &str, replacement: &str) -> String {
    let pattern = format!("{name}<{arg}>");
    s.replace(&pattern, replacement)
}

/// Replace `Name<T>` → `f(T)` for an arbitrary inner type expression.
///
/// Handles nested generics by counting angle-bracket depth.
fn replace_generic1_passthrough<F>(s: &str, name: &str, f: F) -> String
where
    F: Fn(&str) -> String,
{
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let prefix = format!("{name}<");
    let pbytes = prefix.as_bytes();
    let bytes = s.as_bytes();

    while i < bytes.len() {
        if bytes[i..].starts_with(pbytes) {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            if before_ok {
                let inner_start = i + pbytes.len();
                let mut depth = 1usize;
                let mut j = inner_start;
                while j < bytes.len() {
                    match bytes[j] {
                        b'<' => depth += 1,
                        b'>' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if depth == 0 && j < bytes.len() {
                    let inner = &s[inner_start..j];
                    out.push_str(&f(inner));
                    i = j + 1;
                    continue;
                }
            }
        }
        i = advance_char(s, &mut out, i);
    }
    out
}

/// Replace `Name<K, V>` → `f(K, V)` for two-argument generics (e.g. `HashMap`).
fn replace_generic2<F>(s: &str, name: &str, f: &F) -> String
where
    F: Fn(&str, &str) -> String,
{
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let prefix = format!("{name}<");
    let pbytes = prefix.as_bytes();
    let bytes = s.as_bytes();

    while i < bytes.len() {
        if bytes[i..].starts_with(pbytes) {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            if before_ok {
                let inner_start = i + pbytes.len();
                let mut depth = 1usize;
                let mut j = inner_start;
                while j < bytes.len() {
                    match bytes[j] {
                        b'<' => depth += 1,
                        b'>' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if depth == 0 && j < bytes.len() {
                    let inner = &s[inner_start..j];
                    let split = split_on_comma_at_top_level(inner);
                    if let Some((k, v)) = split {
                        out.push_str(&f(k.trim(), v.trim()));
                        i = j + 1;
                        continue;
                    }
                }
            }
        }
        i = advance_char(s, &mut out, i);
    }
    out
}

/// Split `s` on the first comma that is at angle-bracket depth 0.
fn split_on_comma_at_top_level(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    for (idx, ch) in s.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => return Some((&s[..idx], &s[idx + 1..])),
            _ => {}
        }
    }
    None
}
