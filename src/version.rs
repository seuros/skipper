use crate::error::{CliError, Result};
use semver::Version;
use std::borrow::Cow;

pub mod minimum {
    use semver::Version;

    pub const fn github() -> Version {
        Version::new(2, 50, 0)
    }

    pub const fn tea() -> Version {
        Version::new(0, 9, 0)
    }

    pub const fn gitlab() -> Version {
        Version::new(1, 40, 0)
    }

    pub const fn git() -> Version {
        Version::new(2, 25, 0)
    }
}

pub fn parse_version(output: &str, cli: &str) -> Result<Version> {
    let stripped = strip_ansi(output);
    let version_str = extract_version_string(&stripped)
        .ok_or_else(|| CliError::parse_error(cli, output, "could not find version pattern"))?;

    Version::parse(version_str)
        .map_err(|e| CliError::parse_error(cli, output, format!("invalid semver: {e}")))
}

fn extract_version_string(output: &str) -> Option<&str> {
    for line in output.lines() {
        let line = line.trim();

        // The offset is into `line` itself: lowercasing a copy to search it
        // shifts offsets past any non-ASCII char.
        if let Some(version_start) = crate::ascii::find_ignore_case(line, "version") {
            let rest = line[version_start + 7..].trim_start().trim_start_matches(':').trim_start();

            if rest.starts_with("development") {
                return Some("999.0.0");
            }

            let version_end = rest
                .find(|c: char| c.is_whitespace() || c == '(' || c == '-')
                .unwrap_or(rest.len());

            let version = &rest[..version_end];

            let version = version.strip_prefix('v').unwrap_or(version);

            if looks_like_version(version) {
                return Some(version);
            }
        }

        if let Some(second) = line.split_whitespace().nth(1) {
            let potential_version = second.trim_start_matches('v');
            if looks_like_version(potential_version) {
                return Some(potential_version);
            }
        }
    }

    None
}

/// `s` without ANSI escapes; borrowed when it has none.
fn strip_ansi(s: &str) -> Cow<'_, str> {
    if !s.contains('\x1b') {
        return Cow::Borrowed(s);
    }
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }

    Cow::Owned(result)
}

fn looks_like_version(s: &str) -> bool {
    let mut parts = s.split('.');
    matches!(
        (parts.next(), parts.next()),
        (Some(major), Some(minor)) if major.parse::<u64>().is_ok() && minor.parse::<u64>().is_ok()
    )
}

#[cfg(test)]
mod tests;
