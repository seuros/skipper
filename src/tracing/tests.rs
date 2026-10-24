use super::*;

#[test]
fn subscriber_preserves_level_filter_and_structured_fields() {
    let directory = tempfile::tempdir().expect("log directory");
    let path = directory.path().join("skipper.log");
    let file = std::fs::File::create(&path).expect("log file");
    let subscriber = subscriber::fmt()
        .with_writer(std::sync::Mutex::new(file))
        .with_max_level(Level::INFO)
        .with_ansi(false)
        .without_time()
        .finish();

    dispatcher::with_default(&Dispatch::new(subscriber), || {
        info!(provider = "github", "provider ready");
        debug!("filtered debug event");
    });

    let output = std::fs::read_to_string(path).expect("logs");
    assert!(output.contains("provider ready"));
    assert!(output.contains("provider=\"github\""));
    assert!(!output.contains("filtered debug event"));
}
