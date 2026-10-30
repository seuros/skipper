use super::*;

#[test]
fn pr_list_uri_encodes_bot_authors_and_repo() {
    for (author, encoded) in [
        ("me", "me"),
        ("seuros", "seuros"),
        ("github-actions[bot]", "github-actions%5Bbot%5D"),
        ("dependabot[bot]", "dependabot%5Bbot%5D"),
    ] {
        for spec in [None, Some("seuros/behavior_machines")] {
            let uri = pr_list_uri("open", author, spec);
            let query = if spec.is_some() { "?repo=seuros%2Fbehavior_machines" } else { "" };
            assert_eq!(uri, format!("skipper://prs/open/{encoded}{query}"));

            let parsed = url::Url::parse(&uri).expect("resource URI");
            assert_eq!(
                urlencoding::decode(parsed.path().rsplit('/').next().expect("author"))
                    .expect("decoded author"),
                author
            );
            assert_eq!(
                parsed.query_pairs().find(|(key, _)| key == "repo").map(|(_, value)| value),
                spec.map(std::borrow::Cow::Borrowed)
            );
        }
    }
}
