use webrtc::runtime::{Runtime, TokioRuntime, default_runtime};

fn assert_runtime<T: Runtime>() {}

#[test]
fn packaged_stack_exposes_one_tokio_runtime() {
    assert_runtime::<TokioRuntime>();
    assert!(default_runtime().is_some());
}

#[test]
fn package_manifest_does_not_offer_smol_runtime() {
    let manifest = include_str!("../Cargo.toml");

    for removed_entry in [
        "runtime-smol",
        "[dependencies.smol]",
        "[dependencies.async-broadcast]",
    ] {
        assert!(
            !manifest.contains(removed_entry),
            "removed Smol entry reappeared: {removed_entry}"
        );
    }
}
