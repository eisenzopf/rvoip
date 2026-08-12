//! Pins `examples/pbx/amr_probe.sh` against captured PBX CLI output.
//!
//! The probe decides whether an AMR interop cell runs or records SKIP, so a
//! parser that drifts into always answering "supported" would silently turn
//! every release-runner AMR cell into a guaranteed failure — and one that
//! answers "unsupported" would let a lab regression hide as a skip (that half
//! is guarded at runtime by `PBX_REQUIRE_AMR=1` in the lab env files). The
//! `-without-amr` fixtures are what make the always-yes mutation fail here.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn probe_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/pbx/amr_probe.sh")
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/pbx/probe-fixtures")
        .join(name)
}

fn parse(provider: &str, fixture_name: &str) -> String {
    let input = std::fs::read(fixture(fixture_name))
        .unwrap_or_else(|error| panic!("fixture {fixture_name}: {error}"));
    let mut child = Command::new("sh")
        .arg(probe_script())
        .arg("parse")
        .arg(provider)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawns");
    {
        use std::io::Write;
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(&input)
            .expect("writes fixture");
    }
    let output = child.wait_with_output().expect("waits");
    assert!(output.status.success(), "parse exited nonzero");
    String::from_utf8(output.stdout).expect("utf8").trim().to_string()
}

fn detect_with_assumption(provider: &str, assume: &str) -> String {
    let transcript = std::env::temp_dir().join(format!(
        "amr-probe-{provider}-{assume}-{}.txt",
        std::process::id()
    ));
    let output = Command::new("sh")
        .arg(probe_script())
        .arg("detect")
        .arg(provider)
        .arg(&transcript)
        .env("PBX_ASSUME_AMR", assume)
        .output()
        .expect("runs");
    let _ = std::fs::remove_file(&transcript);
    assert!(output.status.success(), "detect exited nonzero");
    String::from_utf8(output.stdout).expect("utf8").trim().to_string()
}

#[test]
fn the_amr_capable_captures_parse_as_supported() {
    assert_eq!(
        parse("asterisk", "asterisk-core-show-codecs-with-amr.txt"),
        "amr=yes amrwb=yes"
    );
    assert_eq!(
        parse("freeswitch", "freeswitch-show-codec-with-amr.txt"),
        "amr=yes amrwb=yes"
    );
}

/// The mutation check: a parser rewritten to always say yes fails here, on
/// realistic output whose surrounding rows are genuine captures.
#[test]
fn the_amr_less_captures_parse_as_unsupported() {
    assert_eq!(
        parse("asterisk", "asterisk-core-show-codecs-without-amr.txt"),
        "amr=no amrwb=no"
    );
    assert_eq!(
        parse("freeswitch", "freeswitch-show-codec-without-amr.txt"),
        "amr=no amrwb=no"
    );
}

/// Asterisk's NAME column must be matched exactly: `amrwb` alone must not
/// satisfy `amr`, or a wideband-only image would run narrowband cells.
#[test]
fn asterisk_name_column_is_matched_exactly() {
    let with = std::fs::read_to_string(fixture("asterisk-core-show-codecs-with-amr.txt"))
        .expect("fixture");
    let wideband_only: String = with
        .lines()
        .filter(|line| {
            let mut columns = line.split_whitespace();
            let (_, kind, name) = (columns.next(), columns.next(), columns.next());
            !(kind == Some("audio") && name == Some("amr"))
        })
        .map(|line| format!("{line}\n"))
        .collect();
    let scratch = std::env::temp_dir().join(format!("amr-probe-wbonly-{}.txt", std::process::id()));
    std::fs::write(&scratch, wideband_only).expect("writes");
    let input = std::fs::read(&scratch).expect("reads back");
    let _ = std::fs::remove_file(&scratch);

    use std::io::Write;
    let mut child = Command::new("sh")
        .arg(probe_script())
        .arg("parse")
        .arg("asterisk")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawns");
    child.stdin.take().expect("stdin").write_all(&input).expect("writes");
    let output = child.wait_with_output().expect("waits");
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8").trim(),
        "amr=no amrwb=yes"
    );
}

/// `PBX_ASSUME_AMR` must decide without docker: the release gates pin 0 so
/// gate behaviour never depends on a container being reachable.
#[test]
fn the_assumption_short_circuits_the_probe() {
    assert_eq!(
        detect_with_assumption("asterisk", "0"),
        "status=assumed amr=no amrwb=no"
    );
    assert_eq!(
        detect_with_assumption("freeswitch", "1"),
        "status=assumed amr=yes amrwb=yes"
    );
}
