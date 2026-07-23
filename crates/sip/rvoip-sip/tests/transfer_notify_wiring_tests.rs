//! Structural + action-ordering tests for the RFC 3515 §2.4.5 progress-NOTIFY
//! wiring added for b2bua support.
//!
//! These tests exercise the default state table — they don't touch the wire.
//! The invariants being guarded:
//!
//! 1. `Active + TransferRequested` fires `SendRefer100Trying` **before**
//!    `SendReferAccepted` so the 100 Trying ack goes out first per
//!    RFC 3515 §2.4.5.
//! 2. Transfer-leg UAC transitions (`Dialog180Ringing`,
//!    `Dialog200OK` on `Initiating`, `Dialog200OK` on `Ringing`)
//!    include the corresponding `SendTransferNotify*` action. Failure and
//!    timeout rows also carry `SendTransferNotifyFailure`, making the YAML
//!    table the single lifecycle owner instead of an adapter fallback.
//! 3. `SendTransferNotify*` actions land **after** media/state-commit
//!    actions (`NegotiateSDPAsUAC`, `SendACK`, `StartMediaSession`)
//!    so a NOTIFY-send failure cannot roll back dialog / media
//!    establishment.
//!
//! Semantics covered: the "race" guarded by
//! `StateMachineHelpers::make_transfer_leg` is *structural* — exact linkage is
//! set on `SessionState` before `MakeCall` dispatches, so these actions always
//! carry the generation-qualified transferor handle. The actions are no-ops
//! otherwise, so appending them to shared `Both`-role transitions is safe for
//! non-transfer calls.

use rvoip_sip::state_table::{Action, EventType, Role, StateKey, StateTable, YamlTableLoader};
use rvoip_sip::types::CallState;
use std::path::Path;

fn load_default() -> StateTable {
    let path = Path::new("state_tables").join("default.yaml");
    YamlTableLoader::load_from_file(path).expect("default.yaml should load")
}

fn actions_at(table: &StateTable, key: &StateKey) -> Vec<Action> {
    table
        .get(key)
        .unwrap_or_else(|| panic!("expected transition for {:?}", key))
        .actions
        .clone()
}

fn position(actions: &[Action], target: &Action) -> Option<usize> {
    actions.iter().position(|a| a == target)
}

fn assert_contains(actions: &[Action], target: &Action, ctx: &str) {
    assert!(
        position(actions, target).is_some(),
        "{} — expected {:?} in action list, got {:?}",
        ctx,
        target,
        actions
    );
}

fn assert_ordered(actions: &[Action], first: &Action, second: &Action, ctx: &str) {
    let first_idx = position(actions, first)
        .unwrap_or_else(|| panic!("{} — missing {:?} in {:?}", ctx, first, actions));
    let second_idx = position(actions, second)
        .unwrap_or_else(|| panic!("{} — missing {:?} in {:?}", ctx, second, actions));
    assert!(
        first_idx < second_idx,
        "{} — expected {:?} BEFORE {:?} but got {:?}",
        ctx,
        first,
        second,
        actions
    );
}

#[test]
fn inbound_refer_notify_uses_explicit_uac_and_uas_wildcards() {
    let table = load_default();
    for role in [Role::UAC, Role::UAS] {
        for state in [
            CallState::Idle,
            CallState::Initiating,
            CallState::Active,
            CallState::OnHold,
            CallState::Subscribed,
            CallState::Terminating,
        ] {
            let key = StateKey {
                role,
                state,
                event: EventType::ReceiveNOTIFY,
            };
            let transition = table
                .get(&key)
                .unwrap_or_else(|| panic!("missing {role:?}/{state:?}/ReceiveNOTIFY wildcard"));
            assert_eq!(transition.next_state, None);
            assert_eq!(transition.actions, vec![Action::ProcessNOTIFY]);
        }
    }

    assert!(table
        .get(&StateKey {
            role: Role::Both,
            state: CallState::Active,
            event: EventType::ReceiveNOTIFY,
        })
        .is_none());
}

#[test]
fn transfer_requested_fires_100_trying_before_202_accepted() {
    let table = load_default();
    let key = StateKey {
        role: Role::Both,
        state: CallState::Active,
        event: EventType::TransferRequested {
            refer_to: String::new(),
            transfer_type: String::new(),
            transaction_id: String::new(),
        },
    };
    let actions = actions_at(&table, &key);
    assert_ordered(
        &actions,
        &Action::SendRefer100Trying,
        &Action::SendReferAccepted,
        "Active+TransferRequested",
    );
}

