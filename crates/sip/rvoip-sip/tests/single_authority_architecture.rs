//! Static fences for the signaling single-authority cleanup.
//!
//! These checks intentionally describe the current exceptions instead of
//! pretending they are already gone.  A cleanup change must lower the exact
//! count for its owning work item in the same commit.  Raising a count or
//! adding a new source file to an inventory is an architecture regression.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

#[derive(Clone, Copy, Debug)]
struct Allowance {
    path: &'static str,
    count: usize,
    owner: &'static str,
}

fn rust_sources(root: &Path, output: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(root)
        .unwrap_or_else(|error| panic!("read {}: {error}", root.display()))
        .map(|entry| entry.expect("read source directory entry").path())
        .collect::<Vec<_>>();
    entries.sort();

    for path in entries {
        if path.is_dir() {
            rust_sources(&path, output);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            output.push(path);
        }
    }
}

fn source_roots(relative_roots: &[&str]) -> Vec<(String, String)> {
    let manifest_dir = Path::new(MANIFEST_DIR);
    let mut sources = Vec::new();
    for relative_root in relative_roots {
        let root = manifest_dir.join(relative_root);
        let mut paths = Vec::new();
        rust_sources(&root, &mut paths);
        for path in paths {
            let relative = path
                .strip_prefix(manifest_dir)
                .expect("source root must be relative to the crate")
                .to_string_lossy()
                .replace('\\', "/");
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            sources.push((relative, strip_cfg_test_items(&source)));
        }
    }
    sources
}

/// Remove `#[cfg(test)]` items while retaining production items that follow
/// them. Several large modules interleave test-only helpers with production
/// implementations, so truncating at the first test attribute is incorrect.
fn strip_cfg_test_items(source: &str) -> String {
    const MARKER: &str = "#[cfg(test)]";
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;

    while let Some(relative) = source[cursor..].find(MARKER) {
        let marker = cursor + relative;
        output.push_str(&source[cursor..marker]);
        let item_start = marker + MARKER.len();
        let Some(item_end) = cfg_test_item_end(source, item_start) else {
            // Keep malformed/unrecognised text visible to the inventories.
            output.push_str(MARKER);
            cursor = item_start;
            continue;
        };
        output.extend(std::iter::repeat_n(
            '\n',
            source[marker..item_end].matches('\n').count(),
        ));
        cursor = item_end;
    }
    output.push_str(&source[cursor..]);
    output
}

fn cfg_test_item_end(source: &str, mut cursor: usize) -> Option<usize> {
    cursor = skip_space_and_attributes(source, cursor);
    let tail = &source[cursor..];
    let item_tail = tail
        .strip_prefix("pub(crate) ")
        .or_else(|| tail.strip_prefix("pub(super) "))
        .or_else(|| tail.strip_prefix("pub "))
        .unwrap_or(tail);
    let is_item = [
        "async fn ",
        "fn ",
        "mod ",
        "impl ",
        "trait ",
        "struct ",
        "enum ",
        "type ",
        "const ",
        "static ",
        "use ",
    ]
    .iter()
    .any(|prefix| item_tail.starts_with(prefix));
    if !is_item {
        return None;
    }

    let semicolon = tail.find(';').map(|offset| cursor + offset + 1);
    let opening = tail.find('{').map(|offset| cursor + offset);
    match (opening, semicolon) {
        (Some(opening), Some(semicolon)) if semicolon < opening => Some(semicolon),
        (Some(opening), _) => matching_delimiter(source, opening, b'{', b'}').map(|end| end + 1),
        (None, Some(semicolon)) => Some(semicolon),
        (None, None) => None,
    }
}

fn skip_space_and_attributes(source: &str, mut cursor: usize) -> usize {
    loop {
        cursor += source[cursor..]
            .find(|character: char| !character.is_whitespace())
            .unwrap_or(source.len() - cursor);
        if source[cursor..].starts_with("//") {
            cursor = source[cursor..]
                .find('\n')
                .map_or(source.len(), |offset| cursor + offset + 1);
            continue;
        }
        if source[cursor..].starts_with("/*") {
            let Some(end) = source[cursor + 2..].find("*/") else {
                return cursor;
            };
            cursor += end + 4;
            continue;
        }
        if source[cursor..].starts_with("#[") {
            let Some(end) = source[cursor..].find(']') else {
                return cursor;
            };
            cursor += end + 1;
            continue;
        }
        return cursor;
    }
}

/// Find a matching delimiter while ignoring comments and Rust string/character
/// literals. This is deliberately a small lexer, not a Rust parser; the guard
/// matches stable symbol names and leaves semantic enforcement to runtime
/// tests.
fn matching_delimiter(source: &str, opening: usize, open: u8, close: u8) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut cursor = opening;
    let mut depth = 0usize;
    let mut block_comment_depth = 0usize;
    let mut string = false;
    let mut character = false;
    let mut escaped = false;

    while cursor < bytes.len() {
        let current = bytes[cursor];
        let next = bytes.get(cursor + 1).copied();

        if block_comment_depth > 0 {
            if current == b'/' && next == Some(b'*') {
                block_comment_depth += 1;
                cursor += 2;
                continue;
            }
            if current == b'*' && next == Some(b'/') {
                block_comment_depth -= 1;
                cursor += 2;
                continue;
            }
            cursor += 1;
            continue;
        }
        if string || character {
            if escaped {
                escaped = false;
            } else if current == b'\\' {
                escaped = true;
            } else if (string && current == b'"') || (character && current == b'\'') {
                string = false;
                character = false;
            }
            cursor += 1;
            continue;
        }
        if current == b'/' && next == Some(b'/') {
            cursor = source[cursor..]
                .find('\n')
                .map_or(bytes.len(), |offset| cursor + offset + 1);
            continue;
        }
        if current == b'/' && next == Some(b'*') {
            block_comment_depth = 1;
            cursor += 2;
            continue;
        }
        if current == b'"' {
            string = true;
            cursor += 1;
            continue;
        }
        // Treat only a quote followed by one character (or an escaped one) and
        // a closing quote as a character literal; ordinary lifetimes stay code.
        if current == b'\''
            && (bytes.get(cursor + 2) == Some(&b'\'')
                || (next == Some(b'\\') && bytes.get(cursor + 3) == Some(&b'\'')))
        {
            character = true;
            cursor += 1;
            continue;
        }
        if current == open {
            depth += 1;
        } else if current == close {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(cursor);
            }
        }
        cursor += 1;
    }
    None
}

fn compact(source: &str) -> String {
    code_only(source)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn function_source<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("missing architecture-sensitive function `{signature}`"));
    let opening = source[start..]
        .find('{')
        .map(|offset| start + offset)
        .unwrap_or_else(|| {
            panic!("missing body for architecture-sensitive function `{signature}`")
        });
    let closing = matching_delimiter(source, opening, b'{', b'}').unwrap_or_else(|| {
        panic!("unclosed body for architecture-sensitive function `{signature}`")
    });
    &source[start..=closing]
}

