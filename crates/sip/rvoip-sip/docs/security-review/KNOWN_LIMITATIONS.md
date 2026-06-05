# Known Limitations And Required Tracking

These items are tracked in `../COMPLETE_AUTH_USER_SERVICE_PLAN.md`.

- Outbound UAC auth retry now uses dialog selector-backed transport context,
  but the transport layer does not yet emit post-send selected transport
  telemetry for auth policy decisions.
- PostgreSQL users-core support currently covers `UserStore` and `ApiKeyStore`.
  Authentication-service security-table methods still need a database-pool
  abstraction for refresh-token revocation, access-token revocation, last-login
  updates, password-change updates, and SIP Digest HA1 storage.
- OpenTelemetry/OTLP and vendor/SIEM-specific audit exporters are not yet
  implemented; JSON-lines, tracing, and fanout sinks are available.
- Active Directory-specific compatibility testing is separate from the
  OpenLDAP baseline.
- Generic auth-cache behavior is not defined as an auth-core provider contract.
  Do not add deployment-specific auth caches without a reviewed invalidation
  and revocation model.
- IMS AKA remains provider-backed. The API does not claim built-in SIM/USIM,
  Milenage certification, HSS/UDM integration, or carrier IMS certification.
- Basic exists for legacy compatibility only and should be used over TLS/WSS.
