use super::*;

#[test]
fn test_readable_opens_details_and_drops_html_comments() {
    let body = "_Minor_\n\n<details>\n<summary>Suggested fix</summary>\n\n<details open>nested</details>\nfix\n</details>\n\n\n**Check the allocation.**\n<!-- fingerprint -->\n\nKeep this.\n";
    assert_eq!(
        readable(body),
        "_Minor_\n\nSuggested fix\n\nnested\nfix\n\n**Check the allocation.**\n\nKeep this."
    );
    assert_eq!(readable("<!-- open"), "");
}

#[test]
fn test_readable_keeps_long_bodies_whole() {
    let body = format!("{}\n\nLast suggested test.", "spec ".repeat(1225).trim_end());
    assert_eq!(readable(&body), body);
}

#[cfg(feature = "github")]
#[test]
fn test_clip_marks_the_cut() {
    let clipped = clip("é".repeat(EVENT_BODY_LIMIT + 5), EVENT_BODY_LIMIT);
    assert_eq!(clipped.chars().count(), EVENT_BODY_LIMIT + 1);
    assert!(clipped.ends_with('…'));
    assert_eq!(clip("short".into(), EVENT_BODY_LIMIT), "short");
}

#[test]
fn test_readable_drops_html_tags_but_keeps_code() {
    let body = "<a href=\"https://x\"><img src=\"y.svg\" alt=\"z\"></a>\n\n> [!WARNING]\n> ## Review limit reached\n\nKeep `a < b` and `Vec<u8>` and <sub>tiny</sub>.";
    assert_eq!(readable(body), "> ## Review limit reached\n\nKeep `a < b` and `Vec<u8>` and tiny.");
}

#[test]
fn test_readable_decodes_entities_outside_code() {
    let body = "Use &lt;details&gt; &amp; keep &amp;lt;\n```html\n&lt;b&gt;\n```";
    assert_eq!(readable(body), "Use <details> & keep &lt;\n```html\n&lt;b&gt;\n```");
}
