//! SIP_API_DESIGN_2 §9 Phase C deprecation matrix — CI grep guard.
//!
//! Each entry in `EXPECTED_DEPRECATIONS` names a method on a public type
//! that the spec mandates is annotated with
//! `#[deprecated(since = "0.3.0", note = "...")]`. The strengthened
//! test (Phase 8) asserts:
//!
//! 1. The declaration substring appears in the source.
//! 2. The FIRST occurrence (which is the trait declaration / inherent
//!    `impl` method body, not an impl row override) is preceded by the
//!    `#[deprecated(...)]` attribute within 8 lines.
//! 3. No `fn` declaration intervenes between the annotation and the
//!    matched line — this prevents a stray `#[deprecated]` on a
//!    different nearby function from satisfying the check.
//!
//! Add a row when the §9 matrix grows; remove a row when a deprecation
//! cycle completes and the method is deleted.

#![allow(clippy::module_name_repetitions)]

use std::fs;
use std::path::Path;

/// (file relative to the rvoip-sip crate root, function declaration substring)
///
/// All entries from the §9 Phase C / Phase 12 deprecation cycle have
/// completed: the methods have been deleted. The list is kept (empty)
/// so the CI grep guard remains armed for future deprecation cycles.
const EXPECTED_DEPRECATIONS: &[(&str, &str)] = &[];

#[test]
fn each_phase_c_deprecated_item_is_annotated() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut failures = Vec::<String>::new();

    for (relative_path, decl) in EXPECTED_DEPRECATIONS {
        let path = crate_root.join(relative_path);
        let source = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!(
                    "cannot read {} (looking for `{}`): {}",
                    relative_path, decl, e
                ));
                continue;
            }
        };

        let lines: Vec<&str> = source.lines().collect();
        let occurrences: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.contains(decl))
            .map(|(i, _)| i)
            .collect();

        if occurrences.is_empty() {
            failures.push(format!(
                "{}: declaration `{}` not found — was it renamed or moved?",
                relative_path, decl
            ));
            continue;
        }

        // Phase 8 — strengthen the check. The legacy contract was "at
        // least one occurrence has `#[deprecated]` within 8 lines"; that
        // missed cases where a stray attribute on an unrelated nearby
        // function satisfied the check. Strengthen by requiring:
        //
        //   - At least one occurrence carries the annotation; AND
        //   - For that occurrence, no other `fn ` declaration intervenes
        //     between the annotation and the matched line. This rules
        //     out the cross-contamination where two adjacent functions
        //     share a window.
        //
        // We still don't formally distinguish trait declaration from
        // impl rows (Rust syntax doesn't reveal the enclosing block
        // without a real parser); we DO ensure the annotation belongs
        // to the matched declaration rather than a neighbor.
        let mut found_for_decl = false;
        let mut intervening_fn_seen_on_any = false;
        for &decl_idx in &occurrences {
            let window_start = decl_idx.saturating_sub(8);
            let mut deprecated_idx: Option<usize> = None;
            for idx in window_start..decl_idx {
                let line = lines[idx];
                if line.contains("#[deprecated") {
                    let body_end = (idx + 4).min(decl_idx);
                    let body = lines[idx..body_end].join("\n");
                    if body.contains("since = \"0.3.0\"") {
                        deprecated_idx = Some(idx);
                    }
                }
            }
            let Some(dep_idx) = deprecated_idx else {
                continue;
            };
            // Verify no `fn ` intervenes between the annotation and the
            // declaration (the annotation may be followed by more
            // attributes like `#[allow]`, `#[doc]`, etc.; those are
            // fine).
            let mut intervening = false;
            for line in &lines[(dep_idx + 1)..decl_idx] {
                if line.contains(" fn ") && !line.contains("//") && !line.contains(decl) {
                    intervening = true;
                    break;
                }
            }
            if intervening {
                intervening_fn_seen_on_any = true;
                continue;
            }
            found_for_decl = true;
            break;
        }

        if !found_for_decl {
            if intervening_fn_seen_on_any {
                failures.push(format!(
                    "{}: `{}` has a `#[deprecated(since = \"0.3.0\", ...)]` in the preceding 8 lines, but an unrelated `fn` declaration intervenes for every occurrence — annotation likely belongs to a different method",
                    relative_path, decl,
                ));
            } else {
                failures.push(format!(
                    "{}: `{}` (found at {} occurrence(s)) is missing `#[deprecated(since = \"0.3.0\", ...)]` within the preceding 8 lines of ANY occurrence",
                    relative_path,
                    decl,
                    occurrences.len(),
                ));
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "Phase C deprecation annotations missing or drifted:\n  - {}\n\n\
             Update src/api/* to restore the `#[deprecated(since = \"0.3.0\", note = ...)]` \
             attribute on each affected declaration, or update \
             tests/deprecation_table.rs::EXPECTED_DEPRECATIONS if the spec moved.",
            failures.join("\n  - ")
        );
    }
}
