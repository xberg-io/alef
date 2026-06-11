/// Advance byte position `i` in `s` past one full UTF-8 character, push that
/// character to `out`, and return the new byte position.
///
/// All the byte-crawling helpers below look for ASCII special characters only.
/// When none matches, they must advance by one full character (not one byte)
/// to avoid splitting multi-byte UTF-8 sequences.
#[inline]
pub(super) fn advance_char(s: &str, out: &mut String, i: usize) -> usize {
    // Safety: `i` must be a valid char boundary; callers guarantee this
    // because all branch points look for ASCII bytes which are always
    // single-byte char boundaries.
    let ch = s[i..].chars().next().expect("valid UTF-8 position");
    out.push(ch);
    i + ch.len_utf8()
}
