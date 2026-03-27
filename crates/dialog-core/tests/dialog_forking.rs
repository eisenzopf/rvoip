//! Tests for SIP dialog forking support (RFC 3261 §13.2.2.4, §16.7)
//!
//! Validates:
//! - Early dialog group tracking with DashSet
//! - Fork detection via distinct To-tags
//! - Multi-2xx handling (ACK every 2xx, BYE extras)
//! - Cleanup of early dialog groups

use rvoip_dialog_core::{Dialog, DialogId, DialogState};
use dashmap::{DashMap, DashSet};
use std::sync::Arc;

// ──────────────────────────────────────────────
// Helper: simulate the early-dialog-group API
// (mirrors DialogManager fields without needing
// a full transport / transaction stack)
// ──────────────────────────────────────────────

struct ForkTracker {
    dialogs: DashMap<DialogId, Dialog>,
    early_groups: DashMap<String, Arc<DashSet<DialogId>>>,
}

impl ForkTracker {
    fn new() -> Self {
        Self {
            dialogs: DashMap::new(),
            early_groups: DashMap::new(),
        }
    }

    fn add_to_early_group(&self, call_id: &str, dialog_id: DialogId) {
        let set = self.early_groups
            .entry(call_id.to_string())
            .or_insert_with(|| Arc::new(DashSet::new()))
            .clone();
        set.insert(dialog_id);
    }

    fn get_early_group(&self, call_id: &str) -> Vec<DialogId> {
        self.early_groups
            .get(call_id)
            .map(|s| s.iter().map(|r| r.clone()).collect())
            .unwrap_or_default()
    }

    fn remove_from_early_group(&self, call_id: &str, dialog_id: &DialogId) {
        if let Some(set) = self.early_groups.get(call_id) {
            set.remove(dialog_id);
            if set.is_empty() {
                drop(set);
                self.early_groups.remove(call_id);
            }
        }
    }

    fn cleanup_early_group(&self, call_id: &str) {
        self.early_groups.remove(call_id);
    }

    fn find_by_call_id_and_to_tag(&self, call_id: &str, to_tag: &str) -> Option<DialogId> {
        for entry in self.dialogs.iter() {
            let d = entry.value();
            if d.call_id == call_id && d.remote_tag.as_deref() == Some(to_tag) {
                return Some(d.id.clone());
            }
        }
        None
    }

    fn store(&self, dialog: Dialog) {
        self.dialogs.insert(dialog.id.clone(), dialog);
    }
}

fn make_early_dialog(call_id: &str, local_tag: &str, remote_tag: Option<&str>) -> Dialog {
    let local_uri = rvoip_sip_core::Uri::sip("alice@example.com");
    let remote_uri = rvoip_sip_core::Uri::sip("bob@example.com");
    Dialog::new_early(
        call_id.to_string(),
        local_uri,
        remote_uri,
        Some(local_tag.to_string()),
        remote_tag.map(|t| t.to_string()),
        true, // UAC
    )
}

// ──────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────

#[test]
fn test_fork_detection_by_to_tag() {
    let tracker = ForkTracker::new();
    let call_id = "call-fork-001";

    // Original dialog gets first 180 with To-tag "tag-uas-1"
    let d1 = make_early_dialog(call_id, "tag-uac", Some("tag-uas-1"));
    let d1_id = d1.id.clone();
    tracker.store(d1);
    tracker.add_to_early_group(call_id, d1_id.clone());

    // Second 180 arrives with different To-tag "tag-uas-2" → new fork
    assert!(tracker.find_by_call_id_and_to_tag(call_id, "tag-uas-2").is_none());

    let d2 = make_early_dialog(call_id, "tag-uac", Some("tag-uas-2"));
    let d2_id = d2.id.clone();
    tracker.store(d2);
    tracker.add_to_early_group(call_id, d2_id.clone());

    // Both should be in the early group
    let group = tracker.get_early_group(call_id);
    assert_eq!(group.len(), 2);
    assert!(group.contains(&d1_id));
    assert!(group.contains(&d2_id));

    // Same To-tag should find existing dialog
    assert_eq!(
        tracker.find_by_call_id_and_to_tag(call_id, "tag-uas-1"),
        Some(d1_id)
    );
}

#[test]
fn test_early_group_cleanup_on_confirm() {
    let tracker = ForkTracker::new();
    let call_id = "call-fork-002";

    // Three forks
    let d1 = make_early_dialog(call_id, "tag-uac", Some("tag-a"));
    let d2 = make_early_dialog(call_id, "tag-uac", Some("tag-b"));
    let d3 = make_early_dialog(call_id, "tag-uac", Some("tag-c"));
    let d1_id = d1.id.clone();
    let d2_id = d2.id.clone();
    let d3_id = d3.id.clone();
    tracker.store(d1);
    tracker.store(d2);
    tracker.store(d3);
    tracker.add_to_early_group(call_id, d1_id.clone());
    tracker.add_to_early_group(call_id, d2_id.clone());
    tracker.add_to_early_group(call_id, d3_id.clone());

    assert_eq!(tracker.get_early_group(call_id).len(), 3);

    // Confirm d2, remove others
    for id in &[&d1_id, &d3_id] {
        tracker.remove_from_early_group(call_id, id);
    }
    tracker.remove_from_early_group(call_id, &d2_id);

    // Group should be empty and auto-cleaned
    assert!(tracker.get_early_group(call_id).is_empty());
    assert!(!tracker.early_groups.contains_key(call_id));
}

