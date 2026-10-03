use super::*;

#[test]
fn test_readable_drops_details_and_html_comments() {
    let body = "_Minor_\n\n<details>\n<summary>x</summary>\n\n<details>inner</details>\nstill hidden\n</details>\n\n\n**Check the allocation.**\n<!-- fingerprint -->\n\nKeep this.\n";
    assert_eq!(readable(body, NOTE_LIMIT), "_Minor_\n\n**Check the allocation.**\n\nKeep this.");
    assert_eq!(readable("<!-- open", NOTE_LIMIT), "");
    let long = "é".repeat(NOTE_LIMIT + 5);
    let clipped = readable(&long, NOTE_LIMIT);
    assert_eq!(clipped.chars().count(), NOTE_LIMIT + 1);
    assert!(clipped.ends_with('…'));
}

#[test]
fn test_readable_drops_html_tags_but_keeps_code() {
    let body = "<a href=\"https://x\"><img src=\"y.svg\" alt=\"z\"></a>\n\n> [!WARNING]\n> ## Review limit reached\n\nKeep `a < b` and `Vec<u8>` and <sub>tiny</sub>.";
    assert_eq!(
        readable(body, NOTE_LIMIT),
        "> ## Review limit reached\n\nKeep `a < b` and `Vec<u8>` and tiny."
    );
}
