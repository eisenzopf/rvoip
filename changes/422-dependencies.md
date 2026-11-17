---
section: Changed
breaking: false
---
**axum** 0.7 → 0.8 and **axum-server** 0.8 → 0.9 across users-core, scim,
rvoip-webrtc (WHIP/WHEP signalling), the `rvoip` facade and vapi's test mock.
The ten users-core routes, the two SCIM resource routes, the WHIP/WHEP
resource routes and the browser-demo file route move from `/:id` to `/{id}`;
new tests send every parameterised users-core and SCIM route a request and
fail if any is unmatched. users-core's extractors drop `#[async_trait]`, and
its rate-limit middleware reads `ConnectInfo` from the request extensions
(axum 0.8 no longer extracts `Option<ConnectInfo<_>>`). The workspace
axum-server uses its new `tls-rustls-ring` feature, so it no longer turns on
rustls's aws-lc-rs provider next to the workspace's ring provider.
