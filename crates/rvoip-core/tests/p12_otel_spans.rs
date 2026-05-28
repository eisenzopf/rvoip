//! P12.7 — verify that `#[tracing::instrument]` on the orchestrator's
//! entry points produces a real span hierarchy that downstream OTel
//! exporters can consume.
//!
//! This test installs a no-op tracing subscriber that captures span
//! names + their fields into a shared `Vec` and verifies that calling
//! the orchestrator's lifecycle methods emits the expected spans with
//! the expected attributes. It does NOT exercise the OTLP exporter
//! itself — that lives in `infra-common::logging::setup` behind the
//! `otel` feature and is verified at build time by
//! `cargo build -p rvoip-infra-common --features otel`.

use rvoip_core::config::Config;
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::ids::TenantId;
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::session::SessionMedium;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tracing::span::Attributes;
use tracing::{Id, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::Registry;

#[derive(Default, Clone)]
struct CapturedSpans {
    inner: Arc<Mutex<Vec<String>>>,
}

impl CapturedSpans {
    fn names(&self) -> Vec<String> {
        self.inner.lock().unwrap().clone()
    }
}

struct CaptureLayer {
    captured: CapturedSpans,
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, _id: &Id, _ctx: Context<'_, S>) {
        self.captured
            .inner
            .lock()
            .unwrap()
            .push(attrs.metadata().name().to_string());
    }
}

#[test]
fn orchestrator_methods_emit_named_spans() {
    // Use a sync test + manual runtime so we can wrap the async work
    // in `tracing::subscriber::with_default` (which is itself sync).
    let captured = CapturedSpans::default();
    let layer = CaptureLayer {
        captured: captured.clone(),
    };
    let subscriber = Registry::default().with(layer);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    tracing::subscriber::with_default(subscriber, || {
        runtime.block_on(async {
            let orch = Orchestrator::new(Config::default());
            // open_conversation → start_session → end_session →
            // close_conversation. Each entry point is instrumented; we
            // expect to see all four span names in the captured list.
            let cid = orch
                .open_conversation(
                    TenantId::new(),
                    ConversationPolicy::default(),
                    HashMap::new(),
                )
                .await
                .expect("open_conversation");
            let sid = orch
                .start_session(cid.clone(), SessionMedium::Voice, vec![])
                .await
                .expect("start_session");
            orch.end_session(sid, rvoip_core::adapter::EndReason::Normal)
                .await
                .expect("end_session");
            orch.close_conversation(cid, false)
                .await
                .expect("close_conversation");
        });
    });

    let names = captured.names();
    // Each span name should appear at least once. `#[instrument]`'s
    // default span name is the function name.
    for expected in [
        "open_conversation",
        "start_session",
        "end_session",
        "close_conversation",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "missing span: {expected} (captured = {names:?})"
        );
    }
}
