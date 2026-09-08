use crate::error::{CliError, Result};
use semver::Version;

pub mod minimum {
    use semver::Version;

    pub fn github() -> Version {
        Version::new(2, 50, 0)
    }

    pub fn tea() -> Version {
        Version::new(0, 9, 0)
    }

    pub fn gitlab() -> Version {
        Version::new(1, 40, 0)
    }

    pub fn git() -> Version {
        Version::new(2, 25, 0)
    }
}

pub fn parse_version(output: &str, cli: &str) -> Result<Version> {
    let version_str = extract_version_string(output)
        .ok_or_else(|| CliError::parse_error(cli, output, "could not find version pattern"))?;

    Version::parse(&version_str)
        .map_err(|e| CliError::parse_error(cli, output, format!("invalid semver: {e}")))
}

fn extract_version_string(output: &str) -> Option<String> {
    let output = strip_ansi(output);

    for line in output.lines() {
        let line = line.trim();

        if let Some(version_start) = line.to_lowercase().find("version") {
            let rest = line[version_start + 7..].trim_start().trim_start_matches(':').trim_start();

            if rest.starts_with("development") {
                return Some("999.0.0".to_string());
            }

            let version_end = rest
                .find(|c: char| c.is_whitespace() || c == '(' || c == '-')
                .unwrap_or(rest.len());

            let version = &rest[..version_end];

            let version = version.strip_prefix('v').unwrap_or(version);

            if looks_like_version(version) {
                return Some(version.to_string());
            }
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let potential_version = parts[1].trim_start_matches('v');
            if looks_like_version(potential_version) {
                return Some(potential_version.to_string());
            }
        }
    }

    None
}

fn strip_ansi(s: &str) -> String {
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

    result
}

fn looks_like_version(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() < 2 {
        return false;
    }

    parts[0].parse::<u64>().is_ok() && parts[1].parse::<u64>().is_ok()
}

#[cfg(test)]
mod tests;