#[test]
fn uac_initiating_180_fires_transfer_notify_ringing() {
    let table = load_default();
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Initiating,
        event: EventType::Dialog180Ringing,
    };
    let actions = actions_at(&table, &key);
    assert_contains(
        &actions,
        &Action::SendTransferNotifyRinging,
        "UAC/Initiating/Dialog180Ringing",
    );
}

#[test]
fn uac_ringing_200_fires_transfer_notify_success_after_media_commit() {
    let table = load_default();
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Ringing,
        event: EventType::Dialog200OK,
    };
    let actions = actions_at(&table, &key);
    assert_contains(
        &actions,
        &Action::SendTransferNotifySuccess,
        "UAC/Ringing/Dialog200OK",
    );
    // Media-commit actions must fire first so a NOTIFY-send failure
    // cannot roll back the dialog / media state we just committed.
    assert_ordered(
        &actions,
        &Action::StartMediaSession,
        &Action::SendTransferNotifySuccess,
        "UAC/Ringing/Dialog200OK ordering",
    );
}

#[test]
fn uac_initiating_200_fires_transfer_notify_success() {
    let table = load_default();
    let key = StateKey {
        role: Role::UAC,
        state: CallState::Initiating,
        event: EventType::Dialog200OK,
    };
    let actions = actions_at(&table, &key);
    assert_contains(
        &actions,
        &Action::SendTransferNotifySuccess,
        "UAC/Initiating/Dialog200OK (fast answer)",
    );
}

#[test]
fn uac_early_media_200_fires_transfer_notify_success() {
    let table = load_default();
    let key = StateKey {
        role: Role::UAC,
        state: CallState::EarlyMedia,
        event: EventType::Dialog200OK,
    };
    let actions = actions_at(&table, &key);
    assert_contains(
        &actions,
        &Action::SendTransferNotifySuccess,
        "UAC/EarlyMedia/Dialog200OK",
    );
    assert_ordered(
        &actions,
        &Action::SendACK,
        &Action::SendTransferNotifySuccess,
        "UAC/EarlyMedia/Dialog200OK ordering",
    );
}

#[test]
fn failure_transitions_route_exact_status_notify_through_yaml() {
    let table = load_default();
    for state in [
        CallState::Initiating,
        CallState::Ringing,
        CallState::EarlyMedia,
    ] {
        for event in [
            EventType::Dialog4xxFailure(400),
            EventType::Dialog5xxFailure(500),
            EventType::Dialog6xxFailure(600),
            EventType::DialogTimeout,
        ] {
            let key = StateKey {
                role: if state == CallState::Initiating {
                    Role::Both
                } else {
                    Role::UAC
                },
                state,
                event,
            };
            let actions = actions_at(&table, &key);
            assert_contains(
                &actions,
                &Action::SendTransferNotifyFailure,
                "terminal transfer failure",
            );
            assert_ordered(
                &actions,
                &Action::SendTransferNotifyFailure,
                &Action::CleanupDialog,
                "terminal transfer failure ownership",
            );
        }
    }
}

#[test]
fn transfer_notify_actions_do_not_change_next_state() {
    // Non-transfer calls must be unaffected by the newly-appended actions:
    // the `Dialog180Ringing → Ringing` and `Dialog200OK → Active` target
    // states must be unchanged.
    let table = load_default();
    let cases = [
        (
            StateKey {
                role: Role::UAC,
                state: CallState::Initiating,
                event: EventType::Dialog180Ringing,
            },
            CallState::Ringing,
        ),
        (
            StateKey {
                role: Role::UAC,
                state: CallState::EarlyMedia,
                event: EventType::Dialog200OK,
            },
            CallState::Active,
        ),
        (
            StateKey {
                role: Role::UAC,
                state: CallState::Ringing,
                event: EventType::Dialog200OK,
            },
            CallState::Active,
        ),
        (
            StateKey {
                role: Role::UAC,
                state: CallState::Initiating,
                event: EventType::Dialog200OK,
            },
            CallState::Active,
        ),
    ];
    for (key, want_state) in cases {
        let transition = table
            .get(&key)
            .unwrap_or_else(|| panic!("expected transition for {:?}", key));
        assert_eq!(
            transition.next_state,
            Some(want_state),
            "{:?} — next_state drifted to {:?}",
            key,
            transition.next_state
        );
    }
}
