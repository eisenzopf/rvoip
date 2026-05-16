# SBC topology-hiding walkthrough

**Pattern mapping:** SIP_API_DESIGN_2 §11.3 (trust-boundary B2BUA) +
§11.2 (B2BUA carry-through report) + §7 (`TraceRedactor`).

## What it demonstrates

- `coord.invite(..).with_headers_from(&IncomingCall, &names)?` carries
  application headers across a trust boundary while filtering every
  stack-managed name (`Via`, `Call-ID`, `CSeq`, `Max-Forwards`, …) into
  the returned `HeaderCarryThroughReport`.
- `.strip_header(&Privacy)` removes the upstream-supplied `Privacy`
  header before egress.
- `.with_raw_header(P-Asserted-Identity, REWRITTEN_PAI)` re-stamps PAI
  with the SBC's trust-asserted identity in place of the untrusted
  upstream value.
- `Config.trace_redaction = Some(Arc<dyn TraceRedactor>)` drops
  `Authorization:` from `SipTrace` output without affecting the wire
  (the §11.3 "redact in observability, never on the wire" guarantee).

## Run

```
cargo run --example sbc_topology_hiding
```

The example boots three coordinators in-process — `alice` (upstream
UAC) → `sbc` (middle) → `bob` (downstream UAS) — and prints the
carry-through report plus pass/fail checks for each contract.

## Wire vs. trace

The `Authorization:` header **does** reach the wire (transport sees
it, the receiving UA can authenticate). The `TraceRedactor` only
affects `Event::SipTrace` payloads — i.e., observability sinks that
ship traces to logs or external systems. This separation is the
load-bearing §11.3 invariant: SIP-layer redaction must never disturb
the bytes a peer actually receives.
