use super::*;
use crate::git::test_support::{git, git_output, repo_with_commit};

#[tokio::test]
async fn pull_fast_forwards_with_rebase_configured_and_an_unstaged_edit() {
    let upstream = tempfile::tempdir().unwrap();
    let checkout = tempfile::tempdir().unwrap();
    let (upstream, checkout) = (upstream.path(), checkout.path());
    repo_with_commit(upstream, "original\n");
    git(checkout, &["clone", "-b", "main", upstream.to_str().unwrap(), "."]);
    git(checkout, &["config", "branch.main.rebase", "true"]);
    git(checkout, &["config", "rebase.autoStash", "false"]);
    git(upstream, &["commit", "--allow-empty", "-m", "advance upstream"]);
    let edited = checkout.join("file.txt");
    std::fs::write(&edited, "local edit\n").unwrap();

    let outcome = pull(checkout).await.unwrap();

    assert!(outcome.ok, "{}", outcome.output);
    assert_eq!(
        git_output(checkout, &["rev-parse", "HEAD"]),
        git_output(upstream, &["rev-parse", "HEAD"])
    );
    assert_eq!(std::fs::read_to_string(edited).unwrap(), "local edit\n");
}

#[test]
fn test_checked_ref_refuses_force_delete_and_options() {
    assert_eq!(checked_ref("master").unwrap(), "master");
    assert_eq!(checked_ref("release/2.x").unwrap(), "release/2.x");
    for unsafe_ref in ["+master", ":master", "master:main", "--force", "-f", "", "a b", "x\n"] {
        assert!(checked_ref(unsafe_ref).is_err(), "{unsafe_ref:?} must be refused");
    }
}

#[test]
fn test_only_network_failures_are_retried() {
    for blip in [
        "fatal: unable to access 'https://github.com/o/r.git/': Recv failure: Connection reset by peer",
        "fatal: unable to access 'https://x/': Could not resolve host: github.com",
        "error: RPC failed; curl 56 GnuTLS recv error\nfatal: early EOF",
        "fatal: unable to access 'https://x/': The requested URL returned error: 502",
        "kex_exchange_identification: read: Connection reset by peer",
    ] {
        assert!(transient(blip), "{blip:?} should be retried");
    }
    for verdict in [
        " ! [rejected]        master -> master (non-fast-forward)",
        "remote: Invalid username or token.\nfatal: Authentication failed for 'https://x/'",
        "fatal: Not possible to fast-forward, aborting.",
        "fatal: The requested URL returned error: 403",
    ] {
        assert!(!transient(verdict), "{verdict:?} should not be retried");
    }
}
