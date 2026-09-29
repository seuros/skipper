use super::*;

#[test]
fn test_readable_body_drops_details_and_html_comments() {
    let body = "_Minor_\n\n<details>\n<summary>x</summary>\n\n<details>inner</details>\nstill hidden\n</details>\n\n\n**Check the allocation.**\n<!-- fingerprint -->\n\nKeep this.\n";
    assert_eq!(readable_body(body), "_Minor_\n\n**Check the allocation.**\n\nKeep this.");
    assert_eq!(readable_body("<!-- open"), "");
    let long = "é".repeat(BODY_LIMIT + 5);
    let clipped = readable_body(&long);
    assert_eq!(clipped.chars().count(), BODY_LIMIT + 1);
    assert!(clipped.ends_with('…'));
}

#[test]
fn test_readable_body_drops_html_tags_but_keeps_code() {
    let body = "<a href=\"https://x\"><img src=\"y.svg\" alt=\"z\"></a>\n\n> [!WARNING]\n> ## Review limit reached\n\nKeep `a < b` and `Vec<u8>` and <sub>tiny</sub>.";
    assert_eq!(
        readable_body(body),
        "> ## Review limit reached\n\nKeep `a < b` and `Vec<u8>` and tiny."
    );
}
