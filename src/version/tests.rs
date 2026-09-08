use super::*;

#[test]
fn test_parse_gh_version() {
    let output = "gh version 2.83.2 (2025-12-10)\nhttps://github.com/cli/cli/releases/tag/v2.83.2";
    let version = parse_version(output, "gh").unwrap();
    assert_eq!(version, Version::new(2, 83, 2));
}

#[test]
fn test_parse_tea_version() {
    let output = "tea version 0.9.2";
    let version = parse_version(output, "tea").unwrap();
    assert_eq!(version, Version::new(0, 9, 2));
}

#[test]
fn test_parse_glab_version() {
    let output = "glab version 1.46.1 (2024-10-01)";
    let version = parse_version(output, "glab").unwrap();
    assert_eq!(version, Version::new(1, 46, 1));
}

#[test]
fn test_parse_version_with_v_prefix() {
    let output = "version v1.2.3";
    let version = parse_version(output, "test").unwrap();
    assert_eq!(version, Version::new(1, 2, 3));
}

#[test]
fn test_minimum_versions() {
    assert!(minimum::github() >= Version::new(2, 0, 0));
    assert!(minimum::tea() >= Version::new(0, 9, 0));
    assert!(minimum::gitlab() >= Version::new(1, 0, 0));
}

#[test]
fn test_invalid_version() {
    let output = "not a version string";
    let result = parse_version(output, "test");
    assert!(result.is_err());
}

#[test]
fn test_parse_tea_development_version() {
    let output = "Version: \x1b[1mdevelopment\x1b[0m\tgolang: 1.25.3";
    let version = parse_version(output, "tea").unwrap();
    assert_eq!(version, Version::new(999, 0, 0));
}

#[test]
fn test_parse_tea_release_version_with_ansi() {
    let output = "Version: \x1b[1m0.14.2\x1b[0m\tgolang: 1.26.0\tgo-sdk: v1.1.0";
    let version = parse_version(output, "tea").unwrap();
    assert_eq!(version, Version::new(0, 14, 2));
}

#[test]
fn test_parse_glab_version_no_version_keyword() {
    let output = "glab 1.80.4 (f4b518e9)";
    let version = parse_version(output, "glab").unwrap();
    assert_eq!(version, Version::new(1, 80, 4));
}

#[test]
fn test_parse_git_version() {
    let output = "git version 2.39.5 (Apple Git-154)";
    let version = parse_version(output, "git").unwrap();
    assert_eq!(version, Version::new(2, 39, 5));
}
