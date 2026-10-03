use super::*;

#[test]
fn test_migrated_issue_keeps_original_author() {
    let issue: Issue = serde_json::from_str(
        r#"{"number":12,"title":"Imported","state":"open","user":{"login":"ghost-importer"},
            "original_author":"seuros","labels":[{"name":"bug"}],"assignees":null,
            "body":"<!-- template -->Steps","comments":1,"updated_at":"2026-10-01T10:00:00Z",
            "pull_request":null}"#,
    )
    .expect("parse");
    let comment: Comment = serde_json::from_str(
        r#"{"id":5,"user":{"login":"maintainer"},"original_author":"","body":"Fixed","created_at":"2026-10-02T10:00:00Z"}"#,
    )
    .expect("parse");

    let thread = issue.into_thread(vec![comment.into_note()]);
    assert_eq!(thread.author, "seuros");
    assert_eq!(thread.labels, ["bug"]);
    assert_eq!(thread.body, "Steps");
    assert_eq!(thread.notes[0].author, "maintainer");
}
