//! ASCII case-insensitive search, without lowercasing a copy first.

/// Byte offset of the first `needle` in `haystack`, ASCII case ignored.
/// `needle` is ASCII, so a match starts on a char boundary of `haystack`.
pub(crate) fn find_ignore_case(haystack: &str, needle: &str) -> Option<usize> {
    debug_assert!(!needle.is_empty() && needle.is_ascii(), "needle must be non-empty ASCII");
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}