/// Mask comments and literals while preserving byte positions/newlines. The
/// inventories must count executable symbol use, not examples in rustdoc or a
/// diagnostic string that happens to name an architecture-sensitive method.
fn code_only(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = bytes.to_vec();
    let mut cursor = 0usize;
    let mut block_depth = 0usize;
    let mut line_comment = false;
    let mut quoted: Option<u8> = None;
    let mut escaped = false;

    while cursor < bytes.len() {
        let current = bytes[cursor];
        let next = bytes.get(cursor + 1).copied();

        if line_comment {
            if current == b'\n' {
                line_comment = false;
            } else {
                output[cursor] = b' ';
            }
            cursor += 1;
            continue;
        }
        if block_depth > 0 {
            if current == b'/' && next == Some(b'*') {
                output[cursor] = b' ';
                output[cursor + 1] = b' ';
                block_depth += 1;
                cursor += 2;
                continue;
            }
            if current == b'*' && next == Some(b'/') {
                output[cursor] = b' ';
                output[cursor + 1] = b' ';
                block_depth -= 1;
                cursor += 2;
                continue;
            }
            if current != b'\n' {
                output[cursor] = b' ';
            }
            cursor += 1;
            continue;
        }
        if let Some(quote) = quoted {
            if current != b'\n' {
                output[cursor] = b' ';
            }
            if escaped {
                escaped = false;
            } else if current == b'\\' {
                escaped = true;
            } else if current == quote {
                quoted = None;
            }
            cursor += 1;
            continue;
        }
        if current == b'/' && next == Some(b'/') {
            output[cursor] = b' ';
            output[cursor + 1] = b' ';
            line_comment = true;
            cursor += 2;
            continue;
        }
        if current == b'/' && next == Some(b'*') {
            output[cursor] = b' ';
            output[cursor + 1] = b' ';
            block_depth = 1;
            cursor += 2;
            continue;
        }
        if current == b'"' {
            output[cursor] = b' ';
            quoted = Some(b'"');
            cursor += 1;
            continue;
        }
        if current == b'\''
            && (bytes.get(cursor + 2) == Some(&b'\'')
                || (next == Some(b'\\') && bytes.get(cursor + 3) == Some(&b'\'')))
        {
            output[cursor] = b' ';
            quoted = Some(b'\'');
            cursor += 1;
            continue;
        }
        cursor += 1;
    }

    String::from_utf8(output).expect("masking source preserves UTF-8")
}

fn exact_inventory(
    name: &str,
    sources: &[(String, String)],
    needles: &[&str],
    allowances: &[Allowance],
) {
    let mut observed = BTreeMap::<String, usize>::new();
    for (path, source) in sources {
        let compact = compact(source);
        let count = needles
            .iter()
            .map(|needle| compact.matches(needle).count())
            .sum();
        if count != 0 {
            observed.insert(path.clone(), count);
        }
    }

    let expected = allowances
        .iter()
        .map(|allowance| {
            assert!(
                allowance.owner.contains('-'),
                "{name}: {} lacks a cleanup work-item owner",
                allowance.path
            );
            (allowance.path.to_string(), allowance.count)
        })
        .collect::<BTreeMap<_, _>>();

    assert_eq!(
        observed, expected,
        "{name} changed. New exceptions are forbidden. When an owned cleanup removes exceptions, lower or delete the matching allowance in the same change; never raise it. Owners: {allowances:#?}"
    );
}

#[test]
fn direct_session_mutation_inventory_is_exact() {
    let sources = source_roots(&["src"]);
    exact_inventory(
        "SessionStore::update_session_with direct writers",
        &sources,
        &[".update_session_with("],
        &[],
    );
}

#[test]
fn exact_session_mutation_inventory_is_exact() {
    let sources = source_roots(&["src"]);
    exact_inventory(
        "SessionStore::update_session_exact_with direct writers",
        &sources,
        &[".update_session_exact_with("],
        &[
            Allowance {
                path: "src/adapters/dialog_adapter.rs",
                count: 2,
                owner: "EX-201/PR-402",
            },
            Allowance {
                path: "src/session_store/store.rs",
                count: 3,
                owner: "IN-102/EX-204",
            },
            Allowance {
                path: "src/state_machine/executor.rs",
                count: 3,
                owner: "IN-101/IN-102/EX-202",
            },
        ],
    );
}

#[test]
fn full_snapshot_replacement_inventory_is_exact() {
    let sources = source_roots(&["src"]);
    exact_inventory(
        "full SessionState snapshot replacement",
        &sources,
        &[
            ".update_session(",
            ".update_session_and_snapshot(",
            ".update_state_machine_session_and_snapshot(",
            ".replace_session_exact(",
            ".replace_session_exact_inner(",
        ],
        &[
            Allowance {
                path: "src/session_store/store.rs",
                count: 4,
                owner: "EX-204",
            },
            Allowance {
                path: "src/state_machine/executor.rs",
                count: 1,
                owner: "EX-201/EX-204",
            },
        ],
    );
}

#[test]
fn generic_cross_crate_publish_is_forbidden_on_signaling_paths() {
    let mut sources = source_roots(&["src", "../sip-dialog/src"]);
    // Transport trace publication is explicitly observational and is not a
    // signaling coordination exception.
    sources.retain(|(path, _)| path != "../sip-dialog/src/transaction/transport/trace.rs");
    exact_inventory(
        "general GlobalEventCoordinator::publish calls",
        &sources,
        &["coordinator.publish("],
        &[],
    );
}

#[test]
fn debug_string_routing_inventory_is_exact() {
    let sources = source_roots(&["src"]);
    exact_inventory(
        "debug-string event routing parameters",
        &sources,
        &["event_str:&str"],
        &[],
    );
    exact_inventory(
        "debug-string extraction helpers",
        &sources,
        &[
            "fnextract_session_id(",
            "fnextract_field(",
            "fnextract_debug_string_field(",
            "fnextract_optional_field(",
        ],
        &[],
    );
}

fn delayed_raw_id_spawn_count(source: &str) -> usize {
    let source = code_only(source);
    let mut count = 0;
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find("tokio::spawn(") {
        let opening = cursor + relative + "tokio::spawn".len();
        let Some(closing) = matching_delimiter(&source, opening, b'(', b')') else {
            break;
        };
        let body = compact(&source[opening..=closing]);
        let delayed =
            body.contains("tokio::time::sleep(") || body.contains("tokio::time::sleep_until(");
        let raw_identity = body.contains("session_id")
            || body.contains("session_for_")
            || body.contains("call_id");
        if delayed && raw_identity {
            count += 1;
        }
        cursor = closing + 1;
    }
    count
}

#[test]
fn raw_id_delayed_task_inventory_is_exact() {
    let sources = source_roots(&["src"]);
    let observed = sources
        .iter()
        .filter_map(|(path, source)| {
            let count = delayed_raw_id_spawn_count(source);
            (count != 0).then(|| (path.clone(), count))
        })
        .collect::<BTreeMap<_, _>>();
    let expected = BTreeMap::<String, usize>::new();

    assert_eq!(
        observed, expected,
        "raw-ID delayed signaling task inventory changed; migrate tasks to exact retained lifecycle ownership under IN-105/EX-205 and lower this allowance"
    );
}

