// SPDX-FileCopyrightText: 2026 Bridgefu contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

const MANIFEST: &str = include_str!("../Cargo.toml");
const LIBRARY: &str = include_str!("../src/lib.rs");
const ADMISSION: &str = include_str!("../src/admission.rs");

#[test]
fn published_package_is_embeddable_only() {
    assert!(MANIFEST.contains("autobins = false"));
    assert!(MANIFEST.contains("default = []"));
    assert!(!MANIFEST.contains("\nruntime = ["));
    assert!(!MANIFEST.contains("[[bin]]"));
    assert!(MANIFEST
        .contains("metrics-prometheus = [\"relay-runtime\", \"dep:metrics-exporter-prometheus\"]"));
}

#[test]
fn embedded_runtime_dependencies_are_optional() {
    for dependency in [
        "web-transport",
        "url",
        "tokio-util",
        "futures",
        "serde",
        "serde_json",
        "tracing",
        "metrics",
    ] {
        let declaration = MANIFEST
            .lines()
            .find(|line| line.starts_with(&format!("{dependency} =")))
            .unwrap_or_else(|| panic!("missing dependency declaration for {dependency}"));
        assert!(
            declaration.contains("optional = true"),
            "runtime dependency is not optional: {dependency}"
        );
    }

    for excluded in [
        "moq-api",
        "axum",
        "hyper-serve",
        "tower-http",
        "fs2",
        "clap",
        "tracing-subscriber",
    ] {
        assert!(
            !MANIFEST
                .lines()
                .any(|line| line.starts_with(&format!("{excluded} ="))),
            "standalone process dependency remains packaged: {excluded}"
        );
    }
}

#[test]
fn admission_is_unconditional_and_relay_modules_use_the_narrowest_gate() {
    assert!(LIBRARY.contains("\nmod admission;\n"));
    assert!(LIBRARY.contains("\npub use admission::*;\n"));

    for declaration in [
        "mod capacity;",
        "mod consumer;",
        "mod coordinator;",
        "mod diagnostics;",
        "mod local;",
        "pub mod metrics;",
        "mod producer;",
        "mod relay;",
        "mod remote;",
        "mod session;",
    ] {
        assert!(
            LIBRARY.contains(&format!(
                "#[cfg(feature = \"relay-runtime\")]\n{declaration}"
            )),
            "relay core module is not feature-gated: {declaration}"
        );
    }

    for declaration in ["mod api;", "mod web;"] {
        assert!(
            LIBRARY.contains(&format!("#[cfg(feature = \"runtime\")]\n{declaration}")),
            "process-facing module is not runtime-gated: {declaration}"
        );
    }
}

#[test]
fn embedded_relay_feature_excludes_http_and_cli_dependencies() {
    let relay_runtime = MANIFEST
        .split("relay-runtime = [")
        .nth(1)
        .and_then(|tail| tail.split("\n]").next())
        .expect("relay-runtime feature");
    for dependency in [
        "moq-api",
        "axum",
        "hyper-serve",
        "tower-http",
        "fs2",
        "clap",
    ] {
        assert!(
            !relay_runtime.contains(dependency),
            "embedded relay feature contains process dependency: {dependency}"
        );
    }
}

#[test]
fn admission_source_does_not_import_relay_runtime_dependencies() {
    for forbidden in [
        "axum",
        "hyper_serve",
        "hyper_util",
        "moq_api",
        "web_transport",
        "tower_http",
        "tokio_util",
        "futures::",
        "metrics::",
        "tracing::",
        "serde::",
        "serde_json",
        "fs2::",
        "url::",
    ] {
        assert!(
            !ADMISSION.contains(forbidden),
            "admission contract references runtime dependency: {forbidden}"
        );
    }

    assert_eq!(ADMISSION.matches("clap::").count(), 1);
    assert!(ADMISSION.contains("#[cfg_attr(feature = \"runtime\", derive(clap::ValueEnum))]"));
}
