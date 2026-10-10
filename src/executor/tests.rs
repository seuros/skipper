use super::*;

#[tokio::test]
async fn test_execute_echo() {
    let output = execute("echo", &["hello"], DEFAULT_TIMEOUT).await.unwrap();
    assert!(output.success());
    assert_eq!(output.stdout.trim(), "hello");
}

#[tokio::test]
async fn test_not_installed() {
    let result = execute("definitely_not_a_real_cli_12345", &["--version"], DEFAULT_TIMEOUT).await;
    assert!(matches!(result, Err(CliError::NotInstalled { .. })));
}
