use super::*;

#[test]
fn test_graphql_issue_maps_deleted_author_and_missing_labels() {
    let raw = r#"{"repository":{"issues":{"nodes":[
        {"number":7,"title":"Crash","state":"OPEN","createdAt":"2026-09-30T10:00:00Z","updatedAt":"2026-10-01T10:00:00Z",
         "author":null,"labels":null,"comments":{"totalCount":0}},
        {"number":3,"title":"Docs","state":"CLOSED","createdAt":"2026-08-30T10:00:00Z","updatedAt":"2026-09-01T10:00:00Z",
         "author":{"login":"seuros"},"labels":{"nodes":[{"name":"docs"}]},"comments":{"totalCount":2}}
    ]}}}"#;
    let response: GqlData = serde_json::from_str(raw).expect("parse");
    let issues: Vec<IssueSummary> = response
        .repository
        .expect("repository")
        .issues
        .nodes
        .into_iter()
        .map(GqlIssue::into_summary)
        .collect();

    assert_eq!((issues[0].author.as_str(), issues[0].state.as_str()), ("ghost", "open"));
    assert_eq!(issues[0].labels, [] as [std::string::String; 0]);
    assert_eq!((issues[1].labels.as_slice(), issues[1].comments), (&["docs".to_string()][..], 2));
}
