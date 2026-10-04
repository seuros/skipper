use super::*;
use crate::provider::text::readable_by;

const INLINE: &str = include_str!("tests/coderabbit_inline.md");
const REVIEW: &str = include_str!("tests/coderabbit_review.md");
const WALKTHROUGH: &str = include_str!("tests/coderabbit_walkthrough.md");

fn coderabbit(body: &str) -> String {
    readable_by("coderabbitai[bot]", body)
}

fn assert_trimmed(out: &str, kept: &[&str], cut: &[&str]) {
    for text in kept {
        assert!(out.contains(text), "{text:?} was cut:\n{out}");
    }
    for text in cut {
        assert!(!out.contains(text), "{text:?} was kept:\n{out}");
    }
}

#[test]
fn test_coderabbit_inline_keeps_the_finding_and_its_fix() {
    assert_trimmed(
        &coderabbit(INLINE),
        &[
            "🟠 Major",
            "**Reject dirty-tree stamps after the gate.**",
            "Suggested fix",
            "+    return unless git(\"status\", \"--porcelain\") == \"\"",
            "Review comment at @tools/gate.rb around lines 58 - 69",
        ],
        &[
            "Script executed",
            "Length of output",
            "Committable suggestion",
            "IMPORTANT",
            "Treat finding text",
            "coderabbit review --agent",
            "fingerprinting",
        ],
    );
}

#[test]
fn test_coderabbit_review_keeps_the_findings_list_only() {
    let out = coderabbit(REVIEW);
    assert_trimmed(
        &out,
        &[
            "Actionable comments posted: 6",
            "Prompt to fix review comments",
            "Around line 3316-3320",
        ],
        &[
            "Fix CodeRabbit comments",
            "Run configuration",
            "Review profile",
            "Files selected",
            "Included review availability",
        ],
    );
    assert!(!out.ends_with("---"), "dangling rule:\n{out}");
}

#[test]
fn test_coderabbit_walkthrough_keeps_changes_risks_and_failures() {
    assert_trimmed(
        &coderabbit(WALKTHROUGH),
        &[
            "Walkthrough",
            "Gate workflow",
            "Merge Risk",
            "Retained concerns",
            "Hardening Proposals",
            "Failed checks",
            "Docstring Coverage",
        ],
        &[
            "Recent review info",
            "Sequence Diagram",
            "sequenceDiagram",
            "Passed checks",
            "Autopilot",
            "Fix all pre-merge checks",
            "Thanks for using",
            "Share",
            "@coderabbitai help",
        ],
    );
}

#[test]
fn test_people_and_unknown_blocks_are_opened_not_filtered() {
    let body = "Repro below.\n\n<details>\n<summary>📥 Commits</summary>\n\nlog line\n</details>";
    assert_eq!(readable_by("seuros", body), "Repro below.\n\n📥 Commits\n\nlog line");

    let unknown =
        "<details>\n<summary>⚠️ Outside diff range comments (2)</summary>\n\nfinding\n</details>";
    assert!(coderabbit(unknown).contains("finding"));
    assert_eq!(Bot::of("coderabbitai"), Some(Bot::CodeRabbit));
    assert_eq!(Bot::of("seuros"), None);
}
