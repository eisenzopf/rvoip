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