#[test]
fn hangup_wait_timeout_does_not_spawn_a_detached_raw_id_observer() {
    let path = Path::new(MANIFEST_DIR).join("src/api/handle.rs");
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let source = strip_cfg_test_items(&source);

    assert!(
        !source.contains("spawn_late_answer_teardown_observer"),
        "the deleted raw-ID late-answer observer was restored"
    );

    let hangup_and_wait = compact(function_source(&source, "pub async fn hangup_and_wait"));
    assert!(
        hangup_and_wait.contains("wait_for_lifecycle("),
        "hangup_and_wait must return the bounded lifecycle wait directly"
    );
    assert!(
        !hangup_and_wait.contains("letresult=")
            && !hangup_and_wait.contains("ifmatches!(result,")
            && !hangup_and_wait.contains("tokio::spawn(")
            && !hangup_and_wait.contains("tokio::time::timeout("),
        "hangup_and_wait must not create a detached raw-ID timeout observer"
    );
}

#[test]
fn session_handle_control_and_reads_delegate_only_with_captured_authority() {
    let path = Path::new(MANIFEST_DIR).join("src/api/handle.rs");
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let source = strip_cfg_test_items(&source);

    for (signature, exact_call, forbidden_raw_call) in [
        (
            "pub async fn hangup(&self)",
            ".hangup_exact(&handle)",
            ".hangup(&self.call_id)",
        ),
        (
            "pub async fn hold(&self)",
            ".hold_exact(&handle)",
            ".hold(&self.call_id)",
        ),
        (
            "pub async fn resume(&self)",
            ".resume_exact(&handle)",
            ".resume(&self.call_id)",
        ),
        (
            "pub async fn accept_refer(&self)",
            ".accept_refer_exact(&handle)",
            ".accept_refer(&self.call_id)",
        ),
        (
            "pub async fn dialog_identity(&self)",
            ".dialog_identity_exact(&handle)",
            ".dialog_identity(&self.call_id)",
        ),
        (
            "pub async fn send_dtmf(&self",
            ".send_dtmf_exact(&handle,digit)",
            ".send_dtmf(&self.call_id,digit)",
        ),
        (
            "pub async fn state(&self)",
            ".get_state_exact(&handle)",
            ".get_state(&self.call_id)",
        ),
        (
            "pub async fn session_info(&self)",
            ".get_session_info_exact(&handle)",
            ".get_session_info(&self.call_id)",
        ),
        (
            "pub async fn lifecycle(&self)",
            ".lifecycle_snapshot_exact(&handle)",
            ".lifecycle_snapshot(&self.call_id)",
        ),
    ] {
        let body = compact(function_source(&source, signature));
        assert!(
            body.contains(&compact(exact_call)),
            "{signature} must delegate with the SessionHandle's captured authority"
        );
        assert!(
            !body.contains(&compact(forbidden_raw_call)),
            "{signature} must not re-resolve its raw CallId"
        );
    }

    for builder in [
        "bye", "cancel", "refer", "notify", "info", "update", "reinvite",
    ] {
        let signature = format!("pub fn {builder}(&self");
        let body = compact(function_source(&source, &signature));
        assert!(
            body.contains("::new_captured("),
            "SessionHandle::{builder} must capture builder authority"
        );
        assert!(
            body.contains("self.lifecycle_handle.clone()"),
            "SessionHandle::{builder} must retain the original registry handle"
        );

        let builder_path = Path::new(MANIFEST_DIR)
            .join("src/api/send")
            .join(format!("{builder}.rs"));
        let builder_source = fs::read_to_string(&builder_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", builder_path.display()));
        let send = compact(function_source(&builder_source, "pub async fn send"));
        assert!(
            send.contains("self.authority.exact_handle(&self.session_id)"),
            "{builder} builder must consult captured authority before dispatch"
        );
        assert!(
            send.contains("_exact(&handle,"),
            "{builder} builder must have an exact dispatch branch"
        );
    }
}

#[test]
fn outbound_spawn_boundaries_capture_exact_handles_before_spawning() {
    let path = Path::new(MANIFEST_DIR).join("src/api/unified.rs");
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let source = strip_cfg_test_items(&source);

    let raw_dispatch = compact(function_source(&source, "pub async fn dispatch_outbound("));
    assert!(raw_dispatch.contains(".lifecycle_handle(session_id)"));
    assert!(raw_dispatch.contains(".dispatch_outbound_exact(&handle,event)"));
    assert!(!raw_dispatch.contains("tokio::spawn("));

    let exact_dispatch = compact(function_source(
        &source,
        "pub(crate) async fn dispatch_outbound_exact(",
    ));
    assert!(exact_dispatch.contains("lettask_handle=handle.clone();"));
    assert!(exact_dispatch.contains(".process_event_exact(&task_handle,event)"));
    assert!(!exact_dispatch.contains("task_session_id"));

    let staged = compact(function_source(
        &source,
        "async fn dispatch_outbound_with_options_and_input_exact(",
    ));
    assert!(staged.contains("lettask_handle=handle.clone();"));
    assert!(staged.contains(".process_event_with_staged_options_exact("));
    assert!(!staged.contains("task_session_id"));
}

#[test]
fn retained_bye_confirmation_is_keyed_by_exact_session_lifetime() {
    let path = Path::new(MANIFEST_DIR).join("src/adapters/dialog_adapter.rs");
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let production = compact(&strip_cfg_test_items(&source));

    for field in [
        "outgoing_bye_tx:Arc<DashMap<SessionRegistryHandle,OutboundByeTransaction>>",
        "outgoing_bye_generation_watch:Arc<DashMap<SessionRegistryHandle,tokio::sync::watch::Sender<u64>>>",
        "outgoing_bye_wait_intents:Arc<DashMap<SessionRegistryHandle,OutgoingByeWaitIntentState>>",
    ] {
        assert!(
            production.contains(field),
            "retained BYE state must be keyed by SessionRegistryHandle: {field}"
        );
    }

    for deleted_raw_helper in [
        "fnbegin_outgoing_bye_wait(",
        "fnoutgoing_bye_generation(",
        "fnhas_outgoing_bye_after(",
        "fnoutgoing_bye_transaction_matches(",
        "fnoutgoing_bye_request_uri(",
        "fnwait_for_outgoing_bye_final_response(",
    ] {
        assert!(
            !production.contains(deleted_raw_helper),
            "zero-caller raw BYE helper was restored: {deleted_raw_helper}"
        );
    }
}

#[test]
fn lifecycle_timeout_fire_paths_use_exact_dispatch() {
    let path = Path::new(MANIFEST_DIR).join("src/api/unified.rs");
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let source = strip_cfg_test_items(&source);

    let setup_fire = compact(function_source(
        &source,
        "async fn fire_setup_teardown_deadline",
    ));
    assert!(
        setup_fire.contains(
            ".process_event_exact(&deadline.handle,EventType::DialogTimeout)"
        ),
        "setup/teardown deadline fire must dispatch DialogTimeout through its captured exact handle"
    );
    assert!(
        !setup_fire.contains(".process_event(&session_id,EventType::DialogTimeout)"),
        "setup/teardown deadline fire regained a raw-SessionId dispatch"
    );

    let media_schedule = compact(function_source(
        &source,
        "pub(crate) async fn schedule_active_call_media_timeout_if_current",
    ));
    assert!(
        media_schedule.contains(".hangup_exact_with_reason(&handle,reason)"),
        "active-media watchdog must join the one exact confirmed-hangup authority"
    );
    assert!(
        !media_schedule.contains("release_exact_local_resources("),
        "active-media watchdog regained a competing terminal-release path"
    );

    let media_hangup = compact(function_source(
        &source,
        "async fn hangup_exact_with_reason",
    ));
    assert!(
        media_hangup.contains("coordinator.hangup_serialized_exact(&retained_handle,reason)"),
        "automatic and public hangup must share the retained exact serialized operation"
    );
    assert!(
        !media_schedule.contains(".process_event("),
        "active-media watchdog must never resolve a raw SessionId"
    );
}

#[test]
fn automatic_180_suppression_has_one_configuration_authority() {
    let executor_path = Path::new(MANIFEST_DIR).join("src/state_machine/executor.rs");
    let executor = fs::read_to_string(&executor_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", executor_path.display()));
    let executor = strip_cfg_test_items(&executor);
    let admission = compact(function_source(&executor, "fn should_skip_action"));
    assert!(
        admission
            .contains("matches!(action,Action::SendSIPResponse(180,_))&&!self.auto_180_ringing"),
        "the executor must remain the sole Config::auto_180_ringing suppression boundary"
    );

    let actions_path = Path::new(MANIFEST_DIR).join("src/state_machine/actions.rs");
    let actions = fs::read_to_string(&actions_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", actions_path.display()));
    let actions = strip_cfg_test_items(&actions);
    let execution = compact(function_source(
        &actions,
        "pub(crate) async fn execute_action",
    ));
    assert!(
        !execution.contains("auto_180_ringing"),
        "SendSIPResponse must not reintroduce a second automatic-180 policy branch"
    );
}

#[test]
fn non_transaction_response_builders_enter_the_state_machine_lane() {
    let manifest = Path::new(MANIFEST_DIR);
    for relative in [
        "src/api/respond/accept.rs",
        "src/api/respond/provisional.rs",
        "src/api/respond/redirect.rs",
    ] {
        let path = manifest.join(relative);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let send = compact(function_source(&source, "pub async fn send"));
        for wire in [
            ".send_response(",
            ".send_response_with_options(",
            ".send_redirect_response(",
            ".send_redirect_response_with_options(",
        ] {
            assert!(
                !send.contains(wire),
                "{relative} bypassed the response state-machine lane via {wire}"
            );
        }
    }

    let generic_path = manifest.join("src/api/respond/generic.rs");
    let generic = fs::read_to_string(&generic_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", generic_path.display()));
    let send = compact(function_source(&generic, "pub async fn send"));
    assert!(
        send.contains(".send_response_with_options_for_transaction_classified("),
        "the exact non-INVITE transaction response must remain transaction-owned"
    );
    let non_transaction_start = send
        .find("if(300..=399).contains(&self.status)")
        .expect("GenericResponseBuilder must retain its non-transaction branch");
    let non_transaction = &send[non_transaction_start..];
    for wire in [
        ".send_response(",
        ".send_response_with_options(",
        ".send_redirect_response(",
        ".send_redirect_response_with_options(",
    ] {
        assert!(
            !non_transaction.contains(wire),
            "GenericResponseBuilder non-transaction branch bypassed YAML via {wire}"
        );
    }
    assert!(non_transaction.contains(".reject_call_with_extras"));
    assert!(non_transaction.contains(".redirect_call_with_extras"));
}

#[test]
fn response_actions_take_each_event_envelope_at_the_wire_boundary() {
    let path = Path::new(MANIFEST_DIR).join("src/state_machine/actions.rs");
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let execution = compact(function_source(
        &strip_cfg_test_items(&source),
        "pub(crate) async fn execute_action",
    ));

    assert_eq!(
        execution.matches(".reject_response_extras.take()").count(),
        3,
        "reject, redirect, and generic SIP response actions must each consume the shared header envelope"
    );
    assert_eq!(
        execution
            .matches(".pending_response_status_override.take()")
            .count(),
        1,
        "only SendSIPResponse may consume a provisional status override"
    );
}

#[test]
fn media_derived_state_stays_in_the_exact_lane() {
    let media_path = Path::new(MANIFEST_DIR).join("src/adapters/media_adapter.rs");
    let media = fs::read_to_string(&media_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", media_path.display()));
    let media = strip_cfg_test_items(&media);
    for signature in [
        "pub async fn negotiate_sdp_as_uac",
        "pub async fn negotiate_sdp_as_uas",
        "pub async fn generate_local_sdp_offer",
    ] {
        let facade = compact(function_source(&media, signature));
        assert!(
            facade.contains("lock_and_load_exact_media_session(session_id).await"),
            "public MediaAdapter facade `{signature}` must acquire and revalidate the exact lane"
        );
    }

    let actions_path = Path::new(MANIFEST_DIR).join("src/state_machine/actions.rs");
    let actions = fs::read_to_string(&actions_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", actions_path.display()));
    let actions = compact(&strip_cfg_test_items(&actions));
    for lane_owned in [
        ".generate_local_sdp_lane_owned(session)",
        ".negotiate_sdp_as_uac_lane_owned(session,",
        ".negotiate_sdp_as_uas_lane_owned(session,",
        ".create_hold_sdp_for_session_lane_owned(session)",
        ".create_active_sdp_for_session_lane_owned(session)",
    ] {
        assert!(
            actions.contains(lane_owned),
            "state-machine media action bypassed its lane-owned path: {lane_owned}"
        );
    }
    for locking_facade in [
        ".generate_local_sdp(&session.session_id)",
        ".negotiate_sdp_as_uac(&session.session_id,",
        ".negotiate_sdp_as_uas(&session.session_id,",
        ".create_hold_sdp_for_session(&session.session_id)",
        ".create_active_sdp_for_session(&session.session_id)",
    ] {
        assert!(
            !actions.contains(locking_facade),
            "executor-held lane called a locking MediaAdapter facade: {locking_facade}"
        );
    }

    let executor_path = Path::new(MANIFEST_DIR).join("src/state_machine/executor.rs");
    let executor = fs::read_to_string(&executor_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", executor_path.display()));
    let executor = strip_cfg_test_items(&executor);
    assert!(!executor.contains("current.sdp_origin_version"));
    assert!(!executor.contains("session.media_security.is_none()"));
}

#[test]
fn lane_owned_dialog_dispatch_never_relocks_the_exact_session() {
    let actions_path = Path::new(MANIFEST_DIR).join("src/state_machine/actions.rs");
    let actions = fs::read_to_string(&actions_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", actions_path.display()));
    let execution = compact(function_source(
        &strip_cfg_test_items(&actions),
        "pub(crate) async fn execute_action",
    ));
    for locking_facade in [
        ".send_reinvite_session(",
        ".send_refer_session(",
        ".send_reinvite_with_options(",
        ".send_refer_with_options(",
        ".send_reinvite(",
        ".send_refer(",
    ] {
        assert!(
            !execution.contains(locking_facade),
            "execute_action called a locking DialogAdapter facade: {locking_facade}"
        );
    }
    assert!(execution.contains(".send_reinvite_with_options_lane_owned(session,"));
    assert!(execution.contains(".send_refer_with_options_lane_owned(session,"));

    let executor_path = Path::new(MANIFEST_DIR).join("src/state_machine/executor.rs");
    let executor = fs::read_to_string(&executor_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", executor_path.display()));
    let deferred = compact(function_source(
        &strip_cfg_test_items(&executor),
        "fn schedule_deferred_action_effects",
    ));
    for locking_facade in [
        ".send_reinvite_session(",
        ".send_reinvite_with_options(",
        ".send_reinvite(",
    ] {
        assert!(
            !deferred.contains(locking_facade),
            "deferred re-INVITE retry called a locking DialogAdapter facade: {locking_facade}"
        );
    }
    let commit = deferred
        .find("letcommitted=commit_lane_state(&dispatch_store,session)")
        .expect("deferred re-INVITE retry must commit its SDP state before dispatch");
    let wire = deferred
        .find(".send_reinvite_with_options_lane_owned(committed.state(),")
        .expect("deferred re-INVITE retry must use its already-owned exact lane");
    assert!(
        commit < wire,
        "deferred re-INVITE reached wire before commit"
    );
}

#[test]
fn adapter_routing_map_write_inventory_is_exact() {
    let sources = source_roots(&["src/adapters"]);
    for (path, source) in &sources {
        assert!(
            !source.contains("callid_to_session"),
            "{path} restored the deleted adapter Call-ID projection"
        );
    }
    let mut needles = Vec::new();
    for map in ["session_to_dialog", "dialog_to_session"] {
        for operation in ["insert", "entry", "remove", "remove_if", "clear", "retain"] {
            needles.push(format!("{map}.{operation}("));
        }
    }
    let needle_refs = needles.iter().map(String::as_str).collect::<Vec<_>>();
    exact_inventory(
        "adapter compatibility routing-map writes",
        &sources,
        &needle_refs,
        &[],
    );
}

#[test]
fn media_adapter_compatibility_routing_maps_stay_deleted() {
    let adapter_path = Path::new(MANIFEST_DIR).join("src/adapters/media_adapter.rs");
    let adapter = fs::read_to_string(&adapter_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", adapter_path.display()));
    let production_adapter = compact(&strip_cfg_test_items(&adapter));
    assert!(
        !production_adapter.contains("session_to_dialog"),
        "MediaAdapter reintroduced its forward compatibility map"
    );
    assert!(
        !production_adapter.contains("dialog_to_session"),
        "MediaAdapter reintroduced its reverse compatibility map"
    );

    let resolver = compact(function_source(&adapter, "fn media_for_handle_exact"));
    let registry = resolver
        .find("get_media_handle_exact")
        .expect("media resolution reads the canonical exact registry association");
    let binding = resolver
        .find("media_resources.get")
        .expect("media resolution checks the exact managed-resource binding");
    let resource = resolver
        .find("binding.resource.upgrade")
        .expect("media resolution retains the exact managed resource");
    assert!(registry < binding && binding < resource);

    let diagnostics = function_source(&adapter, "pub(crate) fn perf_diagnostic_counts");
    assert!(diagnostics.contains("\"session_to_dialog\": 0"));
    assert!(diagnostics.contains("\"registry_media_bindings\""));
    assert!(diagnostics.contains("\"media_resources\""));
}

#[test]
fn dialog_adapter_compatibility_routing_maps_stay_deleted() {
    let adapter_path = Path::new(MANIFEST_DIR).join("src/adapters/dialog_adapter.rs");
    let adapter = fs::read_to_string(&adapter_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", adapter_path.display()));
    let production_adapter = compact(&strip_cfg_test_items(&adapter));
    assert!(
        !production_adapter.contains("session_to_dialog"),
        "DialogAdapter reintroduced its upper forward compatibility map"
    );
    assert!(
        !production_adapter.contains("dialog_to_session"),
        "DialogAdapter reintroduced its upper reverse compatibility map"
    );

    let resolver = compact(function_source(
        &adapter,
        "async fn lock_and_load_exact_legacy_dialog_session",
    ));
    let registry_owner = resolver
        .find("get_handle_by_dialog_exact")
        .expect("legacy Dialog-ID facade resolves the canonical registry owner");
    let lane = resolver
        .find("state_machine_lane_exact")
        .expect("legacy Dialog-ID facade captures the exact lane");
    let wait = resolver
        .find("lock_owned().await")
        .expect("legacy Dialog-ID facade waits for the exact lane");
    let snapshot = resolver
        .find("get_session_snapshot_exact")
        .expect("legacy Dialog-ID facade revalidates the exact session");
    let dialog = resolver
        .find("get_dialog_handle_exact")
        .expect("legacy Dialog-ID facade revalidates canonical dialog ownership");
    assert!(registry_owner < lane && lane < wait && wait < snapshot && snapshot < dialog);
    assert!(resolver.contains("->Result<"));
    assert!(resolver.contains("ok_or_else"));

    let bye = compact(function_source(&adapter, "pub async fn send_bye"));
    assert!(bye.contains("get_handle_by_dialog_exact"));
    assert!(bye.contains("dispatch_state_machine_options_exact"));
    assert!(!bye.contains("lock_and_load_exact_legacy_dialog_session"));
    assert!(!bye.contains("dialog_to_session"));

    for facade in ["send_reinvite", "send_refer"] {
        let body = compact(function_source(&adapter, &format!("pub async fn {facade}")));
        assert!(body.contains("lock_and_load_exact_legacy_dialog_session"));
        assert!(body.contains(".await?"));
        assert!(!body.contains("letSome("));
        assert!(!body.contains("dialog_to_session"));
    }

    let response = compact(function_source(
        &adapter,
        "pub async fn send_response_by_dialog",
    ));
    assert!(response.contains("Err(SessionError::InvalidTransition"));
    assert!(!response.contains("lock_and_load_exact_legacy_dialog_session"));
    assert!(!response.contains("send_response_session"));

    let remote_uri = compact(function_source(&adapter, "pub async fn get_remote_uri"));
    assert!(remote_uri.contains("get_dialog_info"));
    assert!(remote_uri.contains("dialog.remote_uri.to_string()"));

    let ack = compact(function_source(&adapter, "pub async fn send_ack"));
    assert!(ack.contains("Err(SessionError::InvalidTransition"));

    let handler_path = Path::new(MANIFEST_DIR).join("src/adapters/session_event_handler.rs");
    let handler = fs::read_to_string(&handler_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", handler_path.display()));
    let production_handler = compact(&strip_cfg_test_items(&handler));
    assert!(
        !production_handler.contains("dialog_adapter.session_to_dialog"),
        "inbound routing reintroduced the deleted adapter forward map"
    );
    assert!(
        !production_handler.contains("dialog_adapter.dialog_to_session"),
        "inbound routing reintroduced the deleted adapter reverse map"
    );
    let incoming = compact(function_source(
        &handler,
        "async fn handle_incoming_call_parts",
    ));
    assert!(incoming.contains("registry_has_exact_dialog"));
}

#[test]
fn inbound_ack_has_one_exact_causal_owner() {
    let dialog_manifest = Path::new(MANIFEST_DIR).join("../sip-dialog/src");

    let facade_path = dialog_manifest.join("protocol/invite_handler.rs");
    let facade_source = fs::read_to_string(&facade_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", facade_path.display()));
    let facade = compact(function_source(
        &facade_source,
        "pub async fn process_ack_in_dialog",
    ));
    let exact_match = facade
        .find("find_server_invite_for_ack(&request)")
        .expect("ACK facade resolves the exact server INVITE transaction");
    let canonical = facade
        .find("handle_ack_received_event(&dialog_id,&transaction_id,request)")
        .expect("ACK facade delegates to the canonical ACK handler");
    assert!(exact_match < canonical);
    assert!(!facade.contains("TransactionKey::new"));
    assert!(!facade.contains("update_remote_sequence"));
    assert!(!facade.contains("notify_session_layer"));

    let integration_path = dialog_manifest.join("manager/transaction_integration.rs");
    let integration_source = fs::read_to_string(&integration_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", integration_path.display()));
    let owner = compact(function_source(
        &integration_source,
        "pub(crate) async fn handle_ack_received_event",
    ));
    let exact_dialog = owner
        .find("find_dialog_for_transaction(transaction_id)")
        .expect("canonical ACK owner validates the transaction-to-dialog binding");
    let causal_delivery = owner
        .find("notify_session_layer(SessionCoordinationEvent::AckReceived")
        .expect("canonical ACK owner uses acknowledged causal delivery");
    assert!(exact_dialog < causal_delivery);
    assert!(owner.contains("transaction_id.is_server()"));
    assert!(owner.contains("transaction_id.method()!=&rvoip_sip_core::Method::Invite"));
    assert!(!owner.contains("emit_session_coordination_event"));

    let core_path = dialog_manifest.join("manager/core.rs");
    let core_source = fs::read_to_string(&core_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", core_path.display()));
    let ingress = compact(function_source(
        &core_source,
        "async fn process_global_transaction_event(&self",
    ));
    assert!(ingress.contains("elseifmatches!(&event,TransactionEvent::AckReceived{..})"));
    assert!(
        !ingress.contains("find_dialog_for_request(request).await"),
        "ACK ingress must not reconstruct a missing exact transaction binding from dialog tags"
    );
}

#[test]
fn application_projection_surfaces_do_not_own_signaling_state() {
    let manifest = Path::new(MANIFEST_DIR);
    let cases = [
        ("src/api/lifecycle.rs", "pub(crate) fn publish(&self", false),
        ("src/api/endpoint.rs", "fn map_event", false),
        (
            "src/api/stream_peer.rs",
            "pub async fn next(&mut self",
            false,
        ),
        ("src/api/callback_peer.rs", "async fn dispatch", true),
        (
            "src/adapters/session_event_handler.rs",
            "fn project_committed_response_events",
            false,
        ),
    ];

    for (relative, signature, explicit_control_boundary) in cases {
        let path = manifest.join(relative);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let body = compact(function_source(&source, signature));
        for forbidden in ["process_event", "state_machine", "update_session"] {
            assert!(
                !body.contains(forbidden),
                "{relative} projection `{signature}` regained signaling authority via {forbidden}"
            );
        }
        if explicit_control_boundary {
            assert!(body.contains("coordinator"));
        }
    }
}

#[test]
fn deleted_orphan_signaling_modules_stay_absent() {
    let deleted_modules = [
        ("src/api/callbacks.rs", "callbacks"),
        ("src/api/terminal.rs", "terminal"),
        ("src/session_store/inspection.rs", "inspection"),
        ("src/session_store/cleanup.rs", "cleanup"),
    ];
    let manifest_dir = Path::new(MANIFEST_DIR);

    for (relative_path, _) in deleted_modules {
        assert!(
            !manifest_dir.join(relative_path).exists(),
            "deleted orphan signaling module was restored: {relative_path}"
        );
    }

    for (source_path, source) in source_roots(&["src"]) {
        let source = compact(&source);
        for (_, module) in deleted_modules {
            assert!(
                !source.contains(&format!("mod{module};"))
                    && !source.contains(&format!("mod{module}{{")),
                "{source_path} reintroduced the deleted orphan module `{module}`"
            );
        }
    }
}

#[test]
fn typed_dialog_ingress_and_in_dialog_tracker_are_generation_fenced() {
    let handler_path = Path::new(MANIFEST_DIR).join("src/adapters/session_event_handler.rs");
    let handler = fs::read_to_string(&handler_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", handler_path.display()));
    let production = compact(&strip_cfg_test_items(&handler));
    assert!(production.contains(
        "structQueuedDialogToSessionEvent{payload:QueuedDialogPayload,queued_at:Instant,kind:&'staticstr,route_key:Option<String>,exact_handle:Option<SessionRegistryHandle>"
    ));

    let worker = compact(function_source(
        &handler,
        "fn new(\n        handler: SessionCrossCrateEventHandler",
    ));
    let revalidate = worker
        .find("queued_dialog_lifetime_is_current")
        .expect("queued dialog worker revalidates the captured lifetime");
    let dispatch = worker
        .find("dispatch_queued_dialog_payload")
        .expect("queued dialog worker dispatches the typed payload");
    assert!(revalidate < dispatch);

    let capture = compact(function_source(
        &handler,
        "fn capture_dialog_ingress_handle",
    ));
    assert!(capture.contains("store.lifecycle_handle(&session_id).map(Some)"));
    assert!(capture.contains("DialogToSessionEvent::IncomingCall"));
    assert!(capture.contains("DialogToSessionEvent::MessageReceived"));

    let typed = compact(function_source(
        &handler,
        "async fn handle_dialog_to_session_event",
    ));
    assert!(typed.contains("exact_handle:Option<&SessionRegistryHandle>"));
    assert!(typed.matches("get_session_snapshot_exact(handle)").count() >= 3);

    let terminal = compact(function_source(
        &handler,
        "async fn publish_and_release_session",
    ));
    assert!(terminal.contains("get_session_snapshot_exact(&handle)"));
    assert!(!terminal.contains("lifecycle_handle("));

    let flow = compact(function_source(
        &handler,
        "async fn handle_outbound_flow_failed_parts",
    ));
    assert!(flow.contains("state.lifecycle_handle.clone()"));
    assert!(flow.contains("process_event_exact(&handle"));

    let dialog_created = compact(function_source(
        &handler,
        "async fn handle_dialog_created_parts",
    ));
    assert!(!dialog_created.contains("process_event("));
    assert!(!dialog_created.contains("lifecycle_handle("));

    let tracker_path = Path::new(MANIFEST_DIR).join("src/adapters/outbound_request_tracker.rs");
    let tracker = fs::read_to_string(&tracker_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", tracker_path.display()));
    let key = compact(function_source(&tracker, "struct TrackedRequestKey"));
    assert!(key.contains("handle:SessionRegistryHandle"));
    assert!(!key.contains("session_id:SessionId"));
    let deferred = compact(function_source(
        &tracker,
        "enum DeferredTrackedRequestEvent",
    ));
    assert_eq!(deferred.matches("handle:SessionRegistryHandle").count(), 2);
    assert!(
        compact(&tracker).contains("pub(crate)fnclear_exact(&self,handle:&SessionRegistryHandle)")
    );
    assert!(compact(&tracker).contains("pub(crate)fnlive_request_count(&self)->usize"));
    assert!(compact(&tracker).contains("pub(crate)fndeferred_event_count(&self)->usize"));
}

#[test]
fn media_event_bus_is_reporting_only_without_narrowing_yaml_events() {
    let handler_path = Path::new(MANIFEST_DIR).join("src/adapters/session_event_handler.rs");
    let handler = fs::read_to_string(&handler_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", handler_path.display()));
    let production = compact(&strip_cfg_test_items(&handler));
    let media_handler = compact(function_source(
        &handler,
        "async fn handle_media_to_session_event",
    ));
    assert!(media_handler.contains("media_observation_api_event(event)"));
    assert!(media_handler.contains("publish_api_event(&self.app_event_publisher,api_event)"));
    assert!(!media_handler.contains("process_event"));
    assert!(!media_handler.contains("EventType::"));

    for deleted_causal_helper in [
        "handle_media_stream_started_session",
        "handle_media_stream_stopped_parts",
        "handle_media_flow_established_session",
        "handle_media_error_parts",
        "handle_media_quality_degraded_parts",
        "handle_dtmf_detected_parts",
        "handle_rtp_timeout_parts",
        "handle_packet_loss_threshold_exceeded_parts",
    ] {
        assert!(
            !production.contains(deleted_causal_helper),
            "media event bus restored causal helper {deleted_causal_helper}"
        );
    }

    let projection = compact(function_source(&handler, "fn media_observation_api_event"));
    assert!(projection.contains("Event::MediaQualityChanged"));
    assert!(!projection.contains("Event::DtmfReceived"));
    assert!(!projection.contains("process_event"));

    let media_path = Path::new(MANIFEST_DIR).join("src/adapters/media_adapter.rs");
    let media = fs::read_to_string(&media_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", media_path.display()));
    let dtmf = compact(function_source(&media, "async fn install_dtmf_callback"));
    assert!(dtmf.contains("lifecycle_handle:SessionRegistryHandle"));
    assert!(dtmf.contains("publisher.publish_exact(&lifecycle_handle,api_event)"));
    let security = compact(function_source(
        &media,
        "async fn publish_media_security_observation_inner",
    ));
    assert!(security.contains("publisher.publish_exact(&lifecycle_handle,api_event)"));

    let types = fs::read_to_string(Path::new(MANIFEST_DIR).join("src/state_table/types.rs"))
        .expect("read state-table event grammar");
    let loader = fs::read_to_string(Path::new(MANIFEST_DIR).join("src/state_table/yaml_loader.rs"))
        .expect("read YAML event grammar");
    for retained_yaml_event in [
        "MediaSessionReady",
        "MediaFlowEstablished",
        "MediaError",
        "MediaQualityDegraded",
        "DtmfDetected",
        "RtpTimeout",
        "PacketLossThresholdExceeded",
    ] {
        assert!(types.contains(retained_yaml_event));
        assert!(loader.contains(&format!("variant: \"{retained_yaml_event}\"")));
    }
}

#[test]
fn session_handle_audio_keeps_exact_generation_authority_off_the_raw_hot_path() {
    let manifest = Path::new(MANIFEST_DIR);
    let handle_path = manifest.join("src/api/handle.rs");
    let handle = fs::read_to_string(&handle_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", handle_path.display()));
    let audio = compact(function_source(
        &strip_cfg_test_items(&handle),
        "pub async fn audio",
    ));
    assert!(audio.contains("self.exact_lifecycle_handle()?"));
    assert!(
        audio.contains(".try_operation_exact(lifecycle_handle.key(),SessionOperationKind::Media)")
    );
    assert!(audio.contains(".subscribe_to_audio_exact(&lifecycle_handle)"));
    assert!(audio.contains("coordinator.send_audio_exact(&send_handle,frame)"));
    assert!(audio.matches("cancellation.changed()").count() >= 2);
    assert!(!audio.contains("subscribe_to_audio(&self.call_id)"));
    assert!(!audio.contains("send_audio(&call_id"));
    let exact_handle = compact(function_source(
        &strip_cfg_test_items(&handle),
        "fn exact_lifecycle_handle",
    ));
    assert!(exact_handle.contains("self.lifecycle_handle.clone()"));
    assert!(exact_handle.contains("handle.session_id()==&self.call_id"));

    let media_path = manifest.join("src/adapters/media_adapter.rs");
    let media = fs::read_to_string(&media_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", media_path.display()));
    let send = compact(function_source(
        &media,
        "pub(crate) async fn send_audio_frame_exact",
    ));
    assert!(send.contains("media_for_handle_exact(handle)"));
    assert!(send.contains("encode_and_send_audio_frame(&exact.dialog_id"));
    assert!(!send.contains("current_media("));
    assert!(!send.contains("media_is_still_exact("));
    assert!(!send.contains("session_id:&SessionId"));

    let subscribe = compact(function_source(
        &media,
        "pub(crate) async fn subscribe_to_audio_frames_exact",
    ));
    assert!(subscribe.contains("spawn_owned_exact("));
    assert!(subscribe.contains("media_for_handle_exact(&exact_handle)"));
    assert!(!subscribe.contains("current_media("));
}

#[test]
fn incoming_call_control_capability_is_causal_and_generation_qualified() {
    let manifest = Path::new(MANIFEST_DIR);

    let event_path = manifest.join("src/adapters/session_api_event.rs");
    let event = fs::read_to_string(&event_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", event_path.display()));
    let control_envelope = compact(function_source(
        &event,
        "pub(crate) struct SessionControlEvent",
    ));
    assert!(control_envelope.contains("lifecycle_handle:Option<SessionRegistryHandle>"));

    let producer_path = manifest.join("src/adapters/session_event_handler.rs");
    let producer = fs::read_to_string(&producer_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", producer_path.display()));
    let incoming_producer = compact(function_source(
        &strip_cfg_test_items(&producer),
        "async fn handle_incoming_call_parts",
    ));
    assert!(incoming_producer.contains("app_event_publisher.publish_exact(&lifecycle,"));
    assert!(incoming_producer.contains("process_inbound_response_event_exact_on_fresh_task("));
    assert!(incoming_producer.contains("lifecycle.clone(),event_type"));
    assert!(!strip_cfg_test_items(&producer).contains("process_event_on_fresh_task"));

    let lifecycle_path = manifest.join("src/api/lifecycle.rs");
    let lifecycle = fs::read_to_string(&lifecycle_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", lifecycle_path.display()));
    let publish_exact = compact(function_source(
        &strip_cfg_test_items(&lifecycle),
        "pub(crate) fn publish_exact",
    ));
    assert!(publish_exact.contains("offer_to_control_owner(&event,Some(lifecycle_handle))"));
    assert!(publish_exact.contains("sanitize_session_api_observation(&event)"));
    assert!(publish_exact.contains("record_event_exact(lifecycle_handle,&event)"));
    assert!(lifecycle.contains("pub(crate) fn watcher_exact("));
    assert!(lifecycle.contains("pub(crate) fn snapshot_exact("));
    let publish_terminal_exact = compact(function_source(
        &strip_cfg_test_items(&lifecycle),
        "pub(crate) fn publish_terminal_best_effort_exact",
    ));
    assert!(publish_terminal_exact.contains("record_event_exact(lifecycle_handle,&event)"));
    assert!(
        publish_terminal_exact.contains("offer_to_control_owner(&event,Some(lifecycle_handle))")
    );

    let stream_path = manifest.join("src/api/stream_peer.rs");
    let stream = fs::read_to_string(&stream_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", stream_path.display()));
    let receive = compact(function_source(
        &stream,
        "pub(crate) async fn next_with_lifecycle",
    ));
    assert!(receive.contains("SessionControlEvent"));
    assert!(receive.contains("control.lifecycle_handle"));
    let filtered_exact = compact(function_source(&stream, "pub(crate) fn filtered_exact"));
    assert!(filtered_exact.contains("exact_filter:Some(lifecycle_handle)"));
    let wait = compact(function_source(&stream, "pub async fn wait_for_incoming"));
    assert!(wait.contains("next_incoming_exact().await"));
    assert!(wait.contains("pending_incoming_bundle_for_handle_exact(handle)"));
    assert!(wait.contains("IncomingCall::with_request_captured("));
    assert!(wait.contains("IncomingCall::new_captured("));
    assert!(!wait.contains("pending_incoming_bundle_exact(&call_id)"));
    let wait_answered = compact(function_source(&stream, "pub async fn wait_for_answered"));
    assert!(wait_answered.contains("next_with_lifecycle().await"));
    assert!(wait_answered.contains("SessionHandle::new_captured("));

    let incoming_path = manifest.join("src/api/incoming.rs");
    let incoming = fs::read_to_string(&incoming_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", incoming_path.display()));
    let incoming_request_impl = incoming
        .split_once("impl IncomingRequest")
        .expect("IncomingRequest implementation")
        .1;
    let reject = compact(function_source(&incoming, "pub fn reject(mut self"));
    assert!(reject.contains("spawn_exact_incoming_reject("));
    assert!(!reject.contains("tokio::spawn("));
    assert!(!reject.contains(".reject(&call_id)"));
    let defer = compact(function_source(&incoming, "pub fn defer(mut self"));
    assert!(defer.contains("IncomingCallGuard::new_captured("));
    assert!(defer.contains("self.lifecycle_handle.clone()"));
    let guard_impl = incoming
        .split_once("impl IncomingCallGuard")
        .expect("IncomingCallGuard implementation")
        .1;
    let guard_accept = compact(function_source(guard_impl, "pub async fn accept(self)"));
    assert!(guard_accept.contains("accept_call_exact(&lifecycle_handle)"));
    assert!(guard_accept.contains("SessionHandle::new_exact("));
    let request_handle = compact(function_source(
        incoming_request_impl,
        "pub fn session_handle(&self)",
    ));
    assert!(request_handle.contains("self.lifecycle_handle.clone().ok_or_else"));
    assert!(request_handle.contains("SessionHandle::new_exact("));
    assert!(!request_handle.contains("SessionHandle::new("));
    let request_response = compact(function_source(
        incoming_request_impl,
        "pub fn respond_builder(",
    ));
    assert!(request_response.contains("GenericResponseBuilder::new_exact("));
    let request_challenge = compact(function_source(
        incoming_request_impl,
        "pub fn challenge_builder(",
    ));
    assert!(request_challenge.contains("AuthChallengeBuilder::new_exact("));
    let captured_coordinator = compact(function_source(
        incoming_request_impl,
        "pub(crate) fn set_coordinator_captured",
    ));
    assert!(captured_coordinator.contains("self.lifecycle_handle=lifecycle_handle"));
    let clear_response = compact(function_source(
        incoming_request_impl,
        "pub(crate) fn clear_response_capability",
    ));
    assert!(clear_response.contains("self.lifecycle_handle=None"));

    let callback_path = manifest.join("src/api/callback_peer.rs");
    let callback = fs::read_to_string(&callback_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", callback_path.display()));
    let dispatch = compact(function_source(&callback, "async fn dispatch"));
    assert!(dispatch
        .contains("lifecycle_handle:Option<crate::session_registry::SessionRegistryHandle>"));
    assert!(dispatch.contains("accept_call_exact(exact_handle)"));
    assert!(dispatch.contains("accept_call_with_sdp_exact(exact_handle,sdp)"));
    assert!(dispatch.contains("reject_call_exact(exact_handle,status,&reason)"));
    assert!(dispatch.contains("redirect_call_exact(exact_handle,302,vec![target])"));
    assert!(dispatch.contains("SessionHandle::new_captured("));
    assert!(dispatch.contains("set_coordinator_captured("));
    assert!(!dispatch.contains("SessionHandle::new("));
    let configured_refer = compact(function_source(
        &callback,
        "async fn on_refer_received(&self, request:",
    ));
    assert!(configured_refer.contains("request.session_handle()"));
    assert!(!configured_refer.contains("SessionHandle::new("));

    let attach_requests = compact(function_source(
        &stream,
        "fn attach_incoming_request_authority",
    ));
    assert!(attach_requests.contains("set_coordinator_captured("));

    let endpoint_path = manifest.join("src/api/endpoint.rs");
    let endpoint = fs::read_to_string(&endpoint_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", endpoint_path.display()));
    let map_event = compact(function_source(&endpoint, "fn map_event"));
    assert!(map_event.contains("SessionHandle::new_captured("));

    let executor_path = manifest.join("src/state_machine/executor.rs");
    let executor = fs::read_to_string(&executor_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", executor_path.display()));
    assert!(compact(&executor).contains(
        "exact_api_event_publisher:OnceLock<crate::api::lifecycle::SessionEventPublisher>"
    ));
    assert!(compact(&executor).contains("publisher.publish_exact(handle,exact_api_event)"));

    let unified_path = manifest.join("src/api/unified.rs");
    let unified = fs::read_to_string(&unified_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", unified_path.display()));
    assert!(compact(&unified)
        .contains("state_machine.init_exact_api_event_publisher(app_event_publisher.clone())"));
}

#[test]
fn retired_parallel_transaction_dispatcher_cannot_return() {
    let path = Path::new(MANIFEST_DIR).join("../sip-dialog/src/transaction/manager/handlers.rs");
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

    assert!(!source.contains("fn handle_transport_message("));
    assert!(!source.contains("fn determine_ack_destination("));
    assert!(!source.contains("fn resolve_uri_to_socketaddr("));
    assert!(!source.contains("fn resolve_host_to_socketaddr("));
}

#[test]
fn legacy_session_callbacks_observe_only_committed_executor_events() {
    let manifest = Path::new(MANIFEST_DIR);
    let handler_path = manifest.join("src/adapters/session_event_handler.rs");
    let handler = fs::read_to_string(&handler_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", handler_path.display()));
    let observer = compact(function_source(
        &handler,
        "async fn handle_state_machine_event(",
    ));
    assert!(observer.contains("self.helpers.notify_subscribers("));

    let helpers_path = manifest.join("src/state_machine/helpers.rs");
    let helpers = fs::read_to_string(&helpers_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", helpers_path.display()));
    let notifier = compact(function_source(
        &helpers,
        "pub(crate) async fn notify_subscribers(",
    ));
    assert!(notifier.contains("callback(event.clone())"));
}

#[test]
fn signaling_authority_modules_do_not_hide_dead_paths() {
    let manifest = Path::new(MANIFEST_DIR);
    for relative in [
        "src/session_lifecycle.rs",
        "src/adapters/dialog_adapter.rs",
        "src/adapters/session_event_handler.rs",
        "src/state_machine/actions.rs",
        "src/state_machine/executor.rs",
        "src/state_machine/helpers.rs",
    ] {
        let path = manifest.join(relative);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        assert!(
            !source.contains("allow(dead_code)"),
            "{relative} must delete or test-gate dead signaling code instead of suppressing it"
        );
    }
}
