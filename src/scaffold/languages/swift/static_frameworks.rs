use anyhow::Context as _;
use std::ops::Range;
use std::path::Path;

const SYSTEM_CALL: &[&str] = &[
    ".",
    "linkedFramework",
    "(",
    "\"SystemConfiguration\"",
    ",",
    ".",
    "when",
    "(",
    "platforms",
    ":",
    "[",
    ".",
    "macOS",
    "]",
    ")",
    ")",
];
const DIRECTORY: &str = ".linkedFramework(\"OpenDirectory\", .when(platforms: [.macOS])),";

type Token<'a> = (Range<usize>, &'a str);

// Strings remain single tokens so quoted flags cannot impersonate Swift declarations. Unknown
// raw/multiline strings decline the migration instead of guessing at their lexical boundaries. ~keep
fn tokens(source: &str) -> Option<Vec<Token<'_>>> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let start = index;
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index = source[index..].find('\n').map_or(bytes.len(), |end| index + end);
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                let mut depth = 1;
                while depth > 0 {
                    let pair = bytes.get(index..index + 2)?;
                    match pair {
                        b"/*" => depth += 1,
                        b"*/" => depth -= 1,
                        _ => {}
                    }
                    index += if pair == b"/*" || pair == b"*/" { 2 } else { 1 };
                }
            }
            b'#' if matches!(bytes.get(index + 1), Some(b'#' | b'"')) => return None,
            b'"' => {
                if bytes.get(index..index + 3) == Some(b"\"\"\"") {
                    return None;
                }
                index += 1;
                loop {
                    match *bytes.get(index)? {
                        b'\\' => index += 2,
                        b'"' => {
                            index += 1;
                            break;
                        }
                        b'\n' | b'\r' => return None,
                        _ => index += 1,
                    }
                }
                result.push((start..index, &source[start..index]));
            }
            byte if byte.is_ascii_whitespace() => index += 1,
            byte if byte.is_ascii_alphabetic() || byte == b'_' => {
                index += 1;
                while bytes
                    .get(index)
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                {
                    index += 1;
                }
                result.push((start..index, &source[start..index]));
            }
            _ => {
                index += source[index..].chars().next()?.len_utf8();
                result.push((start..index, &source[start..index]));
            }
        }
    }
    Some(result)
}

fn closing(tokens: &[Token<'_>], start: usize) -> Option<usize> {
    let close = match tokens.get(start)?.1 {
        "(" => ")",
        "[" => "]",
        _ => return None,
    };
    let mut index = start + 1;
    while index < tokens.len() {
        match tokens[index].1 {
            value if value == close => return Some(index),
            "(" | "[" => index = closing(tokens, index)?,
            ")" | "]" => return None,
            _ => {}
        }
        index += 1;
    }
    None
}

fn field<'a>(tokens: &'a [Token<'a>], name: &str) -> Option<&'a [Token<'a>]> {
    let mut index = 0;
    let mut found = None;
    while index < tokens.len() {
        let start = index;
        while index < tokens.len() && tokens[index].1 != "," {
            if matches!(tokens[index].1, "(" | "[") {
                index = closing(tokens, index)?;
            }
            index += 1;
        }
        if tokens[start].1 == name && tokens.get(start + 1)?.1 == ":" {
            if found.is_some() {
                return None;
            }
            found = Some(&tokens[start + 2..index]);
        }
        index += 1;
    }
    found
}

fn bridge<'a>(tokens: &'a [Token<'a>]) -> Option<&'a [Token<'a>]> {
    let mut found = None;
    for (index, window) in tokens.windows(3).enumerate() {
        if window.iter().map(|token| token.1).eq([".", "target", "("]) {
            let end = closing(tokens, index + 2)?;
            let args = &tokens[index + 3..end];
            if field(args, "name").is_some_and(|value| value.len() == 1 && value[0].1 == "\"RustBridge\"") {
                if found.is_some() {
                    return None;
                }
                found = Some(args);
            }
        }
    }
    found
}

fn insertion(source: &str, relative: &Path) -> Option<(usize, String)> {
    let tokens = tokens(source)?;
    let target = bridge(&tokens)?;
    let expected = match relative.to_str()? {
        "Package.swift" => "\"packages/swift/Sources/RustBridge\"",
        "packages/swift/Package.swift" => "\"Sources/RustBridge\"",
        _ => return None,
    };
    let path = field(target, "path")?;
    if path.len() != 1 || path[0].1 != expected {
        return None;
    }
    let list = field(target, "linkerSettings")?;
    if list.first()?.1 != "[" || closing(list, 0)? != list.len() - 1 {
        return None;
    }
    let mut index = 1;
    let mut anchor = None;
    while index < list.len() - 1 {
        let start = index;
        while index < list.len() - 1 && list[index].1 != "," {
            if matches!(list[index].1, "(" | "[") {
                index = closing(list, index)?;
            }
            index += 1;
        }
        let entry = &list[start..index];
        if entry
            .iter()
            .take(4)
            .map(|token| token.1)
            .eq([".", "linkedFramework", "(", "\"OpenDirectory\""])
        {
            return None;
        }
        if entry.iter().map(|token| token.1).eq(SYSTEM_CALL.iter().copied()) {
            if anchor.is_some() || list[index].1 != "," {
                return None;
            }
            anchor = Some((entry[0].0.start, list[index].0.end, list.get(index + 1)?.0.start));
        }
        index += 1;
    }
    let (start, end, next) = anchor?;
    let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
    let indent = &source[line_start..start];
    if !indent.bytes().all(|byte| byte == b' ' || byte == b'\t') {
        return None;
    }
    let line_end = end + source[end..].find('\n')? + 1;
    if line_end > next {
        return None;
    }
    // A trailing block comment can cross newlines; the proven whitespace-only prefix is outside it. ~keep
    Some((line_start, format!("{indent}{DIRECTORY}\n")))
}

pub(crate) fn migrate_swift_static_frameworks(base_dir: &Path, relative: &Path) -> anyhow::Result<bool> {
    let path = crate::cli::pipeline::generate::write::contained_output_path(base_dir, relative)?;
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let Some((offset, line)) = insertion(&existing, relative) else {
        tracing::debug!(path = %path.display(), "Swift framework migration unchanged: existing or unrecognized settings");
        return Ok(false);
    };
    let parent = path.parent().context("Swift manifest has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).context("create Swift framework migration file")?;
    use std::io::Write as _;
    write!(temporary, "{}{}{}", &existing[..offset], line, &existing[offset..])
        .context("write Swift framework migration")?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .context("replace Swift manifest")?;
    tracing::info!(path = %path.display(), "added macOS OpenDirectory framework to existing RustBridge target");
    Ok(true)
}
