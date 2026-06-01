//! Architectural guard: the REGISTER send path stays flat.
//!
//! `DialogAdapter::send_register` once dispatched its own follow-up events
//! back into the state machine, which made registration retries recurse
//! through `StateMachine::process_event` and obscured failure attribution.
//! The send path is now expected to return a typed outcome to its caller; the
//! state machine drains queued internal events in a loop instead of boxing
//! itself recursively.
//!
//! This test reads the two source files and rejects any change that
//! re-introduces either pattern. It is a textual guard (no runtime SIP
//! traffic) so it is cheap to run on every CI build and triggers on the
//! review that introduces the regression.

#[test]
fn register_send_path_does_not_reenter_state_machine() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let adapter_path = format!("{manifest_dir}/src/adapters/dialog_adapter.rs");
    let adapter = std::fs::read_to_string(&adapter_path).expect("dialog adapter source");
    let send_register_body = adapter
        .split("async fn send_register")
        .nth(1)
        .expect("send_register function should exist")
        .split("pub async fn send_subscribe")
        .next()
        .expect("send_subscribe should follow send_register");

    assert!(
        !send_register_body.contains("process_event("),
        "DialogAdapter::send_register must return a typed outcome instead of dispatching state-machine events inline"
    );
    assert!(
        !send_register_body.contains("self.send_register("),
        "DialogAdapter::send_register must not recursively retry itself"
    );

    let executor_path = format!("{manifest_dir}/src/state_machine/executor.rs");
    let executor = std::fs::read_to_string(&executor_path).expect("executor source");
    assert!(
        !executor.contains("Box::pin(self.process_event"),
        "StateMachine::process_event should drain queued internal events instead of recursively boxing itself"
    );
}
