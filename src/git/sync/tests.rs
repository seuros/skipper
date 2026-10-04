use super::*;

#[test]
fn test_checked_ref_refuses_force_delete_and_options() {
    assert_eq!(checked_ref("master").unwrap(), "master");
    assert_eq!(checked_ref("release/2.x").unwrap(), "release/2.x");
    for unsafe_ref in ["+master", ":master", "master:main", "--force", "-f", "", "a b", "x\n"] {
        assert!(checked_ref(unsafe_ref).is_err(), "{unsafe_ref:?} must be refused");
    }
}
