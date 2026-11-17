//! Records the Git commit the binary is built from (`git_sha` in
//! `ready.json`). `RVOIP_BENCH_GIT_SHA` overrides it (for builds from a
//! source archive); without either it is `unknown`.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|text| !text.is_empty())
}

fn main() {
    println!("cargo:rerun-if-env-changed=RVOIP_BENCH_GIT_SHA");
    // Rebuild the stamp when HEAD moves (a commit, a checkout).
    for path in [
        git(&["rev-parse", "--git-path", "HEAD"]),
        git(&["rev-parse", "--symbolic-full-name", "HEAD"])
            .and_then(|reference| git(&["rev-parse", "--git-path", &reference])),
    ]
    .into_iter()
    .flatten()
    {
        println!("cargo:rerun-if-changed={path}");
    }
    let sha = std::env::var("RVOIP_BENCH_GIT_SHA")
        .ok()
        .filter(|sha| !sha.is_empty())
        .or_else(|| git(&["rev-parse", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=RVOIP_BENCH_UAS_GIT_SHA={sha}");
}
