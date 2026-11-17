---
section: Changed
breaking: false
---
**ordered-float** 4 → 5, **quick-xml** 0.41 → 0.42, **redis** 0.27 → 1.x,
**cpal** 0.15 → 0.18, **metrics** 0.23 → 0.24 with
**metrics-exporter-prometheus** 0.16 → 0.18 (moq-relay already installed a
0.24 recorder, so rvoip's 0.23 metrics never reached it), and the
**opentelemetry** / **opentelemetry_sdk** / **opentelemetry-otlp** 0.32 → 0.33
group with **tracing-opentelemetry** 0.33 → 0.34 (`rvoip-infra-common`'s
optional `otel` exporter). quick-xml 0.42 events are `str`, so the
registrar's PIDF parser matches names as `str` and reads text with
`xml10_content()`. With redis 1.x the single-node connection manager takes
its retry floor as `set_min_delay` and its timeouts as `Option`s, and rvoip
still sets every timeout explicitly; `cluster_routing::get_slot` is no
longer public, so the live-cluster test computes key slots itself and
checks them against Redis reference values. cpal 0.18 removes
`Device::name()`, so `rvoip-audio-device` and the `sip_client` example name
devices by `description()`. web-transport 0.13 is not in this group: its
raw-QUIC sessions no longer carry a URL, which moq-transport uses as the
session target, so it lands with its own change.
