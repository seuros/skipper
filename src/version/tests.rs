use super::*;

#[test]
fn test_parse_version_outputs() {
    const CASES: &[(&str, (u64, u64, u64))] = &[
        (
            "gh version 2.83.2 (2025-12-10)\nhttps://github.com/cli/cli/releases/tag/v2.83.2",
            (2, 83, 2),
        ),
        ("tea version 0.9.2", (0, 9, 2)),
        ("glab version 1.46.1 (2024-10-01)", (1, 46, 1)),
        ("version v1.2.3", (1, 2, 3)),
        ("Version: \x1b[1mdevelopment\x1b[0m\tgolang: 1.25.3", (999, 0, 0)),
        ("Version: \x1b[1m0.14.2\x1b[0m\tgolang: 1.26.0\tgo-sdk: v1.1.0", (0, 14, 2)),
        ("glab 1.80.4 (f4b518e9)", (1, 80, 4)),
        ("git version 2.39.5 (Apple Git-154)", (2, 39, 5)),
        // Non-ASCII before the keyword: the match offset is into the line as
        // written, not into a lowercased copy of different length.
        ("İİİ version 1.2.3", (1, 2, 3)),
    ];
    for (output, (major, minor, patch)) in CASES {
        let version = parse_version(output, "cli").unwrap_or_else(|e| panic!("{output:?}: {e}"));
        assert_eq!(version, Version::new(*major, *minor, *patch), "output: {output:?}");
    }
}

#[test]
fn test_invalid_version() {
    assert!(parse_version("not a version string", "test").is_err());
}