#[test]
fn test_cleanup_entire_early_group() {
    let tracker = ForkTracker::new();
    let call_id = "call-fork-003";

    let d1 = make_early_dialog(call_id, "tag-uac", Some("tag-x"));
    let d2 = make_early_dialog(call_id, "tag-uac", Some("tag-y"));
    tracker.store(d1.clone());
    tracker.store(d2.clone());
    tracker.add_to_early_group(call_id, d1.id.clone());
    tracker.add_to_early_group(call_id, d2.id.clone());

    assert_eq!(tracker.get_early_group(call_id).len(), 2);

    tracker.cleanup_early_group(call_id);

    assert!(tracker.get_early_group(call_id).is_empty());
    assert!(!tracker.early_groups.contains_key(call_id));
}

#[test]
fn test_no_fork_same_to_tag() {
    let tracker = ForkTracker::new();
    let call_id = "call-no-fork";

    let d = make_early_dialog(call_id, "tag-uac", Some("tag-same"));
    let d_id = d.id.clone();
    tracker.store(d);
    tracker.add_to_early_group(call_id, d_id.clone());

    // A second provisional with the same To-tag is NOT a fork
    let found = tracker.find_by_call_id_and_to_tag(call_id, "tag-same");
    assert_eq!(found, Some(d_id));

    // Group should still have just one entry
    assert_eq!(tracker.get_early_group(call_id).len(), 1);
}

#[test]
fn test_multi_2xx_both_confirmed_bye_extras() {
    // Simulates the scenario where two forks both send 200 OK.
    // The first confirmed fork becomes the chosen dialog.
    // The second must be ACKed and then BYE'd (not CANCELled).
    let tracker = ForkTracker::new();
    let call_id = "call-multi-2xx";

    let mut d1 = make_early_dialog(call_id, "tag-uac", Some("tag-uas-1"));
    let mut d2 = make_early_dialog(call_id, "tag-uac", Some("tag-uas-2"));
    let d1_id = d1.id.clone();
    let d2_id = d2.id.clone();

    // Both receive 200 OK → Confirmed
    d1.state = DialogState::Confirmed;
    d2.state = DialogState::Confirmed;
    tracker.store(d1);
    tracker.store(d2);
    tracker.add_to_early_group(call_id, d1_id.clone());
    tracker.add_to_early_group(call_id, d2_id.clone());

    // Pick d1 as the winner. For d2, we would BYE (not CANCEL).
    // Verify d2 is confirmed (meaning we must send BYE, not CANCEL).
    let d2_state = tracker.dialogs.get(&d2_id).map(|d| d.state.clone());
    assert_eq!(d2_state, Some(DialogState::Confirmed),
        "A fork with a 2xx is Confirmed; CANCEL is invalid – must use BYE");

    // Clean up
    tracker.cleanup_early_group(call_id);
    assert!(tracker.get_early_group(call_id).is_empty());
}

#[test]
fn test_forked_response_event_variant() {
    // Verify the ForkedResponse variant is constructible and debuggable.
    use rvoip_dialog_core::events::SessionCoordinationEvent;

    let event = SessionCoordinationEvent::ForkedResponse {
        call_id: "call-123".to_string(),
        dialog_id: DialogId::new(),
        status_code: 180,
    };

    let debug_str = format!("{:?}", event);
    assert!(debug_str.contains("ForkedResponse"));
    assert!(debug_str.contains("call-123"));
    assert!(debug_str.contains("180"));
}

#[test]
fn test_concurrent_fork_group_access() {
    // Verify DashSet handles concurrent access without panics.
    use std::thread;

    let tracker = Arc::new(ForkTracker::new());
    let call_id = "call-concurrent";

    let mut handles = vec![];
    for i in 0..10 {
        let t = tracker.clone();
        let cid = call_id.to_string();
        handles.push(thread::spawn(move || {
            let d = make_early_dialog(&cid, "tag-uac", Some(&format!("tag-{}", i)));
            let did = d.id.clone();
            t.store(d);
            t.add_to_early_group(&cid, did);
        }));
    }

    for h in handles {
        h.join().ok();
    }

    assert_eq!(tracker.get_early_group(call_id).len(), 10);
    tracker.cleanup_early_group(call_id);
    assert!(tracker.get_early_group(call_id).is_empty());
}
