//! Tiny helpers shared by passes.

/// Is `s` a valid ECMAScript IdentifierName? (Reserved words are allowed,
/// because in member position `obj.if` is legal.) Good enough for ASCII-heavy
/// obfuscator output; extend with full Unicode ID tables if needed.
pub fn is_identifier_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if is_id_start(c) => {}
        _ => return false,
    }
    chars.all(is_id_continue)
}

fn is_id_start(c: char) -> bool {
    c == '$' || c == '_' || c.is_ascii_alphabetic() || (c as u32 > 0x7f && c.is_alphabetic())
}

fn is_id_continue(c: char) -> bool {
    c == '$' || c == '_' || c.is_ascii_alphanumeric() || (c as u32 > 0x7f && c.is_alphanumeric())
}
