use super::*;

#[tokio::test]
async fn test_execute_echo() {
    let output = execute_default("echo", &["hello"]).await.unwrap();
    assert!(output.success());
    assert!(output.stdout.trim() == "hello");
}

#[tokio::test]
async fn test_not_installed() {
    let result = execute_default("definitely_not_a_real_cli_12345", &["--version"]).await;
    assert!(matches!(result, Err(CliError::NotInstalled { .. })));
}

#[tokio::test]
async fn test_is_installed() {
    assert!(is_installed("echo").await);
    assert!(!is_installed("definitely_not_a_real_cli_12345").await);
}
