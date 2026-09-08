use super::*;

#[test]
fn test_error_messages() {
    let err = CliError::not_installed("gh");
    assert_eq!(err.to_string(), "gh is not installed or not in PATH");
    assert_eq!(err.cli(), "gh");
    assert!(err.is_unavailable());

    let err = CliError::auth_required("glab");
    assert!(err.needs_auth());

    let err = CliError::timeout("tea", Duration::from_secs(30));
    assert!(err.to_string().contains("30s"));
}
