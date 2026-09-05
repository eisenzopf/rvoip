# Production remote SIP endpoint profile

Status: shipped in 0.3.9; deterministic and protected hosted qualification
passed. Live two-UA NAT/TLS/SDES deployment qualification remains pending and
is tracked separately.

This profile is the bounded way to serve physical SIP phones and independent
SIP UAs across NAT without making a general ICE/TURN claim. It combines SIP
Outbound registered-flow routing with mandatory encrypted media and fails
startup or registration when the required identity is absent.

## Contract

The server profile requires all of the following:

- a certificate-bearing SIP TLS listener;
- authenticated REGISTER identities in the built-in registrar;
- RFC 5626 `ob`, `+sip.instance`, and a nonzero `reg-id` on every admitted
  remote Contact;
- the exact process-local TLS or WSS connection that carried REGISTER;
- SDES-SRTP required for media, with no plaintext RTP fallback; and
- a concrete advertised media address reachable from the remote endpoint.

An authenticated but incomplete registration receives SIP `439 First Hop
Lacks Outbound Support` and does not mutate registrar state. The compatibility
registrar remains permissive unless the remote-endpoint profile is explicitly
selected.

At the high-level facade, select the profile with
`SipConfig::remote_endpoint_profile()` after configuring the listener,
registrar users, required media security, and public media address:

```rust,ignore
let sip = SipConfig::bind("0.0.0.0:5060")
    .domain("voice.example.com")
    .tls_listener(
        "0.0.0.0:5061".parse()?,
        "/run/secrets/sip-cert.pem",
        "/run/secrets/sip-key.pem",
    )
    .registrar_users([("alice", password_from_secret_store)])
    .media_security(SipMediaSecurity::Required)
    .media_public_addr("203.0.113.10:20000".parse()?)
    .remote_endpoint_profile();
```

Low-level applications can select the same registrar admission policy with
`UnifiedCoordinator::start_remote_endpoint_registration_server`.

## Signaling lifecycle

1. After Digest authentication, the registrar validates the outbound Contact
   against the observed secure stream and stages a random `rf1_` capability.
2. The Contact and route are committed only after the exact REGISTER response
   reaches a terminal written or wire-unknown outcome. A proven zero-wire
   failure discards the staged route and preserves the previous Contact and
   flow.
3. A registered-AOR lookup validates that the Contact is live, reachable, and
   still owns the capability. It returns ordered process-local routes rather
   than the Contact's private address.
4. INVITE uses the exact connection identity. Recoverable failure of the
   preferred flow advances to the next verified flow for that instance.
5. Connection close marks the route unusable immediately and publishes the
   authoritative degraded transition. A successful replacement REGISTER
   publishes one recovered transition.
6. Expiry, unregister, replacement, process restart, copied tokens, and wrong
   AOR/device identity all fail closed.

The endpoint is responsible for sending bounded RFC 5626 CRLF keepalives on
its outbound stream. The RVoIP TLS/TCP listener recognizes the frames, replies
with the required CRLF pong, and observes exact connection closure. RVoIP's UA
mode separately retains its existing configurable outbound keepalive and
automatic re-registration behavior.

## Media and NAT boundary

The qualified target is TLS signaling plus SDES-SRTP media. Media uses the
configured public server address and the existing symmetric RTP/RTCP latching
path, so return media follows the authenticated source address observed on the
wire. The profile does not claim arbitrary endpoint NAT traversal: deployments
that cannot provide symmetric media with a reachable server address need the
separately qualified SIP ICE/TURN profile.

## Replica and restart contract

Registered-flow capabilities are deliberately process-local and are never a
durable socket identifier. A deployment with more than one SIP replica must
preserve AOR/registrar affinity so an inbound request returns to the process
that accepted REGISTER. A process restart invalidates every old capability;
delivery remains unavailable until the UA re-registers. Persisting or copying
numeric transport identities is unsupported.

## Security and observability

- Flow tokens are random, opaque, omitted from serialized Contact projections,
  and redacted from `Debug` and registrar events.
- Numeric transport-flow IDs remain inside the owning process and are omitted
  from public events.
- Degraded/recovered events identify only the registrar user, instance, and
  `reg-id` needed for readiness decisions.
- REGISTER replacement is prepare/response/commit ordered, including a staged
  connection-close race; no failed response may silently replace a live route.
- TLS certificate/key paths and registrar credentials remain redacted by the
  facade configuration diagnostics.

## Release qualification boundary

Local unit and integration tests cover admission preflight, opaque ownership,
stale and wrong-owner refusal, replacement rollback, staged-flow closure,
exact close degradation, ordered multi-flow selection, exact TLS-flow
failover, serialization redaction, and facade startup policy.

The profile is not release-qualified until protected 0.3.9 evidence also
records two independent UAs behind real NAT completing TLS registration,
SDES-SRTP calls in both directions, RFC 4733 DTMF, hold/resume, expiry and
re-registration, NAT rebinding, primary-flow loss and secondary failover,
restart/affinity recovery, clean unregister, and observable failed delivery.
