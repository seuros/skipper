use super::*;

#[test]
fn test_provider_status() {
    let available = ProviderStatus::Available { version: Version::new(1, 0, 0) };
    assert!(available.is_available());
    assert_eq!(available.version(), Some(&Version::new(1, 0, 0)));

    let not_installed = ProviderStatus::NotInstalled;
    assert!(!not_installed.is_available());
    assert_eq!(not_installed.version(), None);
}

#[test]
fn test_registry_new() {
    let registry = Registry::new();
    assert!(registry.enabled_names().is_empty());
}
