#[test]
fn media_core_manifest_does_not_include_an_alternate_runtime() {
    let manifest = include_str!("../Cargo.toml");

    for removed_dependency in ["async-std", "smol"] {
        assert!(
            !manifest.contains(removed_dependency),
            "removed alternate runtime dependency reappeared: {removed_dependency}"
        );
    }
}
